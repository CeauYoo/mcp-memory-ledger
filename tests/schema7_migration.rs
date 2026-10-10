use agent_llm_mm::{
    adapters::sqlite::{
        initialize_database, inspect_database, migrate_database, open_current_database,
    },
    domain::{identity_core::IdentityCore, self_model_version::SelfModelVersionKind},
    ports::ReflectionTransactionRunner,
};
use sqlx::{Connection, SqliteConnection};
use std::{fs, path::Path};

fn url(path: &Path) -> String {
    format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"))
}

// Build the exact historical v6 shape without changing its DDL, rows, or ledger.
async fn remove_v7(connection: &mut SqliteConnection) {
    sqlx::raw_sql(
        "DROP TRIGGER self_model_versions_append;
        DROP TRIGGER self_model_versions_no_update;
        DROP TRIGGER self_model_versions_no_delete;
        DROP TABLE self_model_versions;
        DELETE FROM schema_migrations WHERE version = 7;
        PRAGMA user_version = 6;",
    )
    .execute(connection)
    .await
    .unwrap();
}

#[tokio::test]
async fn fresh_baseline_is_effective_and_reads_never_add_versions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fresh.sqlite");
    let db = url(&path);
    let report = initialize_database(&db).await.unwrap();
    assert_eq!(report.schema_version, Some(7));
    let store = open_current_database(&db).await.unwrap();
    let mut transaction = store.begin_reflection_transaction().await.unwrap();
    let baseline = transaction.load_current_self_model_version().await.unwrap();
    assert_eq!(baseline.version, 0);
    assert_eq!(baseline.previous_version, None);
    assert_eq!(baseline.kind, SelfModelVersionKind::InitializationBaseline);
    assert_eq!(baseline.effective_at, Some(baseline.recorded_at));
    assert!(!baseline.identity_written && !baseline.commitments_written);
    assert_eq!(baseline.identity_source_version, 0);
    assert_eq!(baseline.commitment_source_version, 0);
    assert_eq!(
        baseline.identity,
        transaction.load_identity().await.unwrap()
    );
    assert_eq!(
        baseline.commitments,
        transaction.load_commitments().await.unwrap()
    );
    transaction.commit().await.unwrap();
    assert!(store.rebuild_retrieval_index().await.unwrap().is_usable());
    assert!(inspect_database(&db).await.unwrap().is_current());
    assert_eq!(
        migrate_database(&db).await.unwrap().restore_rehearsal,
        "not_needed_current"
    );
    let mut connection = SqliteConnection::connect(&db).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM self_model_versions")
            .fetch_one(&mut connection)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn v6_migration_snapshots_actual_projections_and_preserves_receipt_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v6.sqlite");
    let db = url(&path);
    initialize_database(&db).await.unwrap();
    let mut connection = SqliteConnection::connect(&db).await.unwrap();
    remove_v7(&mut connection).await;
    sqlx::raw_sql("DELETE FROM identity_claims;
        INSERT INTO identity_claims(position,claim) VALUES (5,'重复'),(9,'next'),(12,'重复');
        INSERT INTO commitments(description,owner) VALUES ('z-before-a','world'),('a-after-z','user');
        INSERT INTO operation_log(operation_id,occurred_at,actor_kind,actor_id,entrypoint,operation_kind,status,request_summary_json,response_summary_json)
        VALUES ('receipt-legacy','2020-01-02T11:04:05.123456789+08:00','system','durable_write_receipt_v1','test','tool','ok','{ \"request_hash\" : \"original\" }','{ \"outcome\" : [2, 1], \"text\" : \"中文\" }');
        INSERT INTO reflections(reflection_id,recorded_at,summary,requested_identity_update)
        VALUES ('historical','2020-01-02T11:04:05.123456789+08:00','legacy patch, not current state','{ \"canonical_claims\" : [\"past\"] }');")
        .execute(&mut connection).await.unwrap();
    let original: (String,String,String,String) = sqlx::query_as("SELECT o.occurred_at,o.request_summary_json,o.response_summary_json,r.requested_identity_update FROM operation_log o CROSS JOIN reflections r WHERE o.operation_id='receipt-legacy' AND r.reflection_id='historical'")
        .fetch_one(&mut connection).await.unwrap();
    connection.close().await.unwrap();
    let before = fs::read(&path).unwrap();
    assert_eq!(inspect_database(&db).await.unwrap().schema_version, Some(6));
    assert_eq!(before, fs::read(&path).unwrap());
    let migrated = migrate_database(&db).await.unwrap();
    assert_eq!(migrated.schema_version, Some(7));
    assert_eq!(migrated.restore_rehearsal, "passed_before_original_write");
    assert!(migrated.preserved_row_counts && migrated.schema_structure_valid);
    assert_eq!(migrated.foreign_key_violations, 0);
    let backup = migrated.backup_path.unwrap();
    assert_eq!(
        inspect_database(&url(Path::new(&backup)))
            .await
            .unwrap()
            .schema_version,
        Some(6)
    );
    let store = open_current_database(&db).await.unwrap();
    let mut transaction = store.begin_reflection_transaction().await.unwrap();
    let baseline = transaction.load_current_self_model_version().await.unwrap();
    assert_eq!(baseline.kind, SelfModelVersionKind::MigrationBaseline);
    assert_eq!(baseline.version, 0);
    assert_eq!(baseline.effective_at, None);
    assert_eq!(baseline.reflection_id, None);
    assert_eq!(
        baseline.identity.canonical_claims(),
        &["重复", "next", "重复"]
    );
    assert_eq!(
        baseline
            .commitments
            .iter()
            .map(|c| c.description())
            .collect::<Vec<_>>(),
        vec![
            "forbid:write_identity_core_directly",
            "z-before-a",
            "a-after-z"
        ]
    );
    assert!(
        transaction
            .load_self_model_version(1)
            .await
            .unwrap()
            .is_none()
    );
    transaction.commit().await.unwrap();
    let mut connection = SqliteConnection::connect(&db).await.unwrap();
    let after: (String,String,String,String) = sqlx::query_as("SELECT o.occurred_at,o.request_summary_json,o.response_summary_json,r.requested_identity_update FROM operation_log o CROSS JOIN reflections r WHERE o.operation_id='receipt-legacy' AND r.reflection_id='historical'")
        .fetch_one(&mut connection).await.unwrap();
    assert_eq!(original, after);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM self_model_versions")
            .fetch_one(&mut connection)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn immutable_guards_reject_update_delete_replace_and_noncontiguous_append() {
    let dir = tempfile::tempdir().unwrap();
    let db = url(&dir.path().join("guards.sqlite"));
    initialize_database(&db).await.unwrap();
    let mut connection = SqliteConnection::connect(&db).await.unwrap();
    for statement in [
        "UPDATE self_model_versions SET recorded_at = 'changed' WHERE version=0",
        "DELETE FROM self_model_versions WHERE version=0",
        "INSERT OR REPLACE INTO self_model_versions SELECT * FROM self_model_versions WHERE version=0",
        "INSERT INTO self_model_versions SELECT 2,1,'update','missing',recorded_at,recorded_at,identity_json,commitments_json,1,0,2,0,NULL FROM self_model_versions WHERE version=0",
    ] {
        assert!(
            sqlx::query(statement)
                .execute(&mut connection)
                .await
                .is_err(),
            "{statement}"
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM self_model_versions")
            .fetch_one(&mut connection)
            .await
            .unwrap(),
        1
    );
    assert!(inspect_database(&db).await.unwrap().is_current());
}

#[tokio::test]
async fn removed_immutable_guard_is_structural_failure_not_reinstalled_by_read_or_migrate() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing-guard.sqlite");
    let db = url(&path);
    initialize_database(&db).await.unwrap();
    let mut connection = SqliteConnection::connect(&db).await.unwrap();
    sqlx::query("DROP TRIGGER self_model_versions_no_update")
        .execute(&mut connection)
        .await
        .unwrap();
    connection.close().await.unwrap();
    let before = fs::read(&path).unwrap();
    let report = inspect_database(&db).await.unwrap();
    assert_eq!(report.status, "schema_structure_invalid");
    assert!(
        report
            .schema_structure_issues
            .iter()
            .any(|issue| issue.contains("self_model_versions_no_update"))
    );
    assert!(open_current_database(&db).await.is_err());
    assert!(migrate_database(&db).await.is_err());
    assert_eq!(before, fs::read(&path).unwrap());
}

#[tokio::test]
async fn drift_fails_closed_and_poisoned_transaction_cannot_commit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("drift.sqlite");
    let db = url(&path);
    initialize_database(&db).await.unwrap();
    let store = open_current_database(&db).await.unwrap();
    let mut connection = SqliteConnection::connect(&db).await.unwrap();
    sqlx::query("UPDATE identity_claims SET claim = 'out-of-band private value'")
        .execute(&mut connection)
        .await
        .unwrap();
    connection.close().await.unwrap();
    let before = fs::read(&path).unwrap();
    let report = inspect_database(&db).await.unwrap();
    assert_eq!(report.status, "self_model_ledger_invalid");
    assert!(!report.message.contains("out-of-band private value"));
    assert!(open_current_database(&db).await.is_err());
    assert!(migrate_database(&db).await.is_err());
    assert_eq!(before, fs::read(&path).unwrap());
    let mut transaction = store.begin_reflection_transaction().await.unwrap();
    assert!(transaction.load_current_self_model_version().await.is_err());
    assert!(
        transaction
            .replace_identity(IdentityCore::new(vec!["attempted repair".into()]))
            .await
            .is_err()
    );
    assert!(transaction.commit().await.is_err());
    // Retrieval repair never rewrites the ledger, including drifted projections.
    assert!(store.rebuild_retrieval_index().await.unwrap().is_usable());
    let mut connection = SqliteConnection::connect(&db).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM self_model_versions")
            .fetch_one(&mut connection)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT claim FROM identity_claims")
            .fetch_one(&mut connection)
            .await
            .unwrap(),
        "out-of-band private value"
    );
}

