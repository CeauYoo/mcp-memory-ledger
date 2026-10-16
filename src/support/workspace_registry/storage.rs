use std::{
    fs::{self, File, Metadata},
    io::{Read, Write},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
#[cfg(not(windows))]
use std::fs::OpenOptions;
use uuid::Uuid;

use super::{
    SessionGrant, WorkspaceRegistration, WorkspaceRegistry,
    model::{MAX_REGISTRY_BYTES, validate_absolute_path},
};

/// Register an existing directory. Canonical aliases reuse the original identity.
pub fn register_workspace(
    path: impl AsRef<Path>,
    root: impl AsRef<Path>,
) -> Result<WorkspaceRegistration> {
    let root = canonical_root(root.as_ref())?;
    update(path.as_ref(), true, |registry| {
        registry.register_canonical(root, Uuid::new_v4().to_string())
    })
}

/// Explicitly add another directory to an existing stable identity.
pub fn bind_workspace_root(
    path: impl AsRef<Path>,
    id: impl AsRef<str>,
    root: impl AsRef<Path>,
) -> Result<WorkspaceRegistration> {
    let root = canonical_root(root.as_ref())?;
    update(path.as_ref(), false, |registry| {
        registry.bind_canonical(id.as_ref(), root)
    })
}

/// Revoke the exact registered absolute path, even when the old directory is gone.
/// Repeating an unbind is harmless. Zero roots makes the workspace unavailable.
pub fn unbind_workspace_root(
    path: impl AsRef<Path>,
    id: impl AsRef<str>,
    root: impl AsRef<Path>,
) -> Result<WorkspaceRegistration> {
    update(path.as_ref(), false, |registry| {
        registry.unbind_exact(id.as_ref(), root.as_ref())
    })
}

/// Preserve identity, grants and ledger data. Enabling needs a live registered root.
/// Binding a new root never implicitly enables a disabled workspace.
pub fn set_workspace_active(
    path: impl AsRef<Path>,
    id: impl AsRef<str>,
    active: bool,
) -> Result<WorkspaceRegistration> {
    update(path.as_ref(), false, |registry| {
        if active {
            let workspace = registry
                .workspaces
                .iter()
                .find(|workspace| workspace.id == id.as_ref())
                .ok_or_else(|| anyhow::anyhow!("workspace id is not registered"))?;
            ensure!(
                workspace
                    .roots
                    .iter()
                    .any(|root| canonical_root(root).is_ok_and(|canonical| canonical == *root)),
                "enabling workspace requires an existing registered root"
            );
        }
        registry.set_active(id.as_ref(), active)
    })
}

/// Both permissions false revokes the grant. Writes always require reads.
pub fn set_personal_grant(
    path: impl AsRef<Path>,
    id: impl AsRef<str>,
    namespace: impl AsRef<str>,
    read: bool,
    write: bool,
) -> Result<WorkspaceRegistration> {
    update(path.as_ref(), false, |registry| {
        registry.set_grant(id.as_ref(), namespace.as_ref(), read, write)
    })
}

/// Resolve exactly one explicitly registered canonical root, with no ancestor or
/// default-namespace fallback. The root must still exist as a directory.
pub fn resolve_workspace(path: impl AsRef<Path>, root: impl AsRef<Path>) -> Result<SessionGrant> {
    let root = canonical_root(root.as_ref())?;
    let registry = load(path.as_ref())?;
    let workspace = registry
        .workspaces
        .iter()
        .find(|workspace| workspace.roots.contains(&root))
        .ok_or_else(|| anyhow::anyhow!("workspace root is not registered; bind it explicitly"))?;
    registry.session_for_id(&workspace.id)
}

/// Reload on each request, so a previously issued session cannot retain a revoked
/// personal grant merely by keeping its old in-memory copy.
pub fn resolve_workspace_id(path: impl AsRef<Path>, id: impl AsRef<str>) -> Result<SessionGrant> {
    load(path.as_ref())?.session_for_id(id.as_ref())
}

pub(crate) fn load(path: &Path) -> Result<WorkspaceRegistry> {
    validate_registry_path(path)?;
    #[cfg(windows)]
    let before_identity = crate::support::windows_security::path_identity(path)?;
    let before = registry_metadata(path)?;
    ensure!(
        before.len() <= MAX_REGISTRY_BYTES,
        "workspace registry exceeds size limit"
    );
    #[cfg(not(windows))]
    let file = File::open(path).context("open workspace registry")?;
    #[cfg(windows)]
    let file = crate::support::windows_security::open_private_file(path)?;
    #[cfg(not(windows))]
    {
        ensure_same_file(&before, &file.metadata()?)?;
        ensure_same_file(&before, &registry_metadata(path)?)?;
    }
    #[cfg(windows)]
    ensure!(
        before_identity == crate::support::windows_security::file_identity(&file)?
            && before_identity == crate::support::windows_security::path_identity(path)?,
        "registry changed while opening; retry explicitly"
    );
    let mut data = Vec::new();
    file.take(MAX_REGISTRY_BYTES + 1).read_to_end(&mut data)?;
    ensure!(
        data.len() as u64 <= MAX_REGISTRY_BYTES,
        "workspace registry exceeds size limit"
    );
    let registry: WorkspaceRegistry = serde_json::from_slice(&data)
        .context("invalid workspace registry JSON; original file preserved")?;
    registry.validate()?;
    Ok(registry)
}

pub(super) fn update<F>(path: &Path, create: bool, change: F) -> Result<WorkspaceRegistration>
where
    F: FnOnce(&mut WorkspaceRegistry) -> Result<WorkspaceRegistration>,
{
    validate_registry_path(path)?;
    let _lock = RegistryLock::acquire(path)?;
    let mut registry = match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && create => {
            WorkspaceRegistry::default()
        }
        _ => load(path)?,
    };
    let before = registry.clone();
    let result = change(&mut registry)?;
    if before != registry {
        atomic_replace(path, &registry, || Ok(()))?;
    }
    Ok(result)
}

