//! Native SQLite online backup and restore-to-new-path. Never loads application config.
use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{Connection, SqliteConnection, sqlite::SqliteConnectOptions};

use crate::adapters::sqlite::{CURRENT_DATABASE_SCHEMA_VERSION, inspect_database};

#[derive(Debug, Serialize)]
pub struct BackupReport {
    pub operation: String,
    pub output: PathBuf,
    pub schema_version: i64,
    pub sha256: String,
    pub registry_included: bool,
    pub client_configuration_changed: bool,
}

/// Args exclude the executable. Unrecognized commands belong to the other CLI handlers.
pub async fn try_run_cli(args: &[String]) -> Result<bool> {
    let Some(operation) = args.first().map(String::as_str) else {
        return Ok(false);
    };
    if !matches!(operation, "backup" | "restore") {
        return Ok(false);
    }
    let usage = "usage: agent_llm_mm backup|restore --db EXISTING_FILE --output NEW_FILE";
    if args.len() == 2 && matches!(args[1].as_str(), "--help" | "-h") {
        println!(
            "{usage}\nUses SQLite online backup API; current-schema databases only. Registry is separate; no configuration or client activation is changed."
        );
        return Ok(true);
    }
    let mut source = None;
    let mut output = None;
    let mut fields = args[1..].chunks_exact(2);
    for field in &mut fields {
        let slot = match field[0].as_str() {
            "--db" => &mut source,
            "--output" => &mut output,
            _ => bail!("{usage}; unknown option {}", field[0]),
        };
        ensure!(
            slot.is_none() && !field[1].is_empty(),
            "{usage}; duplicate or empty argument"
        );
        *slot = Some(PathBuf::from(&field[1]));
    }
    ensure!(fields.remainder().is_empty(), "{usage}");
    let source = source.context(usage)?;
    let output = output.context(usage)?;
    let report = transfer_database(operation, &source, &output).await?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(true)
}

/// Validates a private snapshot before atomic no-clobber publication. Older/newer schemas
/// require a matching binary; this operation never migrates a database.
pub async fn transfer_database(
    operation: &str,
    source: &Path,
    output: &Path,
) -> Result<BackupReport> {
    ensure!(
        matches!(operation, "backup" | "restore"),
        "invalid backup operation"
    );
    reject_symlinks(source)?;
    ensure!(
        fs::symlink_metadata(source)?.is_file(),
        "source must be a regular file"
    );
    reject_symlinks(output)?;
    ensure_absent(output)?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    ensure!(parent.is_dir(), "output parent must already exist");
    // Resolve the existing parent once; publication cannot follow a final-component symlink.
    let parent = parent.canonicalize()?;
    let output = parent.join(output.file_name().context("output requires a filename")?);
    ensure_absent(&output)?;
    #[cfg(not(windows))]
    let staging = {
        let mut builder = tempfile::Builder::new();
        builder.prefix(".agent-sqlite-backup-");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            builder.permissions(fs::Permissions::from_mode(0o700));
        }
        builder.tempdir_in(&parent)?
    };
    #[cfg(windows)]
    let staging =
        crate::support::windows_security::private_tempdir_in(&parent, ".agent-sqlite-backup-")?;
    let snapshot = staging.path().join("snapshot.sqlite");
    #[cfg(not(windows))]
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&snapshot)?;
    #[cfg(windows)]
    let file = crate::support::windows_security::private_create(&snapshot)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    drop(file);
    copy_snapshot(source, &snapshot).await?;
    let report = inspect_database(&crate::support::sqlite_url::sqlite_file_url(&snapshot)).await?;
    ensure!(
        report.is_current(),
        "snapshot rejected: {} (schema {:?}, supported {}); use a matching binary for older backups and explicitly migrate a separate copy",
        report.status,
        report.schema_version,
        CURRENT_DATABASE_SCHEMA_VERSION
    );
    // Windows FlushFileBuffers (used by sync_all) requires GENERIC_WRITE.
    // Reopen the completed private snapshot without create/truncate flags.
    let mut file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&snapshot)
        .context("open completed snapshot for hashing and flush")?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    file.sync_all().context("flush completed backup snapshot")?;
    drop(file);
    // hard_link is an atomic create-if-absent on both Unix and Windows, unlike rename.
    // It stays on the destination filesystem. Unsupported filesystems fail safely.
    ensure_absent(&output)?;
    fs::hard_link(&snapshot, &output).context("cannot publish snapshot without overwriting; destination filesystem must support hard links")?;
    Ok(BackupReport {
        operation: operation.into(),
        output,
        schema_version: CURRENT_DATABASE_SCHEMA_VERSION,
        sha256: format!("{:x}", hash.finalize()),
        registry_included: false,
        client_configuration_changed: false,
    })
}

