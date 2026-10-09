use agent_llm_mm::{
    adapters::sqlite::{SqliteStore, open_read_only_current_database},
    application::{experience, export_memory::export_memory},
    domain::{
        claim::ClaimDraft,
        event::Event,
        experience::*,
        ledger_export::*,
        types::{EventKind, Mode, Namespace, Owner},
    },
    ports::{ClaimStatus, ClaimStore, EventStore, StoredClaim, StoredEvent},
};
use chrono::{TimeZone, Utc};
use sqlx::SqlitePool;

const NS: &str = "project/alpha";
struct Fixture {
    store: SqliteStore,
    pool: SqlitePool,
    url: String,
    _dir: tempfile::TempDir,
}
impl Fixture {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}", dir.path().join("ledger.sqlite").display());
        let store = SqliteStore::bootstrap(&url).await.unwrap();
        let pool = SqlitePool::connect(&url).await.unwrap();
        let f = Self {
            store,
            pool,
            url,
            _dir: dir,
        };
        for (id, ns, owner) in [
            ("a", NS, Owner::World),
            ("z", NS, Owner::World),
            ("foreign-secret-event", "project/beta", Owner::World),
            ("unknown-secret-event", NS, Owner::Unknown),
        ] {
            f.store
                .append_event(StoredEvent::new(
                    id.into(),
                    Utc.with_ymd_and_hms(2026, 10, 9, 9, 0, 0).unwrap(),
                    Event::new_with_namespace(
                        owner,
                        Namespace::parse(ns).unwrap(),
                        EventKind::Observation,
                        format!("summary-{id} 雪\n\"quoted\"\\"),
                    )
                    .unwrap(),
                ))
                .await
                .unwrap();
        }
        for (id, ns) in [("c", NS), ("foreign-secret-claim", "project/beta")] {
            f.store
                .upsert_claim(StoredClaim::new(
                    id.into(),
                    ClaimDraft::new_with_namespace(
                        Owner::World,
                        Namespace::parse(ns).unwrap(),
                        "subject",
                        "predicate",
                        "object",
                        Mode::Observed,
                    ),
                    ClaimStatus::Active,
                ))
                .await
                .unwrap();
        }
        f.store.link_evidence("c".into(), "a".into()).await.unwrap();
        f
    }
    async fn export(&self) -> LedgerExport {
        export_memory(&self.store, ExportMemoryRequest::new(NS))
            .await
            .unwrap()
    }
    async fn reflection(&self, id: &str, evidence: &[&str], affected: &str, status: &str) {
        sqlx::query("INSERT INTO reflections(reflection_id,recorded_at,summary,superseded_claim_id,supporting_evidence_event_ids,scope_status,evidence_normalized) VALUES (?, '2026-10-09T09:00:00Z', ?, 'c', ?, ?, 1)")
            .bind(id).bind(format!("summary-{id}")).bind(serde_json::to_string(evidence).unwrap()).bind(status).execute(&self.pool).await.unwrap();
        for (role, ns) in [("origin", NS), ("affected", affected)] {
            let owner = if ns == "self" { "self" } else { "world" };
            sqlx::query("INSERT INTO reflection_scopes(reflection_id,role,owner,namespace) VALUES (?,?,?,?)").bind(id).bind(role).bind(owner).bind(ns).execute(&self.pool).await.unwrap();
        }
        for (position, id_event) in evidence.iter().enumerate() {
            sqlx::query(
                "INSERT INTO reflection_evidence(reflection_id,event_id,position) VALUES (?,?,?)",
            )
            .bind(id)
            .bind(id_event)
            .bind(position as i64)
            .execute(&self.pool)
            .await
            .unwrap();
        }
    }
    async fn experience(&self) {
        experience::create_episode(
            &self.store,
            CreateEpisodeRequest {
                namespace: NS.into(),
                request_id: "private-retry-episode".into(),
                episode_id: "episode".into(),
                content: EpisodeContent {
                    title: "Test".into(),
                    objective: "Check".into(),
                    actions: vec!["Run".into()],
                    observations: vec!["Pass".into()],
                    outcome: "Pass".into(),
                    lesson: "Learn".into(),
                    limitations: vec![],
                    source_event_refs: vec!["event:a".into()],
                },
            },
        )
        .await
        .unwrap();
        experience::create_candidate(
            &self.store,
            CreateCandidateRequest {
                namespace: NS.into(),
                request_id: "private-retry-candidate".into(),
                candidate_id: "candidate".into(),
                content: ExperienceContent {
                    kind: ExperienceKind::Semantic,
                    title: "Lesson".into(),
                    statement: "Original".into(),
                    steps: vec![],
                    limitations: vec![],
                    source_episode_ids: vec!["episode".into()],
                },
            },
        )
        .await
        .unwrap();
        experience::update_candidate_status(
            &self.store,
            UpdateCandidateStatusRequest {
                namespace: NS.into(),
                request_id: "private-retry-activate".into(),
                candidate_id: "candidate".into(),
                expected_version: 1,
                status: ExperienceStatus::Active,
            },
        )
        .await
        .unwrap();
        experience::rollback_candidate(
            &self.store,
            RollbackCandidateRequest {
                namespace: NS.into(),
                request_id: "private-retry-rollback".into(),
                candidate_id: "candidate".into(),
                expected_version: 2,
                target_version: 1,
            },
        )
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn explicit_scope_omits_unknown_mixed_relations_and_global_audit_material() {
    let f = Fixture::new().await;
    f.store
        .link_evidence("c".into(), "foreign-secret-event".into())
        .await
        .unwrap();
    f.store
        .link_evidence("foreign-secret-claim".into(), "a".into())
        .await
        .unwrap();
    for (episode, event) in [
        ("safe-episode", "a"),
        ("mixed-secret-episode", "a"),
        ("mixed-secret-episode", "foreign-secret-event"),
    ] {
        sqlx::query("INSERT INTO episode_events(episode_reference,event_id) VALUES (?,?)")
            .bind(episode)
            .bind(event)
            .execute(&f.pool)
            .await
            .unwrap();
    }
    // Keep one independently complete Claim/Reflection while the mixed-source
    // Claim is quarantined in full.
    sqlx::query("INSERT INTO claims(claim_id,owner,namespace,subject,predicate,object,mode,status) SELECT 'safe-claim',owner,namespace,subject,predicate,object,mode,status FROM claims WHERE claim_id='c'")
        .execute(&f.pool).await.unwrap();
    f.store
        .link_evidence("safe-claim".into(), "z".into())
        .await
        .unwrap();
    f.reflection("safe-reflection", &["a"], NS, "verified")
        .await;
    sqlx::query("UPDATE reflections SET superseded_claim_id='safe-claim' WHERE reflection_id='safe-reflection'")
        .execute(&f.pool).await.unwrap();
    f.reflection(
        "mixed-secret-reflection",
        &["a", "foreign-secret-event"],
        NS,
        "verified",
    )
    .await;
    f.reflection("effect-secret-reflection", &["a"], "self", "verified")
        .await;
    f.reflection("unknown-secret-reflection", &["a"], NS, "unknown")
        .await;
    sqlx::query("INSERT INTO operation_log(operation_id,occurred_at,namespace,actor_kind,actor_id,entrypoint,operation_kind,status,request_summary_json) VALUES ('secret-global-hash','2026-10-09T09:00:00Z',?,'caller','durable_write_receipt_v1','mcp','ingest','ok','secret-private-hash')").bind(NS).execute(&f.pool).await.unwrap();
    let export = f.export().await;
    validate_export(&export).unwrap();
    assert_eq!(export.events.len(), 2);
    assert_eq!(export.claims.len(), 1);
    assert_eq!(export.claims[0].claim_id, "safe-claim");
    assert_eq!(export.evidence_links.len(), 1);
    assert_eq!(export.evidence_links[0].claim_id, "safe-claim");
    assert_eq!(export.episode_memberships.len(), 1);
    assert_eq!(export.reflections.len(), 1);
    let serialized = serde_json::to_string(&export).unwrap();
    for marker in [
        "foreign-secret",
        "unknown-secret",
        "mixed-secret",
        "effect-secret",
        "secret-global-hash",
        "secret-private-hash",
    ] {
        assert!(!serialized.contains(marker), "leaked {marker}");
    }
    assert!(!export.replayable_backup);
    assert!(export.claims[0].recorded_at.is_none());
    assert!(
        export_memory(&f.store, ExportMemoryRequest::new(""))
            .await
            .is_err()
    );
    assert!(
        serde_json::from_value::<ExportMemoryRequest>(
            serde_json::json!({"namespace":NS,"owner":"unknown"})
        )
        .is_err()
    );
}

#[tokio::test]
async fn exact_full_response_budget_stability_and_read_only_no_mutations() {
    let f = Fixture::new().await;
    f.experience().await;
    let mut observer = f.pool.acquire().await.unwrap();
    let before: i64 = sqlx::query_scalar("PRAGMA data_version")
        .fetch_one(&mut *observer)
        .await
        .unwrap();
    let first = f.export().await;
    let second = f.export().await;
    assert_eq!(first, second);
    assert_eq!(serde_json::to_vec(&first).unwrap().len(), first.used_bytes);
    let mut exact = ExportMemoryRequest::new(NS);
    exact.max_bytes = first.used_bytes;
    assert_eq!(export_memory(&f.store, exact.clone()).await.unwrap(), first);
    exact.max_bytes -= 1;
    assert!(
        export_memory(&f.store, exact)
            .await
            .unwrap_err()
            .to_string()
            .contains("budget")
    );
    let read_only = open_read_only_current_database(&f.url)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        export_memory(&read_only, ExportMemoryRequest::new(NS))
            .await
            .unwrap(),
        first
    );
    let after: i64 = sqlx::query_scalar("PRAGMA data_version")
        .fetch_one(&mut *observer)
        .await
        .unwrap();
    assert_eq!(
        before, after,
        "export must not append operation logs or rebuild indexes"
    );
    let mut records = ExportMemoryRequest::new(NS);
    records.max_records = 1;
    assert!(export_memory(&f.store, records).await.is_err());
    let mut relations = ExportMemoryRequest::new(NS);
    relations.max_relations = 1;
    assert!(export_memory(&f.store, relations).await.is_err());
}