pub(super) fn canonical_root(root: &Path) -> Result<PathBuf> {
    // Relative roots are ambiguous across callers; canonical aliases of an
    // absolute directory are deliberately supported.
    let text = root
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("workspace paths must be UTF-8"))?;
    ensure!(
        root.is_absolute() && text.len() <= 4096 && !text.chars().any(char::is_control),
        "workspace root must be an absolute UTF-8 path"
    );
    let root = root.canonicalize().context("workspace root must exist")?;
    validate_absolute_path(&root)?;
    ensure!(root.is_dir(), "workspace root must be a directory");
    Ok(root)
}

pub(super) fn validate_registry_path(path: &Path) -> Result<()> {
    validate_absolute_path(path)?;
    #[cfg(windows)]
    crate::support::windows_security::validate_private_parent(path)?;
    ensure!(
        path.file_name().is_some(),
        "registry path requires a file name"
    );
    let parent = path
        .parent()
        .context("registry path requires a parent directory")?;
    for component in parent.ancestors() {
        let metadata = fs::symlink_metadata(component).context("registry parent must exist")?;
        ensure!(
            !metadata.file_type().is_symlink() && metadata.is_dir(),
            "registry parent must be a real directory without symlinks"
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            fs::metadata(parent)?.permissions().mode() & 0o022 == 0,
            "registry parent must not be writable by group or others"
        );
    }
    match fs::symlink_metadata(path) {
        Ok(_) => {
            registry_metadata(path)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn registry_metadata(path: &Path) -> Result<Metadata> {
    #[cfg(windows)]
    crate::support::windows_security::validate_private_file(path)?;
    let metadata = fs::symlink_metadata(path).context("workspace registry is unavailable")?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "registry must be a regular non-symlink file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        ensure!(
            metadata.permissions().mode() & 0o077 == 0,
            "registry must have private Unix permissions (0600)"
        );
        ensure!(
            metadata.nlink() == 1,
            "registry must not have hard-link aliases"
        );
    }
    Ok(metadata)
}

#[cfg(not(windows))]
fn ensure_same_file(before: &Metadata, after: &Metadata) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            before.dev() == after.dev() && before.ino() == after.ino(),
            "registry changed while opening; retry explicitly"
        );
    }
    #[cfg(not(unix))]
    {
        ensure!(
            before.len() == after.len() && before.modified()? == after.modified()?,
            "registry changed while opening; retry explicitly"
        );
    }
    Ok(())
}

