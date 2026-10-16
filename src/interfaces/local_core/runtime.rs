//! Shared IPC session and lifecycle. Authorization remains in pure session policy;
//! registry/CLI binding is trusted only against model-supplied tool arguments.
use std::{path::PathBuf, str::FromStr, sync::Arc, time::Duration};

use anyhow::{Context, Result, ensure};
use rmcp::ServiceExt;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteConnectOptions;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::{Semaphore, mpsc, oneshot},
    task::{JoinHandle, JoinSet},
};
use uuid::Uuid;

use super::session::{BoundSession, SessionDispatchGate};
use crate::{
    adapters::sqlite::{SqliteStore, open_current_database},
    interfaces::{
        dashboard::{DashboardObserver, OperationRecorder},
        mcp::server::Server,
    },
    support::{
        config::AppConfig,
        workspace_registry::{WorkspaceRegistry, resolve_workspace},
    },
};

const HANDSHAKE_LIMIT: usize = 8192;
const MAX_SESSIONS: usize = 64;
const MAX_INFLIGHT_CALLS: usize = 32;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    version: u32,
    workspace_id: String,
    workspace_root: PathBuf,
    client_label: String,
}

impl Binding {
    fn validate(&self) -> Result<()> {
        ensure!(self.version == 1, "unsupported binding protocol");
        ensure!(
            Uuid::parse_str(&self.workspace_id)?.to_string() == self.workspace_id,
            "invalid workspace id"
        );
        ensure!(
            self.workspace_root.is_absolute(),
            "workspace root must be absolute"
        );
        ensure!(
            !self.client_label.is_empty()
                && self.client_label.len() <= 64
                && self
                    .client_label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)),
            "invalid client label"
        );
        Ok(())
    }
}

pub struct CoreHandle {
    task: Option<JoinHandle<()>>,
    observer: DashboardObserver,
    store: SqliteStore,
    shutdown: Option<oneshot::Sender<()>>,
    dispatch_gate: Arc<SessionDispatchGate>,
}

impl CoreHandle {
    pub fn observer(&self) -> DashboardObserver {
        self.observer.clone()
    }
    pub fn store(&self) -> SqliteStore {
        self.store.clone()
    }
    pub fn active_requests(&self) -> usize {
        self.dispatch_gate.active_count()
    }
    /// Authorized tool dispatches still executing, including their final audit.
    /// Unlike active_requests, this excludes cancellable preflight handlers.
    pub fn dispatched_requests(&self) -> usize {
        self.dispatch_gate.dispatched_count()
    }
    pub async fn stop(mut self) {
        if let Some(task) = self.task.take() {
            if let Some(shutdown) = self.shutdown.take() {
                let _ = shutdown.send(());
            }
            let _ = task.await;
        }
    }
}

impl Drop for CoreHandle {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        // Dropping a JoinHandle detaches it; the signal drains sessions before
        // releasing the locks and socket. No task lives beyond runtime shutdown.
        self.task.take();
    }
}

/// Starts exactly one cooperating core per canonical database and socket.
/// Existing databases are never initialized/migrated implicitly. The owner must
/// also keep legacy `serve` and administrative DB writers stopped for this DB.
/// No HTTP listener is opened: an owner may explicitly attach a separate gated
/// dashboard through the admin API using this handle's shared observer/store.
pub async fn start_core(
    config: AppConfig,
    socket: PathBuf,
    registry: PathBuf,
) -> Result<CoreHandle> {
    config.validate().map_err(anyhow::Error::msg)?;
    ensure!(
        !config.dashboard.enabled,
        "core requires dashboard.enabled=false; use explicit owner-gated monitoring"
    );
    ensure!(!config.daemon.enabled, "core requires daemon.enabled=false");
    WorkspaceRegistry::load(&registry)?;
    let options = SqliteConnectOptions::from_str(&config.database_url)?;
    let database = options
        .get_filename()
        .canonicalize()
        .context("core requires an existing file-backed ledger")?;
    let mut listener = super::platform::Listener::bind(&socket, &registry, &database).await?;
    let store = open_current_database(&config.database_url).await?;
    let observer =
        DashboardObserver::enabled(OperationRecorder::new(config.dashboard.event_capacity));
    let server = Server::from_parts(config, store.clone(), DashboardObserver::disabled())
        .await?
        .restricted();
    let session_store = store.clone();
    let session_observer = observer.clone();
    let request_limit = Arc::new(Semaphore::new(MAX_INFLIGHT_CALLS));
    let dispatch_gate = Arc::new(SessionDispatchGate::new());
    let handle_gate = dispatch_gate.clone();
    let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
    let task = tokio::spawn(async move {
        let mut sessions = JoinSet::new();
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                accepted = listener.accept() => {
                    let Ok(stream) = accepted else { break; };
                    if sessions.len() >= MAX_SESSIONS { continue; }
                    let (server, store, observer, registry) = (server.clone(), session_store.clone(), session_observer.clone(), registry.clone());
                    let request_limit = request_limit.clone();
                    let dispatch_gate = dispatch_gate.clone();
                    sessions.spawn(async move {
                        if accept_session(stream, server, store, observer, registry, request_limit, dispatch_gate).await.is_err() {
                            tracing::debug!("local bridge session closed or rejected");
                        }
                    });
                }
                _ = sessions.join_next(), if !sessions.is_empty() => {}
            }
        }
        sessions.abort_all();
        while sessions.join_next().await.is_some() {}
        // rmcp 0.5 spawns handler futures separately from its connection loop.
        // Retain the database lock until every admitted tool/audit has finished.
        dispatch_gate.close_and_wait().await;
        tracing::info!("local core stopped; all bridge sessions closed");
    });
    Ok(CoreHandle {
        task: Some(task),
        observer,
        store,
        shutdown: Some(shutdown_tx),
        dispatch_gate: handle_gate,
    })
}