#[tokio::test]
async fn oversized_source_rows_fail_before_materialization_and_never_truncate() {
    let f = Fixture::new().await;
    sqlx::query("UPDATE events SET summary=? WHERE event_id='a'")
        .bind("X".repeat(2 * 1024 * 1024))
        .execute(&f.pool)
        .await
        .unwrap();
    let err = export_memory(&f.store, ExportMemoryRequest::new(NS))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("budget"));
    assert!(!err.contains("XXXX"));
    // An arbitrarily large foreign row is never loaded and does not consume this scope's budget.
    sqlx::query("UPDATE events SET summary='short' WHERE event_id='a'")
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE events SET summary=? WHERE event_id='foreign-secret-event'")
        .bind("foreign-secret".repeat(200_000))
        .execute(&f.pool)
        .await
        .unwrap();
    f.export().await;
}

#[tokio::test]
async fn complete_experience_history_sources_and_temporal_metadata_survive() {
    let f = Fixture::new().await;
    f.experience().await;
    sqlx::query("UPDATE events SET observed_at='2025-01-01T01:00:00+01:00' WHERE event_id='a'")
        .execute(&f.pool)
        .await
        .unwrap();
    let export = f.export().await;
    assert_eq!(export.experience_heads[0].current_version, 3);
    assert_eq!(export.experience_versions.len(), 3);
    assert_eq!(
        export.experience_versions[1].status,
        ExperienceStatus::Active
    );
    assert_eq!(
        export.experience_versions[2].status,
        ExperienceStatus::Pending
    );
    assert_eq!(
        export.experience_versions[2].rollback_target_version,
        Some(1)
    );
    assert_eq!(
        export.experience_episodes[0].content.source_event_refs,
        vec!["event:a"]
    );
    assert_eq!(
        export.events[0].observed_at.as_deref(),
        Some("2025-01-01T01:00:00+01:00")
    );
    let serialized = serde_json::to_string(&export).unwrap();
    assert!(!serialized.contains("private-retry"));
    assert!(!serialized.contains("request_hash"));
    assert!(!serialized.contains("operation_id"));
    let decoded: LedgerExport = serde_json::from_str(&serialized).unwrap();
    validate_export(&decoded).unwrap();
    // External corruption of even one historical dependency quarantines the
    // candidate and every version, avoiding a misleading partial history.
    sqlx::query("UPDATE experience_candidate_versions SET payload_json=json_set(payload_json,'$.content.source_episode_ids[0]','foreign-secret-episode') WHERE candidate_id='candidate' AND version=1").execute(&f.pool).await.unwrap();
    let safe = f.export().await;
    assert!(safe.experience_heads.is_empty());
    assert!(safe.experience_versions.is_empty());
    assert!(
        !serde_json::to_string(&safe)
            .unwrap()
            .contains("foreign-secret")
    );
}

