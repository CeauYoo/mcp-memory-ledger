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

mod v5_retry_compatibility {
    use super::{legacy_schema, url};
    use agent_llm_mm::{
        adapters::sqlite::{
            SqliteStore, inspect_database, migrate_database, open_current_database,
        },
        application::{
            feedback_candidate::{
                self, FeedbackCandidateActionInput, ProposeFeedbackCandidateInput,
            },
            ingest_interaction::{self, IngestInput, IngestResult},
        },
        domain::{
            claim::{ClaimDraft, ClaimReference},
            event::{Event, EventReference},
            feedback::{FeedbackMetadata, FeedbackSourceKind, FeedbackVerificationResult},
            feedback_candidate::{FeedbackCandidate, FeedbackCandidateState, claim_version},
            types::{EventKind, MemoryScope, Mode, Namespace, Owner},
        },
        error::AppError,
        ports::{
            ClaimRecordQuery, ClaimStatus, Clock, EventStore, EvidenceQuery, IdGenerator,
            IngestTransaction, IngestTransactionRunner, MemoryReadStore, ReflectionTransaction,
            ReflectionTransactionRunner, StoredClaim, StoredEvent, WriteReceiptRequest,
            feedback_candidate_store::FeedbackCandidateStore, write_receipt::receipt_result,
        },
    };
    use async_trait::async_trait;
    use chrono::{DateTime, Utc};
    use serde::Serialize;
    use sha2::{Digest, Sha256};
    use sqlx::{Connection, SqliteConnection};
    use std::sync::atomic::{AtomicU64, Ordering};

    const EVENT_ID: &str = "v5-ingest-event";
    const CLAIM_ID: &str = "v5-ingest-event:claim:0";
    const RAW_TIME: &str = "2020-01-02T11:04:05.123456789+08:00";

