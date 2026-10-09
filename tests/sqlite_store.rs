use agent_llm_mm::{
    application::{
        build_self_snapshot::{BuildSelfSnapshotInput, execute as build_self_snapshot},
        get_evidence_relation::{GetEvidenceRelationInput, execute as get_evidence_relation},
        get_self_model_history::{GetSelfModelHistoryInput, execute as get_self_model_history},
        search_memory::{
            MemoryRecordType, SearchMemoryInput, SearchMemoryRecord, execute as search_memory,
        },
        supersede_memory::{SupersedeMemoryInput, prepare_scoped_supersede},
    },
    domain::{
        claim::{ClaimDraft, ClaimReference},
        commitment::Commitment,
        event::{Event, EventReference, MAX_EVIDENCE_MANIFEST_ITEMS},
        identity_core::IdentityCore,
        reflection::{Reflection, ReflectionIdentityUpdate},
        self_revision::TriggerType,
        snapshot::{SnapshotBudget, SnapshotTimeWindow},
        types::{EventKind, MemoryScope, Mode, Namespace, Owner},
    },
    error::AppError,
    ports::{
        ClaimRecordQuery, ClaimReflectionHistoryQuery, ClaimRevisionLinks, ClaimStatus, ClaimStore,
        CommitmentStore, EpisodeRecordQuery, EpisodeStore, EventRecordQuery, EventStore,
        EvidenceQuery, IdentityStore, IngestTransactionRunner, MemoryReadStore,
        ReflectionRecordQuery, ReflectionStore, ReflectionTransactionRunner, ScopedEventIdQuery,
        SelfModelHistoryKind, SelfModelHistoryQuery, StoredClaim, StoredEvent, StoredReflection,
        StoredTriggerLedgerEntry, TriggerLedgerStatus, TriggerLedgerStore,
    },
};
use chrono::{DateTime, Utc};
use sqlx::{Row, sqlite::SqlitePool};
use std::path::PathBuf;

#[tokio::test]
async fn sqlite_store_bootstraps_all_tables() {
    let context = test_support::new_sqlite_store().await;
    let tables = sqlx::query(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )
    .fetch_all(&context.pool)
    .await
    .unwrap()
    .into_iter()
    .map(|row| row.get::<String, _>("name"))
    .collect::<Vec<_>>();

    assert!(tables.contains(&"events".to_string()));
    assert!(tables.contains(&"claims".to_string()));
    assert!(tables.contains(&"evidence_links".to_string()));
    assert!(tables.contains(&"episode_events".to_string()));
    assert!(tables.contains(&"reflections".to_string()));
    assert!(tables.contains(&"reflection_trigger_ledger".to_string()));
    assert!(tables.contains(&"identity_claims".to_string()));
    assert!(tables.contains(&"commitments".to_string()));

    let claims_sql = sqlx::query_scalar::<_, String>(
        "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'claims'",
    )
    .fetch_one(&context.pool)
    .await
    .unwrap();
    assert!(
        claims_sql.contains("CHECK"),
        "claims table should include a database-level namespace compatibility check"
    );
}

#[tokio::test]
async fn sqlite_lists_only_episodes_reached_through_claim_evidence_links() {
    let context = test_support::new_sqlite_store().await;
    let now = test_support::fixed_now();

    for (index, claim_id) in ["claim-support-1", "claim-support-2", "claim-support-3"]
        .into_iter()
        .enumerate()
    {
        context
            .store
            .upsert_claim(StoredClaim::new(
                claim_id.to_string(),
                ClaimDraft::new(
                    Owner::World,
                    "self.role",
                    "is",
                    "principal_architect",
                    Mode::Observed,
                ),
                ClaimStatus::Active,
            ))
            .await
            .unwrap();
        context
            .store
            .append_event(StoredEvent::new(
                format!("evt-support-{index}"),
                now + chrono::Duration::seconds(index as i64),
                Event::new(
                    Owner::World,
                    EventKind::Observation,
                    format!("supporting observation {index}"),
                ),
            ))
            .await
            .unwrap();
    }
    context
        .store
        .append_event(StoredEvent::new(
            "evt-unrelated".to_string(),
            now + chrono::Duration::seconds(10),
            Event::new(
                Owner::World,
                EventKind::Observation,
                "unrelated episode event",
            ),
        ))
        .await
        .unwrap();

    context
        .store
        .link_evidence("claim-support-1".to_string(), "evt-support-0".to_string())
        .await
        .unwrap();
    context
        .store
        .link_evidence("claim-support-2".to_string(), "evt-support-1".to_string())
        .await
        .unwrap();
    context
        .store
        .link_evidence("claim-support-3".to_string(), "evt-support-2".to_string())
        .await
        .unwrap();
    for (episode_reference, event_id) in [
        ("episode:support-a", "evt-support-0"),
        ("episode:support-a", "evt-support-1"),
        ("episode:support-b", "evt-support-2"),
        ("episode:unrelated", "evt-unrelated"),
    ] {
        context
            .store
            .record_event_in_episode(episode_reference.to_string(), event_id.to_string())
            .await
            .unwrap();
    }

    let world_scope = MemoryScope::for_namespace(Namespace::world());
    let episodes = context
        .store
        .list_episode_references_supporting_claims(
            &world_scope,
            &[
                "claim-support-1".to_string(),
                "claim-support-2".to_string(),
                "claim-support-3".to_string(),
            ],
        )
        .await
        .unwrap();

    assert_eq!(
        episodes,
        vec![
            "episode:support-a".to_string(),
            "episode:support-b".to_string()
        ]
    );
    assert!(
        context
            .store
            .list_episode_references_supporting_claims(&world_scope, &[])
            .await
            .unwrap()
            .is_empty()
    );
    let unscoped_error = context
        .store
        .list_episode_references_supporting_claims(&MemoryScope::legacy_unscoped(), &[])
        .await
        .expect_err("unscoped identity support lookup must fail closed");
    assert!(
        unscoped_error
            .to_string()
            .contains("requires an explicit namespace")
    );
}

#[tokio::test]
async fn sqlite_identity_support_ignores_cross_scope_evidence_links() {
    let context = test_support::new_sqlite_store().await;
    let now = test_support::fixed_now();
    let world_scope = MemoryScope::for_namespace(Namespace::world());
    let foreign_namespace = Namespace::for_project("other");

    for (index, claim_id) in ["claim-world-1", "claim-world-2", "claim-world-3"]
        .into_iter()
        .enumerate()
    {
        context
            .store
            .upsert_claim(StoredClaim::new(
                claim_id.to_string(),
                ClaimDraft::new(
                    Owner::World,
                    "self.role",
                    "is",
                    "principal_architect",
                    Mode::Observed,
                ),
                ClaimStatus::Active,
            ))
            .await
            .unwrap();
        context
            .store
            .append_event(StoredEvent::new(
                format!("evt-world-{index}"),
                now + chrono::Duration::seconds(index as i64),
                Event::new(
                    Owner::World,
                    EventKind::Observation,
                    format!("world evidence {index}"),
                ),
            ))
            .await
            .unwrap();
        context
            .store
            .append_event(StoredEvent::new(
                format!("evt-foreign-{index}"),
                now + chrono::Duration::seconds(20 + index as i64),
                Event::new_with_namespace(
                    Owner::World,
                    foreign_namespace.clone(),
                    EventKind::Observation,
                    format!("foreign evidence {index}"),
                )
                .unwrap(),
            ))
            .await
            .unwrap();
    }

    context
        .store
        .link_evidence("claim-world-1".to_string(), "evt-world-0".to_string())
        .await
        .unwrap();
    context
        .store
        .link_evidence("claim-world-2".to_string(), "evt-foreign-1".to_string())
        .await
        .unwrap();
    context
        .store
        .link_evidence("claim-world-3".to_string(), "evt-foreign-2".to_string())
        .await
        .unwrap();
    for (episode_reference, event_id) in [
        ("episode:world-a", "evt-world-0"),
        ("episode:foreign-a", "evt-foreign-1"),
        ("episode:foreign-b", "evt-foreign-2"),
    ] {
        context
            .store
            .record_event_in_episode(episode_reference.to_string(), event_id.to_string())
            .await
            .unwrap();
    }

    let world_claim_ids = [
        "claim-world-1".to_string(),
        "claim-world-2".to_string(),
        "claim-world-3".to_string(),
    ];
    assert_eq!(
        context
            .store
            .list_episode_references_supporting_claims(&world_scope, &world_claim_ids)
            .await
            .unwrap(),
        vec!["episode:world-a".to_string()]
    );
    assert!(
        context
            .store
            .list_episode_references_supporting_claims(
                &MemoryScope::for_namespace(foreign_namespace),
                &world_claim_ids,
            )
            .await
            .unwrap()
            .is_empty(),
        "foreign-scope query must not count world claims even when they link to foreign events"
    );
}

#[tokio::test]
async fn sqlite_query_evidence_event_ids_is_recent_first_and_filtered() {
    let context = test_support::new_sqlite_store().await;
    let now = test_support::fixed_now();

    context
        .store
        .append_event(StoredEvent::new(
            "evt-world-old".to_string(),
            now,
            Event::new(Owner::World, EventKind::Observation, "older world obs"),
        ))
        .await
        .unwrap();
    context
        .store
        .append_event(StoredEvent::new(
            "evt-user-note".to_string(),
            now + chrono::Duration::seconds(60),
            Event::new(Owner::User, EventKind::Conversation, "user conversation"),
        ))
        .await
        .unwrap();
    context
        .store
        .append_event(StoredEvent::new(
            "evt-world-new".to_string(),
            now + chrono::Duration::seconds(120),
            Event::new(Owner::World, EventKind::Observation, "newer world obs"),
        ))
        .await
        .unwrap();

    let results = context
        .store
        .query_evidence_event_ids(EvidenceQuery {
            namespace: None,
            owner: Some(Owner::World),
            kind: Some(EventKind::Observation),
            limit: Some(2),
            recorded_after: None,
            recorded_before: None,
            event_id_prefix: None,
        })
        .await
        .unwrap();

    assert_eq!(
        results,
        vec!["evt-world-new".to_string(), "evt-world-old".to_string()]
    );
}

#[tokio::test]
async fn sqlite_query_evidence_event_ids_filters_by_namespace_before_limit() {
    let context = test_support::new_sqlite_store().await;
    let now = test_support::fixed_now();

    for (event_id, namespace, offset_seconds) in [
        ("evt-project-a-old", Namespace::for_project("a"), 10),
        ("evt-project-b-new", Namespace::for_project("b"), 30),
        ("evt-project-a-new", Namespace::for_project("a"), 20),
        ("evt-project-a-newest", Namespace::for_project("a"), 40),
    ] {
        context
            .store
            .append_event(StoredEvent::new(
                event_id.to_string(),
                now + chrono::Duration::seconds(offset_seconds),
                Event::new_with_namespace(
                    Owner::World,
                    namespace,
                    EventKind::Observation,
                    event_id,
                )
                .unwrap(),
            ))
            .await
            .unwrap();
    }

    let results = context
        .store
        .query_evidence_event_ids(EvidenceQuery {
            namespace: Some(Namespace::for_project("a")),
            owner: Some(Owner::World),
            kind: Some(EventKind::Observation),
            limit: Some(2),
            recorded_after: None,
            recorded_before: None,
            event_id_prefix: None,
        })
        .await
        .unwrap();

    assert_eq!(
        results,
        vec![
            "evt-project-a-newest".to_string(),
            "evt-project-a-new".to_string()
        ]
    );
}

#[tokio::test]
async fn sqlite_snapshot_queries_do_not_leak_across_owner_or_namespace() {
    let context = test_support::new_sqlite_store().await;
    let now = test_support::fixed_now();
    context
        .store
        .save_identity(IdentityCore::new(vec!["identity:self=test".to_string()]))
        .await
        .unwrap();
    let cases = [
        ("self", Owner::Self_, Namespace::self_()),
        ("world", Owner::World, Namespace::world()),
        ("project-a", Owner::World, Namespace::for_project("a")),
        ("project-b", Owner::World, Namespace::for_project("b")),
        ("user-alice", Owner::User, Namespace::for_user("alice")),
        ("user-bob", Owner::User, Namespace::for_user("bob")),
    ];

    for (offset, (label, owner, namespace)) in cases.iter().enumerate() {
        let event_id = format!("evt-{label}");
        context
            .store
            .append_event(StoredEvent::new(
                event_id.clone(),
                now + chrono::Duration::seconds(offset as i64),
                Event::new_with_namespace(
                    *owner,
                    namespace.clone(),
                    EventKind::Observation,
                    *label,
                )
                .unwrap(),
            ))
            .await
            .unwrap();
        context
            .store
            .upsert_claim(StoredClaim::new(
                format!("claim-{label}"),
                ClaimDraft::new_with_namespace(
                    *owner,
                    namespace.clone(),
                    format!("subject-{label}"),
                    "is",
                    "scoped",
                    Mode::Observed,
                ),
                ClaimStatus::Active,
            ))
            .await
            .unwrap();
        context
            .store
            .record_event_in_episode(format!("episode:{label}"), event_id)
            .await
            .unwrap();
    }

    for (label, _owner, namespace) in cases {
        let snapshot = build_self_snapshot(
            &context.store,
            BuildSelfSnapshotInput {
                scope: MemoryScope::for_namespace(namespace.clone()),
                evidence_manifest: None,
                time_window: SnapshotTimeWindow::unbounded(),
                budget: SnapshotBudget::new(10),
            },
        )
        .await
        .unwrap()
        .snapshot;

        assert_eq!(snapshot.evidence, vec![format!("event:evt-{label}")]);
        assert_eq!(snapshot.episodes, vec![format!("episode:{label}")]);
        assert_eq!(
            snapshot.claims,
            vec![format!("{}:subject-{label} is scoped", namespace.as_str())]
        );
    }
}

#[tokio::test]
async fn sqlite_snapshot_manifest_intersects_scope_without_widening() {
    let context = test_support::new_sqlite_store().await;
    let now = test_support::fixed_now();
    context
        .store
        .save_identity(IdentityCore::new(vec!["identity:self=test".to_string()]))
        .await
        .unwrap();

    let unscoped_manifest = [EventReference::parse("evt-project-a").unwrap()];
    let error = context
        .store
        .list_event_references_in_scope(&MemoryScope::legacy_unscoped(), Some(&unscoped_manifest))
        .await
        .expect_err("store callers must not bypass the scoped manifest invariant");
    assert!(
        error
            .to_string()
            .contains("evidence_manifest requires an explicit namespace")
    );

    let oversized_manifest =
        vec![EventReference::parse("evt-project-a").unwrap(); MAX_EVIDENCE_MANIFEST_ITEMS + 1];
    let error = context
        .store
        .list_event_references_in_scope(
            &MemoryScope::for_namespace(Namespace::for_project("a")),
            Some(&oversized_manifest),
        )
        .await
        .expect_err("SQLite must reject oversized manifests before building bind parameters");
    assert!(
        error
            .to_string()
            .contains("evidence_manifest must contain at most 256 entries")
    );

    for (event_id, namespace, offset) in [
        ("evt-project-a", Namespace::for_project("a"), 1),
        ("evt-project-b", Namespace::for_project("b"), 2),
    ] {
        context
            .store
            .append_event(StoredEvent::new(
                event_id.to_string(),
                now + chrono::Duration::seconds(offset),
                Event::new_with_namespace(
                    Owner::World,
                    namespace,
                    EventKind::Observation,
                    event_id,
                )
                .unwrap(),
            ))
            .await
            .unwrap();
    }

    let scoped = build_self_snapshot(
        &context.store,
        BuildSelfSnapshotInput {
            scope: MemoryScope::for_namespace(Namespace::for_project("a")),
            evidence_manifest: Some(vec![
                EventReference::parse("evt-project-a").unwrap(),
                EventReference::parse("event:evt-project-b").unwrap(),
            ]),
            time_window: SnapshotTimeWindow::unbounded(),
            budget: SnapshotBudget::new(10),
        },
    )
    .await
    .unwrap()
    .snapshot;
    assert_eq!(scoped.evidence, vec!["event:evt-project-a"]);

    let empty_intersection = build_self_snapshot(
        &context.store,
        BuildSelfSnapshotInput {
            scope: MemoryScope::for_namespace(Namespace::for_project("a")),
            evidence_manifest: Some(vec![EventReference::parse("event:evt-project-b").unwrap()]),
            time_window: SnapshotTimeWindow::unbounded(),
            budget: SnapshotBudget::new(10),
        },
    )
    .await
    .unwrap()
    .snapshot;
    assert!(empty_intersection.evidence.is_empty());

    let explicit_empty = build_self_snapshot(
        &context.store,
        BuildSelfSnapshotInput {
            scope: MemoryScope::for_namespace(Namespace::for_project("a")),
            evidence_manifest: Some(Vec::new()),
            time_window: SnapshotTimeWindow::unbounded(),
            budget: SnapshotBudget::new(10),
        },
    )
    .await
    .unwrap()
    .snapshot;
    assert!(explicit_empty.evidence.is_empty());
}