async fn accept_session(
    mut stream: impl AsyncRead + AsyncWrite + Unpin,
    server: Server,
    store: SqliteStore,
    observer: DashboardObserver,
    registry: PathBuf,
    request_limit: Arc<Semaphore>,
    dispatch_gate: Arc<SessionDispatchGate>,
) -> Result<()> {
    let binding = tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
        let binding: Binding = serde_json::from_slice(&read_line_bounded(&mut stream).await?)?;
        binding.validate()?;
        let grant = resolve_workspace(&registry, &binding.workspace_root)?;
        ensure!(
            grant.workspace_id == binding.workspace_id,
            "workspace binding mismatch"
        );
        Ok::<_, anyhow::Error>((binding, grant))
    })
    .await;
    let (binding, grant) = match binding {
        Ok(Ok(binding)) => binding,
        _ => {
            let _ = stream.write_all(b"{\"accepted\":false}\n").await;
            anyhow::bail!("binding rejected");
        }
    };
    stream.write_all(b"{\"accepted\":true}\n").await?;
    let server = BoundSession::new(
        server,
        store,
        observer,
        registry,
        grant,
        binding.workspace_root,
        binding.client_label,
    )
    .with_request_limit(request_limit)
    .with_dispatch_gate(dispatch_gate);
    let (relay_io, server_io) = tokio::io::duplex(64 * 1024);
    let serving = async {
        let service = tokio::time::timeout(HANDSHAKE_TIMEOUT, server.serve(server_io)).await??;
        service.waiting().await?;
        Ok::<_, anyhow::Error>(())
    };
    tokio::select! {
        result = super::framing::relay(stream, relay_io) => result,
        result = serving => result,
    }
}

pub(super) async fn run_bridge(
    socket: PathBuf,
    workspace_id: String,
    workspace_root: PathBuf,
    client_label: String,
) -> Result<()> {
    let binding = Binding {
        version: 1,
        workspace_id,
        workspace_root,
        client_label,
    };
    binding.validate()?;
    let mut stream = super::platform::connect(&socket).await?;
    let mut bytes = serde_json::to_vec(&binding)?;
    ensure!(bytes.len() < HANDSHAKE_LIMIT, "binding exceeds limit");
    bytes.push(b'\n');
    stream.write_all(&bytes).await?;
    let reply = tokio::time::timeout(HANDSHAKE_TIMEOUT, read_line_bounded(&mut stream)).await??;
    ensure!(
        reply == b"{\"accepted\":true}\n",
        "core rejected workspace binding"
    );
    let (mut reader, mut writer) = tokio::io::split(stream);
    // Tokio's stdin uses a blocking-pool read that cannot be cancelled when the
    // core disconnects. A detached OS reader thread lets the bridge process exit
    // promptly even while its client's stdin is still open and idle.
    let (input_tx, mut input_rx) = mpsc::channel::<Vec<u8>>(4);
    std::thread::spawn(move || {
        use std::io::Read;
        let mut input = std::io::stdin().lock();
        let mut bytes = [0u8; 8192];
        loop {
            match input.read(&mut bytes) {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    if input_tx.blocking_send(bytes[..count].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    let mut output = tokio::io::stdout();
    // No retry/replay after disconnect: the caller owns request IDs and decides
    // whether to reconnect. EOF closes only this connection, never the core.
    tokio::select! {
        result = async {
            while let Some(bytes) = input_rx.recv().await { writer.write_all(&bytes).await?; }
            writer.shutdown().await
        } => { result?; }
        result = async {
            tokio::io::copy(&mut reader, &mut output).await?;
            output.flush().await
        } => { result?; }
    }
    Ok(())
}

async fn read_line_bounded(reader: &mut (impl AsyncRead + Unpin)) -> Result<Vec<u8>> {
    let mut line = Vec::new();
    loop {
        ensure!(line.len() < HANDSHAKE_LIMIT, "binding exceeds limit");
        let byte = reader.read_u8().await?;
        line.push(byte);
        if byte == b'\n' {
            return Ok(line);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn binding_line_limit_and_unknown_fields_fail_closed() {
        let input = vec![b'x'; HANDSHAKE_LIMIT + 1];
        assert!(read_line_bounded(&mut input.as_slice()).await.is_err());
        let binding = serde_json::json!({"version":1,"workspace_id":Uuid::new_v4().to_string(),"workspace_root":"/tmp","client_label":"codex","namespace":"user/default"});
        assert!(serde_json::from_value::<Binding>(binding).is_err());
    }
}
