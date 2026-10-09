#[path = "support/legacy_schema6.rs"]
mod legacy_schema;
use agent_llm_mm::{
    adapters::sqlite::{
        initialize_database, inspect_database, migrate_database, open_current_database,
    },
    domain::{
        reflection_scope::ReflectionScopeStatus,
        types::{MemoryScope, Namespace},
    },
    ports::{MemoryReadStore, ReflectionRecordQuery},
};
use sqlx::{Connection, SqliteConnection};
use std::fs;
fn url(path: &std::path::Path) -> String {
    format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"))
}

#[tokio::test]
async fn v5_migration_preserves_raw_unknown_and_quarantines_ambiguous_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v5.sqlite");
    let db = url(&path);
    initialize_database(&db).await.unwrap();
    let mut c = SqliteConnection::connect(&db).await.unwrap();
    sqlx::raw_sql("INSERT INTO events(event_id,recorded_at,owner,namespace,kind,summary) VALUES
 ('a','2026-02-03T08:00:00.999999999+08:00','world','project/a','observation','A'),
 ('b','2026-02-03T00:00:00Z','world','project/b','observation','B'),
 ('bad-time','not-a-time','world','project/a','observation','preserve raw');
 INSERT INTO claims(claim_id,owner,namespace,subject,predicate,object,mode,status) VALUES ('old','world','project/a','s','p','o','observed','active');
 INSERT INTO reflections(reflection_id,recorded_at,summary,supporting_evidence_event_ids) VALUES
 ('known','2026-02-03T00:00:01Z','known source','[\"a\"]'),
 ('mixed','2026-02-03T00:00:02Z','mixed sources','[\"a\",\"b\"]'),
 ('missing','2026-02-03T00:00:03Z','orphan source','[\"gone\"]');")
 .execute(&mut c).await.unwrap();
    legacy_schema::remove_v6_objects(&mut c).await;
    sqlx::raw_sql("DELETE FROM schema_migrations WHERE version>=6;PRAGMA user_version=5;")
        .execute(&mut c)
        .await
        .unwrap();
    c.close().await.unwrap();
    let old_bytes = fs::read(&path).unwrap();
    let old = inspect_database(&db).await.unwrap();
    assert_eq!(old.schema_version, Some(5));
    assert_eq!(fs::read(&path).unwrap(), old_bytes);
    let migrated = migrate_database(&db).await.unwrap();
    assert_eq!(migrated.schema_version, Some(6));
    assert!(migrated.preserved_row_counts);
    assert_eq!(migrated.foreign_key_violations, 0);
    let mut c = SqliteConnection::connect(&db).await.unwrap();
    let raw: (String, i64, i64) = sqlx::query_as(
        "SELECT recorded_at,recorded_at_seconds,recorded_at_nanos FROM events WHERE event_id='a'",
    )
    .fetch_one(&mut c)
    .await
    .unwrap();
    assert_eq!(raw.0, "2026-02-03T08:00:00.999999999+08:00");
    assert_eq!(raw.2, 999999999);
    let claim: (Option<String>, Option<String>) =
        sqlx::query_as("SELECT recorded_at,recorded_at_sort_key FROM claims WHERE claim_id='old'")
            .fetch_one(&mut c)
            .await
            .unwrap();
    assert_eq!(claim, (None, None));
    let invalid: (String, Option<String>) = sqlx::query_as(
        "SELECT recorded_at,recorded_at_sort_key FROM events WHERE event_id='bad-time'",
    )
    .fetch_one(&mut c)
    .await
    .unwrap();
    assert_eq!(invalid, ("not-a-time".into(), None));
    let orphan:(String,i64)=sqlx::query_as("SELECT supporting_evidence_event_ids,evidence_normalized FROM reflections WHERE reflection_id='missing'").fetch_one(&mut c).await.unwrap();
    assert_eq!(orphan, ("[\"gone\"]".into(), 0));
    c.close().await.unwrap();
    let store = open_current_database(&db).await.unwrap();
    let rows = store
        .query_reflection_records(ReflectionRecordQuery {
            scope: MemoryScope::for_namespace(Namespace::parse("project/a").unwrap()),
            reflection_reference: None,
            limit: 100,
        })
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].reflection_id, "known");
    assert_eq!(
        rows[0].scope.status,
        ReflectionScopeStatus::LegacyUnambiguous
    );
    assert!(store.inspect_retrieval_index().await.unwrap().is_usable());
    let restored = dir.path().join("restored.sqlite");
    fs::copy(migrated.backup_path.unwrap(), &restored).unwrap();
    let restored = url(&restored);
    assert_eq!(
        inspect_database(&restored).await.unwrap().schema_version,
        Some(5)
    );
    assert!(migrate_database(&restored).await.unwrap().is_current());
    let restored_store = open_current_database(&restored).await.unwrap();
    assert!(
        restored_store
            .inspect_retrieval_index()
            .await
            .unwrap()
            .is_usable()
    );
}
