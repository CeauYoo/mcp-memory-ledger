//! Bounded stdio MCP reads through the existing, revocation-aware core bridge.
use super::protocol::Binding;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

const FRAME_LIMIT: usize = 1024 * 1024;
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub fn read_context(
    executable: &Path,
    socket: &Path,
    binding: &Binding,
    max_bytes: usize,
) -> Result<Value> {
    let mut process = Process(
        Command::new(executable)
            .args(["bridge", "--socket"])
            .arg(socket)
            .args(["--workspace-id", &binding.workspace_id, "--workspace-root"])
            .arg(&binding.workspace_root)
            .args(["--client-label", "codex-context"])
            .current_dir(&binding.workspace_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("core bridge unavailable")?,
    );
    let stdout = process.0.stdout.take().context("bridge stdout missing")?;
    let mut stdin = process.0.stdin.take().context("bridge stdin missing")?;
    let (tx, rx) = mpsc::sync_channel(4);
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut bytes = Vec::new();
            let result = (&mut reader)
                .take((FRAME_LIMIT + 1) as u64)
                .read_until(b'\n', &mut bytes);
            match result {
                Ok(0) => break,
                Ok(_) if bytes.len() <= FRAME_LIMIT => {
                    if tx
                        .send(
                            serde_json::from_slice::<Value>(&bytes)
                                .map_err(|_| "invalid bridge JSON"),
                        )
                        .is_err()
                    {
                        break;
                    }
                }
                _ => {
                    let _ = tx.send(Err("bridge frame exceeds limit or read failed"));
                    break;
                }
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(4);
    {
        let mut request = |id: u64, method: &str, params: Value| -> Result<Value> {
            serde_json::to_writer(
                &mut stdin,
                &json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
            )?;
            stdin.write_all(b"\n")?;
            stdin.flush()?;
            for _ in 0..32 {
                let timeout = deadline
                    .checked_duration_since(Instant::now())
                    .context("core bridge timeout; result unconfirmed")?;
                let response = rx
                    .recv_timeout(timeout)
                    .context("core bridge timeout or disconnect; result unconfirmed")?
                    .map_err(anyhow::Error::msg)?;
                if response.get("id") == Some(&json!(id)) {
                    ensure!(
                        response.get("error").is_none(),
                        "core bridge rejected request"
                    );
                    return response
                        .get("result")
                        .cloned()
                        .context("bridge result missing");
                }
                ensure!(
                    response.get("id").is_none(),
                    "unexpected bridge response id"
                );
            }
            anyhow::bail!("too many bridge notifications")
        };
        let initialized = request(
            1,
            "initialize",
            json!({"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"codex-context","version":"0.1"}}),
        )?;
        ensure!(
            initialized["protocolVersion"].is_string(),
            "invalid MCP initialization"
        );
        // rmcp accepts tools/call after initialization; a separate notification is
        // sent using the same writer after releasing the request closure borrow.
    }
    serde_json::to_writer(
        &mut stdin,
        &json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )?;
    stdin.write_all(b"\n")?;
    serde_json::to_writer(
        &mut stdin,
        &json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"build_task_resume_context","arguments":{"namespace":binding.namespace,"task_id":binding.task_id,"max_bytes":max_bytes}}}),
    )?;
    stdin.write_all(b"\n")?;
    stdin.flush()?;
    for _ in 0..32 {
        let timeout = deadline
            .checked_duration_since(Instant::now())
            .context("core bridge timeout; result unconfirmed")?;
        let response = rx
            .recv_timeout(timeout)
            .context("core bridge timeout or disconnect; result unconfirmed")?
            .map_err(anyhow::Error::msg)?;
        if response.get("id") == Some(&json!(2)) {
            ensure!(
                response.get("error").is_none(),
                "task context request rejected"
            );
            let result = response
                .get("result")
                .context("task context result missing")?;
            ensure!(
                result.get("isError") != Some(&Value::Bool(true)),
                "task context tool failed"
            );
            return result
                .get("structuredContent")
                .cloned()
                .context("structured task context missing");
        }
        ensure!(
            response.get("id").is_none(),
            "unexpected bridge response id"
        );
    }
    anyhow::bail!("too many bridge notifications")
}
