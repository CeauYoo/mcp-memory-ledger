use agent_llm_mm::adapters::sqlite::CURRENT_DATABASE_SCHEMA_VERSION;
use agent_llm_mm::{
    adapters::sqlite::initialize_database,
    support::database_backup::{transfer_database, try_run_cli},
};
use sqlx::{Connection, SqliteConnection, sqlite::SqliteConnectOptions};
use std::{fs, path::Path};
use tempfile::tempdir;

async fn database(path: &Path) {
    initialize_database(&agent_llm_mm::support::sqlite_url::sqlite_file_url(path))
        .await
        .unwrap();
}
async fn connect(path: &Path) -> SqliteConnection {
    SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(path))
        .await
        .unwrap()
}

#[tokio::test]
async fn online_wal_backup_preserves_uncheckpointed_rows_and_source() {
    let dir = tempdir().unwrap();
    let source = dir.path().canonicalize().unwrap().join("source.sqlite");
    let backup = dir.path().canonicalize().unwrap().join("backup.sqlite");
    let restored = dir.path().canonicalize().unwrap().join("restored.sqlite");
    database(&source).await;
    let mut writer = connect(&source).await;
    sqlx::query("PRAGMA journal_mode=WAL")
        .execute(&mut writer)
        .await
        .unwrap();
    sqlx::query("PRAGMA wal_autocheckpoint=0")
        .execute(&mut writer)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE backup_probe (value TEXT)")
        .execute(&mut writer)
        .await
        .unwrap();
    sqlx::query("INSERT INTO backup_probe VALUES ('committed in WAL')")
        .execute(&mut writer)
        .await
        .unwrap();
    let source_before = fs::read(&source).unwrap();
    let wal_path = source.with_extension("sqlite-wal");
    let wal_before = fs::read(&wal_path).unwrap();
    assert!(!wal_before.is_empty());
    let report = transfer_database("backup", &source, &backup).await.unwrap();
    assert_eq!(report.sha256.len(), 64);
    assert!(!report.registry_included);
    assert_eq!(fs::read(&source).unwrap(), source_before);
    assert_eq!(fs::read(&wal_path).unwrap(), wal_before);
    transfer_database("restore", &backup, &restored)
        .await
        .unwrap();
    let mut restored_connection = connect(&restored).await;
    let row: String = sqlx::query_scalar("SELECT value FROM backup_probe")
        .fetch_one(&mut restored_connection)
        .await
        .unwrap();
    assert_eq!(row, "committed in WAL");
    restored_connection.close().await.unwrap();
    writer.close().await.unwrap();
    assert_eq!(
        fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e
                .file_name()
                .to_string_lossy()
                .starts_with(".agent-sqlite-backup-"))
            .count(),
        0
    );
}

#[tokio::test]
async fn refuses_existing_paths_sidecars_corruption_and_incompatible_versions() {
    let dir = tempdir().unwrap();
    let source = dir.path().canonicalize().unwrap().join("source.sqlite");
    database(&source).await;
    let before = fs::read(&source).unwrap();
    assert!(
        transfer_database("restore", &source, &source)
            .await
            .is_err()
    );
    assert_eq!(fs::read(&source).unwrap(), before);
    let output = dir.path().canonicalize().unwrap().join("output.sqlite");
    fs::write(
        output.with_extension("sqlite-wal"),
        b"belongs to somebody else",
    )
    .unwrap();
    assert!(transfer_database("backup", &source, &output).await.is_err());
    fs::remove_file(output.with_extension("sqlite-wal")).unwrap();
    let corrupt = dir.path().canonicalize().unwrap().join("corrupt.sqlite");
    fs::write(&corrupt, b"not SQLite").unwrap();
    assert!(
        transfer_database("restore", &corrupt, &output)
            .await
            .is_err()
    );
    assert!(!output.exists());
    for version in [
        CURRENT_DATABASE_SCHEMA_VERSION - 1,
        CURRENT_DATABASE_SCHEMA_VERSION + 1,
    ] {
        let mut connection = connect(&source).await;
        sqlx::query(&format!("PRAGMA user_version={version}"))
            .execute(&mut connection)
            .await
            .unwrap();
        connection.close().await.unwrap();
        assert!(
            transfer_database("restore", &source, &output)
                .await
                .is_err()
        );
        assert!(!output.exists());
    }
    assert_eq!(
        fs::read_dir(dir.path()).unwrap().count(),
        2,
        "failed attempts clean only private staging files"
    );
}