#[tokio::test]
async fn append_projection_mismatch_rolls_back_prior_projection_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let db = url(&dir.path().join("append-failure.sqlite"));
    initialize_database(&db).await.unwrap();
    let store = open_current_database(&db).await.unwrap();
    let mut transaction = store.begin_reflection_transaction().await.unwrap();
    let head = transaction.load_current_self_model_version().await.unwrap();
    transaction
        .replace_identity(IdentityCore::new(vec!["temporary".into()]))
        .await
        .unwrap();
    let mut attempted = head.clone();
    attempted.version = 1;
    attempted.previous_version = Some(0);
    attempted.kind = SelfModelVersionKind::Update;
    attempted.identity_written = true;
    attempted.identity_source_version = 1;
    assert!(
        transaction
            .append_self_model_version(0, attempted)
            .await
            .is_err()
    );
    assert!(transaction.commit().await.is_err());
    let mut transaction = store.begin_reflection_transaction().await.unwrap();
    assert_eq!(
        transaction.load_current_self_model_version().await.unwrap(),
        head
    );
}

#[tokio::test]
async fn failed_v6_rehearsal_keeps_original_and_backup_without_partial_baseline() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("invalid-v6.sqlite");
    let db = url(&path);
    initialize_database(&db).await.unwrap();
    let mut connection = SqliteConnection::connect(&db).await.unwrap();
    remove_v7(&mut connection).await;
    sqlx::query("INSERT INTO commitments(description,owner) VALUES ('private legacy payload','invalid private owner')")
        .execute(&mut connection).await.unwrap();
    connection.close().await.unwrap();
    let before = fs::read(&path).unwrap();
    let error = migrate_database(&db).await.unwrap_err().to_string();
    assert!(error.contains("unknown stored owner"));
    assert!(!error.contains("invalid private owner"));
    assert_eq!(before, fs::read(&path).unwrap());
    assert_eq!(inspect_database(&db).await.unwrap().schema_version, Some(6));
    let backups = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".bak"))
        .collect::<Vec<_>>();
    assert_eq!(backups.len(), 1);
    let mut connection = SqliteConnection::connect(&db).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM sqlite_master WHERE name='self_model_versions'"
        )
        .fetch_one(&mut connection)
        .await
        .unwrap(),
        0
    );
}