#[tokio::test]
async fn sqlite_snapshot_time_window_intersects_scope_manifest_and_orders_real_instants() {
    let context = test_support::new_sqlite_store().await;
    context
        .store
        .save_identity(IdentityCore::new(vec!["identity:self=test".to_string()]))
        .await
        .unwrap();

    for (event_id, namespace, raw_recorded_at) in [
        (
            "evt-window-start",
            Namespace::for_project("a"),
            "2026-07-11T10:00:00+08:00",
        ),
        (
            "evt-same-early-row",
            Namespace::for_project("a"),
            "2026-07-11T03:00:00Z",
        ),
        (
            "evt-same-late-row",
            Namespace::for_project("a"),
            "2026-07-11T11:00:00+08:00",
        ),
        (
            "evt-window-end",
            Namespace::for_project("a"),
            "2026-07-11T04:00:00+00:00",
        ),
        (
            "evt-after-window",
            Namespace::for_project("a"),
            "2026-07-11T05:00:00Z",
        ),
        (
            "evt-sibling-scope",
            Namespace::for_project("b"),
            "2026-07-11T03:30:00Z",
        ),
        (
            "evt-not-in-manifest",
            Namespace::for_project("a"),
            "2026-07-11T03:30:00Z",
        ),
    ] {
        let recorded_at = DateTime::parse_from_rfc3339(raw_recorded_at)
            .unwrap()
            .with_timezone(&Utc);
        context
            .store
            .append_event(StoredEvent::new(
                event_id.to_string(),
                recorded_at,
                Event::new_with_namespace(
                    Owner::World,
                    namespace,
                    EventKind::Observation,
                    event_id,
                )
                .unwrap(),
            ))
            .await
            .unwrap();
        sqlx::query("UPDATE events SET recorded_at = ? WHERE event_id = ?")
            .bind(raw_recorded_at)
            .bind(event_id)
            .execute(&context.pool)
            .await
            .unwrap();
    }

    let time_window = SnapshotTimeWindow::new(
        Some(
            DateTime::parse_from_rfc3339("2026-07-11T02:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        ),
        Some(
            DateTime::parse_from_rfc3339("2026-07-11T04:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        ),
    )
    .unwrap();
    let input = BuildSelfSnapshotInput {
        scope: MemoryScope::for_namespace(Namespace::for_project("a")),
        evidence_manifest: Some(
            [
                "evt-window-start",
                "evt-same-early-row",
                "evt-same-late-row",
                "evt-window-end",
                "evt-after-window",
                "evt-sibling-scope",
            ]
            .into_iter()
            .map(|event_id| EventReference::parse(event_id).unwrap())
            .collect(),
        ),
        time_window,
        budget: SnapshotBudget::new(20),
    };
    let first = build_self_snapshot(&context.store, input.clone())
        .await
        .unwrap()
        .snapshot;
    let second = build_self_snapshot(&context.store, input)
        .await
        .unwrap()
        .snapshot;
    let expected = vec![
        "event:evt-window-end".to_string(),
        "event:evt-same-late-row".to_string(),
        "event:evt-same-early-row".to_string(),
        "event:evt-window-start".to_string(),
    ];
    assert_eq!(first.evidence, expected);
    assert_eq!(second.evidence, expected);

    let empty = build_self_snapshot(
        &context.store,
        BuildSelfSnapshotInput {
            scope: MemoryScope::for_namespace(Namespace::for_project("a")),
            evidence_manifest: None,
            time_window: SnapshotTimeWindow::new(
                Some(
                    DateTime::parse_from_rfc3339("2026-07-11T06:00:00Z")
                        .unwrap()
                        .with_timezone(&Utc),
                ),
                None,
            )
            .unwrap(),
            budget: SnapshotBudget::new(20),
        },
    )
    .await
    .expect("an empty explicit time window must not widen to older evidence")
    .snapshot;
    assert!(empty.evidence.is_empty());

    let legacy = build_self_snapshot(
        &context.store,
        BuildSelfSnapshotInput {
            scope: MemoryScope::legacy_unscoped(),
            evidence_manifest: None,
            time_window: SnapshotTimeWindow::unbounded(),
            budget: SnapshotBudget::new(20),
        },
    )
    .await
    .unwrap()
    .snapshot;
    assert_eq!(
        &legacy.evidence[..4],
        [
            "event:evt-after-window",
            "event:evt-window-end",
            "event:evt-not-in-manifest",
            "event:evt-sibling-scope",
        ]
    );
}

#[tokio::test]
async fn sqlite_snapshot_preserves_submillisecond_time_window_precision() {
    let context = test_support::new_sqlite_store().await;
    context
        .store
        .save_identity(IdentityCore::new(vec!["identity:self=test".to_string()]))
        .await
        .unwrap();
    let namespace = Namespace::for_project("precision");

    for (event_id, raw_recorded_at) in [
        ("evt-micro-late", "2026-07-11T11:00:00.000002+08:00"),
        ("evt-micro-early", "2026-07-11T03:00:00.000001Z"),
    ] {
        context
            .store
            .append_event(StoredEvent::new(
                event_id.to_string(),
                DateTime::parse_from_rfc3339(raw_recorded_at)
                    .unwrap()
                    .with_timezone(&Utc),
                Event::new_with_namespace(
                    Owner::World,
                    namespace.clone(),
                    EventKind::Observation,
                    event_id,
                )
                .unwrap(),
            ))
            .await
            .unwrap();
        sqlx::query("UPDATE events SET recorded_at = ? WHERE event_id = ?")
            .bind(raw_recorded_at)
            .bind(event_id)
            .execute(&context.pool)
            .await
            .unwrap();
    }

    let narrow = build_self_snapshot(
        &context.store,
        BuildSelfSnapshotInput {
            scope: MemoryScope::for_namespace(namespace.clone()),
            evidence_manifest: Some(vec![
                EventReference::parse("evt-micro-early").unwrap(),
                EventReference::parse("evt-micro-late").unwrap(),
            ]),
            time_window: SnapshotTimeWindow::new(
                Some(
                    DateTime::parse_from_rfc3339("2026-07-11T03:00:00.000001500Z")
                        .unwrap()
                        .with_timezone(&Utc),
                ),
                Some(
                    DateTime::parse_from_rfc3339("2026-07-11T03:00:00.000002Z")
                        .unwrap()
                        .with_timezone(&Utc),
                ),
            )
            .unwrap(),
            budget: SnapshotBudget::new(10),
        },
    )
    .await
    .unwrap()
    .snapshot;
    assert_eq!(narrow.evidence, vec!["event:evt-micro-late"]);

    let ordered = build_self_snapshot(
        &context.store,
        BuildSelfSnapshotInput {
            scope: MemoryScope::for_namespace(namespace),
            evidence_manifest: None,
            time_window: SnapshotTimeWindow::new(
                Some(
                    DateTime::parse_from_rfc3339("2026-07-11T03:00:00.000001Z")
                        .unwrap()
                        .with_timezone(&Utc),
                ),
                Some(
                    DateTime::parse_from_rfc3339("2026-07-11T03:00:00.000002Z")
                        .unwrap()
                        .with_timezone(&Utc),
                ),
            )
            .unwrap(),
            budget: SnapshotBudget::new(10),
        },
    )
    .await
    .unwrap()
    .snapshot;
    assert_eq!(
        ordered.evidence,
        vec!["event:evt-micro-late", "event:evt-micro-early"]
    );
}

#[tokio::test]
async fn sqlite_snapshot_orders_episodes_by_latest_in_window_event_tuple() {
    let context = test_support::new_sqlite_store().await;
    context
        .store
        .save_identity(IdentityCore::new(vec!["identity:self=test".to_string()]))
        .await
        .unwrap();
    let namespace = Namespace::for_project("episodes");

    for (event_id, timestamp) in [
        ("evt-a-latest", "2026-07-11T10:05:00Z"),
        ("evt-b-latest", "2026-07-11T18:05:00+08:00"),
        ("evt-a-older-but-later-row", "2026-07-11T10:01:00Z"),
        ("evt-outside", "2026-07-11T10:11:00Z"),
        ("evt-shared", "2026-07-11T10:04:00Z"),
        ("evt-micro-out", "2026-07-11T10:00:00.000001Z"),
    ] {
        context
            .store
            .append_event(StoredEvent::new(
                event_id.to_string(),
                DateTime::parse_from_rfc3339(timestamp)
                    .unwrap()
                    .with_timezone(&Utc),
                Event::new_with_namespace(
                    Owner::World,
                    namespace.clone(),
                    EventKind::Observation,
                    event_id,
                )
                .unwrap(),
            ))
            .await
            .unwrap();
    }
    for (episode_reference, event_id) in [
        ("episode:a", "evt-a-latest"),
        ("episode:a", "evt-a-older-but-later-row"),
        ("episode:b", "evt-b-latest"),
        ("episode:outside", "evt-outside"),
        ("episode:z", "evt-shared"),
        ("episode:y", "evt-shared"),
        ("episode:micro-out", "evt-micro-out"),
    ] {
        context
            .store
            .record_event_in_episode(episode_reference.to_string(), event_id.to_string())
            .await
            .unwrap();
    }

    let snapshot = build_self_snapshot(
        &context.store,
        BuildSelfSnapshotInput {
            scope: MemoryScope::for_namespace(namespace),
            evidence_manifest: None,
            time_window: SnapshotTimeWindow::new(
                Some(
                    DateTime::parse_from_rfc3339("2026-07-11T10:00:00.000001500Z")
                        .unwrap()
                        .with_timezone(&Utc),
                ),
                Some(
                    DateTime::parse_from_rfc3339("2026-07-11T10:10:00Z")
                        .unwrap()
                        .with_timezone(&Utc),
                ),
            )
            .unwrap(),
            budget: SnapshotBudget::new(20),
        },
    )
    .await
    .unwrap()
    .snapshot;

    assert_eq!(
        snapshot.episodes,
        vec!["episode:b", "episode:a", "episode:y", "episode:z"]
    );
}

#[cfg(target_pointer_width = "64")]
#[tokio::test]
async fn sqlite_query_evidence_event_ids_rejects_limit_above_i64_max() {
    let context = test_support::new_sqlite_store().await;
    let excessive_limit = (i64::MAX as u64 + 1) as usize;

    let result = context
        .store
        .query_evidence_event_ids(EvidenceQuery {
            namespace: None,
            owner: None,
            kind: None,
            limit: Some(excessive_limit),
            recorded_after: None,
            recorded_before: None,
            event_id_prefix: None,
        })
        .await;

    assert!(matches!(result, Err(AppError::InvalidParams(_))));
}

#[tokio::test]
async fn sqlite_query_evidence_event_ids_rejects_zero_limit() {
    let context = test_support::new_sqlite_store().await;

    let result = context
        .store
        .query_evidence_event_ids(EvidenceQuery {
            namespace: None,
            owner: None,
            kind: None,
            limit: Some(0),
            recorded_after: None,
            recorded_before: None,
            event_id_prefix: None,
        })
        .await;

    assert!(matches!(result, Err(AppError::InvalidParams(_))));
}

#[tokio::test]
async fn sqlite_query_evidence_event_ids_filters_by_kind_only() {
    let context = test_support::new_sqlite_store().await;
    let now = test_support::fixed_now();

    context
        .store
        .append_event(StoredEvent::new(
            "evt-observation".to_string(),
            now,
            Event::new(Owner::World, EventKind::Observation, "world observation"),
        ))
        .await
        .unwrap();
    context
        .store
        .append_event(StoredEvent::new(
            "evt-conversation".to_string(),
            now + chrono::Duration::seconds(60),
            Event::new(Owner::User, EventKind::Conversation, "user conversation"),
        ))
        .await
        .unwrap();

    // kind-only 过滤（不带 owner / namespace）：直接映射 event kind taxonomy，
    // intersect-only 且确定性 newest-first，不引入 ranking。独立 evidence-kind 仍 deferred。
    let results = context
        .store
        .query_evidence_event_ids(EvidenceQuery {
            namespace: None,
            owner: None,
            kind: Some(EventKind::Observation),
            limit: None,
            recorded_after: None,
            recorded_before: None,
            event_id_prefix: None,
        })
        .await
        .unwrap();

    assert_eq!(results, vec!["evt-observation".to_string()]);
}

#[tokio::test]
async fn sqlite_query_evidence_event_ids_unbounded_ignores_default_limit() {
    let context = test_support::new_sqlite_store().await;
    let now = test_support::fixed_now();

    for index in 0..11 {
        context
            .store
            .append_event(StoredEvent::new(
                format!("evt-world-{index}"),
                now + chrono::Duration::seconds(index as i64),
                Event::new(
                    Owner::World,
                    EventKind::Observation,
                    format!("world observation {index}"),
                ),
            ))
            .await
            .unwrap();
    }

    let bounded_results = context
        .store
        .query_evidence_event_ids(EvidenceQuery {
            namespace: None,
            owner: Some(Owner::World),
            kind: Some(EventKind::Observation),
            limit: None,
            recorded_after: None,
            recorded_before: None,
            event_id_prefix: None,
        })
        .await
        .unwrap();
    let unbounded_results = context
        .store
        .query_evidence_event_ids_unbounded(EvidenceQuery {
            namespace: None,
            owner: Some(Owner::World),
            kind: Some(EventKind::Observation),
            limit: None,
            recorded_after: None,
            recorded_before: None,
            event_id_prefix: None,
        })
        .await
        .unwrap();

    assert_eq!(bounded_results.len(), 10);
    assert_eq!(unbounded_results.len(), 11);
    assert_eq!(unbounded_results[0], "evt-world-10".to_string());
    assert_eq!(unbounded_results[10], "evt-world-0".to_string());
}

#[test]
fn sqlite_owner_namespace_sql_rules_have_single_source() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let schema_source =
        std::fs::read_to_string(manifest_dir.join("src/adapters/sqlite/schema.rs")).unwrap();
    let compatibility_source =
        std::fs::read_to_string(manifest_dir.join("src/adapters/sqlite/store/schema_compat.rs"))
            .unwrap();
    let mut store_source =
        std::fs::read_to_string(manifest_dir.join("src/adapters/sqlite/store.rs")).unwrap();
    for entry in std::fs::read_dir(manifest_dir.join("src/adapters/sqlite/store")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|extension| extension == "rs") {
            store_source.push_str(&std::fs::read_to_string(path).unwrap());
        }
    }

    assert!(
        schema_source.contains("claims_table_sql("),
        "schema.rs should define the shared claims table SQL builder"
    );
    assert!(
        schema_source.contains("legacy_namespace_backfill_expression("),
        "schema.rs should define the shared legacy namespace backfill expression"
    );
    assert!(
        compatibility_source.contains("claims_table_sql("),
        "schema compatibility helpers should use the shared claims table SQL builder"
    );
    assert!(
        compatibility_source.contains("legacy_namespace_backfill_expression("),
        "schema compatibility helpers should use the shared legacy namespace backfill expression"
    );

    for forbidden_fragment in [
        "CONSTRAINT owner_namespace_scope CHECK (",
        "OR (owner = 'user' AND namespace LIKE 'user/%')",
        "OR (owner = 'world' AND (namespace = 'world' OR namespace LIKE 'project/%'))",
        "OR (owner = 'unknown' AND (namespace = 'world' OR namespace LIKE 'project/%'))",
        "CASE owner WHEN 'self' THEN 'self' WHEN 'user' THEN 'user/default' ELSE 'world' END",
        "COALESCE(NULLIF(namespace, ''), CASE owner WHEN 'self' THEN 'self' WHEN 'user' THEN 'user/default' ELSE 'world' END)",
    ] {
        assert!(
            !store_source.contains(forbidden_fragment),
            "store modules should not inline owner/namespace SQL fragment: {forbidden_fragment}"
        );
    }
}

#[tokio::test]
async fn sqlite_round_trips_claim_with_evidence() {
    let context = test_support::new_sqlite_store().await;
    let ids = test_support::seed_event_and_claim(&context.store)
        .await
        .unwrap();
    let claim_row = sqlx::query(
        r#"
        SELECT claim_id, namespace, subject, object, status
        FROM claims
        WHERE claim_id = ?
        "#,
    )
    .bind(&ids.claim_id)
    .fetch_one(&context.pool)
    .await
    .unwrap();
    let evidence_rows = sqlx::query(
        r#"
        SELECT event_id
        FROM evidence_links
        WHERE claim_id = ?
        ORDER BY rowid
        "#,
    )
    .bind(&ids.claim_id)
    .fetch_all(&context.pool)
    .await
    .unwrap();

    assert_eq!(claim_row.get::<String, _>("claim_id"), ids.claim_id);
    assert_eq!(claim_row.get::<String, _>("namespace"), "self");
    assert_eq!(claim_row.get::<String, _>("subject"), "self.role");
    assert_eq!(claim_row.get::<String, _>("object"), "architect");
    assert_eq!(claim_row.get::<String, _>("status"), "active");
    assert_eq!(evidence_rows.len(), 1);
    assert_eq!(evidence_rows[0].get::<String, _>("event_id"), ids.event_id);
}

#[tokio::test]
async fn sqlite_bootstrap_backfills_namespace_for_legacy_claim_rows() {
    let context = test_support::new_legacy_claim_store().await;

    let namespace = sqlx::query_scalar::<_, String>(
        "SELECT namespace FROM claims WHERE claim_id = 'legacy-claim'",
    )
    .fetch_one(&context.pool)
    .await
    .unwrap();
    let namespace_not_null = sqlx::query_scalar::<_, i64>(
        r#"SELECT "notnull" FROM pragma_table_info('claims') WHERE name = 'namespace'"#,
    )
    .fetch_one(&context.pool)
    .await
    .unwrap();
    let legacy_invalid_insert = sqlx::query(
        r#"
        INSERT INTO claims (claim_id, owner, namespace, subject, predicate, object, mode, status)
        VALUES ('legacy-invalid-check', 'self', 'user/default', 'self.role', 'is', 'architect', 'observed', 'active')
        "#,
    )
    .execute(&context.pool)
    .await;

    assert_eq!(namespace, "user/default");
    assert_eq!(namespace_not_null, 1);
    assert!(
        legacy_invalid_insert.is_err(),
        "legacy migrations should restore the same namespace check constraint as fresh databases"
    );
}

#[tokio::test]
async fn sqlite_bootstrap_backfills_namespace_for_legacy_event_rows() {
    let context = test_support::new_legacy_event_store().await;

    let namespace = sqlx::query_scalar::<_, String>(
        "SELECT namespace FROM events WHERE event_id = 'legacy-world-event'",
    )
    .fetch_one(&context.pool)
    .await
    .unwrap();
    let namespace_not_null = sqlx::query_scalar::<_, i64>(
        r#"SELECT "notnull" FROM pragma_table_info('events') WHERE name = 'namespace'"#,
    )
    .fetch_one(&context.pool)
    .await
    .unwrap();
    let legacy_invalid_insert = sqlx::query(
        r#"
        INSERT INTO events (event_id, recorded_at, owner, namespace, kind, summary)
        VALUES ('legacy-invalid-event-check', '2026-03-23T10:00:00+00:00', 'self', 'project/agent-llm-mm', 'action', 'invalid namespace')
        "#,
    )
    .execute(&context.pool)
    .await;

    assert_eq!(namespace, "world");
    assert_eq!(namespace_not_null, 1);
    assert!(
        legacy_invalid_insert.is_err(),
        "legacy event migration should restore the same owner/namespace check constraint as fresh databases"
    );

    let evidence_link_event_fk = sqlx::query_scalar::<_, String>(
        r#"SELECT "table" FROM pragma_foreign_key_list('evidence_links') WHERE "from" = 'event_id'"#,
    )
    .fetch_one(&context.pool)
    .await
    .unwrap();
    let episode_event_fk = sqlx::query_scalar::<_, String>(
        r#"SELECT "table" FROM pragma_foreign_key_list('episode_events') WHERE "from" = 'event_id'"#,
    )
    .fetch_one(&context.pool)
    .await
    .unwrap();
    assert_eq!(evidence_link_event_fk, "events");
    assert_eq!(episode_event_fk, "events");

    sqlx::query(
        r#"
        INSERT INTO events (event_id, recorded_at, owner, namespace, kind, summary)
        VALUES ('legacy-new-event', '2026-03-23T10:01:00+00:00', 'world', 'world', 'observation', 'post-migration event')
        "#,
    )
    .execute(&context.pool)
    .await
    .unwrap();
    sqlx::query(
        r#"
        INSERT INTO claims (claim_id, owner, namespace, subject, predicate, object, mode, status)
        VALUES ('legacy-new-claim', 'self', 'self', 'self.role', 'is', 'architect', 'observed', 'active')
        "#,
    )
    .execute(&context.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO evidence_links (claim_id, event_id) VALUES ('legacy-new-claim', 'legacy-new-event')",
    )
    .execute(&context.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO episode_events (episode_reference, event_id) VALUES ('legacy-episode', 'legacy-new-event')",
    )
    .execute(&context.pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn sqlite_store_rejects_owner_namespace_mismatch_on_write() {
    let context = test_support::new_sqlite_store().await;

    let result = context
        .store
        .upsert_claim(StoredClaim::new(
            "claim-invalid-write".to_string(),
            ClaimDraft::new_with_namespace(
                Owner::Self_,
                Namespace::for_user("default"),
                "self.role",
                "is",
                "architect",
                Mode::Observed,
            ),
            ClaimStatus::Active,
        ))
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn sqlite_database_rejects_corrupt_namespace_owner_pair_before_read() {
    let context = test_support::new_sqlite_store().await;

    let insert_result = sqlx::query(
        r#"
        INSERT INTO claims (claim_id, owner, namespace, subject, predicate, object, mode, status)
        VALUES ('claim-invalid-read', 'self', 'user/default', 'self.role', 'is', 'architect', 'observed', 'active')
        "#,
    )
    .execute(&context.pool)
    .await;

    assert!(
        insert_result.is_err(),
        "database check constraints should reject corrupt owner/namespace pairs before reads"
    );
}

#[tokio::test]
async fn sqlite_reflection_transactions_commit_and_roll_back_as_expected() {
    let context = test_support::new_sqlite_store().await;
    test_support::seed_claim(&context.store, "claim-old")
        .await
        .unwrap();

    let mut ok_tx = context.store.begin_reflection_transaction().await.unwrap();
    ok_tx
        .upsert_claim(StoredClaim::new(
            "claim-new".to_string(),
            ClaimDraft::new(
                Owner::Self_,
                "self.role",
                "is",
                "senior_architect",
                Mode::Observed,
            ),
            ClaimStatus::Active,
        ))
        .await
        .unwrap();
    ok_tx
        .append_reflection(StoredReflection::new(
            "refl-1".to_string(),
            test_support::fixed_now(),
            Reflection::new("replace old claim"),
            Some("claim-old".to_string()),
            Some("claim-new".to_string()),
        ))
        .await
        .unwrap();
    ok_tx
        .update_claim_status("claim-old", ClaimStatus::Superseded)
        .await
        .unwrap();
    ok_tx.commit().await.unwrap();

    let committed_reflection_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM reflections WHERE reflection_id = 'refl-1'",
    )
    .fetch_one(&context.pool)
    .await
    .unwrap();
    let old_status =
        sqlx::query_scalar::<_, String>("SELECT status FROM claims WHERE claim_id = 'claim-old'")
            .fetch_one(&context.pool)
            .await
            .unwrap();
    assert_eq!(committed_reflection_count, 1);
    assert_eq!(old_status, "superseded");

    let mut failing_tx = context.store.begin_reflection_transaction().await.unwrap();
    failing_tx
        .upsert_claim(StoredClaim::new(
            "claim-rolled-back".to_string(),
            ClaimDraft::new(
                Owner::Self_,
                "self.role",
                "is",
                "staff_architect",
                Mode::Observed,
            ),
            ClaimStatus::Active,
        ))
        .await
        .unwrap();
    failing_tx
        .append_reflection(StoredReflection::new(
            "refl-missing".to_string(),
            test_support::fixed_now(),
            Reflection::new("this should fail"),
            Some("claim-missing".to_string()),
            Some("claim-rolled-back".to_string()),
        ))
        .await
        .unwrap();

    let update_result = failing_tx
        .update_claim_status("claim-missing", ClaimStatus::Superseded)
        .await;
    assert!(update_result.is_err());
    let commit_result = failing_tx.commit().await;
    assert!(commit_result.is_err());

    let rolled_back_reflection_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM reflections WHERE reflection_id = 'refl-missing'",
    )
    .fetch_one(&context.pool)
    .await
    .unwrap();
    let rolled_back_claim_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM claims WHERE claim_id = 'claim-rolled-back'",
    )
    .fetch_one(&context.pool)
    .await
    .unwrap();
    assert_eq!(rolled_back_reflection_count, 0);
    assert_eq!(rolled_back_claim_count, 0);
}

#[tokio::test]
async fn sqlite_reflection_transactions_replace_identity_and_commitments_atomically() {
    let context = test_support::new_sqlite_store().await;
    test_support::seed_claim(&context.store, "claim-old")
        .await
        .unwrap();
    context
        .store
        .save_identity(IdentityCore::new(vec![
            "identity:self=architect".to_string(),
            "identity:style=rigorous".to_string(),
        ]))
        .await
        .unwrap();

    // Reflection evidence is now an FK-backed ledger relation. Seed real
    // same-scope sources so this fixture continues to test transaction behavior.
    for event_id in ["evt-reflection-1", "evt-reflection-3", "evt-reflection-9"] {
        context
            .store
            .append_event(StoredEvent::new(
                event_id.to_string(),
                test_support::fixed_now(),
                Event::new(
                    Owner::Self_,
                    EventKind::Reflection,
                    "self-model update evidence",
                ),
            ))
            .await
            .unwrap();
    }

    let mut ok_tx = context.store.begin_reflection_transaction().await.unwrap();
    let loaded_identity = ok_tx.load_identity().await.unwrap();
    let loaded_commitments = ok_tx.load_commitments().await.unwrap();
    assert_eq!(
        loaded_identity.canonical_claims(),
        &[
            "identity:self=architect".to_string(),
            "identity:style=rigorous".to_string(),
        ]
    );
    assert_eq!(
        loaded_commitments,
        vec![agent_llm_mm::domain::commitment::Commitment::new(
            Owner::Self_,
            "forbid:write_identity_core_directly",
        )]
    );

    ok_tx
        .replace_identity(IdentityCore::new(vec![
            "identity:self=staff_architect".to_string(),
            "identity:style=evidence-first".to_string(),
        ]))
        .await
        .unwrap();
    ok_tx
        .replace_commitments(vec![
            agent_llm_mm::domain::commitment::Commitment::new(
                Owner::Self_,
                "prefer:evidence_backed_identity_updates",
            ),
            agent_llm_mm::domain::commitment::Commitment::new(
                Owner::Self_,
                "forbid:write_identity_core_directly",
            ),
        ])
        .await
        .unwrap();
    ok_tx
        .append_reflection(
            StoredReflection::new(
                "refl-audit".to_string(),
                test_support::fixed_now(),
                Reflection::new("replace claim and update deeper self state"),
                Some("claim-old".to_string()),
                Some("claim-new".to_string()),
            )
            .with_supporting_evidence_event_ids(vec![
                "evt-reflection-1".to_string(),
                "evt-reflection-3".to_string(),
            ])
            .with_requested_identity_update(Some(
                agent_llm_mm::domain::reflection::ReflectionIdentityUpdate::new(vec![
                    "identity:self=staff_architect".to_string(),
                    "identity:style=evidence-first".to_string(),
                ]),
            ))
            .with_requested_commitment_updates(Some(vec![
                agent_llm_mm::domain::commitment::Commitment::new(
                    Owner::Self_,
                    "prefer:evidence_backed_identity_updates",
                ),
                agent_llm_mm::domain::commitment::Commitment::new(
                    Owner::Self_,
                    "forbid:write_identity_core_directly",
                ),
            ])),
        )
        .await
        .unwrap();
    ok_tx
        .update_claim_status("claim-old", ClaimStatus::Superseded)
        .await
        .unwrap();
    ok_tx.commit().await.unwrap();

    let persisted_identity = context.store.load_identity().await.unwrap();
    let persisted_commitments = context.store.list_commitments().await.unwrap();
    let reflection_row = sqlx::query(
        r#"
        SELECT
            supporting_evidence_event_ids,
            requested_identity_update,
            requested_commitment_updates
        FROM reflections
        WHERE reflection_id = 'refl-audit'
        "#,
    )
    .fetch_one(&context.pool)
    .await
    .unwrap();

    assert_eq!(
        persisted_identity.canonical_claims(),
        &[
            "identity:self=staff_architect".to_string(),
            "identity:style=evidence-first".to_string(),
        ]
    );
    assert_eq!(
        persisted_commitments,
        vec![
            agent_llm_mm::domain::commitment::Commitment::new(
                Owner::Self_,
                "prefer:evidence_backed_identity_updates",
            ),
            agent_llm_mm::domain::commitment::Commitment::new(
                Owner::Self_,
                "forbid:write_identity_core_directly",
            ),
        ]
    );
    assert_eq!(
        serde_json::from_str::<Vec<String>>(
            &reflection_row.get::<String, _>("supporting_evidence_event_ids"),
        )
        .unwrap(),
        vec![
            "evt-reflection-1".to_string(),
            "evt-reflection-3".to_string(),
        ]
    );
    assert_eq!(
        serde_json::from_str::<agent_llm_mm::domain::reflection::ReflectionIdentityUpdate>(
            &reflection_row.get::<String, _>("requested_identity_update"),
        )
        .unwrap()
        .canonical_claims,
        vec![
            "identity:self=staff_architect".to_string(),
            "identity:style=evidence-first".to_string(),
        ]
    );
    assert_eq!(
        serde_json::from_str::<Vec<agent_llm_mm::domain::commitment::Commitment>>(
            &reflection_row.get::<String, _>("requested_commitment_updates"),
        )
        .unwrap(),
        vec![
            agent_llm_mm::domain::commitment::Commitment::new(
                Owner::Self_,
                "prefer:evidence_backed_identity_updates",
            ),
            agent_llm_mm::domain::commitment::Commitment::new(
                Owner::Self_,
                "forbid:write_identity_core_directly",
            ),
        ]
    );

    let mut failing_tx = context.store.begin_reflection_transaction().await.unwrap();
    failing_tx
        .replace_identity(IdentityCore::new(vec![
            "identity:self=rolled_back".to_string(),
        ]))
        .await
        .unwrap();
    failing_tx
        .replace_commitments(vec![agent_llm_mm::domain::commitment::Commitment::new(
            Owner::Self_,
            "prefer:should_roll_back",
        )])
        .await
        .unwrap();
    failing_tx
        .append_reflection(
            StoredReflection::new(
                "refl-audit-rollback".to_string(),
                test_support::fixed_now(),
                Reflection::new("this deeper update should roll back"),
                Some("claim-missing".to_string()),
                None,
            )
            .with_supporting_evidence_event_ids(vec!["evt-reflection-9".to_string()]),
        )
        .await
        .unwrap();

    let update_result = failing_tx
        .update_claim_status("claim-missing", ClaimStatus::Superseded)
        .await;
    assert!(update_result.is_err());
    let commit_result = failing_tx.commit().await;
    assert!(commit_result.is_err());

    let rolled_back_identity = context.store.load_identity().await.unwrap();
    let rolled_back_commitments = context.store.list_commitments().await.unwrap();
    let rolled_back_reflection_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM reflections WHERE reflection_id = 'refl-audit-rollback'",
    )
    .fetch_one(&context.pool)
    .await
    .unwrap();

    assert_eq!(rolled_back_identity, persisted_identity);
    assert_eq!(rolled_back_commitments, persisted_commitments);
    assert_eq!(rolled_back_reflection_count, 0);
}

#[tokio::test]
async fn sqlite_handled_ledger_failure_rolls_back_deeper_reflection_updates() {
    let context = test_support::new_sqlite_store().await;
    context
        .store
        .save_identity(IdentityCore::new(vec![
            "identity:self=architect".to_string(),
        ]))
        .await
        .unwrap();
    let baseline_identity = context.store.load_identity().await.unwrap();
    let baseline_commitments = context.store.list_commitments().await.unwrap();
    let duplicate_ledger_id = "ledger-existing";
    context
        .store
        .record_trigger_attempt(StoredTriggerLedgerEntry::new(
            duplicate_ledger_id,
            TriggerType::Conflict,
            Namespace::world(),
            "world:conflict",
            TriggerLedgerStatus::Rejected,
        ))
        .await
        .unwrap();

    let mut transaction = context.store.begin_reflection_transaction().await.unwrap();
    transaction
        .replace_identity(IdentityCore::new(vec![
            "identity:self=must-roll-back".to_string(),
        ]))
        .await
        .unwrap();
    transaction
        .replace_commitments(vec![agent_llm_mm::domain::commitment::Commitment::new(
            Owner::Self_,
            "prefer:must_roll_back",
        )])
        .await
        .unwrap();
    transaction
        .append_reflection(StoredReflection::new(
            "refl-handled-ledger-rollback".to_string(),
            test_support::fixed_now(),
            Reflection::new("handled ledger failure must roll back deeper writes"),
            None,
            None,
        ))
        .await
        .unwrap();

    let handled_ledger_error = transaction
        .append_trigger_ledger(StoredTriggerLedgerEntry::new(
            duplicate_ledger_id,
            TriggerType::Conflict,
            Namespace::world(),
            "world:conflict",
            TriggerLedgerStatus::Handled,
        ))
        .await;
    assert!(handled_ledger_error.is_err());
    assert!(transaction.commit().await.is_err());

    assert_eq!(
        context.store.load_identity().await.unwrap(),
        baseline_identity
    );
    assert_eq!(
        context.store.list_commitments().await.unwrap(),
        baseline_commitments
    );
    let reflection_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM reflections WHERE reflection_id = 'refl-handled-ledger-rollback'",
    )
    .fetch_one(&context.pool)
    .await
    .unwrap();
    let ledger_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM reflection_trigger_ledger WHERE ledger_id = 'ledger-existing'",
    )
    .fetch_one(&context.pool)
    .await
    .unwrap();
    assert_eq!(reflection_count, 0);
    assert_eq!(
        ledger_count, 1,
        "only the pre-existing rejected audit remains"
    );
}