#[tokio::test]
async fn mixed_feedback_source_quarantines_event_and_dependent_experience() {
    let f = Fixture::new().await;
    f.experience().await;
    let metadata = serde_json::json!({"source_kind":"caller_reported","producer":"caller","observed_target":"build","expected":"pass","actual":"pass","verification_method":"test","verification_result":"passed","limitations":[],"evidence_refs":["event:foreign-secret-event"]});
    sqlx::query("UPDATE events SET feedback_json=? WHERE event_id='a'")
        .bind(metadata.to_string())
        .execute(&f.pool)
        .await
        .unwrap();
    let export = f.export().await;
    assert_eq!(
        export
            .events
            .iter()
            .map(|e| e.event_id.as_str())
            .collect::<Vec<_>>(),
        vec!["z"]
    );
    assert!(
        export.claims.is_empty(),
        "a Claim must not survive its quarantined source Event"
    );
    assert!(export.evidence_links.is_empty());
    assert!(export.experience_episodes.is_empty());
    assert!(export.experience_versions.is_empty());
    assert!(
        !serde_json::to_string(&export)
            .unwrap()
            .contains("foreign-secret")
    );
}

#[tokio::test]
async fn structural_validation_rejects_duplicate_records_and_dangling_references() {
    let f = Fixture::new().await;
    let mut export = f.export().await;
    export.events.push(export.events[0].clone());
    (export.record_count, export.relation_count) = export.counts();
    loop {
        let size = serde_json::to_vec(&export).unwrap().len();
        if size == export.used_bytes {
            break;
        }
        export.used_bytes = size;
    }
    assert!(validate_export(&export).is_err());
    let mut export = f.export().await;
    export.evidence_links[0].event_id = "missing".into();
    // Recompute the public envelope to ensure failure is graph validation,
    // rather than merely detecting the stale byte-size declaration.
    loop {
        let size = serde_json::to_vec(&export).unwrap().len();
        if size == export.used_bytes {
            break;
        }
        export.used_bytes = size;
    }
    assert!(validate_export(&export).is_err());
}