#[tokio::test]
async fn corrupt_snapshot_parse_errors_never_disclose_snapshot_values() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("corrupt-snapshot.sqlite");
    let db = url(&path);
    initialize_database(&db).await.unwrap();
    let mut connection = SqliteConnection::connect(&db).await.unwrap();
    let guard: String = sqlx::query_scalar(
        "SELECT sql FROM sqlite_master WHERE name='self_model_versions_no_update'",
    )
    .fetch_one(&mut connection)
    .await
    .unwrap();
    sqlx::query("DROP TRIGGER self_model_versions_no_update")
        .execute(&mut connection)
        .await
        .unwrap();
    // Test-only tampering outside supported operations, followed by restoring
    // canonical guards, checks data validation independently of structural checks.
    sqlx::query("UPDATE self_model_versions SET commitments_json = ?")
        .bind(r#"[{"owner":"secret-cross-project-owner","description":"secret-cross-project-description"}]"#)
        .execute(&mut connection).await.unwrap();
    sqlx::query(&guard).execute(&mut connection).await.unwrap();
    connection.close().await.unwrap();
    let report = inspect_database(&db).await.unwrap();
    assert_eq!(report.status, "self_model_ledger_invalid");
    assert!(
        report
            .message
            .contains("malformed stored commitments snapshot")
    );
    assert!(!report.message.contains("secret-cross-project"));
}