#[tokio::test]
async fn sqlite_store_persists_identity_and_reads_commitments_for_snapshot_ports() {
    let context = test_support::new_sqlite_store().await;

    context
        .store
        .save_identity(IdentityCore::new(vec![
            "identity:self=architect".to_string(),
            "identity:style=rigorous".to_string(),
        ]))
        .await
        .unwrap();

    let identity = context.store.load_identity().await.unwrap();
    let commitments = context.store.list_commitments().await.unwrap();

    assert_eq!(
        identity.canonical_claims(),
        &[
            "identity:self=architect".to_string(),
            "identity:style=rigorous".to_string()
        ]
    );
    assert_eq!(commitments.len(), 1);
    assert_eq!(
        commitments[0].description(),
        "forbid:write_identity_core_directly"
    );
}

#[tokio::test]
async fn sqlite_bootstrap_migrates_legacy_reflections_table_with_audit_columns() {
    let path = std::env::temp_dir().join(format!(
        "agent-llm-mm-legacy-reflections-{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let database_url = format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"));
    std::fs::File::create(&path).unwrap();
    let pool = SqlitePool::connect(&database_url).await.unwrap();

    sqlx::query(
        r#"
        CREATE TABLE reflections (
            reflection_id TEXT PRIMARY KEY,
            recorded_at TEXT NOT NULL,
            summary TEXT NOT NULL,
            superseded_claim_id TEXT,
            replacement_claim_id TEXT
        )
        "#,
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        r#"
        INSERT INTO reflections (
            reflection_id,
            recorded_at,
            summary,
            superseded_claim_id,
            replacement_claim_id
        )
        VALUES ('legacy-refl', '2026-03-23T10:00:00Z', 'legacy reflection row', 'claim-old', NULL)
        "#,
    )
    .execute(&pool)
    .await
    .unwrap();

    drop(pool);

    let _store = agent_llm_mm::adapters::sqlite::SqliteStore::bootstrap(&database_url)
        .await
        .unwrap();
    let migrated_pool = SqlitePool::connect(&database_url).await.unwrap();

    let columns = sqlx::query("SELECT name FROM pragma_table_info('reflections') ORDER BY cid")
        .fetch_all(&migrated_pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.get::<String, _>("name"))
        .collect::<Vec<_>>();
    let trigger_ledger_table_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'reflection_trigger_ledger'",
    )
    .fetch_one(&migrated_pool)
    .await
    .unwrap();
    let legacy_summary = sqlx::query_scalar::<_, String>(
        "SELECT summary FROM reflections WHERE reflection_id = 'legacy-refl'",
    )
    .fetch_one(&migrated_pool)
    .await
    .unwrap();

    assert!(columns.contains(&"supporting_evidence_event_ids".to_string()));
    assert!(columns.contains(&"requested_identity_update".to_string()));
    assert!(columns.contains(&"requested_commitment_updates".to_string()));
    assert_eq!(trigger_ledger_table_count, 1);
    assert_eq!(legacy_summary, "legacy reflection row");
}

