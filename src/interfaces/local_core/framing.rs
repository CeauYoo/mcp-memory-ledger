//! Bounded JSON-RPC framing between a bridge socket and the MCP service.
//! Requests are never retried, split, truncated, or forwarded before validation.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result, ensure};
use serde_json::{Map, Value};
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, DuplexStream,
};

const INBOUND_LIMIT: usize = 1024 * 1024;
const OUTBOUND_LIMIT: usize = 4 * 1024 * 1024;
const MAX_OUTSTANDING: usize = 8;
const READ_BUFFER: usize = 4096;
type Pending = Arc<Mutex<HashMap<String, bool>>>;

/// Both directions apply bounded buffering. Ending either direction drops the
/// other future and its halves, closing the session instead of replaying writes.
pub(super) async fn relay(
    socket: impl AsyncRead + AsyncWrite + Unpin,
    transport: DuplexStream,
) -> Result<()> {
    let (socket_read, socket_write) = tokio::io::split(socket);
    let (transport_read, transport_write) = tokio::io::split(transport);
    let pending = Arc::new(Mutex::new(HashMap::with_capacity(MAX_OUTSTANDING)));
    tokio::select! {
        result = requests(socket_read, transport_write, pending.clone()) => result,
        result = responses(transport_read, socket_write, pending) => result,
    }
}

async fn requests(
    reader: impl AsyncRead + Unpin,
    mut writer: impl AsyncWrite + Unpin,
    pending: Pending,
) -> Result<()> {
    let mut reader = BufReader::with_capacity(READ_BUFFER, reader);
    let mut initialized = false;
    while let Some(frame) = read_frame(&mut reader, INBOUND_LIMIT).await? {
        let object = rpc_object(&frame)?;
        ensure!(
            object.get("method").is_some_and(Value::is_string),
            "inbound RPC must be a request or notification"
        );
        ensure!(
            !object.contains_key("result") && !object.contains_key("error"),
            "inbound responses are unsupported"
        );
        match object.get("id") {
            Some(id) => {
                let id = request_id(id)?;
                let mut pending = pending
                    .lock()
                    .map_err(|_| anyhow::anyhow!("session state unavailable"))?;
                ensure!(
                    !pending.contains_key(&id),
                    "duplicate outstanding request id"
                );
                ensure!(
                    pending.len() < MAX_OUTSTANDING,
                    "too many outstanding requests"
                );
                pending.insert(id, false);
            }
            None => {
                validate_notification(&object)?;
                // rmcp spawns notification handlers. Forward only one init and
                // one cancellation per live request; roots/progress/unknown
                // notifications cannot alter a fixed host binding and are dropped.
                match object.get("method").and_then(Value::as_str) {
                    Some("notifications/initialized") if !initialized => initialized = true,
                    Some("notifications/cancelled") => {
                        let id = request_id(
                            object
                                .get("params")
                                .and_then(|params| params.get("requestId"))
                                .context("cancellation requires requestId")?,
                        )?;
                        let mut pending = pending
                            .lock()
                            .map_err(|_| anyhow::anyhow!("session state unavailable"))?;
                        match pending.get_mut(&id) {
                            Some(cancelled) if !*cancelled => *cancelled = true,
                            _ => continue,
                        }
                    }
                    _ => continue,
                }
            }
        }
        writer
            .write_all(&frame)
            .await
            .context("forward request frame")?;
        writer.flush().await?;
    }
    Ok(())
}

