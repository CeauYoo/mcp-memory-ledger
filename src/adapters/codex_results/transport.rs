//! Bounded MCP calls through a fresh, scoped, revocation-aware core bridge.
//! Neither bridge stderr nor raw requests, responses, or errors are logged.
use crate::adapters::codex_context::protocol::Binding;
use serde_json::{Value, json};
use std::{
    fmt,
    io::{self, BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

const FRAME_LIMIT: usize = 1024 * 1024;
const NOTIFICATION_LIMIT: usize = 32;
const CALL_TIMEOUT: Duration = Duration::from_secs(4);
const PROTOCOL_VERSION: &str = "2025-03-26";

/// Only confirmed rejections may be treated as known failures. An uncertain
/// result does not establish whether a write committed and must not be retried
/// with new identities or refreshed compare-and-swap expectations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallError {
    Rejected,
    Uncertain,
}

impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Rejected => "core bridge request rejected",
            Self::Uncertain => "core bridge outcome unconfirmed",
        })
    }
}

impl std::error::Error for CallError {}

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Opens a new bridge rather than caching authorization or a previous session.
/// The core still checks the bound workspace and current grants on each call.
pub fn call(
    executable: &Path,
    socket: &Path,
    binding: &Binding,
    name: &str,
    arguments: Value,
) -> Result<Value, CallError> {
    call_with_timeout(executable, socket, binding, name, arguments, CALL_TIMEOUT)
}

fn call_with_timeout(
    executable: &Path,
    socket: &Path,
    binding: &Binding,
    name: &str,
    arguments: Value,
    timeout: Duration,
) -> Result<Value, CallError> {
    // Reject unsupported operations and oversized frames before starting a
    // process, so no tool request can have been sent in these cases.
    let request = prepare_request(name, arguments)?;
    let deadline = Instant::now() + timeout;
    let mut process = Process(
        Command::new(executable)
            .args(["bridge", "--socket"])
            .arg(socket)
            .args(["--workspace-id", &binding.workspace_id, "--workspace-root"])
            .arg(&binding.workspace_root)
            .args(["--client-label", "codex-results"])
            .current_dir(&binding.workspace_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| CallError::Uncertain)?,
    );
    let stdout = process.0.stdout.take().ok_or(CallError::Uncertain)?;
    let mut stdin = process.0.stdin.take().ok_or(CallError::Uncertain)?;
    let (tx, rx) = mpsc::sync_channel(1);
    // Both reads AND writes happen off the caller thread. A bridge which stops
    // draining stdin cannot bypass the deadline with a blocked pipe write.
    // Killing and reaping our bridge on every exit closes its pipe endpoints.
    thread::Builder::new()
        .name("codex-results-bridge".into())
        .spawn(move || {
            let result = exchange(&mut BufReader::new(stdout), &mut stdin, &request);
            let _ = tx.send(result);
        })
        .map_err(|_| CallError::Uncertain)?;
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .ok_or(CallError::Uncertain)?;
    rx.recv_timeout(remaining)
        .map_err(|_| CallError::Uncertain)?
}

fn prepare_request(name: &str, arguments: Value) -> Result<Vec<u8>, CallError> {
    if !matches!(
        name,
        "get_task"
            | "get_experience_candidate"
            | "list_experience_uses"
            | "ingest_interaction"
            | "record_experience_use"
    ) || !arguments.is_object()
    {
        return Err(CallError::Rejected);
    }
    encode_frame(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments}
    }))
}