#[tokio::test]
async fn sqlite_trigger_ledger_records_namespace_periodic_watermark_and_cooldown() {
    let context = test_support::new_sqlite_store().await;
    let trigger_key = "periodic:project/agent-llm-mm";
    let first_handled_at = test_support::fixed_now();
    let second_handled_at = first_handled_at + chrono::Duration::minutes(15);
    let second_cooldown_until = second_handled_at + chrono::Duration::hours(6);

    context
        .store
        .record_trigger_attempt(StoredTriggerLedgerEntry {
            ledger_id: "ledger-1".to_string(),
            trigger_type: TriggerType::Periodic,
            namespace: Namespace::for_project("agent-llm-mm"),
            trigger_key: trigger_key.to_string(),
            status: TriggerLedgerStatus::Handled,
            evidence_window: vec!["event:1".to_string()],
            handled_at: Some(first_handled_at),
            cooldown_until: Some(first_handled_at + chrono::Duration::hours(1)),
            episode_watermark: Some(20),
            reflection_id: Some("refl-1".to_string()),
        })
        .await
        .unwrap();

    context
        .store
        .record_trigger_attempt(StoredTriggerLedgerEntry {
            ledger_id: "ledger-2".to_string(),
            trigger_type: TriggerType::Periodic,
            namespace: Namespace::for_project("agent-llm-mm"),
            trigger_key: trigger_key.to_string(),
            status: TriggerLedgerStatus::Suppressed,
            evidence_window: vec!["event:2".to_string(), "event:3".to_string()],
            handled_at: Some(second_handled_at),
            cooldown_until: Some(second_cooldown_until),
            episode_watermark: Some(42),
            reflection_id: None,
        })
        .await
        .unwrap();

    let entry = context
        .store
        .latest_trigger_entry(trigger_key)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(entry.namespace.as_str(), "project/agent-llm-mm");
    assert_eq!(entry.status, TriggerLedgerStatus::Suppressed);
    assert_eq!(entry.episode_watermark, Some(42));
    assert_eq!(entry.cooldown_until, Some(second_cooldown_until));
    assert_eq!(entry.handled_at, Some(second_handled_at));
    assert_eq!(
        entry.evidence_window,
        vec!["event:2".to_string(), "event:3".to_string()]
    );
}

#[cfg(target_pointer_width = "64")]
#[tokio::test]
async fn sqlite_trigger_ledger_rejects_episode_watermark_above_i64_max() {
    let context = test_support::new_sqlite_store().await;

    let result = context
        .store
        .record_trigger_attempt(StoredTriggerLedgerEntry {
            ledger_id: "ledger-overflow".to_string(),
            trigger_type: TriggerType::Periodic,
            namespace: Namespace::for_project("agent-llm-mm"),
            trigger_key: "periodic:project/agent-llm-mm".to_string(),
            status: TriggerLedgerStatus::Handled,
            evidence_window: Vec::new(),
            handled_at: Some(test_support::fixed_now()),
            cooldown_until: None,
            episode_watermark: Some(i64::MAX as u64 + 1),
            reflection_id: None,
        })
        .await;

    assert!(matches!(result, Err(AppError::InvalidParams(_))));
}

#[tokio::test]
async fn sqlite_trigger_ledger_latest_entry_uses_append_order_not_handled_at() {
    let context = test_support::new_sqlite_store().await;
    let trigger_key = "periodic:project/agent-llm-mm";
    let later_business_time = test_support::fixed_now() + chrono::Duration::hours(4);
    let earlier_business_time = test_support::fixed_now() - chrono::Duration::hours(2);

    context
        .store
        .record_trigger_attempt(StoredTriggerLedgerEntry {
            ledger_id: "ledger-business-late".to_string(),
            trigger_type: TriggerType::Periodic,
            namespace: Namespace::for_project("agent-llm-mm"),
            trigger_key: trigger_key.to_string(),
            status: TriggerLedgerStatus::Handled,
            evidence_window: vec!["event:late".to_string()],
            handled_at: Some(later_business_time),
            cooldown_until: Some(later_business_time + chrono::Duration::hours(1)),
            episode_watermark: Some(100),
            reflection_id: Some("refl-late".to_string()),
        })
        .await
        .unwrap();

    context
        .store
        .record_trigger_attempt(StoredTriggerLedgerEntry {
            ledger_id: "ledger-appended-last".to_string(),
            trigger_type: TriggerType::Periodic,
            namespace: Namespace::for_project("agent-llm-mm"),
            trigger_key: trigger_key.to_string(),
            status: TriggerLedgerStatus::Suppressed,
            evidence_window: vec!["event:appended-last".to_string()],
            handled_at: Some(earlier_business_time),
            cooldown_until: None,
            episode_watermark: Some(101),
            reflection_id: None,
        })
        .await
        .unwrap();

    let entry = context
        .store
        .latest_trigger_entry(trigger_key)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(entry.ledger_id, "ledger-appended-last");
    assert_eq!(entry.handled_at, Some(earlier_business_time));
    assert_eq!(entry.status, TriggerLedgerStatus::Suppressed);
}

#[tokio::test]
async fn sqlite_trigger_ledger_latest_handled_entry_skips_newer_non_handled_rows() {
    let context = test_support::new_sqlite_store().await;
    let trigger_key = "periodic:project/agent-llm-mm";
    let first_handled_at = test_support::fixed_now() + chrono::Duration::hours(2);
    let second_handled_at = test_support::fixed_now() - chrono::Duration::hours(1);

    context
        .store
        .record_trigger_attempt(StoredTriggerLedgerEntry {
            ledger_id: "ledger-handled-first".to_string(),
            trigger_type: TriggerType::Periodic,
            namespace: Namespace::for_project("agent-llm-mm"),
            trigger_key: trigger_key.to_string(),
            status: TriggerLedgerStatus::Handled,
            evidence_window: vec!["event:handled-first".to_string()],
            handled_at: Some(first_handled_at),
            cooldown_until: Some(first_handled_at + chrono::Duration::hours(1)),
            episode_watermark: Some(10),
            reflection_id: Some("refl-first".to_string()),
        })
        .await
        .unwrap();

    context
        .store
        .record_trigger_attempt(StoredTriggerLedgerEntry {
            ledger_id: "ledger-suppressed-newer".to_string(),
            trigger_type: TriggerType::Periodic,
            namespace: Namespace::for_project("agent-llm-mm"),
            trigger_key: trigger_key.to_string(),
            status: TriggerLedgerStatus::Suppressed,
            evidence_window: vec!["event:suppressed".to_string()],
            handled_at: None,
            cooldown_until: Some(test_support::fixed_now() + chrono::Duration::hours(1)),
            episode_watermark: Some(11),
            reflection_id: Some("refl-first".to_string()),
        })
        .await
        .unwrap();

    context
        .store
        .record_trigger_attempt(StoredTriggerLedgerEntry {
            ledger_id: "ledger-rejected-newest".to_string(),
            trigger_type: TriggerType::Periodic,
            namespace: Namespace::for_project("agent-llm-mm"),
            trigger_key: trigger_key.to_string(),
            status: TriggerLedgerStatus::Rejected,
            evidence_window: vec!["event:rejected".to_string()],
            handled_at: None,
            cooldown_until: None,
            episode_watermark: Some(12),
            reflection_id: None,
        })
        .await
        .unwrap();

    context
        .store
        .record_trigger_attempt(StoredTriggerLedgerEntry {
            ledger_id: "ledger-handled-second".to_string(),
            trigger_type: TriggerType::Periodic,
            namespace: Namespace::for_project("agent-llm-mm"),
            trigger_key: trigger_key.to_string(),
            status: TriggerLedgerStatus::Handled,
            evidence_window: vec!["event:handled-second".to_string()],
            handled_at: Some(second_handled_at),
            cooldown_until: Some(second_handled_at + chrono::Duration::hours(1)),
            episode_watermark: Some(20),
            reflection_id: Some("refl-second".to_string()),
        })
        .await
        .unwrap();

    let entry = context
        .store
        .latest_handled_trigger_entry(trigger_key)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(entry.ledger_id, "ledger-handled-second");
    assert_eq!(entry.status, TriggerLedgerStatus::Handled);
    assert_eq!(entry.handled_at, Some(second_handled_at));
    assert_eq!(
        entry.evidence_window,
        vec!["event:handled-second".to_string()]
    );
    assert_eq!(entry.reflection_id.as_deref(), Some("refl-second"));
}

