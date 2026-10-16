//! Local-only Windows named pipes behind the same filesystem endpoint CLI.
//! No TCP fallback, server autostart, ambient impersonation, or request replay.
use crate::support::windows_security::{
    PrivateLock, UserSid, acquire_database_lock, sidecar, validate_local_path,
    validate_private_file, validate_private_parent, with_private_security,
};
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    fs, io,
    os::windows::io::AsRawHandle,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};
use windows_sys::Win32::{
    Foundation::ERROR_PIPE_BUSY,
    Storage::FileSystem::SECURITY_ANONYMOUS,
    System::Pipes::{GetNamedPipeClientProcessId, GetNamedPipeServerProcessId},
};

pub(super) type Stream = NamedPipeServer;

pub(super) struct Listener {
    pending: NamedPipeServer,
    name: PathBuf,
    user: UserSid,
    _socket_lock: PrivateLock,
    _database_lock: PrivateLock,
}

impl Listener {
    pub(super) async fn bind(socket: &Path, registry: &Path, database: &Path) -> Result<Self> {
        validate_private_file(registry)?;
        validate_private_parent(socket)?;
        validate_local_path(database)?;
        let database_lock = acquire_database_lock(database, registry)?;
        let socket_lock = PrivateLock::acquire(&sidecar(socket, ".lock"))?;
        let user = UserSid::current()?;
        let name = endpoint_name(socket, &user)?;
        // The first-instance flag fails if any process has already created this
        // pipe name, including an attacker. Never connect/reuse an existing server.
        let pending =
            create_server(&name, true).context("create exclusive local-core named pipe")?;
        Ok(Self {
            pending,
            name,
            user,
            _socket_lock: socket_lock,
            _database_lock: database_lock,
        })
    }

    pub(super) async fn accept(&mut self) -> Result<Stream> {
        loop {
            self.pending.connect().await?;
            // Keep at least one server instance alive throughout each accept and
            // session lifetime, so another process cannot squat between sessions.
            let next = create_server(&self.name, false)?;
            let connected = std::mem::replace(&mut self.pending, next);
            let mut pid = 0;
            // SAFETY: connected pipe handle remains live; pid is writable output.
            let valid =
                unsafe { GetNamedPipeClientProcessId(connected.as_raw_handle(), &mut pid) } != 0;
            if !valid || self.user.verify_process(pid).is_err() {
                tracing::debug!("local bridge peer rejected");
                continue;
            }
            return Ok(connected);
        }
    }
}

fn create_server(name: &Path, first: bool) -> Result<NamedPipeServer> {
    with_private_security(|attributes| {
        // SAFETY: helper keeps the owner-only security descriptor alive for create.
        unsafe {
            ServerOptions::new()
                .first_pipe_instance(first)
                .reject_remote_clients(true)
                .create_with_security_attributes_raw(name, attributes)
        }
    })
}

/// The endpoint argument remains an absolute local filesystem path. Its private
/// parent is validated; no endpoint file is created or deleted. Hashing the user
/// SID and canonical parent makes the actual pipe local, deterministic and bounded.
fn endpoint_name(socket: &Path, user: &UserSid) -> Result<PathBuf> {
    validate_private_parent(socket)?;
    match fs::symlink_metadata(socket) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
        Ok(_) => {
            anyhow::bail!("Windows core endpoint path must be unused; existing file preserved")
        }
    }
    let parent = socket
        .parent()
        .context("endpoint parent required")?
        .canonicalize()?;
    let path = parent.join(socket.file_name().context("endpoint file name required")?);
    let mut hash = Sha256::new();
    hash.update(user.text()?.as_bytes());
    hash.update([0]);
    // Windows drive paths are case insensitive by default. A collision in an
    // explicitly case-sensitive directory rejects concurrent cores conservatively.
    for word in path
        .as_os_str()
        .to_string_lossy()
        .to_lowercase()
        .encode_utf16()
    {
        hash.update(word.to_le_bytes());
    }
    Ok(PathBuf::from(format!(
        r"\\.\pipe\mcp-memory-ledger-{:x}",
        hash.finalize()
    )))
}

pub(super) async fn connect(socket: &Path) -> Result<NamedPipeClient> {
    let user = UserSid::current()?;
    let name = endpoint_name(socket, &user)?;
    // Retry only a busy live endpoint, with a bounded connection establishment
    // wait. A missing pipe fails immediately: bridge never starts a core.
    let client = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match ClientOptions::new()
                .security_qos_flags(SECURITY_ANONYMOUS)
                .open(&name)
            {
                Ok(client) => break Ok(client),
                Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(error) => break Err(error),
            }
        }
    })
    .await
    .context("local core endpoint remained busy")??;
    let mut pid = 0;
    // SAFETY: client owns the live pipe handle; pid is writable output.
    ensure!(
        unsafe { GetNamedPipeServerProcessId(client.as_raw_handle(), &mut pid) } != 0,
        "cannot verify local core server: {}",
        io::Error::last_os_error()
    );
    // This check precedes every workspace binding byte. No pid/username supplied
    // by a peer or by model arguments participates in the identity decision.
    user.verify_process(pid)?;
    Ok(client)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::support::windows_security::{private_create, private_tempdir_in};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn same_user_pipe_roundtrip_and_first_instance_squatting_guard() {
        let root = std::env::temp_dir().canonicalize().unwrap();
        let dir = private_tempdir_in(&root, "ledger-pipe-").unwrap();
        let socket = dir.path().join("core.endpoint");
        let registry = dir.path().join("registry.json");
        let database = dir.path().join("ledger.sqlite");
        drop(private_create(&registry).unwrap());
        drop(private_create(&database).unwrap());
        let mut listener = Listener::bind(&socket, &registry, &database).await.unwrap();
        assert!(create_server(&listener.name, true).is_err());
        assert!(Listener::bind(&socket, &registry, &database).await.is_err());
        let (accepted, connected) = tokio::join!(listener.accept(), connect(&socket));
        let mut server = accepted.unwrap();
        let mut client = connected.unwrap();
        client.write_all(b"ping").await.unwrap();
        let mut data = [0; 4];
        server.read_exact(&mut data).await.unwrap();
        assert_eq!(&data, b"ping");
        drop(server);
        drop(client);
        drop(listener);
        assert!(connect(&socket).await.is_err());
        assert!(!socket.exists());
    }

    #[tokio::test]
    async fn missing_core_never_creates_a_server_or_endpoint_file() {
        let root = std::env::temp_dir().canonicalize().unwrap();
        let dir = private_tempdir_in(&root, "ledger-missing-").unwrap();
        let socket = dir.path().join("absent.endpoint");
        assert!(connect(&socket).await.is_err());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