#[tokio::test]
async fn malformed_current_schema_is_not_published() {
    let dir = tempdir().unwrap();
    let source = dir.path().canonicalize().unwrap().join("source.sqlite");
    let output = dir.path().canonicalize().unwrap().join("backup.sqlite");
    database(&source).await;
    let mut connection = connect(&source).await;
    sqlx::query("DROP TABLE operation_log")
        .execute(&mut connection)
        .await
        .unwrap();
    connection.close().await.unwrap();
    assert!(transfer_database("backup", &source, &output).await.is_err());
    assert!(!output.exists());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[cfg(unix)]
#[tokio::test]
async fn refuses_symlinks_including_dangling_destinations() {
    use std::os::unix::fs::symlink;
    let dir = tempdir().unwrap();
    let source = dir.path().canonicalize().unwrap().join("source.sqlite");
    database(&source).await;
    let output = dir.path().canonicalize().unwrap().join("output.sqlite");
    symlink(dir.path().canonicalize().unwrap().join("missing"), &output).unwrap();
    assert!(transfer_database("backup", &source, &output).await.is_err());
    fs::remove_file(&output).unwrap();
    let alias = dir.path().canonicalize().unwrap().join("alias.sqlite");
    symlink(&source, &alias).unwrap();
    assert!(transfer_database("backup", &alias, &output).await.is_err());
    assert!(!output.exists());
}

#[tokio::test]
async fn cli_rejects_implicit_config_duplicate_and_unknown_arguments() {
    for args in [
        vec!["backup"],
        vec!["restore", "--config", "secret.toml"],
        vec!["backup", "--db", "a", "--db", "b"],
    ] {
        assert!(
            try_run_cli(&args.into_iter().map(str::to_owned).collect::<Vec<_>>())
                .await
                .is_err()
        );
    }
    assert!(!try_run_cli(&["serve".into()]).await.unwrap());
}

#[tokio::test]
async fn concurrent_publish_never_clobbers_and_leaves_no_staging() {
    let dir = tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let source = root.join("source.sqlite");
    let output = root.join("backup.sqlite");
    database(&source).await;
    let (first, second) = tokio::join!(
        transfer_database("backup", &source, &output),
        transfer_database("backup", &source, &output),
    );
    assert_ne!(
        first.is_ok(),
        second.is_ok(),
        "exactly one atomic publication succeeds"
    );
    assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
    let mut connection = connect(&output).await;
    let check: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    assert_eq!(check, "ok");
    connection.close().await.unwrap();
}

#[tokio::test]
async fn cancelled_before_publication_leaves_no_output() {
    let dir = tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let source = root.join("source.sqlite");
    let output = root.join("backup.sqlite");
    database(&source).await;
    // Poll once: staging is allocated, then SQLite connection setup yields.
    // Dropping the pending transfer exercises cancellation of its private staging guard.
    let mut transfer = Box::pin(transfer_database("backup", &source, &output));
    let poll = std::future::poll_fn(|cx| std::task::Poll::Ready(transfer.as_mut().poll(cx))).await;
    assert!(poll.is_pending());
    drop(transfer);
    assert!(!output.exists());
    // On Windows the SQLx background connection may finish closing asynchronously.
    for _ in 0..20 {
        if fs::read_dir(&root).unwrap().count() == 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
}