mod test_support {
    use super::*;
    use agent_llm_mm::adapters::sqlite::SqliteStore;

    pub struct TestContext {
        pub store: SqliteStore,
        pub pool: SqlitePool,
    }

    pub struct SeedIds {
        pub claim_id: String,
        pub event_id: String,
    }

    pub async fn new_sqlite_store() -> TestContext {
        let path =
            std::env::temp_dir().join(format!("agent-llm-mm-{}.sqlite", uuid::Uuid::new_v4()));
        let database_url = format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"));
        let store = SqliteStore::bootstrap(&database_url).await.unwrap();
        let pool = SqlitePool::connect(&database_url).await.unwrap();

        TestContext { store, pool }
    }

    pub async fn new_legacy_claim_store() -> TestContext {
        let path = std::env::temp_dir().join(format!(
            "agent-llm-mm-legacy-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let database_url = format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"));
        std::fs::File::create(&path).unwrap();
        let pool = SqlitePool::connect(&database_url).await.unwrap();

        sqlx::query(
            r#"
            CREATE TABLE claims (
                claim_id TEXT PRIMARY KEY,
                owner TEXT NOT NULL,
                subject TEXT NOT NULL,
                predicate TEXT NOT NULL,
                object TEXT NOT NULL,
                mode TEXT NOT NULL,
                status TEXT NOT NULL
            )
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            r#"
            INSERT INTO claims (claim_id, owner, subject, predicate, object, mode, status)
            VALUES ('legacy-claim', 'user', 'user.preference', 'likes', 'concise', 'observed', 'active')
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();

        drop(pool);

        let store = SqliteStore::bootstrap(&database_url).await.unwrap();
        let pool = SqlitePool::connect(&database_url).await.unwrap();

        TestContext { store, pool }
    }

    pub async fn new_legacy_event_store() -> TestContext {
        let path = std::env::temp_dir().join(format!(
            "agent-llm-mm-legacy-event-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let database_url = format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"));
        std::fs::File::create(&path).unwrap();
        let pool = SqlitePool::connect(&database_url).await.unwrap();

        sqlx::query(
            r#"
            CREATE TABLE events (
                event_id TEXT PRIMARY KEY,
                recorded_at TEXT NOT NULL,
                owner TEXT NOT NULL,
                kind TEXT NOT NULL,
                summary TEXT NOT NULL
            )
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            r#"
            INSERT INTO events (event_id, recorded_at, owner, kind, summary)
            VALUES ('legacy-world-event', '2026-03-23T10:00:00+00:00', 'world', 'observation', 'legacy event')
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();

        drop(pool);

        let store = SqliteStore::bootstrap(&database_url).await.unwrap();
        let pool = SqlitePool::connect(&database_url).await.unwrap();

        TestContext { store, pool }
    }

    pub async fn seed_event_and_claim(
        store: &SqliteStore,
    ) -> Result<SeedIds, agent_llm_mm::error::AppError> {
        let event_id = "evt-1".to_string();
        let claim_id = "claim-1".to_string();
        let recorded_at = fixed_now();
        let event = StoredEvent::new(
            event_id.clone(),
            recorded_at,
            Event::new(
                Owner::User,
                EventKind::Conversation,
                "The user asked for stronger memory.",
            ),
        );
        let claim = StoredClaim::new(
            claim_id.clone(),
            ClaimDraft::new(Owner::Self_, "self.role", "is", "architect", Mode::Observed),
            ClaimStatus::Active,
        );
        let mut tx = store.begin_ingest_transaction().await?;
        tx.append_event(event).await?;
        tx.upsert_claim(claim).await?;
        tx.link_evidence(claim_id.clone(), event_id.clone()).await?;
        tx.commit().await?;

        Ok(SeedIds { claim_id, event_id })
    }

    pub async fn seed_claim(
        store: &SqliteStore,
        claim_id: &str,
    ) -> Result<(), agent_llm_mm::error::AppError> {
        store
            .upsert_claim(StoredClaim::new(
                claim_id.to_string(),
                ClaimDraft::new(Owner::Self_, "self.role", "is", "architect", Mode::Observed),
                ClaimStatus::Active,
            ))
            .await
    }
    pub fn fixed_now() -> DateTime<Utc> {
        chrono::DateTime::parse_from_rfc3339("2026-03-23T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }
}

#[tokio::test]
async fn sqlite_query_evidence_event_ids_filters_by_recency_window() {
    let context = test_support::new_sqlite_store().await;
    let now = test_support::fixed_now();

    context
        .store
        .append_event(StoredEvent::new(
            "evt-old".to_string(),
            now,
            Event::new(Owner::World, EventKind::Observation, "old"),
        ))
        .await
        .unwrap();
    context
        .store
        .append_event(StoredEvent::new(
            "evt-mid".to_string(),
            now + chrono::Duration::seconds(60),
            Event::new(Owner::World, EventKind::Observation, "mid"),
        ))
        .await
        .unwrap();
    context
        .store
        .append_event(StoredEvent::new(
            "evt-new".to_string(),
            now + chrono::Duration::seconds(120),
            Event::new(Owner::World, EventKind::Observation, "new"),
        ))
        .await
        .unwrap();

    let after_only = context
        .store
        .query_evidence_event_ids(EvidenceQuery {
            namespace: None,
            owner: None,
            kind: None,
            limit: None,
            recorded_after: Some(now + chrono::Duration::seconds(30)),
            recorded_before: None,
            event_id_prefix: None,
        })
        .await
        .unwrap();
    assert_eq!(after_only, vec!["evt-new", "evt-mid"]);

    let before_only = context
        .store
        .query_evidence_event_ids(EvidenceQuery {
            namespace: None,
            owner: None,
            kind: None,
            limit: None,
            recorded_after: None,
            recorded_before: Some(now + chrono::Duration::seconds(90)),
            event_id_prefix: None,
        })
        .await
        .unwrap();
    assert_eq!(before_only, vec!["evt-mid", "evt-old"]);

    let window = context
        .store
        .query_evidence_event_ids(EvidenceQuery {
            namespace: None,
            owner: None,
            kind: None,
            limit: None,
            recorded_after: Some(now + chrono::Duration::seconds(30)),
            recorded_before: Some(now + chrono::Duration::seconds(90)),
            event_id_prefix: None,
        })
        .await
        .unwrap();
    assert_eq!(window, vec!["evt-mid"]);

    let inclusive_window = context
        .store
        .query_evidence_event_ids(EvidenceQuery {
            namespace: None,
            owner: None,
            kind: None,
            limit: None,
            recorded_after: Some(now + chrono::Duration::seconds(60)),
            recorded_before: Some(now + chrono::Duration::seconds(120)),
            event_id_prefix: None,
        })
        .await
        .unwrap();
    assert_eq!(inclusive_window, vec!["evt-new", "evt-mid"]);
}

#[tokio::test]
async fn query_evidence_filters_by_event_id_prefix() {
    let context = test_support::new_sqlite_store().await;
    let now = test_support::fixed_now();
    context
        .store
        .append_event(StoredEvent::new(
            "alpha-1".to_string(),
            now,
            Event::new(Owner::World, EventKind::Observation, "a"),
        ))
        .await
        .unwrap();
    context
        .store
        .append_event(StoredEvent::new(
            "beta-1".to_string(),
            now + chrono::Duration::seconds(60),
            Event::new(Owner::World, EventKind::Observation, "b"),
        ))
        .await
        .unwrap();

    let matched = context
        .store
        .query_evidence_event_ids(EvidenceQuery {
            namespace: None,
            owner: None,
            kind: None,
            limit: None,
            recorded_after: None,
            recorded_before: None,
            event_id_prefix: Some("alpha-".to_string()),
        })
        .await
        .unwrap();

    assert_eq!(matched, vec!["alpha-1"]);
}

#[tokio::test]
async fn sqlite_event_recall_is_scoped_recent_first_and_returns_provenance() {
    let context = test_support::new_sqlite_store().await;
    let now = test_support::fixed_now();
    let project_a = Namespace::for_project("a");
    let project_b = Namespace::for_project("b");

    for (event_id, namespace, recorded_at, summary) in [
        ("a-old", project_a.clone(), now, "project a old"),
        (
            "b-newest",
            project_b,
            now + chrono::Duration::seconds(120),
            "project b interference",
        ),
        (
            "a-new",
            project_a.clone(),
            now + chrono::Duration::seconds(60),
            "project a new",
        ),
    ] {
        context
            .store
            .append_event(StoredEvent::new(
                event_id.to_string(),
                recorded_at,
                Event::new_with_namespace(Owner::World, namespace, EventKind::Observation, summary)
                    .unwrap(),
            ))
            .await
            .unwrap();
    }

    context
        .store
        .upsert_claim(StoredClaim::new(
            "claim-a".to_string(),
            ClaimDraft::new_with_namespace(
                Owner::World,
                project_a.clone(),
                "project.a",
                "has",
                "new evidence",
                Mode::Observed,
            ),
            ClaimStatus::Active,
        ))
        .await
        .unwrap();
    context
        .store
        .link_evidence("claim-a".to_string(), "a-new".to_string())
        .await
        .unwrap();
    context
        .store
        .upsert_claim(StoredClaim::new(
            "claim-b-linked-to-a".to_string(),
            ClaimDraft::new_with_namespace(
                Owner::World,
                Namespace::for_project("b"),
                "project.b",
                "must_not",
                "leak through project a event provenance",
                Mode::Observed,
            ),
            ClaimStatus::Active,
        ))
        .await
        .unwrap();
    context
        .store
        .link_evidence("claim-b-linked-to-a".to_string(), "a-new".to_string())
        .await
        .unwrap();
    context
        .store
        .record_event_in_episode("episode-a".to_string(), "a-new".to_string())
        .await
        .unwrap();

    let records = context
        .store
        .query_event_records(EventRecordQuery {
            scope: MemoryScope::for_namespace(project_a),
            event_reference: None,
            kind: Some(EventKind::Observation),
            recorded_after: Some(now),
            recorded_before: Some(now + chrono::Duration::seconds(60)),
            limit: 10,
        })
        .await
        .unwrap();

    assert_eq!(
        records
            .iter()
            .map(|record| record.event.event_id.as_str())
            .collect::<Vec<_>>(),
        vec!["a-new", "a-old"]
    );
    assert_eq!(
        records[0].event.recorded_at,
        now + chrono::Duration::seconds(60)
    );
    assert_eq!(records[0].event.event.owner(), Owner::World);
    assert_eq!(
        records[0].event.event.namespace(),
        &Namespace::for_project("a")
    );
    assert_eq!(records[0].event.event.kind(), EventKind::Observation);
    assert_eq!(records[0].event.event.summary(), "project a new");
    assert_eq!(records[0].claim_ids, vec!["claim-a"]);
    assert_eq!(records[0].episode_references, vec!["episode-a"]);
    assert!(records[1].claim_ids.is_empty());
    assert!(records[1].episode_references.is_empty());
}

#[tokio::test]
async fn sqlite_event_recall_rejects_unscoped_queries_and_never_widens_exact_ids() {
    let context = test_support::new_sqlite_store().await;
    let now = test_support::fixed_now();
    context
        .store
        .append_event(StoredEvent::new(
            "project-b-only".to_string(),
            now,
            Event::new_with_namespace(
                Owner::World,
                Namespace::for_project("b"),
                EventKind::Conversation,
                "belongs only to project b",
            )
            .unwrap(),
        ))
        .await
        .unwrap();

    let unscoped = context
        .store
        .query_event_records(EventRecordQuery {
            scope: MemoryScope::legacy_unscoped(),
            event_reference: None,
            kind: None,
            recorded_after: None,
            recorded_before: None,
            limit: 10,
        })
        .await;
    assert!(matches!(unscoped, Err(AppError::InvalidParams(_))));

    let cross_scope = context
        .store
        .query_event_records(EventRecordQuery {
            scope: MemoryScope::for_namespace(Namespace::for_project("a")),
            event_reference: Some(EventReference::parse("event:project-b-only").unwrap()),
            kind: None,
            recorded_after: None,
            recorded_before: None,
            limit: 10,
        })
        .await
        .unwrap();
    assert!(cross_scope.is_empty());
}

#[tokio::test]
async fn sqlite_episode_recall_is_scoped_ordered_and_returns_distinct_provenance() {
    let context = test_support::new_sqlite_store().await;
    let now = test_support::fixed_now();
    let project_a = Namespace::for_project("episode-a");
    let project_b = Namespace::for_project("episode-b");

    for (event_id, namespace, recorded_at) in [
        ("episode-a-old", project_a.clone(), now),
        (
            "episode-a-tie",
            project_a.clone(),
            now + chrono::Duration::seconds(60),
        ),
        (
            "episode-a-later-rowid",
            project_a.clone(),
            now + chrono::Duration::seconds(60),
        ),
        (
            "episode-b-new",
            project_b.clone(),
            now + chrono::Duration::seconds(120),
        ),
    ] {
        context
            .store
            .append_event(StoredEvent::new(
                event_id.to_string(),
                recorded_at,
                Event::new_with_namespace(
                    Owner::World,
                    namespace,
                    EventKind::Observation,
                    event_id,
                )
                .unwrap(),
            ))
            .await
            .unwrap();
    }
    context
        .store
        .append_event(StoredEvent::new(
            "episode-a-unknown-owner".to_string(),
            now + chrono::Duration::seconds(180),
            Event::new_with_namespace(
                Owner::Unknown,
                project_a.clone(),
                EventKind::Observation,
                "unknown-owner event must not leak into the World scope",
            )
            .unwrap(),
        ))
        .await
        .unwrap();

    for (episode_reference, event_id) in [
        ("episode:z-tie", "episode-a-old"),
        ("episode:z-tie", "episode-a-tie"),
        ("episode:z-tie", "episode-b-new"),
        ("episode:z-tie", "episode-a-unknown-owner"),
        ("episode:a-tie", "episode-a-tie"),
        ("episode:rowid-winner", "episode-a-later-rowid"),
        ("episode:old", "episode-a-old"),
    ] {
        context
            .store
            .record_event_in_episode(episode_reference.to_string(), event_id.to_string())
            .await
            .unwrap();
    }

    for (claim_id, namespace) in [
        ("claim-a-duplicate", project_a.clone()),
        ("claim-a-new", project_a.clone()),
        ("claim-a-only-cross-event", project_a.clone()),
        ("claim-b", project_b.clone()),
        ("claim-b-cross-link", project_b),
    ] {
        context
            .store
            .upsert_claim(StoredClaim::new(
                claim_id.to_string(),
                ClaimDraft::new_with_namespace(
                    Owner::World,
                    namespace,
                    "episode.fact",
                    "is",
                    claim_id,
                    Mode::Observed,
                ),
                ClaimStatus::Active,
            ))
            .await
            .unwrap();
    }
    context
        .store
        .upsert_claim(StoredClaim::new(
            "claim-unknown-owner".to_string(),
            ClaimDraft::new_with_namespace(
                Owner::Unknown,
                project_a.clone(),
                "episode.fact",
                "must_not",
                "leak through an owner fallback",
                Mode::Observed,
            ),
            ClaimStatus::Active,
        ))
        .await
        .unwrap();
    for (claim_id, event_id) in [
        ("claim-a-duplicate", "episode-a-old"),
        ("claim-a-duplicate", "episode-a-tie"),
        ("claim-a-new", "episode-a-tie"),
        ("claim-a-only-cross-event", "episode-b-new"),
        ("claim-b", "episode-b-new"),
        ("claim-b-cross-link", "episode-a-tie"),
        ("claim-unknown-owner", "episode-a-unknown-owner"),
    ] {
        context
            .store
            .link_evidence(claim_id.to_string(), event_id.to_string())
            .await
            .unwrap();
    }

    let records = context
        .store
        .query_episode_records(EpisodeRecordQuery {
            scope: MemoryScope::for_namespace(project_a.clone()),
            episode_reference: None,
            limit: 3,
        })
        .await
        .unwrap();

    assert_eq!(
        records
            .iter()
            .map(|record| record.episode_reference.as_str())
            .collect::<Vec<_>>(),
        vec!["episode:rowid-winner", "episode:a-tie", "episode:z-tie"]
    );
    let shared = &records[2];
    assert_eq!(
        shared.recorded_at,
        now + chrono::Duration::seconds(60),
        "a newer event from another namespace must not affect the scoped episode timestamp"
    );
    assert_eq!(shared.owner, Owner::World);
    assert_eq!(shared.namespace, project_a);
    assert_eq!(
        shared
            .event_references
            .iter()
            .map(EventReference::canonical)
            .collect::<Vec<_>>(),
        vec!["event:episode-a-tie", "event:episode-a-old"]
    );
    assert_eq!(
        shared
            .claim_references
            .iter()
            .map(ClaimReference::canonical)
            .collect::<Vec<_>>(),
        vec!["claim:claim-a-duplicate", "claim:claim-a-new"],
        "episode claim provenance must be same-scope and distinct"
    );

    let exact = context
        .store
        .query_episode_records(EpisodeRecordQuery {
            scope: MemoryScope::for_namespace(Namespace::for_project("episode-a")),
            episode_reference: Some("episode:z-tie".to_string()),
            limit: 1,
        })
        .await
        .unwrap();
    assert_eq!(exact.len(), 1);
    assert_eq!(exact[0].episode_reference, "episode:z-tie");
}

#[tokio::test]
async fn sqlite_episode_recall_rejects_unscoped_limits_and_cross_scope_exact_references() {
    let context = test_support::new_sqlite_store().await;
    context
        .store
        .append_event(StoredEvent::new(
            "episode-b-only-event".to_string(),
            test_support::fixed_now(),
            Event::new_with_namespace(
                Owner::World,
                Namespace::for_project("episode-b"),
                EventKind::Observation,
                "cross-scope episode",
            )
            .unwrap(),
        ))
        .await
        .unwrap();
    context
        .store
        .record_event_in_episode(
            "episode:b-only".to_string(),
            "episode-b-only-event".to_string(),
        )
        .await
        .unwrap();

    for query in [
        EpisodeRecordQuery {
            scope: MemoryScope::legacy_unscoped(),
            episode_reference: None,
            limit: 10,
        },
        EpisodeRecordQuery {
            scope: MemoryScope::for_namespace(Namespace::for_project("episode-a")),
            episode_reference: None,
            limit: 0,
        },
        EpisodeRecordQuery {
            scope: MemoryScope::for_namespace(Namespace::for_project("episode-a")),
            episode_reference: None,
            limit: 101,
        },
        EpisodeRecordQuery {
            scope: MemoryScope::for_namespace(Namespace::for_project("episode-a")),
            episode_reference: Some(" episode:b-only".to_string()),
            limit: 10,
        },
    ] {
        assert!(matches!(
            context.store.query_episode_records(query).await,
            Err(AppError::InvalidParams(_))
        ));
    }

    let cross_scope = context
        .store
        .query_episode_records(EpisodeRecordQuery {
            scope: MemoryScope::for_namespace(Namespace::for_project("episode-a")),
            episode_reference: Some("episode:b-only".to_string()),
            limit: 10,
        })
        .await
        .unwrap();
    assert!(cross_scope.is_empty());
}

#[tokio::test]
async fn sqlite_claim_recall_is_scoped_status_aware_and_returns_provenance() {
    let context = test_support::new_sqlite_store().await;
    let now = test_support::fixed_now();
    let project_a = Namespace::for_project("a");
    let project_b = Namespace::for_project("b");

    for (event_id, namespace, summary, episode) in [
        (
            "event-a-old",
            project_a.clone(),
            "old project a evidence",
            "episode:a-old",
        ),
        (
            "event-a-new",
            project_a.clone(),
            "new project a evidence",
            "episode:a-new",
        ),
        (
            "event-b",
            project_b.clone(),
            "project b interference",
            "episode:b",
        ),
    ] {
        context
            .store
            .append_event(StoredEvent::new(
                event_id.to_string(),
                now,
                Event::new_with_namespace(Owner::World, namespace, EventKind::Observation, summary)
                    .unwrap(),
            ))
            .await
            .unwrap();
        context
            .store
            .record_event_in_episode(episode.to_string(), event_id.to_string())
            .await
            .unwrap();
    }

    for (claim_id, namespace, object, status, event_id) in [
        (
            "claim-a-old",
            project_a.clone(),
            "old value",
            ClaimStatus::Superseded,
            "event-a-old",
        ),
        (
            "claim-a-new",
            project_a.clone(),
            "new value",
            ClaimStatus::Active,
            "event-a-new",
        ),
        (
            "claim-b",
            project_b,
            "interference",
            ClaimStatus::Active,
            "event-b",
        ),
    ] {
        context
            .store
            .upsert_claim(StoredClaim::new(
                claim_id.to_string(),
                ClaimDraft::new_with_namespace(
                    Owner::World,
                    namespace,
                    "project.fact",
                    "is",
                    object,
                    Mode::Observed,
                ),
                status,
            ))
            .await
            .unwrap();
        context
            .store
            .link_evidence(claim_id.to_string(), event_id.to_string())
            .await
            .unwrap();
    }
    context
        .store
        .append_reflection(StoredReflection::new(
            "reflection-a".to_string(),
            now,
            Reflection::new("replace the old project fact"),
            Some("claim-a-old".to_string()),
            Some("claim-a-new".to_string()),
        ))
        .await
        .unwrap();
    context
        .store
        .link_evidence("claim-a-new".to_string(), "event-b".to_string())
        .await
        .unwrap();

    let active = context
        .store
        .query_claim_records(ClaimRecordQuery {
            scope: MemoryScope::for_namespace(project_a.clone()),
            claim_reference: None,
            status: Some(ClaimStatus::Active),
            mode: Some(Mode::Observed),
            limit: 10,
        })
        .await
        .unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].claim.claim_id, "claim-a-new");
    assert_eq!(active[0].claim.claim.object(), "new value");
    assert_eq!(
        active[0]
            .evidence_event_references
            .iter()
            .map(EventReference::canonical)
            .collect::<Vec<_>>(),
        vec!["event:event-a-new"]
    );
    assert_eq!(active[0].episode_references, vec!["episode:a-new"]);
    assert_eq!(
        active[0].revision.source_reflection_id.as_deref(),
        Some("reflection-a")
    );
    assert_eq!(
        active[0]
            .revision
            .supersedes_claim_reference
            .as_ref()
            .map(ClaimReference::canonical)
            .as_deref(),
        Some("claim:claim-a-old")
    );

    let superseded = context
        .store
        .query_claim_records(ClaimRecordQuery {
            scope: MemoryScope::for_namespace(project_a.clone()),
            claim_reference: Some(ClaimReference::parse("claim:claim-a-old").unwrap()),
            status: Some(ClaimStatus::Superseded),
            mode: None,
            limit: 1,
        })
        .await
        .unwrap();
    assert_eq!(superseded.len(), 1);
    assert_eq!(
        superseded[0]
            .revision
            .replacement_claim_reference
            .as_ref()
            .map(ClaimReference::canonical)
            .as_deref(),
        Some("claim:claim-a-new")
    );

    let cross_scope = context
        .store
        .query_claim_records(ClaimRecordQuery {
            scope: MemoryScope::for_namespace(project_a),
            claim_reference: Some(ClaimReference::parse("claim:claim-b").unwrap()),
            status: None,
            mode: None,
            limit: 1,
        })
        .await
        .unwrap();
    assert!(cross_scope.is_empty());

    let unscoped = context
        .store
        .query_claim_records(ClaimRecordQuery {
            scope: MemoryScope::legacy_unscoped(),
            claim_reference: None,
            status: None,
            mode: None,
            limit: 10,
        })
        .await;
    assert!(matches!(unscoped, Err(AppError::InvalidParams(_))));
}

#[tokio::test]
async fn sqlite_canonical_owner_namespace_writes_are_reachable_through_scoped_reads() {
    let context = test_support::new_sqlite_store().await;
    let now = test_support::fixed_now();
    let cases = [
        Namespace::self_(),
        Namespace::world(),
        Namespace::for_user("alice"),
        Namespace::for_project("demo"),
    ];

    for (index, namespace) in cases.into_iter().enumerate() {
        let owner = namespace.derived_owner();
        let event_id = format!("canonical-event-{index}");
        let claim_id = format!("canonical-claim-{index}");
        context
            .store
            .append_event(StoredEvent::new(
                event_id.clone(),
                now + chrono::Duration::seconds(index as i64),
                Event::new_with_namespace(
                    owner,
                    namespace.clone(),
                    EventKind::Observation,
                    format!("canonical {namespace}"),
                )
                .unwrap(),
            ))
            .await
            .unwrap();
        context
            .store
            .upsert_claim(StoredClaim::new(
                claim_id.clone(),
                ClaimDraft::new_with_namespace(
                    owner,
                    namespace.clone(),
                    "canonical.fact",
                    "is",
                    event_id.clone(),
                    Mode::Observed,
                ),
                ClaimStatus::Active,
            ))
            .await
            .unwrap();

        let events = context
            .store
            .query_event_records(EventRecordQuery {
                scope: MemoryScope::for_namespace(namespace.clone()),
                event_reference: Some(EventReference::parse(&event_id).unwrap()),
                kind: None,
                recorded_after: None,
                recorded_before: None,
                limit: 1,
            })
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event.event.owner(), owner);
        assert_eq!(events[0].event.event.namespace(), &namespace);

        let claims = context
            .store
            .query_claim_records(ClaimRecordQuery {
                scope: MemoryScope::for_namespace(namespace.clone()),
                claim_reference: Some(ClaimReference::parse(&claim_id).unwrap()),
                status: None,
                mode: None,
                limit: 1,
            })
            .await
            .unwrap();
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].claim.claim.owner(), owner);
        assert_eq!(claims[0].claim.claim.namespace(), &namespace);