fn reject_symlinks(path: &Path) -> Result<()> {
    let mut prefix = PathBuf::new();
    for part in path.components() {
        ensure!(
            !matches!(part, Component::ParentDir),
            "parent-directory components are not accepted"
        );
        prefix.push(part);
        if matches!(part, Component::Prefix(_)) {
            ensure!(
                path.has_root(),
                "drive-prefixed database paths must be absolute"
            );
            // A Windows prefix such as \\?\C: is not an inspectable root until
            // the following RootDir component supplies its final separator.
            continue;
        }
        match fs::symlink_metadata(&prefix) {
            Ok(meta) => ensure!(
                !meta.file_type().is_symlink(),
                "symlink paths are not accepted: {}",
                prefix.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn ensure_absent(path: &Path) -> Result<()> {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        match fs::symlink_metadata(PathBuf::from(name)) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
            Ok(_) => bail!(
                "output or SQLite sidecar already exists; refusing overwrite: {}{suffix}",
                path.display()
            ),
        }
    }
    Ok(())
}

async fn copy_snapshot(source: &Path, target: &Path) -> Result<()> {
    let mut source = SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(source)
            .read_only(true)
            .busy_timeout(Duration::from_secs(5)),
    )
    .await?;
    let mut target = SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(target)
            .busy_timeout(Duration::from_secs(5)),
    )
    .await?;
    let result = async {
        {
            let mut source_handle = source.lock_handle().await?;
            let mut target_handle = target.lock_handle().await?;
            // SAFETY: SQLx locks give exclusive handle access; both connections outlive
            // the backup. No SQL or async suspension occurs until backup_finish returns.
            unsafe {
                let backup = libsqlite3_sys::sqlite3_backup_init(
                    target_handle.as_raw_handle().as_ptr(),
                    c"main".as_ptr(),
                    source_handle.as_raw_handle().as_ptr(),
                    c"main".as_ptr(),
                );
                ensure!(!backup.is_null(), "SQLite backup initialization failed");
                let start = Instant::now();
                let result = loop {
                    let status = libsqlite3_sys::sqlite3_backup_step(backup, 256);
                    if status == libsqlite3_sys::SQLITE_DONE {
                        break Ok(());
                    }
                    if !matches!(
                        status,
                        libsqlite3_sys::SQLITE_OK
                            | libsqlite3_sys::SQLITE_BUSY
                            | libsqlite3_sys::SQLITE_LOCKED
                    ) {
                        break Err(anyhow::anyhow!("SQLite backup failed (code {status})"));
                    }
                    if start.elapsed() > Duration::from_secs(60) {
                        break Err(anyhow::anyhow!("SQLite backup timed out; source unchanged"));
                    }
                    if status != libsqlite3_sys::SQLITE_OK {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                };
                let finish = libsqlite3_sys::sqlite3_backup_finish(backup);
                result?;
                ensure!(
                    finish == libsqlite3_sys::SQLITE_OK,
                    "SQLite backup finalization failed ({finish})"
                );
            }
        }
        // Make the artifact self-contained, even when its source uses WAL.
        sqlx::query("PRAGMA journal_mode = DELETE")
            .execute(&mut target)
            .await?;
        let checks: Vec<String> = sqlx::query_scalar("PRAGMA integrity_check")
            .fetch_all(&mut target)
            .await?;
        ensure!(checks == ["ok"], "snapshot integrity check failed");
        Ok::<(), anyhow::Error>(())
    }
    .await;
    let source_close = source.close().await;
    let target_close = target.close().await;
    result?;
    source_close?;
    target_close?;
    Ok(())
}

#[cfg(all(test, windows))]
mod windows_path_tests {
    use super::*;

    #[test]
    fn canonical_drive_prefix_is_not_opened_before_its_root_separator() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        assert!(matches!(
            root.components().next(),
            Some(Component::Prefix(_))
        ));
        reject_symlinks(&root).unwrap();
        reject_symlinks(&root.join("new-snapshot.sqlite")).unwrap();
        assert!(reject_symlinks(Path::new(r"C:relative.sqlite")).is_err());
        // PathBuf::join normalizes away `..` after a Windows verbatim prefix.
        // Append raw OS text so the validator actually receives traversal input.
        let mut traversal = root.as_os_str().to_os_string();
        traversal.push(r"\..\snapshot.sqlite");
        let traversal = PathBuf::from(traversal);
        assert!(
            traversal
                .components()
                .any(|part| part == Component::ParentDir)
        );
        assert!(reject_symlinks(&traversal).is_err());
    }
}