#[cfg(windows)]
fn private_create(path: &Path) -> Result<File> {
    crate::support::windows_security::private_create(path)
}

#[cfg(not(windows))]
fn private_create(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}

struct RegistryLock {
    path: PathBuf,
    file: Option<File>,
    #[cfg(not(windows))]
    metadata: Metadata,
    #[cfg(windows)]
    identity: crate::support::windows_security::FileIdentity,
    #[cfg(windows)]
    _parent: crate::support::windows_security::DirectoryGuard,
}

impl RegistryLock {
    fn acquire(registry_path: &Path) -> Result<Self> {
        #[cfg(windows)]
        let parent = crate::support::windows_security::lock_private_parent(registry_path)?;
        let path = sibling(registry_path, ".lock");
        let started = Instant::now();
        #[cfg(windows)]
        let mut unexplained_denial_since = None;
        loop {
            match private_create(&path) {
                Ok(file) => {
                    #[cfg(not(windows))]
                    let metadata = file.metadata()?;
                    #[cfg(windows)]
                    let identity = crate::support::windows_security::file_identity(&file)?;
                    return Ok(Self {
                        path,
                        file: Some(file),
                        #[cfg(not(windows))]
                        metadata,
                        #[cfg(windows)]
                        identity,
                        #[cfg(windows)]
                        _parent: parent,
                    });
                }
                Err(error)
                    if error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|error| error.kind() == std::io::ErrorKind::AlreadyExists) =>
                {
                    #[cfg(windows)]
                    {
                        unexplained_denial_since = None;
                    }
                    // Never delete an unknown lock, even if its owner appears gone.
                    if started.elapsed() >= Duration::from_secs(2) {
                        bail!(
                            "workspace registry lock is busy or stale; inspect it before an explicit retry: {}",
                            path.display()
                        );
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => {
                    #[cfg(windows)]
                    if let Some(io_error) = error.downcast_ref::<std::io::Error>() {
                        // Windows can return ACCESS_DENIED while the previous lock
                        // is delete-pending, or sharing/lock violations on cleanup.
                        // A real ACL denial must remain a denial, not a stale-lock error.
                        let known_lock = fs::symlink_metadata(&path).is_ok_and(|metadata| {
                            metadata.is_file() && !metadata.file_type().is_symlink()
                        });
                        let unconfirmed_elapsed = if known_lock {
                            unexplained_denial_since = None;
                            Duration::ZERO
                        } else {
                            unexplained_denial_since
                                .get_or_insert_with(Instant::now)
                                .elapsed()
                        };
                        if windows_lock_retry(
                            io_error.raw_os_error(),
                            known_lock,
                            started.elapsed(),
                            unconfirmed_elapsed,
                        ) {
                            thread::sleep(Duration::from_millis(10));
                            continue;
                        }
                    }
                    return Err(error.context("create workspace registry lock"));
                }
            }
        }
    }
}

#[cfg(any(windows, test))]
fn windows_lock_retry(
    code: Option<i32>,
    known_lock: bool,
    elapsed: Duration,
    unconfirmed_elapsed: Duration,
) -> bool {
    matches!(code, Some(5 | 32 | 33))
        && elapsed < Duration::from_secs(2)
        && (known_lock || unconfirmed_elapsed < Duration::from_millis(100))
}

impl Drop for RegistryLock {
    fn drop(&mut self) {
        // Close first for Windows, and only clean the lock this guard created.
        self.file.take();
        #[cfg(not(windows))]
        if fs::symlink_metadata(&self.path).is_ok_and(|metadata| {
            metadata.is_file()
                && !metadata.file_type().is_symlink()
                && ensure_same_file(&self.metadata, &metadata).is_ok()
        }) {
            let _ = fs::remove_file(&self.path);
        }
        #[cfg(windows)]
        if crate::support::windows_security::path_identity(&self.path)
            .is_ok_and(|identity| identity == self.identity)
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path
        .file_name()
        .expect("validated registry path")
        .to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

struct TemporaryFile(PathBuf);
impl Drop for TemporaryFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn atomic_replace(
    path: &Path,
    registry: &WorkspaceRegistry,
    before_commit: impl FnOnce() -> Result<()>,
) -> Result<()> {
    atomic_replace_with_sync(path, registry, before_commit, || {
        #[cfg(unix)]
        File::open(path.parent().expect("validated registry path"))?.sync_all()?;
        Ok(())
    })
}

fn atomic_replace_with_sync(
    path: &Path,
    registry: &WorkspaceRegistry,
    before_commit: impl FnOnce() -> Result<()>,
    sync_directory: impl FnOnce() -> Result<()>,
) -> Result<()> {
    registry.validate()?;
    let mut data = serde_json::to_vec_pretty(registry)?;
    data.push(b'\n');
    ensure!(
        data.len() as u64 <= MAX_REGISTRY_BYTES,
        "workspace registry exceeds size limit"
    );
    let temp = TemporaryFile(sibling(path, &format!(".{}.tmp", Uuid::new_v4())));
    let mut file = private_create(&temp.0)?;
    file.write_all(&data)?;
    file.sync_all()?;
    drop(file);
    before_commit()?;
    validate_registry_path(path)?;
    fs::rename(&temp.0, path).context("atomically replace workspace registry")?;
    sync_directory().context("workspace registry replacement is already visible, but directory durability is unconfirmed; inspect the registry before retrying")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_lock_contention_retry_is_bounded_and_preserves_other_errors() {
        for code in [5, 32, 33] {
            assert!(windows_lock_retry(
                Some(code),
                false,
                Duration::ZERO,
                Duration::ZERO
            ));
            assert!(windows_lock_retry(
                Some(code),
                false,
                Duration::from_secs(1),
                Duration::ZERO
            ));
            assert!(!windows_lock_retry(
                Some(code),
                false,
                Duration::from_millis(100),
                Duration::from_millis(100)
            ));
            assert!(windows_lock_retry(
                Some(code),
                true,
                Duration::from_millis(100),
                Duration::from_millis(100)
            ));
            assert!(!windows_lock_retry(
                Some(code),
                true,
                Duration::from_secs(2),
                Duration::ZERO
            ));
        }
        for code in [None, Some(2), Some(3), Some(87)] {
            assert!(!windows_lock_retry(
                code,
                true,
                Duration::ZERO,
                Duration::ZERO
            ));
        }
    }

    #[test]
    fn failed_atomic_commit_preserves_previous_registry_and_removes_own_temp_file() {
        let parent =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("target/workspace-core/registry-tests");
        fs::create_dir_all(&parent).unwrap();
        #[cfg(not(windows))]
        let temp = tempfile::tempdir_in(parent).unwrap();
        #[cfg(windows)]
        let temp = crate::support::windows_security::private_tempdir_in(
            &parent.canonicalize().unwrap(),
            "registry-tests-",
        )
        .unwrap();
        let path = temp.path().join("registry.json");
        let original = WorkspaceRegistry::default();
        atomic_replace(&path, &original, || Ok(())).unwrap();
        let bytes = fs::read(&path).unwrap();
        let mut changed = original;
        changed
            .register_canonical(
                temp.path().canonicalize().unwrap(),
                Uuid::new_v4().to_string(),
            )
            .unwrap();
        assert!(
            atomic_replace(&path, &changed, || bail!("injected failure before commit")).is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
        atomic_replace(&path, &changed, || Ok(())).unwrap();
        assert_eq!(load(&path).unwrap(), changed);
    }
    #[test]
    fn failed_directory_sync_reports_publication_instead_of_claiming_no_change() {
        #[cfg(not(windows))]
        let dir = tempfile::tempdir().unwrap();
        #[cfg(windows)]
        let dir = crate::support::windows_security::private_tempdir_in(
            &std::env::temp_dir().canonicalize().unwrap(),
            "registry-sync-",
        )
        .unwrap();
        let path = dir.path().canonicalize().unwrap().join("registry.json");
        let mut registry = WorkspaceRegistry::default();
        registry
            .register_canonical(
                dir.path().canonicalize().unwrap(),
                Uuid::new_v4().to_string(),
            )
            .unwrap();
        let error = atomic_replace_with_sync(
            &path,
            &registry,
            || Ok(()),
            || bail!("injected directory sync failure"),
        )
        .unwrap_err();
        assert!(error.to_string().contains("already visible"));
        assert!(
            error
                .to_string()
                .contains("inspect the registry before retrying")
        );
        assert_eq!(load(&path).unwrap(), registry);
    }
}