        let other = if namespace.as_str() == "world" {
            Namespace::for_project("demo")
        } else {
            Namespace::world()
        };
        assert!(
            context
                .store
                .query_event_records(EventRecordQuery {
                    scope: MemoryScope::for_namespace(other),
                    event_reference: Some(EventReference::parse(&event_id).unwrap()),
                    kind: None,
                    recorded_after: None,
                    recorded_before: None,
                    limit: 1,
                })
                .await
                .unwrap()
                .is_empty()
        );
    }
}

#[tokio::test]
async fn sqlite_claim_revision_links_hide_mixed_scope_edges() {
    let context = test_support::new_sqlite_store().await;
    let now = test_support::fixed_now();
    let project_a = Namespace::for_project("revision-a");
    let project_b = Namespace::for_project("revision-b");

    for (claim_id, namespace, status) in [
        ("claim-a-old", project_a.clone(), ClaimStatus::Superseded),
        ("claim-b-new", project_b.clone(), ClaimStatus::Active),
        (
            "claim-a-same-old",
            project_a.clone(),
            ClaimStatus::Superseded,
        ),
        ("claim-a-same-new", project_a.clone(), ClaimStatus::Active),
    ] {
        context
            .store
            .upsert_claim(StoredClaim::new(
                claim_id.to_string(),
                ClaimDraft::new_with_namespace(
                    Owner::World,
                    namespace,
                    "project.fact",
                    "is",
                    claim_id,
                    Mode::Observed,
                ),
                status,
            ))
            .await
            .unwrap();
    }
    context
        .store
        .append_reflection(StoredReflection::new(
            "reflection-mixed".to_string(),
            now,
            Reflection::new("cross-scope replacement must stay hidden"),
            Some("claim-a-old".to_string()),
            Some("claim-b-new".to_string()),
        ))
        .await
        .unwrap();
    context
        .store
        .append_reflection(StoredReflection::new(
            "reflection-same".to_string(),
            now + chrono::Duration::seconds(1),
            Reflection::new("same-scope replacement remains visible"),
            Some("claim-a-same-old".to_string()),
            Some("claim-a-same-new".to_string()),
        ))
        .await
        .unwrap();

    let mixed_source = context
        .store
        .query_claim_records(ClaimRecordQuery {
            scope: MemoryScope::for_namespace(project_a.clone()),
            claim_reference: Some(ClaimReference::parse("claim:claim-a-old").unwrap()),
            status: None,
            mode: None,
            limit: 1,
        })
        .await
        .unwrap();
    assert_eq!(mixed_source.len(), 1);
    assert_eq!(mixed_source[0].revision, ClaimRevisionLinks::default());

    let mixed_replacement = context
        .store
        .query_claim_records(ClaimRecordQuery {
            scope: MemoryScope::for_namespace(project_b),
            claim_reference: Some(ClaimReference::parse("claim:claim-b-new").unwrap()),
            status: None,
            mode: None,
            limit: 1,
        })
        .await
        .unwrap();
    assert_eq!(mixed_replacement.len(), 1);
    assert_eq!(mixed_replacement[0].revision, ClaimRevisionLinks::default());

    let same_scope = context
        .store
        .query_claim_records(ClaimRecordQuery {
            scope: MemoryScope::for_namespace(project_a),
            claim_reference: Some(ClaimReference::parse("claim:claim-a-same-new").unwrap()),
            status: None,
            mode: None,
            limit: 1,
        })
        .await
        .unwrap();
    assert_eq!(
        same_scope[0].revision.source_reflection_id.as_deref(),
        Some("reflection-same")
    );
    assert_eq!(
        same_scope[0]
            .revision
            .supersedes_claim_reference
            .as_ref()
            .map(ClaimReference::canonical)
            .as_deref(),
        Some("claim:claim-a-same-old")
    );
}