/// Bounds allocation during serialization as well as the eventual pipe frame.
fn encode_frame(value: &Value) -> Result<Vec<u8>, CallError> {
    struct Frame(Vec<u8>);
    impl Write for Frame {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            // Reserve one byte for the mandatory newline.
            if bytes.len() > (FRAME_LIMIT - 1).saturating_sub(self.0.len()) {
                return Err(io::Error::other("frame limit"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut frame = Frame(Vec::new());
    serde_json::to_writer(&mut frame, value).map_err(|_| CallError::Rejected)?;
    frame.0.push(b'\n');
    Ok(frame.0)
}

fn exchange(
    reader: &mut impl BufRead,
    writer: &mut impl Write,
    request: &[u8],
) -> Result<Value, CallError> {
    let initialize = encode_frame(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {"name": "codex-results", "version": "0.1"}
        }
    }))?;
    write_frame(writer, &initialize)?;
    let mut notifications = 0;
    let initialized = read_response(reader, 1, &mut notifications)?;
    if initialized.get("protocolVersion").and_then(Value::as_str) != Some(PROTOCOL_VERSION) {
        return Err(CallError::Uncertain);
    }
    write_frame(
        writer,
        b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
    )?;
    write_frame(writer, request)?;
    let result = read_response(reader, 2, &mut notifications)?;
    match result.get("isError") {
        Some(Value::Bool(true)) => return Err(CallError::Uncertain),
        None | Some(Value::Bool(false)) => {}
        _ => return Err(CallError::Uncertain),
    }
    // Null and arrays are legitimate results (missing task / paginated uses).
    // Missing structuredContent, however, cannot establish a successful write.
    result
        .get("structuredContent")
        .cloned()
        .ok_or(CallError::Uncertain)
}

fn write_frame(writer: &mut impl Write, frame: &[u8]) -> Result<(), CallError> {
    writer
        .write_all(frame)
        .and_then(|()| writer.flush())
        .map_err(|_| CallError::Uncertain)
}

fn read_response(
    reader: &mut impl BufRead,
    expected_id: u64,
    notifications: &mut usize,
) -> Result<Value, CallError> {
    loop {
        let mut bytes = Vec::new();
        (&mut *reader)
            .take((FRAME_LIMIT + 1) as u64)
            .read_until(b'\n', &mut bytes)
            .map_err(|_| CallError::Uncertain)?;
        if bytes.len() > FRAME_LIMIT || bytes.last() != Some(&b'\n') {
            return Err(CallError::Uncertain);
        }
        let response: Value = serde_json::from_slice(&bytes).map_err(|_| CallError::Uncertain)?;
        if response.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Err(CallError::Uncertain);
        }
        if response.get("id").is_none() {
            // Ignore only well-formed notifications, not malformed responses
            // or server requests. The budget spans the entire exchange.
            if response.get("method").and_then(Value::as_str).is_none()
                || response.get("result").is_some()
                || response.get("error").is_some()
                || *notifications >= NOTIFICATION_LIMIT
            {
                return Err(CallError::Uncertain);
            }
            *notifications += 1;
            continue;
        }
        if response.get("id").and_then(Value::as_u64) != Some(expected_id)
            || response.get("method").is_some()
        {
            return Err(CallError::Uncertain);
        }
        match (response.get("result"), response.get("error")) {
            (None, Some(error))
                if error.get("code").and_then(Value::as_i64).is_some()
                    && error.get("message").and_then(Value::as_str).is_some() =>
            {
                // This repository's pinned core maps validation/CAS failures to
                // InvalidParams before commit. An internal/tool failure can follow
                // a durable write, so a generic error must NEVER unlock a new key.
                return Err(if error["code"].as_i64() == Some(-32602) {
                    CallError::Rejected
                } else {
                    CallError::Uncertain
                });
            }
            (Some(result), None) => return Ok(result.clone()),
            _ => return Err(CallError::Uncertain),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn initialized() -> Value {
        json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":PROTOCOL_VERSION}})
    }

    fn tool_result(value: Value) -> Value {
        json!({"jsonrpc":"2.0","id":2,"result":{"structuredContent":value}})
    }

    fn run(responses: &[Value]) -> (Result<Value, CallError>, Vec<Value>) {
        let bytes: Vec<u8> = responses
            .iter()
            .flat_map(|value| encode_frame(value).unwrap())
            .collect();
        let mut written = Vec::new();
        let request = prepare_request("get_task", json!({"namespace":"project/test"})).unwrap();
        let result = exchange(&mut Cursor::new(bytes), &mut written, &request);
        let frames = written
            .split(|byte| *byte == b'\n')
            .filter(|frame| !frame.is_empty())
            .map(|frame| serde_json::from_slice(frame).unwrap())
            .collect();
        (result, frames)
    }

    #[test]
    fn allowlist_and_outgoing_frame_limit_are_enforced_before_spawn() {
        for name in [
            "get_task",
            "get_experience_candidate",
            "list_experience_uses",
            "ingest_interaction",
            "record_experience_use",
        ] {
            assert!(prepare_request(name, json!({})).is_ok());
        }
        for name in [
            "derive_experience_from_use",
            "run_reflection",
            "GET_TASK",
            "",
        ] {
            assert_eq!(prepare_request(name, json!({})), Err(CallError::Rejected));
        }
        assert_eq!(
            prepare_request("get_task", Value::Null),
            Err(CallError::Rejected)
        );
        assert_eq!(
            prepare_request("get_task", json!({"large": "x".repeat(FRAME_LIMIT)})),
            Err(CallError::Rejected)
        );
    }

    #[test]
    fn handshake_and_structured_content_preserve_values_without_text_fallback() {
        for value in [json!({"ok": true}), json!([1, 2]), Value::Null] {
            let (result, written) = run(&[initialized(), tool_result(value.clone())]);
            assert_eq!(result, Ok(value));
            assert_eq!(written.len(), 3);
            assert_eq!(written[0]["method"], "initialize");
            assert_eq!(written[0]["params"]["clientInfo"]["name"], "codex-results");
            assert_eq!(written[1]["method"], "notifications/initialized");
            assert!(written[1].get("id").is_none());
            assert_eq!(written[2]["method"], "tools/call");
            assert_eq!(written[2]["params"]["name"], "get_task");
        }
        assert_eq!(
            run(&[
                initialized(),
                json!({"jsonrpc":"2.0","id":2,"result":{"content":[{"text":"not structured"}]}})
            ])
            .0,
            Err(CallError::Uncertain)
        );
    }

    #[test]
    fn confirmed_rejections_have_no_raw_error_details() {
        let response =
            json!({"jsonrpc":"2.0","id":2,"error":{"code":-32602,"message":"private data"}});
        let error = run(&[initialized(), response]).0.unwrap_err();
        assert_eq!(error, CallError::Rejected);
        assert!(!format!("{error:?} {error}").contains("private"));
        let (result, written) = run(&[
            json!({"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"private data"}}),
        ]);
        assert_eq!(result, Err(CallError::Rejected));
        assert_eq!(written.len(), 1);
    }

    #[test]
    fn generic_error_does_not_prove_write_nonexecution() {
        for response in [
            json!({"jsonrpc":"2.0","id":2,"error":{"code":-32603,"message":"private data"}}),
            json!({"jsonrpc":"2.0","id":2,"error":{"code":-32000,"message":"private data"}}),
            json!({"jsonrpc":"2.0","id":2,"result":{"isError":true,"content":[{"text":"private data"}]}}),
        ] {
            let error = run(&[initialized(), response]).0.unwrap_err();
            assert_eq!(error, CallError::Uncertain);
            assert!(!format!("{error:?} {error}").contains("private"));
        }
    }

    #[test]
    fn malformed_or_uncorrelated_replies_remain_uncertain() {
        for response in [
            json!({"jsonrpc":"2.0","id":3,"result":{}}),
            json!({"jsonrpc":"2.0","id":"2","error":{"code":-1,"message":"failed"}}),
            json!({"jsonrpc":"1.0","id":2,"result":{}}),
            json!({"jsonrpc":"2.0","id":2,"error":{"code":"bad","message":"failed"}}),
            json!({"jsonrpc":"2.0","id":2,"result":{},"error":{"code":-1,"message":"failed"}}),
            json!({"jsonrpc":"2.0","id":2,"result":{"isError":"true","structuredContent":{}}}),
            json!({"jsonrpc":"2.0","result":{}}),
            json!({"jsonrpc":"2.0","id":2,"method":"ping"}),
        ] {
            assert_eq!(run(&[initialized(), response]).0, Err(CallError::Uncertain));
        }
        let (result, written) =
            run(&[json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"unsupported"}})]);
        assert_eq!(result, Err(CallError::Uncertain));
        assert_eq!(written.len(), 1);
    }

    #[test]
    fn notifications_share_a_bounded_budget_across_handshake_and_call() {
        let notification = json!({"jsonrpc":"2.0","method":"notifications/message"});
        let mut responses = vec![notification.clone(); NOTIFICATION_LIMIT - 1];
        responses.push(initialized());
        responses.push(notification.clone());
        responses.push(tool_result(json!(true)));
        assert_eq!(run(&responses).0, Ok(json!(true)));
        responses.insert(responses.len() - 1, notification);
        assert_eq!(run(&responses).0, Err(CallError::Uncertain));
    }

    #[test]
    fn incomplete_oversized_or_invalid_frames_remain_uncertain() {
        for bytes in [
            Vec::new(),
            b"{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{}}".to_vec(),
            b"not json\n".to_vec(),
            vec![b' '; FRAME_LIMIT + 1],
        ] {
            assert_eq!(
                read_response(&mut Cursor::new(bytes), 2, &mut 0),
                Err(CallError::Uncertain)
            );
        }
    }

    #[test]
    fn pipe_write_failure_is_not_a_confirmed_rejection() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::from(io::ErrorKind::BrokenPipe))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        assert_eq!(write_frame(&mut Broken, b"{}\n"), Err(CallError::Uncertain));
    }

    #[cfg(unix)]
    fn fixture(script: &str) -> (tempfile::TempDir, Binding, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("bridge-fixture");
        std::fs::write(&executable, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let binding = Binding {
            host_session_id: "session".into(),
            workspace_id: "workspace".into(),
            namespace: "project/test".into(),
            task_id: "task".into(),
            workspace_root: dir.path().to_path_buf(),
        };
        (dir, binding, executable)
    }

    #[cfg(unix)]
    fn assert_reaped(pid: &str) {
        assert!(
            !Command::new("/bin/sh")
                .args(["-c", "kill -0 \"$1\" 2>/dev/null", "probe", pid.trim()])
                .status()
                .unwrap()
                .success(),
            "the owned bridge process must be reaped"
        );
    }

    #[cfg(unix)]
    #[test]
    fn actual_bridge_receives_scope_and_is_reaped_after_success() {
        let (dir, binding, executable) = fixture(
            r#"
printf '%s\n' "$@" > args
pwd > cwd
printf '%s\n' "$$" > pid
IFS= read -r initialize
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-03-26"}}'
IFS= read -r notification
IFS= read -r request
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"structuredContent":{"ok":true}}}'
while IFS= read -r ignored; do :; done
"#,
        );
        let socket = dir.path().join("core.sock");
        assert_eq!(
            call(&executable, &socket, &binding, "get_task", json!({})),
            Ok(json!({"ok":true}))
        );
        let args = std::fs::read_to_string(dir.path().join("args")).unwrap();
        let expected = [
            "bridge",
            "--socket",
            socket.to_str().unwrap(),
            "--workspace-id",
            "workspace",
            "--workspace-root",
            dir.path().to_str().unwrap(),
            "--client-label",
            "codex-results",
        ];
        assert_eq!(args.lines().collect::<Vec<_>>(), expected);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("cwd"))
                .unwrap()
                .trim(),
            dir.path().to_str().unwrap()
        );
        assert_reaped(&std::fs::read_to_string(dir.path().join("pid")).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn stalled_bridge_times_out_and_is_reaped() {
        let (dir, binding, executable) =
            fixture("printf '%s\\n' \"$$\" > pid\nwhile :; do :; done");
        let start = Instant::now();
        assert_eq!(
            call_with_timeout(
                &executable,
                &dir.path().join("core.sock"),
                &binding,
                "get_task",
                json!({}),
                Duration::from_millis(100)
            ),
            Err(CallError::Uncertain)
        );
        assert!(start.elapsed() < Duration::from_secs(2));
        assert_reaped(&std::fs::read_to_string(dir.path().join("pid")).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn bridge_that_does_not_drain_stdin_cannot_block_past_deadline() {
        let (dir, binding, executable) = fixture(
            r#"
printf '%s\n' "$$" > pid
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-03-26"}}'
while :; do :; done
"#,
        );
        let start = Instant::now();
        assert_eq!(
            call_with_timeout(
                &executable,
                &dir.path().join("core.sock"),
                &binding,
                "ingest_interaction",
                json!({"fixture": "x".repeat(FRAME_LIMIT / 2)}),
                Duration::from_millis(100)
            ),
            Err(CallError::Uncertain)
        );
        assert!(start.elapsed() < Duration::from_secs(2));
        assert_reaped(&std::fs::read_to_string(dir.path().join("pid")).unwrap());
    }
}
