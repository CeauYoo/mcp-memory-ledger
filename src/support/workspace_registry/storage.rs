use std::{
    fs::{self, File, Metadata, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
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
    let before = registry_metadata(path)?;
    ensure!(
        before.len() <= MAX_REGISTRY_BYTES,
        "workspace registry exceeds size limit"
    );
    let file = File::open(path).context("open workspace registry")?;
    ensure_same_file(&before, &file.metadata()?)?;
    ensure_same_file(&before, &registry_metadata(path)?)?;
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

fn update<F>(path: &Path, create: bool, change: F) -> Result<WorkspaceRegistration>
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

fn canonical_root(root: &Path) -> Result<PathBuf> {
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

fn validate_registry_path(path: &Path) -> Result<()> {
    validate_absolute_path(path)?;
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
    metadata: Metadata,
}

impl RegistryLock {
    fn acquire(registry_path: &Path) -> Result<Self> {
        let path = sibling(registry_path, ".lock");
        let started = Instant::now();
        loop {
            match private_create(&path) {
                Ok(file) => {
                    let metadata = file.metadata()?;
                    return Ok(Self {
                        path,
                        file: Some(file),
                        metadata,
                    });
                }
                Err(error)
                    if error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|error| error.kind() == std::io::ErrorKind::AlreadyExists) =>
                {
                    // Never delete an unknown lock, even if its owner appears gone.
                    if started.elapsed() >= Duration::from_secs(2) {
                        bail!(
                            "workspace registry lock is busy or stale; inspect it before an explicit retry: {}",
                            path.display()
                        );
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(error.context("create workspace registry lock")),
            }
        }
    }
}

impl Drop for RegistryLock {
    fn drop(&mut self) {
        // Close first for Windows, and only clean the lock this guard created.
        self.file.take();
        if fs::symlink_metadata(&self.path).is_ok_and(|metadata| {
            metadata.is_file()
                && !metadata.file_type().is_symlink()
                && ensure_same_file(&self.metadata, &metadata).is_ok()
        }) {
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
    #[cfg(unix)]
    File::open(path.parent().expect("validated registry path"))?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_atomic_commit_preserves_previous_registry_and_removes_own_temp_file() {
        let parent =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("target/workspace-core/registry-tests");
        fs::create_dir_all(&parent).unwrap();
        let temp = tempfile::tempdir_in(parent).unwrap();
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
}