#[tokio::test]
async fn sqlite_claim_reflection_history_is_scoped_reachable_bounded_and_evidence_filtered() {
    let context = test_support::new_sqlite_store().await;
    let now = test_support::fixed_now();
    let project_a = Namespace::for_project("history-a");
    let project_b = Namespace::for_project("history-b");

    for (event_id, namespace) in [
        ("history-event-a-1", project_a.clone()),
        ("history-event-a-2", project_a.clone()),
        ("history-event-b", project_b.clone()),
    ] {
        context
            .store
            .append_event(StoredEvent::new(
                event_id.to_string(),
                now,
                Event::new_with_namespace(Owner::World, namespace, EventKind::Reflection, event_id)
                    .unwrap(),
            ))
            .await
            .unwrap();
    }

    for (claim_id, namespace, status) in [
        (
            "history-claim-old",
            project_a.clone(),
            ClaimStatus::Superseded,
        ),
        (
            "history-claim-middle",
            project_a.clone(),
            ClaimStatus::Superseded,
        ),
        (
            "history-claim-new",
            project_a.clone(),
            ClaimStatus::Disputed,
        ),
        ("history-claim-b", project_b.clone(), ClaimStatus::Active),
    ] {
        context
            .store
            .upsert_claim(StoredClaim::new(
                claim_id.to_string(),
                ClaimDraft::new_with_namespace(
                    Owner::World,
                    namespace,
                    "history.fact",
                    "is",
                    claim_id,
                    Mode::Observed,
                ),
                status,
            ))
            .await
            .unwrap();
    }

    for reflection in [
        StoredReflection::new(
            "history-reflection-old".to_string(),
            now,
            Reflection::new("replace old with middle"),
            Some("history-claim-old".to_string()),
            Some("history-claim-middle".to_string()),
        )
        .with_supporting_evidence_event_ids(vec![
            "history-event-a-1".to_string(),
            "history-event-b".to_string(),
            "history-event-a-1".to_string(),
        ]),
        StoredReflection::new(
            "history-reflection-new".to_string(),
            now + chrono::Duration::seconds(1),
            Reflection::new("replace middle with new"),
            Some("history-claim-middle".to_string()),
            Some("history-claim-new".to_string()),
        )
        .with_supporting_evidence_event_ids(vec!["history-event-a-2".to_string()]),
        StoredReflection::new(
            "history-reflection-cycle".to_string(),
            now + chrono::Duration::seconds(1),
            Reflection::new("malformed legacy cycle remains bounded"),
            Some("history-claim-new".to_string()),
            Some("history-claim-old".to_string()),
        ),
        StoredReflection::new(
            "history-reflection-dispute".to_string(),
            now + chrono::Duration::seconds(2),
            Reflection::new("dispute the newest claim"),
            Some("history-claim-new".to_string()),
            None,
        )
        .with_supporting_evidence_event_ids(vec!["history-event-a-2".to_string()]),
        StoredReflection::new(
            "history-reflection-cross-scope".to_string(),
            now + chrono::Duration::seconds(3),
            Reflection::new("must remain hidden"),
            Some("history-claim-old".to_string()),
            Some("history-claim-b".to_string()),
        ),
        StoredReflection::new(
            "history-reflection-record-only".to_string(),
            now + chrono::Duration::seconds(4),
            Reflection::new("not claim-linked"),
            None,
            None,
        ),
    ] {
        context.store.append_reflection(reflection).await.unwrap();
    }

    for anchor in [
        "history-claim-old",
        "history-claim-middle",
        "history-claim-new",
    ] {
        let page = context
            .store
            .query_claim_reflection_history(ClaimReflectionHistoryQuery {
                scope: MemoryScope::for_namespace(project_a.clone()),
                claim_reference: ClaimReference::parse(anchor).unwrap(),
                limit: 10,
            })
            .await
            .unwrap();
        assert!(!page.has_more);
        assert_eq!(
            page.records
                .iter()
                .map(|record| record.reflection_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "history-reflection-dispute",
                "history-reflection-cycle",
                "history-reflection-new",
                "history-reflection-old",
            ]
        );
        assert_eq!(
            page.records[3]
                .supporting_evidence_event_references
                .iter()
                .map(EventReference::canonical)
                .collect::<Vec<_>>(),
            vec!["event:history-event-a-1"]
        );
    }

    let bounded = context
        .store
        .query_claim_reflection_history(ClaimReflectionHistoryQuery {
            scope: MemoryScope::for_namespace(project_a.clone()),
            claim_reference: ClaimReference::parse("claim:history-claim-middle").unwrap(),
            limit: 2,
        })
        .await
        .unwrap();
    assert!(bounded.has_more);
    assert_eq!(bounded.records.len(), 2);

    for reference in ["history-claim-b", "history-claim-missing"] {
        let empty = context
            .store
            .query_claim_reflection_history(ClaimReflectionHistoryQuery {
                scope: MemoryScope::for_namespace(project_a.clone()),
                claim_reference: ClaimReference::parse(reference).unwrap(),
                limit: 10,
            })
            .await
            .unwrap();
        assert!(empty.records.is_empty());
        assert!(!empty.has_more);
    }

    let unscoped = context
        .store
        .query_claim_reflection_history(ClaimReflectionHistoryQuery {
            scope: MemoryScope::legacy_unscoped(),
            claim_reference: ClaimReference::parse("history-claim-old").unwrap(),
            limit: 10,
        })
        .await;
    assert!(matches!(unscoped, Err(AppError::InvalidParams(_))));
}

#[tokio::test]
async fn sqlite_claim_reflection_history_rejects_malformed_legacy_evidence_json() {
    let context = test_support::new_sqlite_store().await;
    let namespace = Namespace::for_project("history-malformed");
    context
        .store
        .upsert_claim(StoredClaim::new(
            "history-malformed-claim".to_string(),
            ClaimDraft::new_with_namespace(
                Owner::World,
                namespace.clone(),
                "history.fact",
                "is",
                "malformed",
                Mode::Observed,
            ),
            ClaimStatus::Disputed,
        ))
        .await
        .unwrap();
    sqlx::query(
        r#"
        INSERT INTO reflections (
            reflection_id, recorded_at, summary, superseded_claim_id,
            replacement_claim_id, supporting_evidence_event_ids
        ) VALUES (?, ?, ?, ?, NULL, ?)
        "#,
    )
    .bind("history-reflection-malformed")
    .bind(test_support::fixed_now().to_rfc3339())
    .bind("malformed legacy evidence must fail closed")
    .bind("history-malformed-claim")
    .bind("not-json")
    .execute(&context.pool)
    .await
    .unwrap();

    let result = context
        .store
        .query_claim_reflection_history(ClaimReflectionHistoryQuery {
            scope: MemoryScope::for_namespace(namespace),
            claim_reference: ClaimReference::parse("history-malformed-claim").unwrap(),
            limit: 10,
        })
        .await;

    assert!(matches!(result, Err(AppError::Message(_))));
}