#[tokio::test]
async fn weakened_version_table_check_foreign_key_and_unique_index_are_structural_failures() {
    for (name, old, replacement) in [
        ("check", "CHECK (version >= 0)", "CHECK (version >= -1)"),
        (
            "foreign-key",
            "    FOREIGN KEY (reflection_id) REFERENCES reflections(reflection_id),\n",
            "",
        ),
        (
            "unique-index",
            "reflection_id TEXT UNIQUE",
            "reflection_id TEXT",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(format!("weakened-{name}.sqlite"));
        let db = url(&path);
        initialize_database(&db).await.unwrap();
        let mut connection = SqliteConnection::connect(&db).await.unwrap();
        let ddl: String = sqlx::query_scalar(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='self_model_versions'",
        )
        .fetch_one(&mut connection)
        .await
        .unwrap();
        assert!(ddl.contains(old));
        let triggers: Vec<String> = sqlx::query_scalar("SELECT sql FROM sqlite_master WHERE type='trigger' AND tbl_name='self_model_versions' ORDER BY name").fetch_all(&mut connection).await.unwrap();
        sqlx::raw_sql(
            "PRAGMA foreign_keys=OFF;
            DROP TRIGGER self_model_versions_append;
            DROP TRIGGER self_model_versions_no_update;
            DROP TRIGGER self_model_versions_no_delete;
            ALTER TABLE self_model_versions RENAME TO test_old_versions;",
        )
        .execute(&mut connection)
        .await
        .unwrap();
        sqlx::query(&ddl.replace(old, replacement))
            .execute(&mut connection)
            .await
            .unwrap();
        sqlx::raw_sql("INSERT INTO self_model_versions SELECT * FROM test_old_versions; DROP TABLE test_old_versions;")
            .execute(&mut connection).await.unwrap();
        for trigger in triggers {
            sqlx::query(&trigger)
                .execute(&mut connection)
                .await
                .unwrap();
        }
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM self_model_versions")
                .fetch_one(&mut connection)
                .await
                .unwrap(),
            1
        );
        connection.close().await.unwrap();
        let before = fs::read(&path).unwrap();
        let report = inspect_database(&db).await.unwrap();
        assert_eq!(report.status, "schema_structure_invalid", "{name}");
        assert!(
            report
                .schema_structure_issues
                .iter()
                .any(|issue| issue.starts_with("self_model_versions:")),
            "{name}"
        );
        assert!(migrate_database(&db).await.is_err(), "{name}");
        assert_eq!(before, fs::read(&path).unwrap(), "{name}");
    }
}

#[tokio::test]
async fn missing_current_baseline_is_never_seeded_by_inspection_migration_or_index_repair() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing-baseline.sqlite");
    let db = url(&path);
    initialize_database(&db).await.unwrap();
    let store = open_current_database(&db).await.unwrap();
    let mut connection = SqliteConnection::connect(&db).await.unwrap();
    let guard: String = sqlx::query_scalar(
        "SELECT sql FROM sqlite_master WHERE name='self_model_versions_no_delete'",
    )
    .fetch_one(&mut connection)
    .await
    .unwrap();
    sqlx::raw_sql("DROP TRIGGER self_model_versions_no_delete; DELETE FROM self_model_versions;")
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query(&guard).execute(&mut connection).await.unwrap();
    connection.close().await.unwrap();
    let before = fs::read(&path).unwrap();
    let report = inspect_database(&db).await.unwrap();
    assert_eq!(report.status, "self_model_ledger_invalid");
    assert!(report.message.contains("missing baseline"));
    assert!(open_current_database(&db).await.is_err());
    assert!(migrate_database(&db).await.is_err());
    assert_eq!(before, fs::read(&path).unwrap());
    assert!(store.rebuild_retrieval_index().await.unwrap().is_usable());
    let mut connection = SqliteConnection::connect(&db).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM self_model_versions")
            .fetch_one(&mut connection)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        inspect_database(&db).await.unwrap().status,
        "self_model_ledger_invalid"
    );
}

#[tokio::test]
async fn insert_or_replace_cannot_delete_an_older_version_through_unique_reflection_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let db = url(&dir.path().join("replace-provenance.sqlite"));
    initialize_database(&db).await.unwrap();
    let mut connection = SqliteConnection::connect(&db).await.unwrap();
    // SQLite's REPLACE deletes conflicting rows without invoking delete triggers
    // when recursive_triggers is off. The insert guard must independently stop
    // this, rather than rely on callers enabling either PRAGMA.
    sqlx::raw_sql("PRAGMA foreign_keys=OFF; PRAGMA recursive_triggers=OFF;
        INSERT INTO reflections(reflection_id,recorded_at,summary) VALUES ('source','2026-01-01T00:00:00Z','fixture');
        INSERT INTO self_model_versions SELECT 1,0,'update','source',recorded_at,recorded_at,identity_json,commitments_json,1,0,1,0,NULL FROM self_model_versions WHERE version=0;")
        .execute(&mut connection).await.unwrap();
    assert!(sqlx::query("INSERT OR REPLACE INTO self_model_versions SELECT 2,1,'update','source',recorded_at,recorded_at,identity_json,commitments_json,1,0,2,0,NULL FROM self_model_versions WHERE version=1")
        .execute(&mut connection).await.is_err());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM self_model_versions")
            .fetch_one(&mut connection)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT max(version) FROM self_model_versions")
            .fetch_one(&mut connection)
            .await
            .unwrap(),
        1
    );
}
