use std::{
    fs,
    path::{Path, PathBuf},
    str::FromStr,
};

use agent_llm_mm::{
    adapters::sqlite::{
        CURRENT_DATABASE_SCHEMA_VERSION, initialize_database, inspect_database, migrate_database,
    },
    interfaces::mcp::validate_stdio_runtime,
    support::config::AppConfig,
};
use sqlx::{Connection, Row, SqliteConnection, sqlite::SqliteConnectOptions};
use tempfile::tempdir;

fn sqlite_url(path: &Path) -> String {
    format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"))
}

async fn create_legacy_database(path: &Path, valid_namespace: bool) {
    let url = sqlite_url(path);
    let mut connection = SqliteConnection::connect_with(
        &SqliteConnectOptions::from_str(&url)
            .expect("options")
            .create_if_missing(true),
    )
    .await
    .expect("legacy connection");
    sqlx::query(
        "CREATE TABLE events (event_id TEXT PRIMARY KEY, recorded_at TEXT NOT NULL, owner TEXT NOT NULL, namespace TEXT, kind TEXT NOT NULL, summary TEXT NOT NULL)",
    )
    .execute(&mut connection)
    .await
    .expect("legacy events table");
    sqlx::query(
        "INSERT INTO events (event_id, recorded_at, owner, namespace, kind, summary) VALUES ('legacy-event', '2026-01-01T00:00:00Z', 'self', ?, 'observation', 'legacy')",
    )
    .bind(if valid_namespace { "self" } else { "world" })
    .execute(&mut connection)
    .await
    .expect("legacy event");
    connection.close().await.expect("close legacy database");
}

fn backup_files(parent: &Path, database_name: &str) -> Vec<PathBuf> {
    fs::read_dir(parent)
        .expect("read parent")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with(&format!("{database_name}.pre-migrate-v"))
                        && name.ends_with(".bak")
                })
        })
        .collect()
}

#[tokio::test]
async fn read_only_inspection_of_missing_database_creates_nothing() {
    let temp = tempdir().expect("tempdir");
    let database_path = temp.path().join("missing-parent").join("missing.sqlite");

    let report = inspect_database(&sqlite_url(&database_path))
        .await
        .expect("read-only report");

    assert_eq!(report.status, "missing");
    assert!(!report.database_exists);
    assert!(!report.bootstrap_performed);
    assert!(!database_path.exists());
    assert!(!database_path.parent().expect("parent").exists());
}