#[tokio::test]
async fn concurrent_writes_never_mix_rows_from_different_snapshots() {
    let f = Fixture::new().await;
    sqlx::query("PRAGMA journal_mode=WAL")
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE events SET summary='0' WHERE event_id='a'")
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE claims SET object='0' WHERE claim_id='c'")
        .execute(&f.pool)
        .await
        .unwrap();
    let pool = f.pool.clone();
    let writer = tokio::spawn(async move {
        for generation in 1..=100 {
            let mut tx = pool.begin().await.unwrap();
            sqlx::query("UPDATE events SET summary=? WHERE event_id='a'")
                .bind(generation.to_string())
                .execute(&mut *tx)
                .await
                .unwrap();
            sqlx::query("UPDATE claims SET object=? WHERE claim_id='c'")
                .bind(generation.to_string())
                .execute(&mut *tx)
                .await
                .unwrap();
            tx.commit().await.unwrap();
            tokio::task::yield_now().await;
        }
    });
    for _ in 0..20 {
        let export = f.export().await;
        assert_eq!(
            export.events[0].event.summary(),
            export.claims[0].claim.object()
        );
    }
    writer.await.unwrap();
}

#[tokio::test]
async fn canonical_feedback_target_cannot_leak_foreign_claim_hash_or_dependent_events() {
    let f = Fixture::new().await;
    let mut metadata = serde_json::json!({"source_kind":"caller_reported","producer":"caller","observed_target":"claim:foreign-secret-claim","observed_version":"foreign-secret-version-hash","expected":"pass","actual":"pass","verification_method":"test","verification_result":"passed","limitations":[],"evidence_refs":[]});
    sqlx::query("UPDATE events SET feedback_json=? WHERE event_id='a'")
        .bind(metadata.to_string())
        .execute(&f.pool)
        .await
        .unwrap();
    metadata["observed_target"] = serde_json::json!("event:a");
    metadata["observed_version"] = serde_json::json!("dependent-version");
    sqlx::query("UPDATE events SET feedback_json=? WHERE event_id='z'")
        .bind(metadata.to_string())
        .execute(&f.pool)
        .await
        .unwrap();
    let export = f.export().await;
    assert!(export.events.is_empty());
    let json = serde_json::to_string(&export).unwrap();
    assert!(!json.contains("foreign-secret"));
    assert!(!json.contains("dependent-version"));
}

