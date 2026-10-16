//! Administrative legacy binding. SQLite is never initialized or rewritten.
use super::{WorkspaceRegistration, WorkspaceRegistry, model::validate_legacy_namespace, storage};
use crate::adapters::sqlite::{AdoptionInventory, namespace_adoption::inventory};
use anyhow::Context;
use anyhow::{Result, ensure};
use serde::Serialize;
use sqlx::{Connection, SqliteConnection, sqlite::SqliteConnectOptions};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;

#[derive(Debug, Serialize)]
pub struct AdoptionReport {
    pub operation: &'static str,
    pub database: PathBuf,
    pub root: PathBuf,
    pub inventory: AdoptionInventory,
    pub workspace: Option<WorkspaceRegistration>,
    pub database_changed: bool,
    pub client_configuration_changed: bool,
}

/// Advisory preview, not an authorization token. No lock files or registry writes.
pub async fn preview_adoption(
    registry: &Path,
    root: &Path,
    namespace: &str,
    database: &Path,
) -> Result<AdoptionReport> {
    validate_legacy_namespace(namespace)?;
    let root = storage::canonical_root(root)?;
    let database = database_path(database)?;
    let mut conn = connect(&database, true).await?;
    sqlx::query("BEGIN").execute(&mut conn).await?;
    let mut inventory = inventory(&mut conn, namespace).await?;
    if let Err(error) = check_registration(registry, root.clone(), namespace) {
        inventory.block(format!("registry binding unavailable: {error}"));
    }
    sqlx::query("ROLLBACK").execute(&mut conn).await?;
    conn.close().await?;
    Ok(AdoptionReport {
        operation: "adopt-preview",
        database,
        root,
        inventory,
        workspace: None,
        database_changed: false,
        client_configuration_changed: false,
    })
}

/// Execution repeats inventory under an SQLite writer reservation and the same
/// advisory database lock used by the core. Standalone writers can resume after
/// this call: the owner must explicitly retire/reconfigure those clients.
#[cfg(any(unix, windows))]
pub async fn adopt_workspace(
    registry: &Path,
    root: &Path,
    namespace: &str,
    database: &Path,
) -> Result<AdoptionReport> {
    validate_legacy_namespace(namespace)?;
    let root = storage::canonical_root(root)?;
    let database = database_path(database)?;
    // Validate registry eligibility before creating the cooperating-core lock.
    check_registration(registry, root.clone(), namespace)?;
    #[cfg(unix)]
    let _core_lock = {
        use std::os::unix::fs::MetadataExt;
        let uid = fs::metadata(registry.parent().context("registry parent required")?)?.uid();
        ensure!(
            fs::metadata(&database)?.uid() == uid,
            "database and registry directory must have the same owner"
        );
        let mut lock = database.as_os_str().to_owned();
        lock.push(".local-core.lock");
        crate::support::local_file_lock::PrivateLock::acquire(Path::new(&lock), uid)?
    };
    #[cfg(windows)]
    let _core_lock = crate::support::windows_security::acquire_database_lock(&database, registry)?;
    let mut conn = connect(&database, false).await?;
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut conn)
        .await
        .context("adoption requires an idle SQLite writer; stop other writers and retry")?;
    let inventory = inventory(&mut conn, namespace).await?;
    ensure!(
        inventory.supported,
        "adoption blocked: {}",
        inventory.blocked_reasons.join("; ")
    );
    // This synchronous atomic write occurs before releasing the same connection's
    // writer reservation. Registry conflicts are checked again under registry lock.
    let workspace = storage::update(registry, true, |state| {
        state.adopt_canonical(root.clone(), namespace, Uuid::new_v4().to_string())
    })?;
    // No SQL mutations were issued. Closing rolls back even if cleanup fails;
    // don't return a misleading retryable failure after registry publication.
    let _ = sqlx::query("ROLLBACK").execute(&mut conn).await;
    let _ = conn.close().await;
    Ok(AdoptionReport {
        operation: "adopt",
        database,
        root,
        inventory,
        workspace: Some(workspace),
        database_changed: false,
        client_configuration_changed: false,
    })
}
#[cfg(not(any(unix, windows)))]
pub async fn adopt_workspace(_: &Path, _: &Path, _: &str, _: &Path) -> Result<AdoptionReport> {
    anyhow::bail!(
        "workspace adoption execution requires the Unix local-core lock; read-only adopt-preview is available"
    )
}
fn check_registration(path: &Path, root: PathBuf, namespace: &str) -> Result<()> {
    storage::validate_registry_path(path)?;
    let mut state = match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => WorkspaceRegistry::default(),
        _ => WorkspaceRegistry::load(path)?,
    };
    state.adopt_canonical(root, namespace, Uuid::new_v4().to_string())?;
    Ok(())
}
fn database_path(path: &Path) -> Result<PathBuf> {
    super::model::validate_absolute_path(path)?;
    for ancestor in path.ancestors() {
        ensure!(
            !fs::symlink_metadata(ancestor)?.file_type().is_symlink(),
            "database path must not contain symlinks"
        );
    }
    let metadata = fs::metadata(path)?;
    ensure!(
        metadata.is_file(),
        "adoption requires an existing regular database file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            metadata.nlink() == 1,
            "database hard-link aliases are not supported"
        );
    }
    Ok(path.canonicalize()?)
}
async fn connect(path: &Path, read_only: bool) -> Result<SqliteConnection> {
    Ok(SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(false)
            .read_only(read_only)
            .busy_timeout(Duration::ZERO),
    )
    .await?)
}