#[tokio::test]
async fn read_only_inspection_of_legacy_database_preserves_bytes_and_schema() {
    let temp = tempdir().expect("tempdir");
    let database_path = temp.path().join("legacy.sqlite");
    create_legacy_database(&database_path, true).await;
    let before = fs::read(&database_path).expect("legacy bytes");

    let report = inspect_database(&sqlite_url(&database_path))
        .await
        .expect("legacy report");

    assert_eq!(report.status, "migration_required");
    assert_eq!(report.schema_version, Some(0));
    assert!(report.migration_required);
    assert_eq!(fs::read(&database_path).expect("bytes after"), before);
    assert!(backup_files(temp.path(), "legacy.sqlite").is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn read_only_inspection_accepts_read_only_file_without_writing() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempdir().expect("tempdir");
    let database_path = temp.path().join("read-only.sqlite");
    initialize_database(&sqlite_url(&database_path))
        .await
        .expect("init");
    let before = fs::read(&database_path).expect("database bytes");
    fs::set_permissions(&database_path, fs::Permissions::from_mode(0o444)).expect("make read-only");

    let report = inspect_database(&sqlite_url(&database_path))
        .await
        .expect("read-only inspection");

    assert_eq!(report.status, "current");
    assert_eq!(report.path_writable_hint, Some(false));
    assert_eq!(fs::read(&database_path).expect("bytes after"), before);
}

#[tokio::test]
async fn init_records_schema_ledger_defaults_counts_and_foreign_key_readback() {
    let temp = tempdir().expect("tempdir");
    let database_path = temp.path().join("initialized.sqlite");

    let report = initialize_database(&sqlite_url(&database_path))
        .await
        .expect("init report");

    assert_eq!(report.status, "current");
    assert_eq!(report.schema_version, Some(CURRENT_DATABASE_SCHEMA_VERSION));
    assert!(report.migration_ledger_consistent);
    assert!(report.required_tables_present);
    assert!(report.runtime_defaults_present);
    assert_eq!(report.foreign_key_violations, 0);
    assert_eq!(
        report
            .table_counts
            .iter()
            .find(|count| count.table == "schema_migrations")
            .expect("ledger count")
            .rows,
        CURRENT_DATABASE_SCHEMA_VERSION
    );
    assert_eq!(report.unknown_owner_inventory.events, 0);
    assert_eq!(report.unknown_owner_inventory.claims, 0);
    assert!(!report.unknown_owner_inventory.rewrite_performed);
    assert!(
        report
            .unknown_owner_inventory
            .rewrite_requires_separate_approval
    );
}

#[tokio::test]
async fn read_only_inspection_inventories_unknown_owner_rows_without_rewriting() {
    let temp = tempdir().expect("tempdir");
    let database_path = temp.path().join("unknown-inventory.sqlite");
    let url = sqlite_url(&database_path);
    initialize_database(&url).await.expect("init");

    let mut connection = SqliteConnection::connect_with(
        &SqliteConnectOptions::from_str(&url)
            .expect("options")
            .create_if_missing(false),
    )
    .await
    .expect("connection");
    sqlx::query(
        "INSERT INTO events (event_id, recorded_at, owner, namespace, kind, summary) VALUES ('unknown-event', '2026-08-13T00:00:00Z', 'unknown', 'world', 'observation', 'legacy unknown')",
    )
    .execute(&mut connection)
    .await
    .expect("unknown event");
    sqlx::query(
        "INSERT INTO claims (claim_id, owner, namespace, subject, predicate, object, mode, status) VALUES ('unknown-claim', 'unknown', 'project/demo', 'legacy.fact', 'is', 'unreadable', 'observed', 'active')",
    )
    .execute(&mut connection)
    .await
    .expect("unknown claim");
    connection.close().await.expect("close");

    let report = inspect_database(&url).await.expect("inventory report");
    assert_eq!(report.status, "current");
    assert_eq!(report.unknown_owner_inventory.events, 1);
    assert_eq!(report.unknown_owner_inventory.claims, 1);
    assert!(!report.unknown_owner_inventory.rewrite_performed);
    assert!(
        report
            .unknown_owner_inventory
            .rewrite_requires_separate_approval
    );

    let mut connection =
        SqliteConnection::connect_with(&SqliteConnectOptions::from_str(&url).expect("options"))
            .await
            .expect("reread");
    let event_owner = sqlx::query_scalar::<_, String>(
        "SELECT owner FROM events WHERE event_id = 'unknown-event'",
    )
    .fetch_one(&mut connection)
    .await
    .expect("event owner");
    let claim_owner = sqlx::query_scalar::<_, String>(
        "SELECT owner FROM claims WHERE claim_id = 'unknown-claim'",
    )
    .fetch_one(&mut connection)
    .await
    .expect("claim owner");
    connection.close().await.expect("close reread");
    assert_eq!(event_owner, "unknown");
    assert_eq!(claim_owner, "unknown");
}

#[tokio::test]
async fn legacy_migration_creates_backup_rehearses_restore_and_preserves_rows() {
    let temp = tempdir().expect("tempdir");
    let database_path = temp.path().join("legacy.sqlite");
    create_legacy_database(&database_path, true).await;

    let report = migrate_database(&sqlite_url(&database_path))
        .await
        .expect("migration");

    assert_eq!(report.status, "current");
    assert_eq!(report.restore_rehearsal, "passed_before_original_write");
    assert!(report.preserved_row_counts);
    assert_eq!(report.foreign_key_violations, 0);
    let backup_path = PathBuf::from(report.backup_path.expect("backup path"));
    assert!(backup_path.is_file());
    let event_count = report
        .table_counts
        .iter()
        .find(|count| count.table == "events")
        .expect("event count");
    assert_eq!(event_count.rows, 1);

    let backup_report = inspect_database(&sqlite_url(&backup_path))
        .await
        .expect("backup readback");
    assert_eq!(backup_report.status, "migration_required");
    assert_eq!(backup_report.schema_version, Some(0));
}

#[tokio::test]
async fn failed_rehearsal_leaves_original_recoverable_and_unmigrated() {
    let temp = tempdir().expect("tempdir");
    let database_path = temp.path().join("invalid.sqlite");
    create_legacy_database(&database_path, false).await;
    let before = fs::read(&database_path).expect("legacy bytes");

    let error = migrate_database(&sqlite_url(&database_path))
        .await
        .expect_err("invalid legacy namespace must fail rehearsal");

    assert!(error.to_string().contains("CHECK constraint failed"));
    assert_eq!(fs::read(&database_path).expect("original bytes"), before);
    let report = inspect_database(&sqlite_url(&database_path))
        .await
        .expect("original readback");
    assert_eq!(report.schema_version, Some(0));
    assert_eq!(
        report
            .table_counts
            .iter()
            .find(|count| count.table == "events")
            .expect("event count")
            .rows,
        1
    );
    let backups = backup_files(temp.path(), "invalid.sqlite");
    assert_eq!(backups.len(), 1);
    assert_eq!(
        inspect_database(&sqlite_url(&backups[0]))
            .await
            .expect("backup readback")
            .schema_version,
        Some(0)
    );
}

#[tokio::test]
async fn serve_validation_refuses_missing_database_without_creating_it() {
    let temp = tempdir().expect("tempdir");
    let database_path = temp.path().join("serve-missing.sqlite");
    let config = AppConfig {
        database_url: sqlite_url(&database_path),
        ..Default::default()
    };

    let error = match validate_stdio_runtime(&config).await {
        Ok(_) => panic!("serve validation must require explicit init"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("run init"));
    assert!(!database_path.exists());
}

#[tokio::test]
async fn current_database_migration_is_a_noop_without_new_backup() {
    let temp = tempdir().expect("tempdir");
    let database_path = temp.path().join("current.sqlite");
    initialize_database(&sqlite_url(&database_path))
        .await
        .expect("init");
    let before = fs::read(&database_path).expect("current bytes");

    let report = migrate_database(&sqlite_url(&database_path))
        .await
        .expect("no-op migration");

    assert_eq!(report.restore_rehearsal, "not_needed_current");
    assert!(report.backup_path.is_none());
    assert_eq!(fs::read(&database_path).expect("bytes after"), before);
    assert!(backup_files(temp.path(), "current.sqlite").is_empty());
}

#[tokio::test]
async fn ledger_rows_match_declared_versions_after_migration() {
    let temp = tempdir().expect("tempdir");
    let database_path = temp.path().join("ledger.sqlite");
    create_legacy_database(&database_path, true).await;
    migrate_database(&sqlite_url(&database_path))
        .await
        .expect("migration");

    let mut connection = SqliteConnection::connect(&sqlite_url(&database_path))
        .await
        .expect("connect migrated");
    let rows = sqlx::query("SELECT version, name FROM schema_migrations ORDER BY version")
        .fetch_all(&mut connection)
        .await
        .expect("ledger rows");
    let observed = rows
        .iter()
        .map(|row| (row.get::<i64, _>("version"), row.get::<String, _>("name")))
        .collect::<Vec<_>>();
    assert_eq!(
        observed,
        vec![
            (1, "baseline_schema".to_string()),
            (2, "owner_namespace_scope".to_string()),
            (3, "reflection_audit_columns".to_string()),
        ]
    );
    connection.close().await.expect("close migrated");
}

#[tokio::test]
async fn current_version_rejects_weakened_schema_without_writing_or_repairing() {
    // Keep user_version and migration ledger unchanged; row-count/FK-check-only
    // inspection would incorrectly call all of these empty databases current.
    for (table, from, to) in [
        (
            "events",
            "owner = 'self' AND namespace = 'self'",
            "owner = 'self'",
        ),
        ("claims", "namespace TEXT NOT NULL", "namespace TEXT"),
        (
            "evidence_links",
            "FOREIGN KEY (event_id) REFERENCES events(event_id)",
            "CHECK (1)",
        ),
        (
            "reflections",
            "supporting_evidence_event_ids TEXT NOT NULL DEFAULT '[]'",
            "supporting_evidence_event_ids TEXT",
        ),
        (
            "operation_log",
            "redaction_version INTEGER NOT NULL DEFAULT 1",
            "redaction_version TEXT NOT NULL DEFAULT '1'",
        ),
    ] {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("weakened.sqlite");
        let url = sqlite_url(&path);
        initialize_database(&url).await.expect("init");
        let mut connection = SqliteConnection::connect(&url).await.expect("connect");
        sqlx::query("PRAGMA writable_schema = ON")
            .execute(&mut connection)
            .await
            .expect("enable fixture mutation");
        let changed = sqlx::query(
            "UPDATE sqlite_master SET sql = replace(sql, ?, ?) WHERE type = 'table' AND name = ?",
        )
        .bind(from)
        .bind(to)
        .bind(table)
        .execute(&mut connection)
        .await
        .expect("weaken fixture schema");
        assert_eq!(changed.rows_affected(), 1);
        connection.close().await.expect("close");
        let before = fs::read(&path).expect("before");
        let report = inspect_database(&url).await.expect("inspect");
        assert_eq!(report.status, "schema_structure_invalid", "{table}");
        assert!(!report.schema_structure_valid);
        assert!(
            report
                .schema_structure_issues
                .iter()
                .any(|issue| issue.starts_with(table))
        );
        assert!(
            !report.migration_required,
            "same-version damage needs explicit repair"
        );
        assert!(migrate_database(&url).await.is_err());
        assert_eq!(fs::read(&path).expect("after"), before);
        assert!(backup_files(temp.path(), "weakened.sqlite").is_empty());
        let config = AppConfig {
            database_url: url,
            ..Default::default()
        };
        assert!(validate_stdio_runtime(&config).await.is_err());
    }
}

#[tokio::test]
async fn current_version_rejects_missing_columns_and_changed_primary_indexes() {
    for mutation in [
        "ALTER TABLE operation_log DROP COLUMN diagnostic_summary_json",
        "DROP TABLE episode_events; CREATE TABLE episode_events (episode_reference TEXT NOT NULL, event_id TEXT NOT NULL, FOREIGN KEY (event_id) REFERENCES events(event_id))",
        "CREATE UNIQUE INDEX unexpected_unique_event_summary ON events(summary)",
    ] {
        let temp = tempdir().expect("tempdir");
        let url = sqlite_url(&temp.path().join("structural.sqlite"));
        initialize_database(&url).await.expect("init");
        let mut connection = SqliteConnection::connect(&url).await.expect("connect");
        sqlx::raw_sql(mutation)
            .execute(&mut connection)
            .await
            .expect("mutate structure");
        connection.close().await.expect("close");
        assert_eq!(
            inspect_database(&url).await.expect("inspect").status,
            "schema_structure_invalid"
        );
    }
}

#[tokio::test]
async fn init_race_has_one_winner_and_does_not_delete_winners_database() {
    let temp = tempdir().expect("tempdir");
    let path = temp.path().join("race.sqlite");
    let url = sqlite_url(&path);
    let (first, second) = tokio::join!(initialize_database(&url), initialize_database(&url));
    assert_ne!(first.is_ok(), second.is_ok());
    let report = inspect_database(&url).await.expect("winner readback");
    assert!(report.is_current());
    assert!(report.schema_structure_valid);
}

#[tokio::test]
async fn init_refuses_preexisting_sidecars_and_preserves_them() {
    let temp = tempdir().expect("tempdir");
    let path = temp.path().join("sidecar.sqlite");
    let sidecar = temp.path().join("sidecar.sqlite-wal");
    fs::write(&sidecar, b"belongs to another operation").expect("write sidecar");
    assert!(initialize_database(&sqlite_url(&path)).await.is_err());
    assert!(!path.exists());
    assert_eq!(
        fs::read(sidecar).expect("sidecar"),
        b"belongs to another operation"
    );
}

#[tokio::test]
async fn migration_refuses_active_external_writer_without_creating_backup() {
    for wal in [false, true] {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("busy.sqlite");
        let url = sqlite_url(&path);
        create_legacy_database(&path, true).await;
        let mut writer = SqliteConnection::connect(&url).await.expect("writer");
        if wal {
            sqlx::query("PRAGMA journal_mode = WAL")
                .execute(&mut writer)
                .await
                .expect("wal");
        }
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut writer)
            .await
            .expect("reserve writer");
        sqlx::query("UPDATE events SET summary = 'same row count, different content'")
            .execute(&mut writer)
            .await
            .expect("update");
        let error = migrate_database(&url).await.expect_err("busy migration");
        assert!(error.to_string().contains("write reservation unavailable"));
        assert!(backup_files(temp.path(), "busy.sqlite").is_empty());
        sqlx::query("ROLLBACK")
            .execute(&mut writer)
            .await
            .expect("rollback");
        writer.close().await.expect("close");
        assert!(
            migrate_database(&url)
                .await
                .expect("migration after unlock")
                .is_current()
        );
    }
}

#[tokio::test]
async fn version_two_reflection_alter_migration_matches_canonical_structure() {
    let temp = tempdir().expect("tempdir");
    let path = temp.path().join("version-two.sqlite");
    let url = sqlite_url(&path);
    initialize_database(&url).await.expect("init");
    let mut connection = SqliteConnection::connect(&url).await.expect("connect");
    sqlx::raw_sql(
        "ALTER TABLE reflections DROP COLUMN supporting_evidence_event_ids;
         ALTER TABLE reflections DROP COLUMN requested_identity_update;
         ALTER TABLE reflections DROP COLUMN requested_commitment_updates;
         DELETE FROM schema_migrations WHERE version = 3;
         PRAGMA user_version = 2;",
    )
    .execute(&mut connection)
    .await
    .expect("version two fixture");
    connection.close().await.expect("close");
    let report = migrate_database(&url).await.expect("migrate version two");
    assert!(report.is_current());
    assert!(report.schema_structure_valid);
    assert!(report.schema_structure_issues.is_empty());
    assert_eq!(report.restore_rehearsal, "passed_before_original_write");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn migration_report_keeps_locked_snapshot_when_external_writer_resumes() {
    for wal in [false, true] {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("resume-writer.sqlite");
        let url = sqlite_url(&path);
        create_legacy_database(&path, true).await;
        let mut writer = SqliteConnection::connect_with(
            &SqliteConnectOptions::from_str(&url)
                .expect("options")
                .busy_timeout(std::time::Duration::ZERO),
        )
        .await
        .expect("writer");
        if wal {
            sqlx::query("PRAGMA journal_mode = WAL")
                .execute(&mut writer)
                .await
                .expect("wal");
        }
        let parent = temp.path().to_path_buf();
        let writer_task = tokio::spawn(async move {
            // The backup appears only after migration owns BEGIN IMMEDIATE.
            // Start contending then, so the write can succeed only after the
            // migration commits and releases its reservation.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while backup_files(&parent, "resume-writer.sqlite").is_empty() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "backup never appeared"
                );
                tokio::task::yield_now().await;
            }
            loop {
                let result = sqlx::query(
                    "INSERT INTO events (event_id, recorded_at, owner, namespace, kind, summary) VALUES ('resumed-writer-event', '2026-01-01T00:00:00Z', 'self', 'self', 'observation', 'written after migration commit')",
                )
                .execute(&mut writer)
                .await;
                match result {
                    Ok(_) => break,
                    Err(error) => {
                        let busy = error.as_database_error().is_some_and(|error| {
                            matches!(
                                error.code().as_deref(),
                                Some("5" | "6" | "261" | "262" | "517")
                            )
                        });
                        assert!(busy, "unexpected writer failure: {error}");
                        assert!(std::time::Instant::now() < deadline, "writer stayed locked");
                        tokio::task::yield_now().await;
                    }
                }
            }
            writer.close().await.expect("close writer");
        });

        let report = migrate_database(&url).await.expect("migration succeeds");
        writer_task.await.expect("writer task");
        assert!(report.is_current());
        assert!(report.schema_structure_valid);
        assert!(report.preserved_row_counts);
        assert!(report.message.contains("migration transaction snapshot"));
        assert_eq!(
            report
                .table_counts
                .iter()
                .find(|count| count.table == "events")
                .expect("snapshot event count")
                .rows,
            1,
            "migration report must retain its own protected snapshot",
        );
        let live = inspect_database(&url).await.expect("live inspection");
        assert!(live.is_current());
        assert_eq!(
            live.table_counts
                .iter()
                .find(|count| count.table == "events")
                .expect("live event count")
                .rows,
            2,
            "ordinary post-commit writer must succeed independently of the migration report",
        );
    }
}