#[tokio::test]
async fn mixed_claim_evidence_closes_feedback_and_experience_without_erasing_source_less_claims() {
    for unsafe_source in ["foreign-secret-event", "unknown-secret-event"] {
        let f = Fixture::new().await;
        f.experience().await;
        f.store
            .link_evidence("c".into(), unsafe_source.into())
            .await
            .unwrap();
        for (id, evidence) in [("source-less", None), ("downstream", Some("z"))] {
            f.store
                .upsert_claim(StoredClaim::new(
                    id.into(),
                    ClaimDraft::new_with_namespace(
                        Owner::World,
                        Namespace::parse(NS).unwrap(),
                        "subject",
                        "predicate",
                        "object",
                        Mode::Observed,
                    ),
                    ClaimStatus::Active,
                ))
                .await
                .unwrap();
            if let Some(evidence) = evidence {
                f.store
                    .link_evidence(id.into(), evidence.into())
                    .await
                    .unwrap();
            }
        }
        // The in-scope Event/Claim cycle is safe only if every original source
        // closes. The extra foreign/unknown edge poisons c, then a, z, downstream,
        // and the persisted Episode/candidate history rooted at a.
        for (event, target) in [("a", "claim:c"), ("z", "event:a")] {
            let feedback = serde_json::json!({
                "source_kind":"caller_reported", "producer":"caller", "observed_target":target,
                "observed_version":"quarantined-version-hash", "expected":"pass", "actual":"pass",
                "verification_method":"test", "verification_result":"passed", "limitations":[], "evidence_refs":[]
            });
            sqlx::query("UPDATE events SET feedback_json=? WHERE event_id=?")
                .bind(feedback.to_string())
                .bind(event)
                .execute(&f.pool)
                .await
                .unwrap();
            sqlx::query("INSERT INTO episode_events(episode_reference,event_id) VALUES ('dependent-episode',?)")
                .bind(event).execute(&f.pool).await.unwrap();
        }
        f.reflection("dependent-reflection", &["a"], NS, "verified")
            .await;
        let export = f.export().await;
        validate_export(&export).unwrap();
        assert_eq!(
            export
                .claims
                .iter()
                .map(|claim| claim.claim_id.as_str())
                .collect::<Vec<_>>(),
            ["source-less"]
        );
        assert_eq!(export.claims[0].claim.mode(), Mode::Observed);
        assert!(export.events.is_empty());
        assert!(export.evidence_links.is_empty());
        assert!(export.episode_memberships.is_empty());
        assert!(export.reflections.is_empty());
        assert!(export.experience_episodes.is_empty());
        assert!(export.experience_heads.is_empty());
        assert!(export.experience_versions.is_empty());
        let json = serde_json::to_string(&export).unwrap();
        for marker in [
            "foreign-secret",
            "unknown-secret",
            "quarantined-version-hash",
            "downstream",
            "dependent-episode",
        ] {
            assert!(!json.contains(marker), "leaked {marker}");
        }
    }
}

#[tokio::test]
async fn every_original_claim_evidence_edge_consumes_the_source_budget() {
    let f = Fixture::new().await;
    f.store
        .link_evidence("c".into(), "foreign-secret-event".into())
        .await
        .unwrap();
    let mut request = ExportMemoryRequest::new(NS);
    request.max_relations = 1;
    let error = export_memory(&f.store, request)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("budget"));
    assert!(!error.contains("foreign-secret"));
    // Without the restrictive source budget, c is omitted rather than emitted
    // with only the surviving a edge. No unrelated scope counts are disclosed.
    let export = f.export().await;
    assert!(export.claims.is_empty());
    assert!(export.evidence_links.is_empty());
    assert_eq!(export.events.len(), 2);
}
