//! Unix socket ownership, exclusion and endpoint lifecycle.
use crate::support::local_file_lock::PrivateLock;
use anyhow::{Context, Result, ensure};
use std::{
    fs, io,
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};
use tokio::net::{UnixListener, UnixStream};

pub(super) struct Listener {
    listener: UnixListener,
    uid: u32,
    _resources: CoreResources,
}
impl Listener {
    pub(super) async fn bind(socket: &Path, registry: &Path, database: &Path) -> Result<Self> {
        let uid = fs::metadata(registry)?.uid();
        validate_private_parent(socket, uid)?;
        let db_meta = fs::metadata(database)?;
        ensure!(
            db_meta.is_file() && db_meta.nlink() == 1 && db_meta.uid() == uid,
            "ledger must be an owned regular file without hard-link aliases"
        );
        let database_lock = PrivateLock::acquire(&sidecar(database, ".local-core.lock"), uid)?;
        let socket_lock = PrivateLock::acquire(&sidecar(socket, ".lock"), uid)?;
        match fs::symlink_metadata(socket) {
            Ok(metadata) => {
                ensure!(
                    metadata.file_type().is_socket() && metadata.uid() == uid,
                    "refusing to replace an unowned or non-socket path"
                );
                match UnixStream::connect(socket).await {
                    Ok(_) => anyhow::bail!("local core socket is already serving"),
                    Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {
                        fs::remove_file(socket)?
                    }
                    Err(_) => anyhow::bail!("cannot verify stale socket; path preserved"),
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let listener = UnixListener::bind(socket)?;
        fs::set_permissions(socket, fs::Permissions::from_mode(0o600))?;
        let socket_metadata = fs::metadata(socket)?;
        let resources = CoreResources {
            socket: socket.to_path_buf(),
            socket_inode: socket_metadata.ino(),
            _socket_lock: socket_lock,
            _database_lock: database_lock,
        };
        Ok(Self {
            listener,
            uid,
            _resources: resources,
        })
    }
    pub(super) async fn accept(&mut self) -> Result<UnixStream> {
        loop {
            let (stream, _) = self.listener.accept().await?;
            if stream
                .peer_cred()
                .map(|cred| cred.uid() == self.uid)
                .unwrap_or(false)
            {
                return Ok(stream);
            }
        }
    }
}
pub(super) async fn connect(socket: &Path) -> Result<UnixStream> {
    let metadata = fs::symlink_metadata(socket)
        .context("core socket is unavailable; start the core explicitly")?;
    ensure!(
        metadata.file_type().is_socket(),
        "core endpoint must be a Unix socket"
    );
    validate_private_parent(socket, metadata.uid())?;
    let stream = UnixStream::connect(socket).await?;
    ensure!(
        stream.peer_cred()?.uid() == metadata.uid(),
        "core peer does not own the socket"
    );
    Ok(stream)
}

fn validate_private_parent(path: &Path, uid: u32) -> Result<()> {
    ensure!(
        path.is_absolute() && path.file_name().is_some(),
        "socket requires an absolute file path"
    );
    let parent = path.parent().context("socket parent is required")?;
    for ancestor in parent.ancestors() {
        let metadata = fs::symlink_metadata(ancestor)?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "socket ancestors must be real directories"
        );
    }
    let metadata = fs::metadata(parent)?;
    ensure!(
        metadata.uid() == uid && metadata.permissions().mode() & 0o077 == 0,
        "socket directory must be owned and private (0700)"
    );
    Ok(())
}

struct CoreResources {
    socket: PathBuf,
    socket_inode: u64,
    _socket_lock: PrivateLock,
    _database_lock: PrivateLock,
}
impl Drop for CoreResources {
    fn drop(&mut self) {
        if let Ok(metadata) = fs::symlink_metadata(&self.socket)
            && metadata.file_type().is_socket()
            && metadata.ino() == self.socket_inode
        {
            let _ = fs::remove_file(&self.socket);
        }
    }
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}