async fn responses(
    reader: impl AsyncRead + Unpin,
    mut writer: impl AsyncWrite + Unpin,
    pending: Pending,
) -> Result<()> {
    let mut reader = BufReader::with_capacity(READ_BUFFER, reader);
    while let Some(frame) = read_frame(&mut reader, OUTBOUND_LIMIT).await? {
        let object = rpc_object(&frame)?;
        let completed = match object.get("id") {
            Some(id) => {
                ensure!(
                    !object.contains_key("method"),
                    "server requests are unsupported"
                );
                ensure!(
                    object.contains_key("result") != object.contains_key("error"),
                    "outbound RPC must be a response"
                );
                let id = request_id(id)?;
                ensure!(
                    pending
                        .lock()
                        .map_err(|_| anyhow::anyhow!("session state unavailable"))?
                        .contains_key(&id),
                    "response id is not outstanding"
                );
                Some(id)
            }
            None => {
                validate_notification(&object)?;
                ensure!(
                    !object.contains_key("result") && !object.contains_key("error"),
                    "outbound notification cannot contain a response"
                );
                None
            }
        };
        writer
            .write_all(&frame)
            .await
            .context("forward response frame")?;
        writer.flush().await?;
        // Releasing only after delivery applies socket backpressure to the
        // request budget. A non-reading client cannot keep filling MCP queues.
        if let Some(id) = completed {
            pending
                .lock()
                .map_err(|_| anyhow::anyhow!("session state unavailable"))?
                .remove(&id);
        }
    }
    Ok(())
}

fn rpc_object(frame: &[u8]) -> Result<Map<String, Value>> {
    let value: Value = serde_json::from_slice(frame).context("invalid JSON-RPC frame")?;
    let Value::Object(object) = value else {
        anyhow::bail!("JSON-RPC batches and non-object frames are unsupported")
    };
    ensure!(
        object.get("jsonrpc").and_then(Value::as_str) == Some("2.0"),
        "unsupported JSON-RPC version"
    );
    Ok(object)
}

fn request_id(id: &Value) -> Result<String> {
    // rmcp 0.5 uses a u32 NumberOrString and casts JSON u64 values. Reject
    // values it would truncate (or reject) before they can alias a pending ID.
    ensure!(
        id.is_string()
            || id
                .as_u64()
                .is_some_and(|number| number <= u64::from(u32::MAX)),
        "request id must be a string or an unsigned 32-bit JSON integer"
    );
    Ok(serde_json::to_string(id)?)
}

fn validate_notification(object: &Map<String, Value>) -> Result<()> {
    ensure!(
        object
            .get("method")
            .and_then(Value::as_str)
            .is_some_and(|method| method.starts_with("notifications/")
                && method.len() > "notifications/".len()),
        "a message without an id must be a notification"
    );
    Ok(())
}