#[tokio::test]
async fn sqlite_evidence_relation_runtime_is_scoped_intersect_only_and_hides_cross_scope_ids() {
    let context = test_support::new_sqlite_store().await;
    let now = test_support::fixed_now();
    let project_a = Namespace::for_project("relation-a");
    let project_b = Namespace::for_project("relation-b");

    for (event_id, namespace) in [
        ("relation-event-a1", project_a.clone()),
        ("relation-event-a2", project_a.clone()),
        ("relation-event-b", project_b.clone()),
    ] {
        context
            .store
            .append_event(StoredEvent::new(
                event_id.to_string(),
                now,
                Event::new_with_namespace(
                    Owner::World,
                    namespace,
                    EventKind::Observation,
                    event_id,
                )
                .unwrap(),
            ))
            .await
            .unwrap();
    }

    let membership = context
        .store
        .query_scoped_event_ids(ScopedEventIdQuery {
            scope: MemoryScope::for_namespace(project_a.clone()),
            event_ids: vec![
                "relation-event-a2".to_string(),
                "relation-event-missing".to_string(),
                "relation-event-b".to_string(),
                "relation-event-a1".to_string(),
            ],
        })
        .await
        .unwrap();
    assert_eq!(
        membership.into_iter().collect::<Vec<_>>(),
        vec![
            "relation-event-a1".to_string(),
            "relation-event-a2".to_string()
        ]
    );

    let report = get_evidence_relation(
        &context.store,
        GetEvidenceRelationInput {
            namespace: project_a.clone(),
            trigger_window: vec![
                EventReference::parse("event:relation-event-a2").unwrap(),
                EventReference::parse("relation-event-b").unwrap(),
                EventReference::parse("relation-event-missing").unwrap(),
                EventReference::parse("relation-event-a1").unwrap(),
                EventReference::parse("relation-event-a2").unwrap(),
            ],
            selected_evidence: vec![EventReference::parse("relation-event-a1").unwrap()],
            selection_basis: Some("explicit_model_ids".to_string()),
        },
    )
    .await
    .unwrap();
    assert_eq!(report.owner, Owner::World);
    assert_eq!(report.namespace, "project/relation-a");
    assert_eq!(report.trigger_window_size, 2);
    assert_eq!(report.selected_count, 1);
    assert_eq!(report.rejected_count, 1);
    assert_eq!(
        report
            .relations
            .iter()
            .map(|relation| {
                (
                    relation.event_reference.as_str(),
                    relation.window_rank,
                    relation.selected,
                    relation.relation_status.as_str(),
                )
            })
            .collect::<Vec<_>>(),
        vec![
            (
                "event:relation-event-a2",
                1,
                false,
                "available_not_selected"
            ),
            ("event:relation-event-a1", 2, true, "selected"),
        ]
    );
    assert_eq!(
        report.relations[1].selection_basis.as_deref(),
        Some("explicit_model_ids")
    );

    let cross_scope_selected = get_evidence_relation(
        &context.store,
        GetEvidenceRelationInput {
            namespace: project_a.clone(),
            trigger_window: vec![
                EventReference::parse("relation-event-a1").unwrap(),
                EventReference::parse("relation-event-b").unwrap(),
            ],
            selected_evidence: vec![EventReference::parse("relation-event-b").unwrap()],
            selection_basis: None,
        },
    )
    .await
    .expect_err("selected evidence outside the scoped window must fail closed");
    assert!(
        cross_scope_selected
            .to_string()
            .contains("outside the trigger window")
    );

    let empty = get_evidence_relation(
        &context.store,
        GetEvidenceRelationInput {
            namespace: project_a,
            trigger_window: Vec::new(),
            selected_evidence: Vec::new(),
            selection_basis: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(empty.trigger_window_size, 0);
    assert!(empty.relations.is_empty());

    let unscoped = context
        .store
        .query_scoped_event_ids(ScopedEventIdQuery {
            scope: MemoryScope::legacy_unscoped(),
            event_ids: vec!["relation-event-a1".to_string()],
        })
        .await;
    assert!(matches!(unscoped, Err(AppError::InvalidParams(_))));
}

#[tokio::test]
async fn sqlite_search_memory_union_is_scoped_stable_sorted_and_hides_cross_scope_types() {
    let context = test_support::new_sqlite_store().await;
    let now = test_support::fixed_now();
    let project_a = Namespace::for_project("union-a");
    let project_b = Namespace::for_project("union-b");

    for (event_id, namespace, recorded_at) in [
        ("union-event-old", project_a.clone(), now),
        (
            "union-event-new",
            project_a.clone(),
            now + chrono::Duration::seconds(60),
        ),
        (
            "union-event-b",
            project_b.clone(),
            now + chrono::Duration::seconds(180),
        ),
    ] {
        context
            .store
            .append_event(StoredEvent::new(
                event_id.to_string(),
                recorded_at,
                Event::new_with_namespace(
                    Owner::World,
                    namespace,
                    EventKind::Observation,
                    event_id,
                )
                .unwrap(),
            ))
            .await
            .unwrap();
    }
    context
        .store
        .record_event_in_episode("episode:union-a".to_string(), "union-event-new".to_string())
        .await
        .unwrap();
    context
        .store
        .record_event_in_episode("episode:union-b".to_string(), "union-event-b".to_string())
        .await
        .unwrap();

    for (claim_id, namespace, status) in [
        ("union-claim-a", project_a.clone(), ClaimStatus::Active),
        (
            "union-claim-old",
            project_a.clone(),
            ClaimStatus::Superseded,
        ),
        ("union-claim-b", project_b.clone(), ClaimStatus::Active),
    ] {
        context
            .store
            .upsert_claim(StoredClaim::new(
                claim_id.to_string(),
                ClaimDraft::new_with_namespace(
                    Owner::World,
                    namespace,
                    "union.fact",
                    "is",
                    claim_id,
                    Mode::Observed,
                ),
                status,
            ))
            .await
            .unwrap();
    }
    context
        .store
        .append_reflection(StoredReflection::new(
            "union-reflection-a".to_string(),
            now + chrono::Duration::seconds(120),
            Reflection::new("scoped union reflection"),
            Some("union-claim-old".to_string()),
            None,
        ))
        .await
        .unwrap();
    context
        .store
        .append_reflection(StoredReflection::new(
            "union-reflection-b".to_string(),
            now + chrono::Duration::seconds(240),
            Reflection::new("other namespace"),
            Some("union-claim-b".to_string()),
            None,
        ))
        .await
        .unwrap();

    let result = search_memory(
        &context.store,
        SearchMemoryInput {
            namespace: project_a.clone(),
            record_types: vec![
                MemoryRecordType::Event,
                MemoryRecordType::Claim,
                MemoryRecordType::Episode,
                MemoryRecordType::Reflection,
            ],
            event_reference: None,
            kind: None,
            recorded_after: None,
            recorded_before: None,
            claim_reference: None,
            claim_status: None,
            mode: None,
            episode_reference: None,
            reflection_reference: None,
            limit: 10,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        result.record_types,
        vec!["event", "claim", "episode", "reflection"]
    );
    assert_eq!(
        result
            .records
            .iter()
            .map(|record| match record {
                SearchMemoryRecord::Event { id, .. } => ("event", id.as_str()),
                SearchMemoryRecord::Claim { id, .. } => ("claim", id.as_str()),
                SearchMemoryRecord::Episode { id, .. } => ("episode", id.as_str()),
                SearchMemoryRecord::Reflection { id, .. } => ("reflection", id.as_str()),
            })
            .collect::<Vec<_>>(),
        vec![
            ("reflection", "union-reflection-a"),
            ("event", "event:union-event-new"),
            ("episode", "episode:union-a"),
            ("event", "event:union-event-old"),
            ("claim", "claim:union-claim-a"),
        ]
    );

    let bounded = search_memory(
        &context.store,
        SearchMemoryInput {
            namespace: project_a.clone(),
            record_types: vec![MemoryRecordType::Event, MemoryRecordType::Reflection],
            event_reference: None,
            kind: None,
            recorded_after: None,
            recorded_before: None,
            claim_reference: None,
            claim_status: None,
            mode: None,
            episode_reference: None,
            reflection_reference: None,
            limit: 2,
        },
    )
    .await
    .unwrap();
    assert_eq!(bounded.records.len(), 2);
    assert!(matches!(
        &bounded.records[0],
        SearchMemoryRecord::Reflection { id, .. } if id == "union-reflection-a"
    ));
    assert!(matches!(
        &bounded.records[1],
        SearchMemoryRecord::Event { id, .. } if id == "event:union-event-new"
    ));

    let events_only = search_memory(
        &context.store,
        SearchMemoryInput {
            namespace: project_a,
            record_types: vec![MemoryRecordType::Event],
            event_reference: None,
            kind: None,
            recorded_after: None,
            recorded_before: None,
            claim_reference: None,
            claim_status: None,
            mode: None,
            episode_reference: None,
            reflection_reference: None,
            limit: 10,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        events_only
            .records
            .iter()
            .map(|record| match record {
                SearchMemoryRecord::Event { id, .. } => id.as_str(),
                _ => "not-event",
            })
            .collect::<Vec<_>>(),
        vec!["event:union-event-new", "event:union-event-old"]
    );
}

#[tokio::test]
async fn sqlite_self_model_history_is_scoped_claim_attributed_and_hides_record_only_rows() {
    let context = test_support::new_sqlite_store().await;
    let now = test_support::fixed_now();
    let project_a = Namespace::for_project("self-model-a");
    let project_b = Namespace::for_project("self-model-b");

    for (event_id, namespace) in [
        ("self-model-event-a", project_a.clone()),
        ("self-model-event-b", project_b.clone()),
    ] {
        context
            .store
            .append_event(StoredEvent::new(
                event_id.to_string(),
                now,
                Event::new_with_namespace(
                    Owner::World,
                    namespace,
                    EventKind::Observation,
                    event_id,
                )
                .unwrap(),
            ))
            .await
            .unwrap();
    }
    for (claim_id, namespace) in [
        ("self-model-claim-a", project_a.clone()),
        ("self-model-claim-b", project_b.clone()),
    ] {
        context
            .store
            .upsert_claim(StoredClaim::new(
                claim_id.to_string(),
                ClaimDraft::new_with_namespace(
                    Owner::World,
                    namespace,
                    "self.role",
                    "is",
                    claim_id,
                    Mode::Observed,
                ),
                ClaimStatus::Superseded,
            ))
            .await
            .unwrap();
    }

    context
        .store
        .append_reflection(
            StoredReflection::new(
                "self-model-identity-a".to_string(),
                now + chrono::Duration::seconds(2),
                Reflection::new("scoped identity update"),
                Some("self-model-claim-a".to_string()),
                None,
            )
            .with_supporting_evidence_event_ids(vec!["self-model-event-a".to_string()])
            .with_requested_identity_update(Some(ReflectionIdentityUpdate::new(vec![
                "identity:self=scoped".to_string(),
            ]))),
        )
        .await
        .unwrap();
    context
        .store
        .append_reflection(
            StoredReflection::new(
                "self-model-identity-a-old".to_string(),
                now,
                Reflection::new("older scoped identity update"),
                Some("self-model-claim-a".to_string()),
                None,
            )
            .with_requested_identity_update(Some(ReflectionIdentityUpdate::new(vec![
                "identity:self=older".to_string(),
            ]))),
        )
        .await
        .unwrap();
    context
        .store
        .append_reflection(
            StoredReflection::new(
                "self-model-mixed-edge".to_string(),
                now + chrono::Duration::seconds(6),
                Reflection::new("mixed-scope replacement hides identity audit"),
                Some("self-model-claim-a".to_string()),
                Some("self-model-claim-b".to_string()),
            )
            .with_requested_identity_update(Some(ReflectionIdentityUpdate::new(vec![
                "identity:self=mixed".to_string(),
            ]))),
        )
        .await
        .unwrap();
    context
        .store
        .append_reflection(
            StoredReflection::new(
                "self-model-commitment-a".to_string(),
                now + chrono::Duration::seconds(1),
                Reflection::new("scoped commitment update"),
                Some("self-model-claim-a".to_string()),
                None,
            )
            .with_requested_commitment_updates(Some(vec![Commitment::new(
                Owner::Self_,
                "prefer:scoped_history",
            )])),
        )
        .await
        .unwrap();
    context
        .store
        .append_reflection(StoredReflection::new(
            "self-model-claim-only-a".to_string(),
            now + chrono::Duration::seconds(3),
            Reflection::new("claim revision without self-model patch"),
            Some("self-model-claim-a".to_string()),
            None,
        ))
        .await
        .unwrap();
    context
        .store
        .append_reflection(
            StoredReflection::new(
                "self-model-record-only".to_string(),
                now + chrono::Duration::seconds(4),
                Reflection::new("record-only identity update"),
                None,
                None,
            )
            .with_requested_identity_update(Some(ReflectionIdentityUpdate::new(vec![
                "identity:self=hidden".to_string(),
            ]))),
        )
        .await
        .unwrap();
    context
        .store
        .append_reflection(
            StoredReflection::new(
                "self-model-identity-b".to_string(),
                now + chrono::Duration::seconds(5),
                Reflection::new("other namespace"),
                Some("self-model-claim-b".to_string()),
                None,
            )
            .with_requested_identity_update(Some(ReflectionIdentityUpdate::new(vec![
                "identity:self=other".to_string(),
            ]))),
        )
        .await
        .unwrap();

    let identity = get_self_model_history(
        &context.store,
        GetSelfModelHistoryInput {
            namespace: project_a.clone(),
            history_kind: SelfModelHistoryKind::Identity,
            limit: 10,
        },
    )
    .await
    .unwrap();
    assert_eq!(identity.history_type, "identity");
    assert_eq!(
        identity
            .records
            .iter()
            .map(|record| record.reflection_id.as_str())
            .collect::<Vec<_>>(),
        vec!["self-model-identity-a", "self-model-identity-a-old"]
    );
    assert_eq!(
        identity.records[0]
            .identity_update
            .as_ref()
            .map(|update| update.canonical_claims.clone()),
        Some(vec!["identity:self=scoped".to_string()])
    );
    assert!(identity.records[0].commitment_updates.is_none());
    assert_eq!(
        identity.records[0].supporting_evidence_event_references,
        vec!["event:self-model-event-a"]
    );

    let commitments = context
        .store
        .query_self_model_history(SelfModelHistoryQuery {
            scope: MemoryScope::for_namespace(project_a.clone()),
            history_kind: SelfModelHistoryKind::Commitment,
            limit: 10,
        })
        .await
        .unwrap();
    assert_eq!(commitments.records.len(), 1);
    assert_eq!(
        commitments.records[0].reflection_id,
        "self-model-commitment-a"
    );
    assert!(commitments.records[0].identity_update.is_none());
    assert_eq!(
        commitments.records[0]
            .commitment_updates
            .as_ref()
            .map(|updates| updates[0].description()),
        Some("prefer:scoped_history")
    );

    let bounded = get_self_model_history(
        &context.store,
        GetSelfModelHistoryInput {
            namespace: project_a,
            history_kind: SelfModelHistoryKind::Identity,
            limit: 1,
        },
    )
    .await
    .unwrap();
    assert_eq!(bounded.records.len(), 1);
    assert_eq!(bounded.records[0].reflection_id, "self-model-identity-a");
    assert!(bounded.has_more);

    let unscoped = context
        .store
        .query_self_model_history(SelfModelHistoryQuery {
            scope: MemoryScope::legacy_unscoped(),
            history_kind: SelfModelHistoryKind::Identity,
            limit: 10,
        })
        .await;
    assert!(matches!(unscoped, Err(AppError::InvalidParams(_))));
}

#[tokio::test]
async fn sqlite_supersede_memory_is_scoped_claim_correction_and_hides_cross_scope_targets() {
    let context = test_support::new_sqlite_store().await;
    let now = test_support::fixed_now();
    let project_a = Namespace::for_project("supersede-a");
    let project_b = Namespace::for_project("supersede-b");

    for (event_id, namespace) in [
        ("supersede-event-a", project_a.clone()),
        ("supersede-event-b", project_b.clone()),
    ] {
        context
            .store
            .append_event(StoredEvent::new(
                event_id.to_string(),
                now,
                Event::new_with_namespace(
                    Owner::World,
                    namespace,
                    EventKind::Observation,
                    event_id,
                )
                .unwrap(),
            ))
            .await
            .unwrap();
    }
    for (claim_id, namespace) in [
        ("supersede-claim-a", project_a.clone()),
        ("supersede-claim-b", project_b.clone()),
    ] {
        context
            .store
            .upsert_claim(StoredClaim::new(
                claim_id.to_string(),
                ClaimDraft::new_with_namespace(
                    Owner::World,
                    namespace,
                    "self.role",
                    "is",
                    claim_id,
                    Mode::Observed,
                ),
                ClaimStatus::Active,
            ))
            .await
            .unwrap();
    }

    let prepared = prepare_scoped_supersede(
        &context.store,
        &SupersedeMemoryInput {
            request_id: None,
            namespace: project_a.clone(),
            claim_reference: ClaimReference::parse("claim:supersede-claim-a").unwrap(),
            replacement_claim: ClaimDraft::new_with_namespace(
                Owner::World,
                project_a.clone(),
                "self.role",
                "is",
                "corrected",
                Mode::Observed,
            ),
            evidence_event_ids: vec![EventReference::parse("event:supersede-event-a").unwrap()],
            summary: "correct the scoped claim".to_string(),
        },
    )
    .await
    .unwrap();
    let reflections_before = context
        .store
        .query_reflection_records(ReflectionRecordQuery {
            scope: MemoryScope::for_namespace(project_a.clone()),
            reflection_reference: None,
            limit: 10,
        })
        .await
        .unwrap();
    assert!(reflections_before.is_empty());
    drop(prepared);

    let cross_claim = prepare_scoped_supersede(
        &context.store,
        &SupersedeMemoryInput {
            request_id: None,
            namespace: project_a.clone(),
            claim_reference: ClaimReference::parse("supersede-claim-b").unwrap(),
            replacement_claim: ClaimDraft::new_with_namespace(
                Owner::World,
                project_a.clone(),
                "self.role",
                "is",
                "leaked",
                Mode::Observed,
            ),
            evidence_event_ids: vec![EventReference::parse("supersede-event-a").unwrap()],
            summary: "must not touch the other namespace".to_string(),
        },
    )
    .await;
    assert!(matches!(cross_claim, Err(AppError::InvalidParams(_))));

    let cross_evidence = prepare_scoped_supersede(
        &context.store,
        &SupersedeMemoryInput {
            request_id: None,
            namespace: project_a.clone(),
            claim_reference: ClaimReference::parse("supersede-claim-a").unwrap(),
            replacement_claim: ClaimDraft::new_with_namespace(
                Owner::World,
                project_a.clone(),
                "self.role",
                "is",
                "leaked-evidence",
                Mode::Observed,
            ),
            evidence_event_ids: vec![EventReference::parse("event:supersede-event-b").unwrap()],
            summary: "must not use foreign evidence".to_string(),
        },
    )
    .await;
    assert!(matches!(cross_evidence, Err(AppError::InvalidParams(_))));

    let missing = prepare_scoped_supersede(
        &context.store,
        &SupersedeMemoryInput {
            request_id: None,
            namespace: project_a.clone(),
            claim_reference: ClaimReference::parse("claim:missing").unwrap(),
            replacement_claim: ClaimDraft::new_with_namespace(
                Owner::World,
                project_a.clone(),
                "self.role",
                "is",
                "missing",
                Mode::Observed,
            ),
            evidence_event_ids: vec![EventReference::parse("event:supersede-event-a").unwrap()],
            summary: "missing target".to_string(),
        },
    )
    .await;
    assert!(matches!(missing, Err(AppError::InvalidParams(_))));

    let mismatched_replacement = SupersedeMemoryInput {
        request_id: None,
        namespace: project_a,
        claim_reference: ClaimReference::parse("supersede-claim-a").unwrap(),
        replacement_claim: ClaimDraft::new_with_namespace(
            Owner::World,
            project_b,
            "self.role",
            "is",
            "other-scope",
            Mode::Observed,
        ),
        evidence_event_ids: vec![EventReference::parse("event:supersede-event-a").unwrap()],
        summary: "replacement leaves the requested namespace".to_string(),
    }
    .validate();
    assert!(matches!(
        mismatched_replacement,
        Err(AppError::InvalidParams(_))
    ));
}

#[tokio::test]
async fn sqlite_reflection_recall_is_scoped_claim_attributed_and_hides_record_only_rows() {
    let context = test_support::new_sqlite_store().await;
    let now = test_support::fixed_now();
    let project_a = Namespace::for_project("reflection-a");
    let project_b = Namespace::for_project("reflection-b");

    for (event_id, namespace) in [
        ("reflection-event-a", project_a.clone()),
        ("reflection-event-b", project_b.clone()),
    ] {
        context
            .store
            .append_event(StoredEvent::new(
                event_id.to_string(),
                now,
                Event::new_with_namespace(Owner::World, namespace, EventKind::Reflection, event_id)
                    .unwrap(),
            ))
            .await
            .unwrap();
    }
    for (claim_id, namespace, status) in [
        (
            "reflection-claim-old",
            project_a.clone(),
            ClaimStatus::Superseded,
        ),
        (
            "reflection-claim-new",
            project_a.clone(),
            ClaimStatus::Active,
        ),
        (
            "reflection-claim-disputed",
            project_a.clone(),
            ClaimStatus::Disputed,
        ),
        ("reflection-claim-b", project_b.clone(), ClaimStatus::Active),
    ] {
        context
            .store
            .upsert_claim(StoredClaim::new(
                claim_id.to_string(),
                ClaimDraft::new_with_namespace(
                    Owner::World,
                    namespace,
                    "reflection.fact",
                    "is",
                    claim_id,
                    Mode::Observed,
                ),
                status,
            ))
            .await
            .unwrap();
    }
    for reflection in [
        StoredReflection::new(
            "reflection-same".to_string(),
            now,
            Reflection::new("same-scope replacement"),
            Some("reflection-claim-old".to_string()),
            Some("reflection-claim-new".to_string()),
        )
        .with_supporting_evidence_event_ids(vec![
            "reflection-event-a".to_string(),
            "reflection-event-b".to_string(),
        ]),
        StoredReflection::new(
            "reflection-dispute".to_string(),
            now + chrono::Duration::seconds(1),
            Reflection::new("same-scope dispute"),
            Some("reflection-claim-disputed".to_string()),
            None,
        )
        .with_supporting_evidence_event_ids(vec!["reflection-event-a".to_string()]),
        StoredReflection::new(
            "reflection-mixed".to_string(),
            now + chrono::Duration::seconds(2),
            Reflection::new("mixed-scope must stay hidden"),
            Some("reflection-claim-old".to_string()),
            Some("reflection-claim-b".to_string()),
        ),
        StoredReflection::new(
            "reflection-record-only".to_string(),
            now + chrono::Duration::seconds(3),
            Reflection::new("record-only has no claim anchor"),
            None,
            None,
        ),
        StoredReflection::new(
            "reflection-b-only".to_string(),
            now + chrono::Duration::seconds(4),
            Reflection::new("other namespace"),
            Some("reflection-claim-b".to_string()),
            None,
        ),
    ] {
        context.store.append_reflection(reflection).await.unwrap();
    }

    let scoped = context
        .store
        .query_reflection_records(ReflectionRecordQuery {
            scope: MemoryScope::for_namespace(project_a.clone()),
            reflection_reference: None,
            limit: 10,
        })
        .await
        .unwrap();
    assert_eq!(
        scoped
            .iter()
            .map(|record| record.reflection_id.as_str())
            .collect::<Vec<_>>(),
        vec!["reflection-dispute", "reflection-same"]
    );
    assert_eq!(scoped[0].owner, Owner::World);
    assert_eq!(scoped[0].namespace, project_a);
    assert_eq!(
        scoped[1]
            .provenance
            .supporting_evidence_event_references
            .iter()
            .map(EventReference::canonical)
            .collect::<Vec<_>>(),
        vec!["event:reflection-event-a"]
    );
    assert_eq!(
        scoped[1]
            .provenance
            .superseded_claim_reference
            .as_ref()
            .map(ClaimReference::canonical)
            .as_deref(),
        Some("claim:reflection-claim-old")
    );

    let exact = context
        .store
        .query_reflection_records(ReflectionRecordQuery {
            scope: MemoryScope::for_namespace(project_a.clone()),
            reflection_reference: Some("reflection-same".to_string()),
            limit: 1,
        })
        .await
        .unwrap();
    assert_eq!(exact.len(), 1);
    assert_eq!(exact[0].reflection_id, "reflection-same");

    for hidden in [
        "reflection-mixed",
        "reflection-record-only",
        "reflection-b-only",
    ] {
        let empty = context
            .store
            .query_reflection_records(ReflectionRecordQuery {
                scope: MemoryScope::for_namespace(project_a.clone()),
                reflection_reference: Some(hidden.to_string()),
                limit: 1,
            })
            .await
            .unwrap();
        assert!(
            empty.is_empty(),
            "{hidden} must be indistinguishable from missing in this scope"
        );
    }

    let unscoped = context
        .store
        .query_reflection_records(ReflectionRecordQuery {
            scope: MemoryScope::legacy_unscoped(),
            reflection_reference: None,
            limit: 10,
        })
        .await;
    assert!(matches!(unscoped, Err(AppError::InvalidParams(_))));
}