    fn namespace() -> Namespace {
        Namespace::parse("project/v5-retry-compatibility").unwrap()
    }
    fn timestamp(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&Utc)
    }
    fn event() -> Event {
        Event::new_with_namespace(
            Owner::World,
            namespace(),
            EventKind::Observation,
            "Original setting observation",
        )
        .unwrap()
    }
    fn draft() -> ClaimDraft {
        ClaimDraft::new_with_namespace(
            Owner::World,
            namespace(),
            "setting",
            "is",
            "old",
            Mode::Observed,
        )
    }
    struct Deps {
        store: SqliteStore,
        ids: AtomicU64,
    }
    #[async_trait]
    impl Clock for Deps {
        async fn now(&self) -> Result<DateTime<Utc>, AppError> {
            Ok(timestamp("2030-01-01T00:00:00.999999999Z"))
        }
    }
    #[async_trait]
    impl IdGenerator for Deps {
        async fn next_id(&self) -> Result<String, AppError> {
            Ok(format!(
                "compat-{}",
                self.ids.fetch_add(1, Ordering::SeqCst)
            ))
        }
    }
    #[async_trait]
    impl IngestTransactionRunner for Deps {
        async fn begin_ingest_transaction(
            &self,
        ) -> Result<Box<dyn IngestTransaction + Send + '_>, AppError> {
            self.store.begin_ingest_transaction().await
        }
    }
    #[async_trait]
    impl ReflectionTransactionRunner for Deps {
        async fn begin_reflection_transaction(
            &self,
        ) -> Result<Box<dyn ReflectionTransaction + Send + '_>, AppError> {
            self.store.begin_reflection_transaction().await
        }
    }
    #[async_trait]
    impl EventStore for Deps {
        async fn append_event(&self, event: StoredEvent) -> Result<(), AppError> {
            self.store.append_event(event).await
        }
        async fn list_event_references(&self) -> Result<Vec<String>, AppError> {
            self.store.list_event_references().await
        }
        async fn list_recorded_at_for_snapshot_manifest(
            &self,
            scope: &MemoryScope,
            manifest: &[EventReference],
        ) -> Result<Vec<DateTime<Utc>>, AppError> {
            self.store
                .list_recorded_at_for_snapshot_manifest(scope, manifest)
                .await
        }
        async fn query_evidence_event_ids(
            &self,
            query: EvidenceQuery,
        ) -> Result<Vec<String>, AppError> {
            self.store.query_evidence_event_ids(query).await
        }
        async fn query_evidence_event_ids_unbounded(
            &self,
            query: EvidenceQuery,
        ) -> Result<Vec<String>, AppError> {
            self.store.query_evidence_event_ids_unbounded(query).await
        }
        async fn has_event(&self, id: &str) -> Result<bool, AppError> {
            self.store.has_event(id).await
        }
    }
    #[async_trait]
    impl FeedbackCandidateStore for Deps {
        async fn get_feedback_candidate(
            &self,
            namespace: &Namespace,
            id: &str,
        ) -> Result<Option<FeedbackCandidate>, AppError> {
            self.store.get_feedback_candidate(namespace, id).await
        }
    }
    async fn receipts(connection: &mut SqliteConnection) -> Vec<(String, String, String)> {
        sqlx::query_as("SELECT operation_id,request_summary_json,response_summary_json FROM operation_log WHERE actor_id='durable_write_receipt_v1' ORDER BY operation_id")
            .fetch_all(connection).await.unwrap()
    }
    async fn counts(connection: &mut SqliteConnection) -> (i64, i64, i64, i64, i64, i64) {
        sqlx::query_as("SELECT (SELECT count(*) FROM events),(SELECT count(*) FROM claims),(SELECT count(*) FROM evidence_links),(SELECT count(*) FROM reflections),(SELECT count(*) FROM feedback_candidates),(SELECT count(*) FROM operation_log WHERE actor_id='durable_write_receipt_v1')")
            .fetch_one(connection).await.unwrap()
    }

    #[tokio::test]
    async fn real_v5_migration_preserves_receipts_pending_candidate_fingerprint_and_replay() {
        let dir = tempfile::tempdir().unwrap();
        let db = url(&dir.path().join("v5-retry.sqlite"));
        let deps = Deps {
            store: SqliteStore::bootstrap(&db).await.unwrap(),
            ids: AtomicU64::new(0),
        };
        let input =
            IngestInput::new(event(), vec![draft()], None).with_request_id("v5-ingest-key".into());
        // Construct the original v5 four-tuple receipt directly, independently
        // of the current ingest implementation and its additive metadata.
        let old_payload = (event(), vec![draft()], None::<String>, Vec::<String>::new());
        let request = WriteReceiptRequest::new(
            "ingest",
            namespace().as_str(),
            "v5-ingest-key",
            &old_payload,
        )
        .unwrap();
        assert_eq!(
            request.request_hash,
            format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&old_payload).unwrap())
            )
        );
        let original = IngestResult {
            event_id: EVENT_ID.into(),
            replayed: false,
        };
        let target = StoredClaim::new(CLAIM_ID.into(), draft(), ClaimStatus::Active);
        // This struct is the exact pre-v6 fingerprint payload, including field
        // order. Do not compute the historical token with the new helper alone.
        #[derive(Serialize)]
        struct LegacyClaim<'a> {
            claim_id: &'a str,
            claim: &'a ClaimDraft,
            status: ClaimStatus,
        }
        let old_fingerprint = format!(
            "claim-version:v1:{:x}",
            Sha256::digest(
                serde_json::to_vec(&LegacyClaim {
                    claim_id: CLAIM_ID,
                    claim: &target.claim,
                    status: target.status,
                })
                .unwrap()
            )
        );
        assert_eq!(claim_version(&target).unwrap(), old_fingerprint);
        let mut tx = deps.store.begin_ingest_transaction().await.unwrap();
        tx.append_event(StoredEvent::new(
            EVENT_ID.into(),
            timestamp(RAW_TIME),
            event(),
        ))
        .await
        .unwrap();
        tx.upsert_claim(target).await.unwrap();
        tx.link_evidence(CLAIM_ID.into(), EVENT_ID.into())
            .await
            .unwrap();
        tx.append_write_receipt(
            &request,
            receipt_result(&request, &original).unwrap(),
            timestamp(RAW_TIME),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        let feedback = FeedbackMetadata {
            source_kind: FeedbackSourceKind::ToolReported,
            producer: "synthetic-migration-test".into(),
            observed_target: format!("claim:{CLAIM_ID}"),
            observed_version: Some(old_fingerprint.clone()),
            expected: "old".into(),
            actual: "new".into(),
            verification_method: "fixture".into(),
            verification_result: FeedbackVerificationResult::Failed,
            limitations: vec![],
            evidence_refs: vec![],
        };
        deps.store
            .append_event(StoredEvent::new(
                "v5-feedback".into(),
                timestamp(RAW_TIME),
                event().with_feedback(feedback).unwrap(),
            ))
            .await
            .unwrap();
        let proposal = ProposeFeedbackCandidateInput {
            namespace: namespace(),
            target_claim_reference: ClaimReference::parse(format!("claim:{CLAIM_ID}")).unwrap(),
            expected_target_version: old_fingerprint.clone(),
            replacement_object: "new".into(),
            evidence_event_ids: vec![EventReference::parse("event:v5-feedback").unwrap()],
            summary: "Correct the synthetic setting".into(),
            request_id: "v5-proposal-key".into(),
        };
        let candidate = feedback_candidate::propose(&deps, proposal.clone())
            .await
            .unwrap();
        assert_eq!(candidate.state, FeedbackCandidateState::Proposed);
        let mut connection = SqliteConnection::connect(&db).await.unwrap();
        sqlx::query("UPDATE events SET recorded_at=? WHERE event_id=?")
            .bind(RAW_TIME)
            .bind(EVENT_ID)
            .execute(&mut connection)
            .await
            .unwrap();
        let before_receipts = receipts(&mut connection).await;
        let before_candidate: (String, String) = sqlx::query_as("SELECT expected_target_version,candidate_json FROM feedback_candidates WHERE candidate_id=?")
            .bind(&candidate.candidate_id).fetch_one(&mut connection).await.unwrap();
        let before_feedback: String =
            sqlx::query_scalar("SELECT feedback_json FROM events WHERE event_id='v5-feedback'")
                .fetch_one(&mut connection)
                .await
                .unwrap();
        let before_counts = counts(&mut connection).await;
        assert_eq!(before_counts, (2, 1, 1, 0, 1, 2));
        // Remove actual v6 columns/relations/triggers; merely changing user_version
        // would not exercise the real v5 table rebuild and compatibility boundary.
        legacy_schema::remove_v6_objects(&mut connection).await;
        sqlx::raw_sql("DELETE FROM schema_migrations WHERE version=6; PRAGMA user_version=5;")
            .execute(&mut connection)
            .await
            .unwrap();
        let temporal_columns: i64 = sqlx::query_scalar("SELECT count(*) FROM pragma_table_info('claims') WHERE name IN ('recorded_at','observed_at')").fetch_one(&mut connection).await.unwrap();
        assert_eq!(temporal_columns, 0);
        connection.close().await.unwrap();
        drop(deps);
        assert_eq!(inspect_database(&db).await.unwrap().schema_version, Some(5));
        let migration = migrate_database(&db).await.unwrap();
        assert!(migration.is_current() && migration.preserved_row_counts);
        assert_eq!(migration.restore_rehearsal, "passed_before_original_write");
        assert!(migration.backup_path.is_some());
        let mut connection = SqliteConnection::connect(&db).await.unwrap();
        assert_eq!(receipts(&mut connection).await, before_receipts);
        assert_eq!(counts(&mut connection).await, before_counts);
        let migrated_candidate: (String, String) = sqlx::query_as("SELECT expected_target_version,candidate_json FROM feedback_candidates WHERE candidate_id=?")
            .bind(&candidate.candidate_id).fetch_one(&mut connection).await.unwrap();
        assert_eq!(migrated_candidate, before_candidate);
        let migrated_feedback: String =
            sqlx::query_scalar("SELECT feedback_json FROM events WHERE event_id='v5-feedback'")
                .fetch_one(&mut connection)
                .await
                .unwrap();
        assert_eq!(migrated_feedback, before_feedback);
        let raw: String = sqlx::query_scalar("SELECT recorded_at FROM events WHERE event_id=?")
            .bind(EVENT_ID)
            .fetch_one(&mut connection)
            .await
            .unwrap();
        assert_eq!(raw, RAW_TIME);
        let deps = Deps {
            store: open_current_database(&db).await.unwrap(),
            ids: AtomicU64::new(100),
        };
        for _ in 0..2 {
            let replay = ingest_interaction::execute(&deps, input.clone())
                .await
                .unwrap();
            assert!(replay.replayed);
            assert_eq!(replay.event_id, original.event_id);
        }
        assert_eq!(
            feedback_candidate::propose(&deps, proposal).await.unwrap(),
            candidate
        );
        assert_eq!(counts(&mut connection).await, before_counts);
        assert_eq!(receipts(&mut connection).await, before_receipts);
        let claim = deps
            .store
            .query_claim_records(ClaimRecordQuery {
                scope: MemoryScope::for_namespace(namespace()),
                claim_reference: Some(ClaimReference::parse(format!("claim:{CLAIM_ID}")).unwrap()),
                status: None,
                mode: None,
                limit: 1,
            })
            .await
            .unwrap()
            .remove(0)
            .claim;
        assert_eq!(claim.recorded_at, None);
        assert_eq!(claim.observed_at, None);
        assert_eq!(claim_version(&claim).unwrap(), old_fingerprint);
        assert_eq!(
            deps.get_feedback_candidate(&namespace(), &candidate.candidate_id)
                .await
                .unwrap()
                .unwrap(),
            candidate
        );
        let action = |key: &str| FeedbackCandidateActionInput {
            namespace: namespace(),
            candidate_id: candidate.candidate_id.clone(),
            request_id: key.into(),
        };
        let validated = feedback_candidate::validate(&deps, action("post-v6-validate"))
            .await
            .unwrap();
        assert_eq!(validated.state, FeedbackCandidateState::Validated);
        assert!(validated.validation.passed);
        assert_eq!(validated.proposal.expected_target_version, old_fingerprint);
        let committed = feedback_candidate::commit(&deps, action("post-v6-commit"))
            .await
            .unwrap();
        assert_eq!(committed.state, FeedbackCandidateState::Committed);
        assert_eq!(committed.proposal.expected_target_version, old_fingerprint);
        assert!(committed.reflection_id.is_some() && committed.replacement_claim_id.is_some());
        let committed_counts = counts(&mut connection).await;
        assert_eq!(committed_counts, (2, 2, 2, 1, 1, 4));
        assert_eq!(
            feedback_candidate::commit(&deps, action("post-v6-commit"))
                .await
                .unwrap(),
            committed
        );
        assert_eq!(counts(&mut connection).await, committed_counts);
        let target_status: String =
            sqlx::query_scalar("SELECT status FROM claims WHERE claim_id=?")
                .bind(CLAIM_ID)
                .fetch_one(&mut connection)
                .await
                .unwrap();
        assert_eq!(target_status, "superseded");
        let replacement: (String, Option<String>) =
            sqlx::query_as("SELECT object,recorded_at FROM claims WHERE claim_id=?")
                .bind(committed.replacement_claim_id.unwrap())
                .fetch_one(&mut connection)
                .await
                .unwrap();
        assert_eq!(replacement.0, "new");
        assert!(replacement.1.is_some());
        let final_receipts = receipts(&mut connection).await;
        for receipt in before_receipts {
            assert!(
                final_receipts.contains(&receipt),
                "migration or replay rewrote an original v5 receipt"
            );
        }
        let final_raw: String =
            sqlx::query_scalar("SELECT recorded_at FROM events WHERE event_id=?")
                .bind(EVENT_ID)
                .fetch_one(&mut connection)
                .await
                .unwrap();
        assert_eq!(final_raw, RAW_TIME);
    }
}
