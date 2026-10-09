#[path = "legacy_schema6.rs"]
mod version6;
pub use version6::remove_v6_objects;

// Test-only removal of additive v5 objects before constructing real old-version fixtures.
// Never used by application migration or recovery.
pub async fn remove_v5_objects(connection: &mut sqlx::SqliteConnection) {
    remove_v6_objects(connection).await;
    for trigger in [
        "text_recall_events_ai",
        "text_recall_events_ad",
        "text_recall_events_au",
        "text_recall_claims_ai",
        "text_recall_claims_ad",
        "text_recall_claims_au",
        "text_recall_documents_ai",
        "text_recall_documents_ad",
        "text_recall_documents_au",
    ] {
        sqlx::query(&format!("DROP TRIGGER IF EXISTS {trigger}"))
            .execute(&mut *connection)
            .await
            .unwrap();
    }
    for table in [
        "text_recall_vocab",
        "text_recall_fts",
        "text_recall_documents",
        "feedback_candidates",
        "experience_candidate_sources",
        "experience_candidate_versions",
        "experience_candidates",
        "experience_episode_sources",
        "experience_episodes",
    ] {
        sqlx::query(&format!("DROP TABLE IF EXISTS {table}"))
            .execute(&mut *connection)
            .await
            .unwrap();
    }
    for index in [
        "idx_recall_events_scope",
        "idx_recall_claims_scope_status",
        "idx_recall_evidence_event",
        "idx_recall_episode_event",
    ] {
        sqlx::query(&format!("DROP INDEX IF EXISTS {index}"))
            .execute(&mut *connection)
            .await
            .unwrap();
    }
}