/// The limit includes the terminating newline. Never forwards a partial frame;
/// EOF with an incomplete line is a protocol error. At most one 4 KiB read-ahead
/// buffer is retained in addition to the bounded frame.
async fn read_frame(
    reader: &mut (impl AsyncBufRead + Unpin),
    limit: usize,
) -> Result<Option<Vec<u8>>> {
    let mut frame = Vec::with_capacity(READ_BUFFER.min(limit));
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            ensure!(frame.is_empty(), "incomplete JSON-RPC frame");
            return Ok(None);
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let count = newline.map_or(available.len(), |index| index + 1);
        ensure!(
            count <= limit.saturating_sub(frame.len()),
            "JSON-RPC frame exceeds limit"
        );
        frame.extend_from_slice(&available[..count]);
        reader.consume(count);
        if newline.is_some() {
            return Ok(Some(frame));
        }
        ensure!(frame.len() < limit, "JSON-RPC frame exceeds limit");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::{
        io::AsyncReadExt,
        task::JoinHandle,
        time::{Duration, timeout},
    };

    struct Fixture {
        client: DuplexStream,
        backend: DuplexStream,
        task: JoinHandle<Result<()>>,
    }

    impl Fixture {
        fn new() -> Self {
            let (client, socket) = tokio::io::duplex(64 * 1024);
            let (transport, backend) = tokio::io::duplex(64 * 1024);
            let task = tokio::spawn(relay(socket, transport));
            Self {
                client,
                backend,
                task,
            }
        }

        async fn rejects(mut self, input: Vec<u8>) -> Vec<u8> {
            let _ = timeout(Duration::from_secs(3), self.client.write_all(&input))
                .await
                .unwrap();
            assert!(
                timeout(Duration::from_secs(3), self.task)
                    .await
                    .unwrap()
                    .unwrap()
                    .is_err()
            );
            let mut forwarded = Vec::new();
            timeout(
                Duration::from_secs(3),
                self.backend.read_to_end(&mut forwarded),
            )
            .await
            .unwrap()
            .unwrap();
            forwarded
        }
    }

    fn frame(value: Value) -> Vec<u8> {
        let mut bytes = serde_json::to_vec(&value).unwrap();
        bytes.push(b'\n');
        bytes
    }
    fn request(id: Value) -> Vec<u8> {
        frame(json!({"jsonrpc":"2.0", "id":id, "method":"ping"}))
    }

    #[tokio::test]
    async fn ninth_outstanding_request_is_rejected_before_forwarding() {
        let input = (0..9).flat_map(|id| request(json!(id))).collect();
        let forwarded = Fixture::new().rejects(input).await;
        let messages: Vec<Value> = forwarded
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).unwrap())
            .collect();
        assert_eq!(messages.len(), 8);
        assert_eq!(messages.last().unwrap()["id"], 7);
    }

    #[tokio::test]
    async fn batch_and_idless_calls_are_never_forwarded() {
        for value in [
            json!([{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{}}]),
            json!({"jsonrpc":"2.0","method":"tools/call","params":{}}),
            json!({"jsonrpc":"2.0","id":null,"method":"tools/call"}),
            json!({"jsonrpc":"2.0","id":true,"method":"ping"}),
            json!({"jsonrpc":"2.0","id":1,"result":{}}),
        ] {
            assert!(Fixture::new().rejects(frame(value)).await.is_empty());
        }
    }

    #[tokio::test]
    async fn duplicate_id_is_rejected_before_second_request_is_forwarded() {
        let first = request(json!("same-id"));
        let mut input = first.clone();
        input.extend(&first);
        assert_eq!(Fixture::new().rejects(input).await, first);
    }

    #[tokio::test]
    async fn numeric_ids_cannot_truncate_or_coerce_into_another_outstanding_request() {
        let first = request(json!(1));
        let mut input = first.clone();
        input.extend(request(json!(4_294_967_297u64)));
        assert_eq!(Fixture::new().rejects(input).await, first);
        for id in [json!(-1), json!(1.0), json!(1e20), json!(u64::MAX)] {
            assert!(Fixture::new().rejects(request(id)).await.is_empty());
        }
        assert!(request_id(&json!(u32::MAX)).is_ok());
        assert!(request_id(&json!("4294967297")).is_ok());
    }

    #[tokio::test]
    async fn notification_flood_is_filtered_and_cancel_ids_cannot_alias() {
        let init = frame(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
        let req = request(json!(1));
        let cancel = frame(
            json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}}),
        );
        let mut input = init.clone();
        input.extend(&req);
        input.extend(&cancel);
        for _ in 0..100 {
            input.extend(&init);
            input.extend(&cancel);
            input.extend(frame(
                json!({"jsonrpc":"2.0","method":"notifications/roots/list_changed"}),
            ));
            input.extend(frame(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":"absent"}})));
        }
        // Reject overflow before rmcp can cast it to request 1.
        input.extend(frame(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":4_294_967_297u64}})));
        let mut expected = init;
        expected.extend(req);
        expected.extend(cancel);
        assert_eq!(Fixture::new().rejects(input).await, expected);
    }

    #[tokio::test]
    async fn oversized_inbound_frame_is_rejected_without_partial_forwarding() {
        let input = frame(
            json!({"jsonrpc":"2.0", "id":1, "method":"ping", "params":{"synthetic":"x".repeat(INBOUND_LIMIT)}}),
        );
        assert!(Fixture::new().rejects(input).await.is_empty());
    }

    #[tokio::test]
    async fn valid_notifications_and_delivered_responses_release_the_request_budget() {
        let Fixture {
            mut client,
            mut backend,
            task,
        } = Fixture::new();
        let notification = frame(json!({"jsonrpc":"2.0", "method":"notifications/initialized"}));
        client.write_all(&notification).await.unwrap();
        let mut forwarded = vec![0; notification.len()];
        backend.read_exact(&mut forwarded).await.unwrap();
        assert_eq!(forwarded, notification);
        for _ in 0..10 {
            let request = request(json!("reusable"));
            client.write_all(&request).await.unwrap();
            let mut forwarded = vec![0; request.len()];
            timeout(Duration::from_secs(3), backend.read_exact(&mut forwarded))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(forwarded, request);
            let response = frame(json!({"jsonrpc":"2.0", "id":"reusable", "result":{}}));
            backend.write_all(&response).await.unwrap();
            let mut received = vec![0; response.len()];
            timeout(Duration::from_secs(3), client.read_exact(&mut received))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(received, response);
        }
        drop(client);
        assert!(
            timeout(Duration::from_secs(3), task)
                .await
                .unwrap()
                .unwrap()
                .is_ok()
        );
    }

    #[tokio::test]
    async fn outbound_frames_must_be_bounded_responses_to_pending_requests() {
        for response in [
            frame(json!({"jsonrpc":"2.0", "id":999, "result":{}})),
            frame(json!({"jsonrpc":"2.0", "id":1, "method":"roots/list"})),
            frame(json!({"jsonrpc":"2.0", "id":1, "result":"x".repeat(OUTBOUND_LIMIT)})),
        ] {
            let Fixture {
                mut client,
                mut backend,
                task,
            } = Fixture::new();
            let request = request(json!(1));
            client.write_all(&request).await.unwrap();
            let mut forwarded = vec![0; request.len()];
            backend.read_exact(&mut forwarded).await.unwrap();
            let _ = timeout(Duration::from_secs(3), backend.write_all(&response))
                .await
                .unwrap();
            assert!(
                timeout(Duration::from_secs(3), task)
                    .await
                    .unwrap()
                    .unwrap()
                    .is_err()
            );
            let mut received = Vec::new();
            client.read_to_end(&mut received).await.unwrap();
            assert!(received.is_empty());
        }
    }

    #[tokio::test]
    async fn read_frame_rejects_incomplete_lines_and_accepts_exact_limit() {
        let (mut writer, reader) = tokio::io::duplex(16);
        writer.write_all(b"abc\n").await.unwrap();
        let mut reader = BufReader::with_capacity(2, reader);
        assert_eq!(
            read_frame(&mut reader, 4).await.unwrap(),
            Some(b"abc\n".to_vec())
        );
        writer.write_all(b"partial").await.unwrap();
        drop(writer);
        assert!(read_frame(&mut reader, 16).await.is_err());
    }

    #[tokio::test]
    async fn request_budget_remains_full_until_response_writer_flushes() {
        // Exercise delivery accounting with a deterministically blocked writer,
        // rather than assuming a particular kernel Unix-socket buffer size.
        let pending = Arc::new(Mutex::new(HashMap::from([("1".to_string(), false)])));
        let (mut backend, transport) = tokio::io::duplex(1024);
        let (writer, mut client) = tokio::io::duplex(1);
        let pending_for_relay = pending.clone();
        let task = tokio::spawn(responses(transport, writer, pending_for_relay));
        let response = frame(json!({"jsonrpc":"2.0", "id":1, "result":{}}));
        backend.write_all(&response).await.unwrap();
        let mut first = [0u8; 1];
        client.read_exact(&mut first).await.unwrap();
        assert!(pending.lock().unwrap().contains_key("1"));
        let mut rest = vec![0; response.len() - 1];
        client.read_exact(&mut rest).await.unwrap();
        timeout(Duration::from_secs(3), async {
            while !pending.lock().unwrap().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        drop(backend);
        assert!(
            timeout(Duration::from_secs(3), task)
                .await
                .unwrap()
                .unwrap()
                .is_ok()
        );
    }
}
