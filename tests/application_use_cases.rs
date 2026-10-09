use std::sync::{Arc, Mutex};

use agent_llm_mm::{
    adapters::model::mock::MockModel,
    application::{
        auto_reflect_if_needed::{self, AutoReflectInput},
        build_self_snapshot::{BuildSelfSnapshotInput, execute as execute_build_snapshot},
        decide_with_snapshot::{DecideWithSnapshotInput, execute as execute_decide},
        ingest_interaction::{IngestInput, execute as execute_ingest},
        run_reflection::{ReflectionInput, execute as execute_reflection},
    },
    domain::{
        claim::ClaimDraft,
        commitment::Commitment,
        event::{Event, EventReference, MAX_EVIDENCE_MANIFEST_ITEMS},
        identity_core::IdentityCore,
        reflection::Reflection,
        self_revision::{SelfRevisionProposal, SelfRevisionRequest, TriggerType},
        snapshot::{SelfSnapshot, SnapshotBudget, SnapshotTimeWindow},
        types::{EventKind, MemoryScope, Mode, Namespace, Owner},
    },
    error::AppError,
    ports::{
        ClaimStatus, ClaimStore, Clock, CommitmentStore, EpisodeStore, EventStore, EvidenceQuery,
        IdGenerator, IdentityStore, IngestTransaction, IngestTransactionRunner, ModelDecision,
        ModelInput, ModelPort, ReflectionStore, ReflectionTransaction, ReflectionTransactionRunner,
        StoredClaim, StoredEvent, StoredReflection, StoredTriggerLedgerEntry, TriggerLedgerStatus,
        TriggerLedgerStore,
    },
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};

#[tokio::test]
async fn ingest_writes_events_before_claims_and_commits_payloads() {
    let deps = test_support::in_memory_deps();

    let result = execute_ingest(&deps, test_support::ingest_input())
        .await
        .unwrap();

    assert_eq!(result.event_id, "id-1");
    assert_eq!(
        deps.log(),
        vec![
            "append_event",
            "record_event_in_episode",
            "upsert_claim",
            "link_evidence"
        ]
    );

    let event = deps.event("id-1").expect("event should be committed");
    assert_eq!(event.event.summary(), "The user asked for stronger memory.");
    assert_eq!(event.event.kind(), EventKind::Conversation);

    let claim = deps
        .claim("id-1:claim:0")
        .expect("derived claim should be committed");
    assert_eq!(claim.claim.subject(), "self.role");
    assert_eq!(claim.claim.object(), "architect");
    assert_eq!(claim.claim.namespace().as_str(), "self");
    assert_eq!(claim.status, ClaimStatus::Active);

    assert_eq!(
        deps.evidence_links(),
        vec![("id-1:claim:0".to_string(), "id-1".to_string())]
    );
    assert_eq!(
        deps.episode_links(),
        vec![("episode:task-4".to_string(), "id-1".to_string())]
    );
}

#[tokio::test]
async fn ingest_preserves_explicit_project_namespace_on_claims() {
    let deps = test_support::in_memory_deps();

    let result = execute_ingest(&deps, test_support::project_ingest_input())
        .await
        .unwrap();

    let claim = deps
        .claim(&format!("{}:claim:0", result.event_id))
        .expect("derived project claim should be committed");
    assert_eq!(claim.claim.namespace().as_str(), "project/agent-llm-mm");
}

#[tokio::test]
async fn reflection_supersedes_old_claim_and_persists_audit_record() {
    let deps = test_support::reflection_deps();

    let result = execute_reflection(&deps, test_support::reflection_input())
        .await
        .unwrap();

    assert_eq!(result.reflection_id, "id-1");
    assert_eq!(
        result.replacement_claim_id.as_deref(),
        Some("id-1:replacement")
    );

    let old_claim = deps.claim("claim-old").expect("old claim should remain");
    assert_eq!(old_claim.status, ClaimStatus::Superseded);

    let new_claim = deps
        .claim("id-1:replacement")
        .expect("replacement claim should be committed");
    assert_eq!(new_claim.claim.object(), "senior_architect");
    assert_eq!(new_claim.status, ClaimStatus::Active);
    assert_eq!(
        deps.evidence_links(),
        vec![(
            "id-1:replacement".to_string(),
            "evt-reflection-1".to_string()
        )]
    );

    let reflection = deps
        .reflection("id-1")
        .expect("reflection audit record should be committed");
    assert_eq!(reflection.superseded_claim_id.as_deref(), Some("claim-old"));
    assert_eq!(
        reflection.replacement_claim_id.as_deref(),
        Some("id-1:replacement")
    );
}

#[tokio::test]
async fn reflection_can_update_identity_and_commitments_with_audited_supporting_evidence() {
    let deps = test_support::reflection_query_deps();

    let result = execute_reflection(
        &deps,
        test_support::reflection_input_with_identity_and_commitment_updates(),
    )
    .await
    .unwrap();

    assert_eq!(result.reflection_id, "id-1");
    assert_eq!(
        result.replacement_claim_id.as_deref(),
        Some("id-1:replacement")
    );

    let old_claim = deps.claim("claim-old").expect("old claim should remain");
    assert_eq!(old_claim.status, ClaimStatus::Superseded);

    let replacement_claim = deps
        .claim("id-1:replacement")
        .expect("replacement claim should be committed");
    assert_eq!(replacement_claim.claim.object(), "staff_architect");
    assert_eq!(replacement_claim.status, ClaimStatus::Active);

    assert_eq!(
        deps.identity().canonical_claims(),
        &[
            "identity:self=staff_architect".to_string(),
            "identity:self=mentor".to_string(),
        ]
    );
    assert_eq!(
        deps.commitments(),
        vec![
            Commitment::new(Owner::Self_, "prefer:evidence_backed_identity_updates"),
            Commitment::new(Owner::Self_, "forbid:write_identity_core_directly"),
        ]
    );

    let reflection = deps
        .reflection("id-1")
        .expect("reflection audit record should be committed");
    assert_eq!(
        reflection.supporting_evidence_event_ids,
        vec![
            "evt-reflection-1".to_string(),
            "evt-reflection-3".to_string(),
        ]
    );
    assert_eq!(
        reflection
            .requested_identity_update
            .as_ref()
            .map(|update| update.canonical_claims.clone()),
        Some(vec![
            "identity:self=staff_architect".to_string(),
            "identity:self=mentor".to_string(),
        ])
    );
    assert_eq!(
        reflection.requested_commitment_updates.as_ref(),
        Some(&vec![
            Commitment::new(Owner::Self_, "prefer:evidence_backed_identity_updates"),
            Commitment::new(Owner::Self_, "forbid:write_identity_core_directly"),
        ])
    );
}

#[tokio::test]
async fn reflection_preserves_baseline_commitment_when_updates_replace_commitments() {
    let deps = test_support::reflection_query_deps();

    execute_reflection(
        &deps,
        test_support::reflection_input_without_baseline_commitment(),
    )
    .await
    .unwrap();

    assert_eq!(
        deps.commitments(),
        vec![
            Commitment::new(Owner::Self_, "prefer:evidence_backed_identity_updates"),
            Commitment::new(Owner::Self_, "forbid:write_identity_core_directly"),
        ]
    );
}

#[tokio::test]
async fn reflection_rejects_identity_update_without_supporting_evidence() {
    let deps = test_support::in_memory_deps();

    let result = execute_reflection(
        &deps,
        ReflectionInput::new(
            Reflection::new("Identity updates must stay evidence-backed."),
            "claim-old",
            None,
            Vec::new(),
        )
        .with_identity_update(vec!["identity:self=principal_architect".to_string()]),
    )
    .await;

    assert!(matches!(result, Err(AppError::InvalidParams(_))));
    assert_eq!(
        deps.identity().canonical_claims(),
        &["identity:self=architect".to_string()]
    );
    assert_eq!(
        deps.claim("claim-old")
            .expect("original claim should remain active")
            .status,
        ClaimStatus::Active
    );
    assert!(deps.reflection("id-1").is_none());
}

#[tokio::test]
async fn reflection_rejects_identity_update_when_evidence_query_resolves_empty() {
    let deps = test_support::reflection_query_deps();

    let result = execute_reflection(
        &deps,
        ReflectionInput::new(
            Reflection::new("Identity updates must stay backed by resolved query evidence."),
            "claim-old",
            None,
            Vec::new(),
        )
        .with_replacement_evidence_query(EvidenceQuery {
            namespace: None,
            owner: Some(Owner::User),
            kind: Some(EventKind::Conversation),
            limit: Some(2),
            recorded_after: None,
            recorded_before: None,
            event_id_prefix: None,
        })
        .with_identity_update(vec!["identity:self=principal_architect".to_string()]),
    )
    .await;

    assert!(matches!(result, Err(AppError::InvalidParams(_))));
    assert!(
        deps.evidence_query_trace()
            .iter()
            .any(|query| query.owner == Some(Owner::User)
                && query.kind == Some(EventKind::Conversation)
                && query.limit == Some(2)),
        "empty-query scenario should exercise evidence query resolution"
    );
    assert_eq!(
        deps.identity().canonical_claims(),
        &["identity:self=architect".to_string()]
    );
    assert_eq!(
        deps.claim("claim-old")
            .expect("original claim should remain active")
            .status,
        ClaimStatus::Active
    );
    assert!(deps.reflection("id-1").is_none());
}

#[tokio::test]
async fn fake_event_store_rejects_zero_limit_for_bounded_and_unbounded_evidence_queries() {
    let deps = test_support::reflection_query_deps();
    let query = EvidenceQuery {
        namespace: None,
        owner: Some(Owner::Self_),
        kind: Some(EventKind::Reflection),
        limit: Some(0),
        recorded_after: None,
        recorded_before: None,
        event_id_prefix: None,
    };

    let bounded = EventStore::query_evidence_event_ids(&deps, query.clone()).await;
    let unbounded = EventStore::query_evidence_event_ids_unbounded(&deps, query).await;

    assert!(matches!(
        bounded,
        Err(AppError::InvalidParams(message)) if message.contains("limit must be at least 1")
    ));
    assert!(matches!(
        unbounded,
        Err(AppError::InvalidParams(message)) if message.contains("limit must be at least 1")
    ));
}

#[tokio::test]
async fn auto_reflection_runs_once_for_repeated_failure_and_records_handled_ledger() {
    let deps = test_support::auto_reflection_deps();
    deps.seed_failure_window(vec![
        "rollback after violating a hard commitment",
        "second rollback after violating the same hard commitment",
    ]);

    let result = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_failure(
            Namespace::for_project("agent-llm-mm"),
            vec!["failure".to_string(), "rollback".to_string()],
        ),
    )
    .await
    .unwrap();
    let second = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_failure(
            Namespace::for_project("agent-llm-mm"),
            vec!["failure".to_string(), "rollback".to_string()],
        ),
    )
    .await
    .unwrap();

    assert!(result.triggered);
    assert_eq!(result.trigger_type, Some(TriggerType::Failure));
    assert!(result.reflection_id.is_some());
    assert_eq!(result.ledger_status, Some(TriggerLedgerStatus::Handled));
    assert_eq!(result.reason, None);
    assert_eq!(
        result.trigger_key.as_deref(),
        Some("project/agent-llm-mm:failure")
    );
    assert_eq!(
        result.evidence_event_ids,
        vec!["evt-failure-2".to_string(), "evt-failure-1".to_string()]
    );
    assert!(result.cooldown_until.is_some());
    assert_eq!(result.suppression_reason, None);
    assert!(!second.triggered);
    assert_eq!(second.trigger_type, Some(TriggerType::Failure));
    assert_eq!(second.ledger_status, Some(TriggerLedgerStatus::Suppressed));
    assert_eq!(deps.reflections().len(), 1);
    assert_eq!(deps.reflections()[0].superseded_claim_id, None);
    assert_eq!(deps.reflections()[0].replacement_claim_id, None);
    assert_eq!(
        deps.claim("claim-old")
            .expect("unrelated self claim should remain untouched")
            .status,
        ClaimStatus::Active
    );
    assert_eq!(
        deps.latest_trigger_status(),
        Some(TriggerLedgerStatus::Suppressed)
    );
}

#[tokio::test]
async fn reflection_can_record_identity_and_commitment_updates_without_claim_transition() {
    let deps = test_support::reflection_query_deps();

    let result = execute_reflection(
        &deps,
        ReflectionInput::record_only(
            Reflection::new("Governed patch-only updates should stay record-only."),
            vec!["evt-reflection-1".to_string()],
        )
        .with_identity_update(vec!["identity:self=staff_architect".to_string()])
        .with_commitment_updates(vec![Commitment::new(
            Owner::Self_,
            "prefer:evidence_backed_identity_updates",
        )]),
    )
    .await
    .unwrap();

    assert_eq!(result.replacement_claim_id, None);
    assert_eq!(
        deps.claim("claim-old")
            .expect("existing claim should remain active")
            .status,
        ClaimStatus::Active
    );
    assert_eq!(
        deps.identity().canonical_claims(),
        &["identity:self=staff_architect".to_string()]
    );
    assert_eq!(
        deps.commitments(),
        vec![
            Commitment::new(Owner::Self_, "prefer:evidence_backed_identity_updates"),
            Commitment::new(Owner::Self_, "forbid:write_identity_core_directly"),
        ]
    );

    let reflection = deps
        .reflection(&result.reflection_id)
        .expect("record-only reflection should be audited");
    assert_eq!(reflection.superseded_claim_id, None);
    assert_eq!(reflection.replacement_claim_id, None);
}

#[tokio::test]
async fn reflection_without_replacement_claim_disputes_old_claim_and_updates_identity() {
    let deps = test_support::reflection_query_deps();

    let result = execute_reflection(
        &deps,
        test_support::reflection_input_with_identity_update_only(),
    )
    .await
    .unwrap();

    assert_eq!(result.replacement_claim_id, None);
    assert_eq!(
        deps.claim("claim-old")
            .expect("original claim should remain")
            .status,
        ClaimStatus::Disputed
    );
    assert_eq!(
        deps.identity().canonical_claims(),
        &["identity:self=staff_architect".to_string()]
    );
}

#[tokio::test]
async fn run_reflection_rejects_blank_canonical_identity_claim() {
    // 收紧校验：identity 更新含纯空白 canonical claim 时应被拒绝，
    // 不得绕过非空检查写入 durable identity。
    let deps = test_support::reflection_query_deps();

    let result = execute_reflection(
        &deps,
        ReflectionInput::record_only(
            Reflection::new("Blank canonical identity claims must be rejected."),
            vec!["evt-reflection-1".to_string()],
        )
        .with_identity_update(vec![" ".to_string()]),
    )
    .await;

    assert!(matches!(result, Err(AppError::InvalidParams(_))));
    assert_eq!(
        deps.identity().canonical_claims(),
        &["identity:self=architect".to_string()]
    );
}

#[tokio::test]
async fn run_reflection_rejects_blank_commitment_description() {
    // 与 identity canonical claim 对称：commitment 更新含纯空白 description 时应被拒绝，
    // 不得绕过非空检查写入 durable commitments。
    let deps = test_support::reflection_query_deps();

    let result = execute_reflection(
        &deps,
        ReflectionInput::record_only(
            Reflection::new("Blank commitment descriptions must be rejected."),
            vec!["evt-reflection-1".to_string()],
        )
        .with_commitment_updates(vec![Commitment::new(Owner::Self_, " ")]),
    )
    .await;

    assert!(matches!(
        result,
        Err(AppError::InvalidParams(message))
            if message.contains("blank descriptions")
    ));
    // 与 identity blank 测试对称：拒绝后 durable commitments 必须维持 baseline 原状，
    // 不得有纯空白 " " 项被写入。
    assert_eq!(
        deps.commitments(),
        vec![Commitment::new(
            Owner::Self_,
            "forbid:write_identity_core_directly"
        )]
    );
}

#[tokio::test]
async fn reflection_rejects_inferred_replacement_without_external_evidence() {
    let deps = test_support::in_memory_deps();

    let result = execute_reflection(&deps, test_support::inferred_reflection_input()).await;

    assert!(result.is_err());
    assert_eq!(
        deps.claim("claim-old")
            .expect("original claim should remain active")
            .status,
        ClaimStatus::Active
    );
    assert!(deps.claim("id-1:replacement").is_none());
    assert!(deps.reflection("id-1").is_none());
}

#[tokio::test]
async fn reflection_accepts_inferred_replacement_with_explicit_evidence() {
    let deps = test_support::reflection_deps();

    let result = execute_reflection(
        &deps,
        test_support::inferred_reflection_with_evidence_input(),
    )
    .await
    .unwrap();

    assert_eq!(
        result.replacement_claim_id.as_deref(),
        Some("id-1:replacement")
    );
    let replacement_claim = deps
        .claim("id-1:replacement")
        .expect("inferred replacement should be committed when evidence is provided");
    assert_eq!(replacement_claim.claim.object(), "principal_architect");
    assert_eq!(replacement_claim.status, ClaimStatus::Active);
    assert_eq!(
        deps.evidence_links(),
        vec![
            (
                "id-1:replacement".to_string(),
                "evt-reflection-1".to_string()
            ),
            (
                "id-1:replacement".to_string(),
                "evt-reflection-2".to_string()
            )
        ]
    );
}

#[tokio::test]
async fn mock_model_returns_default_no_revision_proposal() {
    let model = MockModel;
    let proposal = model
        .propose_self_revision(SelfRevisionRequest::new(
            TriggerType::Failure,
            Namespace::self_(),
            SelfSnapshot {
                identity: vec![],
                commitments: vec![],
                claims: vec![],
                evidence: vec![],
                episodes: vec![],
            },
            vec![],
            vec![],
        ))
        .await
        .unwrap();

    assert_eq!(
        proposal,
        SelfRevisionProposal::no_revision(
            "mock model did not detect a valid Failure revision".to_string()
        )
    );
    assert!(!proposal.should_reflect);
    assert!(proposal.machine_patch.identity_patch.is_none());
    assert!(proposal.machine_patch.commitment_patch.is_none());
}

#[tokio::test]
async fn reflection_supersedes_old_claim_with_query_resolved_evidence() {
    let deps = test_support::reflection_query_deps();

    let result = execute_reflection(&deps, test_support::reflection_input_with_query())
        .await
        .unwrap();

    assert_eq!(
        result.replacement_claim_id.as_deref(),
        Some("id-1:replacement")
    );

    let replacement_claim = deps
        .claim("id-1:replacement")
        .expect("query-resolved replacement claim should be committed");
    assert_eq!(replacement_claim.claim.object(), "principal_architect");
    assert_eq!(replacement_claim.status, ClaimStatus::Active);
    assert_eq!(
        deps.evidence_links(),
        vec![
            (
                "id-1:replacement".to_string(),
                "evt-reflection-3".to_string()
            ),
            (
                "id-1:replacement".to_string(),
                "evt-reflection-1".to_string()
            ),
        ]
    );
}

#[tokio::test]
async fn reflection_replaces_with_query_and_explicit_evidence_ids_without_duplication() {
    let deps = test_support::reflection_query_deps();

    let result = execute_reflection(
        &deps,
        test_support::reflection_input_with_query_and_explicit_overlap(),
    )
    .await
    .unwrap();

    assert_eq!(
        result.replacement_claim_id.as_deref(),
        Some("id-1:replacement")
    );
    assert_eq!(
        deps.evidence_links(),
        vec![
            (
                "id-1:replacement".to_string(),
                "evt-reflection-1".to_string()
            ),
            (
                "id-1:replacement".to_string(),
                "evt-reflection-3".to_string()
            ),
        ]
    );
    assert_eq!(
        deps.reflection("id-1")
            .expect("reflection audit should be committed")
            .supporting_evidence_event_ids,
        vec![
            "evt-reflection-1".to_string(),
            "evt-reflection-3".to_string(),
        ],
        "the compatibility `*_event_ids` audit field must retain ordered raw ids"
    );
}

#[tokio::test]
async fn reflection_rejects_invalid_explicit_event_reference_forms() {
    for invalid_event_id in ["", " ", "event:", "event:event:evt-reflection-1"] {
        let deps = test_support::reflection_query_deps();
        let result = execute_reflection(
            &deps,
            ReflectionInput::new(
                Reflection::new("Invalid event references must fail before reflection writes."),
                "claim-old",
                Some(ClaimDraft::new(
                    Owner::Self_,
                    "self.role",
                    "is",
                    "principal_architect",
                    Mode::Observed,
                )),
                vec![invalid_event_id.to_string()],
            ),
        )
        .await;

        assert!(
            matches!(result, Err(AppError::InvalidParams(_))),
            "invalid event reference {invalid_event_id:?} should be rejected"
        );
        assert!(deps.reflection("id-1").is_none());
    }
}

#[tokio::test]
async fn reflection_rejects_replacement_claim_when_query_returns_no_events() {
    let deps = test_support::reflection_query_deps();

    let result = execute_reflection(
        &deps,
        ReflectionInput::new(
            Reflection::new(
                "No matching events should be a hard error for query-based replacement.",
            ),
            "claim-old",
            Some(ClaimDraft::new(
                Owner::Self_,
                "self.role",
                "is",
                "principal_architect",
                Mode::Inferred,
            )),
            Vec::new(),
        )
        .with_replacement_evidence_query(agent_llm_mm::ports::event_store::EvidenceQuery {
            namespace: None,
            owner: Some(Owner::Self_),
            kind: Some(EventKind::Conversation),
            limit: Some(2),
            recorded_after: None,
            recorded_before: None,
            event_id_prefix: None,
        }),
    )
    .await;

    assert!(matches!(result, Err(AppError::InvalidParams(_))));
    assert!(deps.claim("id-1:replacement").is_none());
}

#[tokio::test]
async fn reflection_rejects_missing_replacement_evidence_event_ids() {
    let deps = test_support::in_memory_deps();

    let result = execute_reflection(
        &deps,
        test_support::inferred_reflection_with_missing_evidence_input(),
    )
    .await;

    assert!(matches!(result, Err(AppError::InvalidParams(_))));
    assert!(deps.claim("id-1:replacement").is_none());
    assert!(deps.reflection("id-1").is_none());
}

#[tokio::test]
async fn conflicting_reflection_marks_existing_claim_as_disputed() {
    let deps = test_support::in_memory_deps();

    let result = execute_reflection(
        &deps,
        ReflectionInput::new(
            Reflection::new("Conflicting evidence should dispute the old claim."),
            "claim-old",
            None,
            Vec::new(),
        ),
    )
    .await
    .unwrap();

    assert_eq!(result.reflection_id, "id-1");
    assert_eq!(result.replacement_claim_id, None);

    let old_claim = deps.claim("claim-old").expect("old claim should remain");
    assert_eq!(old_claim.status, ClaimStatus::Disputed);

    let reflection = deps
        .reflection("id-1")
        .expect("reflection audit record should be committed");
    assert_eq!(reflection.superseded_claim_id.as_deref(), Some("claim-old"));
    assert_eq!(reflection.replacement_claim_id, None);
}

#[tokio::test]
async fn build_self_snapshot_returns_store_backed_snapshot_and_respects_budget() {
    let deps = test_support::snapshot_deps();

    let result = execute_build_snapshot(
        &deps,
        BuildSelfSnapshotInput {
            scope: MemoryScope::self_(),
            evidence_manifest: None,
            time_window: SnapshotTimeWindow::unbounded(),
            budget: SnapshotBudget::new(2),
        },
    )
    .await
    .unwrap();

    assert_eq!(
        result.snapshot.identity,
        vec!["identity:self=architect".to_string()]
    );
    assert_eq!(
        result.snapshot.commitments,
        vec!["forbid:write_identity_core_directly".to_string()]
    );
    assert_eq!(
        result.snapshot.claims,
        vec!["self:self.role is architect".to_string()]
    );
    assert_eq!(
        result.snapshot.evidence,
        vec!["event:evt-3".to_string(), "event:evt-2".to_string()]
    );
    assert_eq!(
        result.snapshot.episodes,
        vec!["episode:memory".to_string(), "episode:task-4".to_string()]
    );
}

#[tokio::test]
async fn build_self_snapshot_rejects_manifest_on_legacy_unscoped_application_input() {
    let deps = test_support::snapshot_deps();

    let error = execute_build_snapshot(
        &deps,
        BuildSelfSnapshotInput {
            scope: MemoryScope::legacy_unscoped(),
            evidence_manifest: Some(vec![EventReference::parse("evt-1").unwrap()]),
            time_window: SnapshotTimeWindow::unbounded(),
            budget: SnapshotBudget::new(2),
        },
    )
    .await
    .expect_err("application callers must not bypass the scoped manifest invariant");

    assert!(
        error
            .to_string()
            .contains("evidence_manifest requires an explicit namespace")
    );
}

#[tokio::test]
async fn build_self_snapshot_rejects_oversized_programmatic_manifest() {
    let deps = test_support::snapshot_deps();
    let reference = EventReference::parse("evt-1").unwrap();

    let error = execute_build_snapshot(
        &deps,
        BuildSelfSnapshotInput {
            scope: MemoryScope::self_(),
            evidence_manifest: Some(vec![reference; MAX_EVIDENCE_MANIFEST_ITEMS + 1]),
            time_window: SnapshotTimeWindow::unbounded(),
            budget: SnapshotBudget::new(2),
        },
    )
    .await
    .expect_err("programmatic callers must not bypass the manifest size bound");

    assert!(
        error
            .to_string()
            .contains("evidence_manifest must contain at most 256 entries")
    );
}

#[tokio::test]
async fn build_self_snapshot_rejects_invalid_or_unscoped_programmatic_time_window() {
    let deps = test_support::snapshot_deps();
    let now = test_support::fixed_now();

    let reversed = execute_build_snapshot(
        &deps,
        BuildSelfSnapshotInput {
            scope: MemoryScope::self_(),
            evidence_manifest: None,
            time_window: SnapshotTimeWindow {
                recorded_after: Some(now + chrono::Duration::seconds(1)),
                recorded_before: Some(now),
            },
            budget: SnapshotBudget::new(2),
        },
    )
    .await
    .expect_err("application callers must not bypass time-window ordering validation");
    assert!(reversed.to_string().contains("less than or equal"));

    let unscoped = execute_build_snapshot(
        &deps,
        BuildSelfSnapshotInput {
            scope: MemoryScope::legacy_unscoped(),
            evidence_manifest: None,
            time_window: SnapshotTimeWindow::new(Some(now), None).unwrap(),
            budget: SnapshotBudget::new(2),
        },
    )
    .await
    .expect_err("time-bounded snapshots must fail closed without an explicit namespace");
    assert!(unscoped.to_string().contains("explicit namespace"));
}

#[test]
fn revision_window_snapshot_input_remains_time_unbounded() {
    assert!(
        BuildSelfSnapshotInput::for_revision_window(4)
            .time_window
            .is_unbounded()
    );
}

#[tokio::test]
async fn build_self_snapshot_keeps_empty_time_window_intersection_empty() {
    let deps = test_support::snapshot_deps();
    let result = execute_build_snapshot(
        &deps,
        BuildSelfSnapshotInput {
            scope: MemoryScope::self_(),
            evidence_manifest: None,
            time_window: SnapshotTimeWindow::new(
                Some(test_support::fixed_now() + chrono::Duration::seconds(1)),
                None,
            )
            .unwrap(),
            budget: SnapshotBudget::new(2),
        },
    )
    .await
    .expect("an explicit narrowing window may produce an empty snapshot intersection");

    assert!(result.snapshot.evidence.is_empty());
    assert!(result.snapshot.episodes.is_empty());
}

struct LegacyEpisodeStore;

#[async_trait]
impl EpisodeStore for LegacyEpisodeStore {
    async fn record_event_in_episode(
        &self,
        _episode_reference: String,
        _event_id: String,
    ) -> Result<(), AppError> {
        Ok(())
    }

    async fn list_episode_references(&self) -> Result<Vec<String>, AppError> {
        Ok(vec!["episode:legacy".to_string()])
    }
}

#[tokio::test]
async fn episode_store_default_preserves_legacy_calls_and_fails_closed_for_scoped_calls() {
    let store = LegacyEpisodeStore;

    assert_eq!(
        store
            .list_episode_references_in_scope(&MemoryScope::legacy_unscoped())
            .await
            .unwrap(),
        vec!["episode:legacy"]
    );
    let error = store
        .list_episode_references_in_scope(&MemoryScope::self_())
        .await
        .expect_err("legacy stores must not return unscoped episodes for a scoped request");
    assert!(
        error
            .to_string()
            .contains("scoped episode lookup is not supported by this store")
    );

    let error = store
        .list_episode_references_for_snapshot(
            &MemoryScope::self_(),
            &SnapshotTimeWindow::new(Some(test_support::fixed_now()), None).unwrap(),
        )
        .await
        .expect_err("legacy stores must not silently ignore snapshot time bounds");
    assert!(error.to_string().contains("time-bounded episode lookup"));

    let error = store
        .list_episode_references_supporting_claims(
            &MemoryScope::legacy_unscoped(),
            &["claim-legacy".to_string()],
        )
        .await
        .expect_err("unscoped identity support lookup must fail closed");
    assert!(error.to_string().contains("requires an explicit namespace"));
    let error = store
        .list_episode_references_supporting_claims(
            &MemoryScope::self_(),
            &["claim-legacy".to_string()],
        )
        .await
        .expect_err("legacy stores must not invent claim-to-episode provenance");
    assert!(
        error
            .to_string()
            .contains("claim-to-evidence-to-episode lookup is not supported")
    );
}

#[tokio::test]
async fn decide_with_snapshot_does_not_call_model_when_gate_blocks() {
    let deps = test_support::in_memory_deps();

    let result = execute_decide(
        &deps,
        DecideWithSnapshotInput {
            task: "decide on identity write".to_string(),
            action: "write_identity_core_directly".to_string(),
            snapshot: test_support::decision_snapshot(),
        },
    )
    .await
    .unwrap();

    assert!(result.blocked);
    assert!(result.decision.is_none());
    assert_eq!(deps.model_call_count(), 0);
}

#[tokio::test]
async fn decide_with_snapshot_calls_model_when_gate_allows() {
    let deps = test_support::in_memory_deps();

    let result = execute_decide(
        &deps,
        DecideWithSnapshotInput {
            task: "read identity safely".to_string(),
            action: "read_identity_core".to_string(),
            snapshot: test_support::decision_snapshot(),
        },
    )
    .await
    .unwrap();

    assert!(!result.blocked);
    assert_eq!(
        result.decision,
        Some(ModelDecision::new("Proceed".to_string()))
    );
    assert_eq!(deps.model_call_count(), 1);

    let model_input = deps.last_model_input().expect("model should be called");
    assert_eq!(model_input.task, "read identity safely");
    assert_eq!(model_input.action, "read_identity_core");
    assert_eq!(
        model_input.snapshot.identity,
        vec!["identity:self=architect".to_string()]
    );
}

#[tokio::test]
async fn ingest_failure_before_commit_does_not_leak_partial_writes() {
    let deps = test_support::deps_with_fail_point(FailPoint::LinkEvidence);

    let result = execute_ingest(&deps, test_support::ingest_input()).await;

    assert!(result.is_err());
    assert!(
        deps.events().is_empty(),
        "event must not be visible before commit"
    );
    assert!(
        deps.claim("id-1:claim:0").is_none(),
        "claim must not be visible before commit"
    );
    assert!(
        deps.evidence_links().is_empty(),
        "link must not leak before commit"
    );
    assert!(
        deps.episode_links().is_empty(),
        "episode link must not leak before commit"
    );
    assert!(
        deps.log().is_empty(),
        "staged operations must not leak into the log"
    );
}

mod test_support {
    use super::*;

    pub fn in_memory_deps() -> InMemoryDeps {
        InMemoryDeps::new(State::default())
    }

    pub fn snapshot_deps() -> InMemoryDeps {
        let mut state = State::default();
        state.committed.events = vec![
            StoredEvent::new(
                "evt-1".to_string(),
                fixed_now(),
                Event::new(Owner::Self_, EventKind::Observation, "evt-1"),
            ),
            StoredEvent::new(
                "evt-2".to_string(),
                fixed_now(),
                Event::new(Owner::Self_, EventKind::Observation, "evt-2"),
            ),
            StoredEvent::new(
                "evt-3".to_string(),
                fixed_now(),
                Event::new(Owner::Self_, EventKind::Observation, "evt-3"),
            ),
        ];
        state.committed.episodes = vec![
            ("episode:task-4".to_string(), "evt-1".to_string()),
            ("episode:memory".to_string(), "evt-2".to_string()),
        ];
        state.committed.claims = vec![
            StoredClaim::new(
                "claim-active".to_string(),
                ClaimDraft::new(Owner::Self_, "self.role", "is", "architect", Mode::Observed),
                ClaimStatus::Active,
            ),
            StoredClaim::new(
                "claim-superseded".to_string(),
                ClaimDraft::new(
                    Owner::Self_,
                    "self.role",
                    "is",
                    "old_architect",
                    Mode::Observed,
                ),
                ClaimStatus::Superseded,
            ),
        ];
        InMemoryDeps::new(state)
    }

    pub fn reflection_deps() -> InMemoryDeps {
        let mut state = State::default();
        state.committed.events = vec![
            StoredEvent::new(
                "evt-reflection-1".to_string(),
                chrono::DateTime::parse_from_rfc3339("2026-03-23T10:01:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
                Event::new(Owner::World, EventKind::Observation, "evt-reflection-1"),
            ),
            StoredEvent::new(
                "evt-reflection-2".to_string(),
                chrono::DateTime::parse_from_rfc3339("2026-03-23T10:02:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
                Event::new(Owner::World, EventKind::Conversation, "evt-reflection-2"),
            ),
            StoredEvent::new(
                "evt-reflection-3".to_string(),
                chrono::DateTime::parse_from_rfc3339("2026-03-23T10:03:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
                Event::new(Owner::World, EventKind::Observation, "evt-reflection-3"),
            ),
        ];
        InMemoryDeps::new(state)
    }

    pub fn reflection_query_deps() -> InMemoryDeps {
        reflection_deps()
    }

    pub fn auto_reflection_deps() -> InMemoryDeps {
        let mut state = State::default();
        state.committed.events = vec![
            StoredEvent::new(
                "evt-failure-1".to_string(),
                chrono::DateTime::parse_from_rfc3339("2026-03-23T10:01:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
                Event::new(
                    Owner::Self_,
                    EventKind::Action,
                    "rollback after violating a hard commitment",
                ),
            ),
            StoredEvent::new(
                "evt-failure-2".to_string(),
                chrono::DateTime::parse_from_rfc3339("2026-03-23T10:02:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
                Event::new(
                    Owner::Self_,
                    EventKind::Action,
                    "second rollback after violating the same hard commitment",
                ),
            ),
        ];
        state.committed.episodes = vec![
            (
                "episode:auto-reflect-1".to_string(),
                "evt-failure-1".to_string(),
            ),
            (
                "episode:auto-reflect-2".to_string(),
                "evt-failure-2".to_string(),
            ),
        ];
        state.self_revision_proposal = SelfRevisionProposal {
            should_reflect: true,
            rationale: "repeated rollback should tighten future commitments".to_string(),
            machine_patch: agent_llm_mm::domain::self_revision::SelfRevisionPatch {
                identity_patch: None,
                commitment_patch: Some(
                    agent_llm_mm::domain::self_revision::SelfRevisionCommitmentPatch::new(vec![
                        "prefer:reflect_after_repeated_rollback".to_string(),
                    ]),
                ),
            },
            proposed_evidence_event_ids: Vec::new(),
            proposed_evidence_query: None,
            confidence: None,
        };
        InMemoryDeps::new(state)
    }

    pub fn deps_with_fail_point(fail_point: FailPoint) -> InMemoryDeps {
        InMemoryDeps::new(State {
            fail_point: Some(fail_point),
            ..State::default()
        })
    }

    pub fn ingest_input() -> IngestInput {
        IngestInput::new(
            Event::new(
                Owner::User,
                EventKind::Conversation,
                "The user asked for stronger memory.",
            ),
            vec![ClaimDraft::new(
                Owner::Self_,
                "self.role",
                "is",
                "architect",
                Mode::Observed,
            )],
            Some("episode:task-4".to_string()),
        )
    }

    pub fn reflection_input() -> ReflectionInput {
        ReflectionInput::new(
            Reflection::new("The previous role claim needs to be superseded by a newer version."),
            "claim-old",
            Some(ClaimDraft::new(
                Owner::Self_,
                "self.role",
                "is",
                "senior_architect",
                Mode::Observed,
            )),
            vec!["evt-reflection-1".to_string()],
        )
    }

    pub fn reflection_input_with_query() -> ReflectionInput {
        ReflectionInput::new(
            Reflection::new("Two recent observations support this inferred replacement."),
            "claim-old",
            Some(ClaimDraft::new(
                Owner::Self_,
                "self.role",
                "is",
                "principal_architect",
                Mode::Inferred,
            )),
            Vec::new(),
        )
        .with_replacement_evidence_query(agent_llm_mm::ports::event_store::EvidenceQuery {
            namespace: None,
            owner: Some(Owner::World),
            kind: Some(EventKind::Observation),
            limit: Some(2),
            recorded_after: None,
            recorded_before: None,
            event_id_prefix: None,
        })
    }

    pub fn reflection_input_with_query_and_explicit_overlap() -> ReflectionInput {
        ReflectionInput::new(
            Reflection::new("Explicit evidence plus query should dedupe overlapping events."),
            "claim-old",
            Some(ClaimDraft::new(
                Owner::Self_,
                "self.role",
                "is",
                "principal_architect",
                Mode::Inferred,
            )),
            vec![
                "event:evt-reflection-1".to_string(),
                "evt-reflection-1".to_string(),
                "event:evt-reflection-1".to_string(),
            ],
        )
        .with_replacement_evidence_query(agent_llm_mm::ports::event_store::EvidenceQuery {
            namespace: None,
            owner: Some(Owner::World),
            kind: Some(EventKind::Observation),
            limit: Some(2),
            recorded_after: None,
            recorded_before: None,
            event_id_prefix: None,
        })
    }

    pub fn reflection_input_with_identity_and_commitment_updates() -> ReflectionInput {
        ReflectionInput::new(
            Reflection::new("Evidence-backed reflection should revise the claim, identity, and commitments together."),
            "claim-old",
            Some(ClaimDraft::new(
                Owner::Self_,
                "self.role",
                "is",
                "staff_architect",
                Mode::Observed,
            )),
            vec!["evt-reflection-1".to_string()],
        )
        .with_replacement_evidence_query(agent_llm_mm::ports::event_store::EvidenceQuery {
            namespace: None,
            owner: Some(Owner::World),
            kind: Some(EventKind::Observation),
            limit: Some(2),
            recorded_after: None,
            recorded_before: None,
            event_id_prefix: None,
        })
        .with_identity_update(vec![
            "identity:self=staff_architect".to_string(),
            "identity:self=mentor".to_string(),
        ])
        .with_commitment_updates(vec![
            Commitment::new(Owner::Self_, "prefer:evidence_backed_identity_updates"),
            Commitment::new(Owner::Self_, "forbid:write_identity_core_directly"),
        ])
    }

    pub fn reflection_input_without_baseline_commitment() -> ReflectionInput {
        ReflectionInput::new(
            Reflection::new(
                "Reflection should not be able to drop the baseline identity write guard.",
            ),
            "claim-old",
            Some(ClaimDraft::new(
                Owner::Self_,
                "self.role",
                "is",
                "staff_architect",
                Mode::Observed,
            )),
            vec!["evt-reflection-1".to_string()],
        )
        .with_replacement_evidence_query(agent_llm_mm::ports::event_store::EvidenceQuery {
            namespace: None,
            owner: Some(Owner::World),
            kind: Some(EventKind::Observation),
            limit: Some(2),
            recorded_after: None,
            recorded_before: None,
            event_id_prefix: None,
        })
        .with_commitment_updates(vec![Commitment::new(
            Owner::Self_,
            "prefer:evidence_backed_identity_updates",
        )])
    }

    pub fn reflection_input_with_identity_update_only() -> ReflectionInput {
        ReflectionInput::new(
            Reflection::new("Conflict-only reflection can still revise identity with evidence."),
            "claim-old",
            None,
            vec!["evt-reflection-1".to_string()],
        )
        .with_replacement_evidence_query(agent_llm_mm::ports::event_store::EvidenceQuery {
            namespace: None,
            owner: Some(Owner::World),
            kind: Some(EventKind::Observation),
            limit: Some(2),
            recorded_after: None,
            recorded_before: None,
            event_id_prefix: None,
        })
        .with_identity_update(vec!["identity:self=staff_architect".to_string()])
    }

    pub fn project_ingest_input() -> IngestInput {
        IngestInput::new(
            Event::new(
                Owner::User,
                EventKind::Conversation,
                "The project memory should stay scoped to the current project.",
            ),
            vec![ClaimDraft::new_with_namespace(
                Owner::World,
                Namespace::for_project("agent-llm-mm"),
                "project.memory",
                "needs",
                "structure",
                Mode::Observed,
            )],
            Some("episode:project-memory".to_string()),
        )
    }

    pub fn inferred_reflection_input() -> ReflectionInput {
        ReflectionInput::new(
            Reflection::new("An inferred replacement without evidence should be rejected."),
            "claim-old",
            Some(ClaimDraft::new(
                Owner::Self_,
                "self.role",
                "is",
                "principal_architect",
                Mode::Inferred,
            )),
            Vec::new(),
        )
    }

    pub fn inferred_reflection_with_evidence_input() -> ReflectionInput {
        ReflectionInput::new(
            Reflection::new("Evidence-backed inferred replacements should be accepted."),
            "claim-old",
            Some(ClaimDraft::new(
                Owner::Self_,
                "self.role",
                "is",
                "principal_architect",
                Mode::Inferred,
            )),
            vec![
                "evt-reflection-1".to_string(),
                "evt-reflection-2".to_string(),
            ],
        )
    }

    pub fn inferred_reflection_with_missing_evidence_input() -> ReflectionInput {
        ReflectionInput::new(
            Reflection::new("Unknown evidence events should be rejected before persistence."),
            "claim-old",
            Some(ClaimDraft::new(
                Owner::Self_,
                "self.role",
                "is",
                "principal_architect",
                Mode::Inferred,
            )),
            vec!["evt-missing".to_string()],
        )
    }

    pub fn decision_snapshot() -> SelfSnapshot {
        SelfSnapshot {
            identity: vec!["identity:self=architect".to_string()],
            commitments: vec!["forbid:write_identity_core_directly".to_string()],
            claims: vec!["self.role is architect".to_string()],
            evidence: vec!["event:evt-1".to_string()],
            episodes: vec!["episode:task-4".to_string()],
        }
    }

    pub fn fixed_now() -> DateTime<Utc> {
        chrono::DateTime::parse_from_rfc3339("2026-03-23T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailPoint {
    LinkEvidence,
}

#[derive(Clone)]
struct InMemoryDeps {
    state: Arc<Mutex<State>>,
}

#[derive(Clone)]
struct State {
    committed: CommittedState,
    next_id: usize,
    now: DateTime<Utc>,
    fail_point: Option<FailPoint>,
    model_calls: Vec<ModelInput>,
    self_revision_proposal: SelfRevisionProposal,
    evidence_query_trace: Vec<EvidenceQuery>,
}

#[derive(Debug, Clone, Default)]
struct CommittedState {
    receipts: Vec<(String, agent_llm_mm::ports::StoredWriteReceipt)>,
    log: Vec<String>,
    events: Vec<StoredEvent>,
    claims: Vec<StoredClaim>,
    evidence_links: Vec<(String, String)>,
    episodes: Vec<(String, String)>,
    reflections: Vec<StoredReflection>,
    trigger_ledger: Vec<StoredTriggerLedgerEntry>,
    commitments: Vec<Commitment>,
    identity: Option<IdentityCore>,
}

#[derive(Debug, Default)]
struct PendingIngest {
    receipts: Vec<(String, agent_llm_mm::ports::StoredWriteReceipt)>,
    log: Vec<String>,
    events: Vec<StoredEvent>,
    claims: Vec<StoredClaim>,
    evidence_links: Vec<(String, String)>,
    episodes: Vec<(String, String)>,
}

#[derive(Debug, Default)]
struct PendingReflection {
    receipts: Vec<(String, agent_llm_mm::ports::StoredWriteReceipt)>,
    claims: Vec<StoredClaim>,
    evidence_links: Vec<(String, String)>,
    reflections: Vec<StoredReflection>,
    trigger_ledger: Vec<StoredTriggerLedgerEntry>,
    status_updates: Vec<(String, ClaimStatus)>,
    identity: Option<IdentityCore>,
    commitments: Option<Vec<Commitment>>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            committed: CommittedState {
                commitments: vec![Commitment::new(
                    Owner::Self_,
                    "forbid:write_identity_core_directly",
                )],
                identity: Some(IdentityCore::new(vec![
                    "identity:self=architect".to_string(),
                ])),
                claims: vec![StoredClaim::new(
                    "claim-old".to_string(),
                    ClaimDraft::new(Owner::Self_, "self.role", "is", "architect", Mode::Observed),
                    ClaimStatus::Active,
                )],
                ..CommittedState::default()
            },
            next_id: 1,
            now: chrono::DateTime::parse_from_rfc3339("2026-03-23T10:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            fail_point: None,
            model_calls: Vec::new(),
            self_revision_proposal: SelfRevisionProposal::no_revision(
                "mock model did not detect a valid Failure revision".to_string(),
            ),
            evidence_query_trace: Vec::new(),
        }
    }
}

impl InMemoryDeps {
    fn new(state: State) -> Self {
        Self {
            state: Arc::new(Mutex::new(state)),
        }
    }

    fn log(&self) -> Vec<String> {
        self.state.lock().unwrap().committed.log.clone()
    }

    fn events(&self) -> Vec<StoredEvent> {
        self.state.lock().unwrap().committed.events.clone()
    }

    fn event(&self, event_id: &str) -> Option<StoredEvent> {
        self.events()
            .into_iter()
            .find(|event| event.event_id == event_id)
    }

    fn claim(&self, claim_id: &str) -> Option<StoredClaim> {
        self.state
            .lock()
            .unwrap()
            .committed
            .claims
            .iter()
            .find(|claim| claim.claim_id == claim_id)
            .cloned()
    }

    fn evidence_links(&self) -> Vec<(String, String)> {
        self.state.lock().unwrap().committed.evidence_links.clone()
    }

    fn episode_links(&self) -> Vec<(String, String)> {
        self.state.lock().unwrap().committed.episodes.clone()
    }

    fn reflection(&self, reflection_id: &str) -> Option<StoredReflection> {
        self.state
            .lock()
            .unwrap()
            .committed
            .reflections
            .iter()
            .find(|reflection| reflection.reflection_id == reflection_id)
            .cloned()
    }

    fn reflections(&self) -> Vec<StoredReflection> {
        self.state.lock().unwrap().committed.reflections.clone()
    }

    fn identity(&self) -> IdentityCore {
        self.state
            .lock()
            .unwrap()
            .committed
            .identity
            .clone()
            .expect("identity should be present")
    }

    fn commitments(&self) -> Vec<Commitment> {
        self.state.lock().unwrap().committed.commitments.clone()
    }

    fn model_call_count(&self) -> usize {
        self.state.lock().unwrap().model_calls.len()
    }

    fn last_model_input(&self) -> Option<ModelInput> {
        self.state.lock().unwrap().model_calls.last().cloned()
    }

    fn evidence_query_trace(&self) -> Vec<EvidenceQuery> {
        self.state.lock().unwrap().evidence_query_trace.clone()
    }

    fn seed_failure_window(&self, summaries: Vec<&str>) {
        let mut state = self.state.lock().unwrap();
        state.committed.events = summaries
            .into_iter()
            .enumerate()
            .map(|(index, summary)| {
                StoredEvent::new(
                    format!("evt-failure-{}", index + 1),
                    chrono::DateTime::parse_from_rfc3339(&format!(
                        "2026-03-23T10:0{}:00Z",
                        index + 1
                    ))
                    .unwrap()
                    .with_timezone(&Utc),
                    Event::new_with_namespace(
                        Owner::World,
                        Namespace::for_project("agent-llm-mm"),
                        EventKind::Action,
                        summary,
                    )
                    .unwrap(),
                )
            })
            .collect();
    }

    fn latest_trigger_status(&self) -> Option<TriggerLedgerStatus> {
        self.state
            .lock()
            .unwrap()
            .committed
            .trigger_ledger
            .last()
            .map(|entry| entry.status)
    }
}

fn upsert_claim(target: &mut Vec<StoredClaim>, next_claim: StoredClaim) {
    if let Some(existing) = target
        .iter_mut()
        .find(|claim| claim.claim_id == next_claim.claim_id)
    {
        *existing = next_claim;
    } else {
        target.push(next_claim);
    }
}

#[async_trait]
impl EventStore for InMemoryDeps {
    async fn append_event(&self, event: StoredEvent) -> Result<(), AppError> {
        let mut state = self.state.lock().unwrap();
        state.committed.log.push("append_event".to_string());
        state.committed.events.push(event);
        Ok(())
    }

    async fn list_event_references(&self) -> Result<Vec<String>, AppError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .committed
            .events
            .iter()
            .map(StoredEvent::event_reference)
            .collect())
    }

    async fn list_recorded_at_for_snapshot_manifest(
        &self,
        scope: &MemoryScope,
        evidence_manifest: &[EventReference],
    ) -> Result<Vec<DateTime<Utc>>, AppError> {
        let state = self.state.lock().unwrap();
        Ok(state
            .committed
            .events
            .iter()
            .filter(|event| {
                scope
                    .owner()
                    .is_none_or(|owner| event.event.owner() == owner)
                    && scope
                        .namespace()
                        .is_none_or(|namespace| event.event.namespace() == namespace)
                    && evidence_manifest
                        .iter()
                        .any(|reference| reference.event_id() == event.event_id)
            })
            .map(|event| event.recorded_at)
            .collect())
    }

    async fn has_event(&self, event_id: &str) -> Result<bool, AppError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .committed
            .events
            .iter()
            .any(|event| event.event_id == event_id))
    }

    async fn query_evidence_event_ids(
        &self,
        query: EvidenceQuery,
    ) -> Result<Vec<String>, AppError> {
        self.state
            .lock()
            .unwrap()
            .evidence_query_trace
            .push(query.clone());
        if query.limit == Some(0) {
            return Err(AppError::InvalidParams(
                "limit must be at least 1".to_string(),
            ));
        }

        let mut events = self.state.lock().unwrap().committed.events.clone();
        filter_and_order_events(
            &mut events,
            query.namespace,
            query.owner,
            query.kind,
            query.recorded_after,
            query.recorded_before,
        );

        let limit = query.limit.unwrap_or(10);
        Ok(events
            .into_iter()
            .take(limit)
            .map(|event| event.event_id)
            .collect())
    }

    async fn query_evidence_event_ids_unbounded(
        &self,
        query: EvidenceQuery,
    ) -> Result<Vec<String>, AppError> {
        if query.limit == Some(0) {
            return Err(AppError::InvalidParams(
                "limit must be at least 1".to_string(),
            ));
        }

        let mut events = self.state.lock().unwrap().committed.events.clone();
        filter_and_order_events(
            &mut events,
            query.namespace,
            query.owner,
            query.kind,
            query.recorded_after,
            query.recorded_before,
        );

        let events = if let Some(limit) = query.limit {
            events.into_iter().take(limit).collect()
        } else {
            events
        };

        Ok(events.into_iter().map(|event| event.event_id).collect())
    }
}

fn filter_and_order_events(
    events: &mut Vec<StoredEvent>,
    namespace: Option<Namespace>,
    owner: Option<Owner>,
    kind: Option<EventKind>,
    recorded_after: Option<DateTime<Utc>>,
    recorded_before: Option<DateTime<Utc>>,
) {
    if let Some(namespace) = namespace {
        events.retain(|event| event.event.namespace() == &namespace);
    }

    if let Some(owner) = owner {
        events.retain(|event| event.event.owner() == owner);
    }

    if let Some(kind) = kind {
        events.retain(|event| event.event.kind() == kind);
    }

    if let Some(recorded_after) = recorded_after {
        events.retain(|event| event.recorded_at >= recorded_after);
    }

    if let Some(recorded_before) = recorded_before {
        events.retain(|event| event.recorded_at <= recorded_before);
    }

    events.sort_by(|lhs, rhs| {
        rhs.recorded_at
            .cmp(&lhs.recorded_at)
            .then_with(|| rhs.event_id.cmp(&lhs.event_id))
    });
}

#[async_trait]
impl ClaimStore for InMemoryDeps {
    async fn upsert_claim(&self, claim: StoredClaim) -> Result<(), AppError> {
        let mut state = self.state.lock().unwrap();
        state.committed.log.push("upsert_claim".to_string());
        upsert_claim(&mut state.committed.claims, claim);
        Ok(())
    }

    async fn link_evidence(&self, claim_id: String, event_id: String) -> Result<(), AppError> {
        let mut state = self.state.lock().unwrap();
        state.committed.log.push("link_evidence".to_string());
        state.committed.evidence_links.push((claim_id, event_id));
        Ok(())
    }

    async fn list_active_claims(&self) -> Result<Vec<StoredClaim>, AppError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .committed
            .claims
            .iter()
            .filter(|claim| claim.status == ClaimStatus::Active)
            .cloned()
            .collect())
    }

    async fn update_claim_status(
        &self,
        claim_id: &str,
        status: ClaimStatus,
    ) -> Result<(), AppError> {
        let mut state = self.state.lock().unwrap();
        if let Some(claim) = state
            .committed
            .claims
            .iter_mut()
            .find(|claim| claim.claim_id == claim_id)
        {
            claim.status = status;
        }
        Ok(())
    }
}

#[async_trait]
impl IdentityStore for InMemoryDeps {
    async fn load_identity(&self) -> Result<IdentityCore, AppError> {
        self.state
            .lock()
            .unwrap()
            .committed
            .identity
            .clone()
            .ok_or_else(|| AppError::Message("missing identity".to_string()))
    }

    async fn save_identity(&self, identity: IdentityCore) -> Result<(), AppError> {
        self.state.lock().unwrap().committed.identity = Some(identity);
        Ok(())
    }
}

#[async_trait]
impl CommitmentStore for InMemoryDeps {
    async fn list_commitments(&self) -> Result<Vec<Commitment>, AppError> {
        Ok(self.state.lock().unwrap().committed.commitments.clone())
    }
}

#[async_trait]
impl agent_llm_mm::ports::EpisodeStore for InMemoryDeps {
    async fn record_event_in_episode(
        &self,
        episode_reference: String,
        event_id: String,
    ) -> Result<(), AppError> {
        let mut state = self.state.lock().unwrap();
        state
            .committed
            .log
            .push("record_event_in_episode".to_string());
        state.committed.episodes.push((episode_reference, event_id));
        Ok(())
    }

    async fn list_episode_references(&self) -> Result<Vec<String>, AppError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .committed
            .episodes
            .iter()
            .map(|(episode_reference, _)| episode_reference.clone())
            .collect())
    }

    async fn list_episode_references_in_scope(
        &self,
        scope: &MemoryScope,
    ) -> Result<Vec<String>, AppError> {
        let state = self.state.lock().unwrap();
        let mut references = Vec::new();
        for (episode_reference, event_id) in &state.committed.episodes {
            let in_scope = state.committed.events.iter().any(|event| {
                &event.event_id == event_id
                    && scope
                        .owner()
                        .is_none_or(|owner| event.event.owner() == owner)
                    && scope
                        .namespace()
                        .is_none_or(|namespace| event.event.namespace() == namespace)
            });
            if in_scope && !references.contains(episode_reference) {
                references.push(episode_reference.clone());
            }
        }
        Ok(references)
    }

    async fn list_episode_references_for_snapshot(
        &self,
        scope: &MemoryScope,
        time_window: &SnapshotTimeWindow,
    ) -> Result<Vec<String>, AppError> {
        time_window.validate().map_err(AppError::from)?;
        let state = self.state.lock().unwrap();
        let mut latest_by_episode = Vec::<(String, DateTime<Utc>, String)>::new();

        for (episode_reference, event_id) in &state.committed.episodes {
            let Some(event) = state
                .committed
                .events
                .iter()
                .find(|event| &event.event_id == event_id)
            else {
                continue;
            };
            let is_eligible = scope
                .owner()
                .is_none_or(|owner| event.event.owner() == owner)
                && scope
                    .namespace()
                    .is_none_or(|namespace| event.event.namespace() == namespace)
                && time_window
                    .recorded_after
                    .is_none_or(|after| event.recorded_at >= after)
                && time_window
                    .recorded_before
                    .is_none_or(|before| event.recorded_at <= before);
            if !is_eligible {
                continue;
            }

            let candidate = (
                episode_reference.clone(),
                event.recorded_at,
                event.event_id.clone(),
            );
            if let Some(current) = latest_by_episode
                .iter_mut()
                .find(|current| current.0 == *episode_reference)
            {
                if (candidate.1, candidate.2.as_str()) > (current.1, current.2.as_str()) {
                    *current = candidate;
                }
            } else {
                latest_by_episode.push(candidate);
            }
        }

        latest_by_episode.sort_by(|lhs, rhs| {
            rhs.1
                .cmp(&lhs.1)
                .then_with(|| rhs.2.cmp(&lhs.2))
                .then_with(|| lhs.0.cmp(&rhs.0))
        });
        Ok(latest_by_episode
            .into_iter()
            .map(|(episode_reference, _, _)| episode_reference)
            .collect())
    }
}

#[async_trait]
impl ReflectionStore for InMemoryDeps {
    async fn append_reflection(&self, reflection: StoredReflection) -> Result<(), AppError> {
        self.state
            .lock()
            .unwrap()
            .committed
            .reflections
            .push(reflection);
        Ok(())
    }
}

#[async_trait]
impl ModelPort for InMemoryDeps {
    async fn decide(&self, input: ModelInput) -> Result<ModelDecision, AppError> {
        self.state.lock().unwrap().model_calls.push(input);
        Ok(ModelDecision::new("Proceed".to_string()))
    }

    async fn propose_self_revision(
        &self,
        request: SelfRevisionRequest,
    ) -> Result<SelfRevisionProposal, AppError> {
        let state = self.state.lock().unwrap();
        if state.self_revision_proposal.should_reflect {
            return Ok(state.self_revision_proposal.clone());
        }

        Ok(SelfRevisionProposal::no_revision(format!(
            "mock model did not detect a valid {:?} revision",
            request.trigger_type
        )))
    }
}

#[async_trait]
impl TriggerLedgerStore for InMemoryDeps {
    async fn record_trigger_attempt(
        &self,
        entry: StoredTriggerLedgerEntry,
    ) -> Result<(), AppError> {
        self.state
            .lock()
            .unwrap()
            .committed
            .trigger_ledger
            .push(entry);
        Ok(())
    }

    async fn latest_trigger_entry(
        &self,
        trigger_key: &str,
    ) -> Result<Option<StoredTriggerLedgerEntry>, AppError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .committed
            .trigger_ledger
            .iter()
            .rev()
            .find(|entry| entry.trigger_key == trigger_key)
            .cloned())
    }

    async fn latest_handled_trigger_entry(
        &self,
        trigger_key: &str,
    ) -> Result<Option<StoredTriggerLedgerEntry>, AppError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .committed
            .trigger_ledger
            .iter()
            .rev()
            .find(|entry| {
                entry.trigger_key == trigger_key && entry.status == TriggerLedgerStatus::Handled
            })
            .cloned())
    }
}

#[async_trait]
impl Clock for InMemoryDeps {
    async fn now(&self) -> Result<DateTime<Utc>, AppError> {
        Ok(self.state.lock().unwrap().now)
    }
}

#[async_trait]
impl IdGenerator for InMemoryDeps {
    async fn next_id(&self) -> Result<String, AppError> {
        let mut state = self.state.lock().unwrap();
        let next_id = format!("id-{}", state.next_id);
        state.next_id += 1;
        Ok(next_id)
    }
}

#[async_trait]
impl IngestTransactionRunner for InMemoryDeps {
    async fn begin_ingest_transaction(
        &self,
    ) -> Result<Box<dyn IngestTransaction + Send + '_>, AppError> {
        Ok(Box::new(InMemoryIngestTransaction {
            deps: self.clone(),
            pending: PendingIngest::default(),
        }))
    }
}

#[async_trait]
impl ReflectionTransactionRunner for InMemoryDeps {
    async fn begin_reflection_transaction(
        &self,
    ) -> Result<Box<dyn ReflectionTransaction + Send + '_>, AppError> {
        Ok(Box::new(InMemoryReflectionTransaction {
            deps: self.clone(),
            pending: PendingReflection::default(),
        }))
    }
}

struct InMemoryIngestTransaction {
    deps: InMemoryDeps,
    pending: PendingIngest,
}

#[async_trait]
impl IngestTransaction for InMemoryIngestTransaction {
    async fn load_event_for_ingest(
        &mut self,
        event_id: &str,
    ) -> Result<Option<StoredEvent>, AppError> {
        Ok(self
            .deps
            .state
            .lock()
            .unwrap()
            .committed
            .events
            .iter()
            .find(|event| event.event_id == event_id)
            .cloned())
    }
    async fn load_write_receipt(
        &mut self,
        operation_id: &str,
    ) -> Result<Option<agent_llm_mm::ports::StoredWriteReceipt>, AppError> {
        Ok(self
            .deps
            .state
            .lock()
            .unwrap()
            .committed
            .receipts
            .iter()
            .find(|(id, _)| id == operation_id)
            .map(|(_, receipt)| receipt.clone()))
    }
    async fn append_write_receipt(
        &mut self,
        request: &agent_llm_mm::ports::WriteReceiptRequest,
        receipt: agent_llm_mm::ports::StoredWriteReceipt,
        _recorded_at: DateTime<Utc>,
    ) -> Result<(), AppError> {
        self.pending
            .receipts
            .push((request.operation_id.clone(), receipt));
        Ok(())
    }
    async fn append_event(&mut self, event: StoredEvent) -> Result<(), AppError> {
        self.pending.log.push("append_event".to_string());
        self.pending.events.push(event);
        Ok(())
    }

    async fn record_event_in_episode(
        &mut self,
        episode_reference: String,
        event_id: String,
    ) -> Result<(), AppError> {
        self.pending.log.push("record_event_in_episode".to_string());
        self.pending.episodes.push((episode_reference, event_id));
        Ok(())
    }

    async fn upsert_claim(&mut self, claim: StoredClaim) -> Result<(), AppError> {
        self.pending.log.push("upsert_claim".to_string());
        self.pending.claims.push(claim);
        Ok(())
    }

    async fn link_evidence(&mut self, claim_id: String, event_id: String) -> Result<(), AppError> {
        let should_fail = self.state_fail_point() == Some(FailPoint::LinkEvidence);
        if should_fail {
            return Err(AppError::Message("injected link failure".to_string()));
        }

        self.pending.log.push("link_evidence".to_string());
        self.pending.evidence_links.push((claim_id, event_id));
        Ok(())
    }

    async fn commit(self: Box<Self>) -> Result<(), AppError> {
        let mut state = self.deps.state.lock().unwrap();
        state.committed.receipts.extend(self.pending.receipts);
        state.committed.log.extend(self.pending.log);
        state.committed.events.extend(self.pending.events);
        for claim in self.pending.claims {
            upsert_claim(&mut state.committed.claims, claim);
        }
        state
            .committed
            .evidence_links
            .extend(self.pending.evidence_links);
        state.committed.episodes.extend(self.pending.episodes);
        Ok(())
    }
}

impl InMemoryIngestTransaction {
    fn state_fail_point(&self) -> Option<FailPoint> {
        self.deps.state.lock().unwrap().fail_point
    }
}

struct InMemoryReflectionTransaction {
    deps: InMemoryDeps,
    pending: PendingReflection,
}

#[async_trait]
impl ReflectionTransaction for InMemoryReflectionTransaction {
    async fn load_write_receipt(
        &mut self,
        operation_id: &str,
    ) -> Result<Option<agent_llm_mm::ports::StoredWriteReceipt>, AppError> {
        Ok(self
            .deps
            .state
            .lock()
            .unwrap()
            .committed
            .receipts
            .iter()
            .find(|(id, _)| id == operation_id)
            .map(|(_, receipt)| receipt.clone()))
    }
    async fn append_write_receipt(
        &mut self,
        request: &agent_llm_mm::ports::WriteReceiptRequest,
        receipt: agent_llm_mm::ports::StoredWriteReceipt,
        _recorded_at: DateTime<Utc>,
    ) -> Result<(), AppError> {
        self.pending
            .receipts
            .push((request.operation_id.clone(), receipt));
        Ok(())
    }
    async fn load_claim_for_reflection(
        &mut self,
        claim_id: &str,
    ) -> Result<Option<StoredClaim>, AppError> {
        Ok(self
            .deps
            .state
            .lock()
            .unwrap()
            .committed
            .claims
            .iter()
            .find(|claim| claim.claim_id == claim_id)
            .cloned())
    }
    async fn load_event_for_reflection(
        &mut self,
        event_id: &str,
    ) -> Result<Option<StoredEvent>, AppError> {
        Ok(self
            .deps
            .state
            .lock()
            .unwrap()
            .committed
            .events
            .iter()
            .find(|event| event.event_id == event_id)
            .cloned())
    }
    async fn compare_and_set_claim_status(
        &mut self,
        claim_id: &str,
        expected: ClaimStatus,
        status: ClaimStatus,
    ) -> Result<(), AppError> {
        let target = self.load_claim_for_reflection(claim_id).await?;
        if target.is_none_or(|target| {
            target.status != expected || target.status == ClaimStatus::Superseded
        }) {
            return Err(AppError::InvalidParams(
                "reflection target claim changed or no longer exists".to_string(),
            ));
        }
        self.update_claim_status(claim_id, status).await
    }
    async fn upsert_claim(&mut self, claim: StoredClaim) -> Result<(), AppError> {
        self.pending.claims.push(claim);
        Ok(())
    }

    async fn link_evidence(&mut self, claim_id: String, event_id: String) -> Result<(), AppError> {
        self.pending.evidence_links.push((claim_id, event_id));
        Ok(())
    }

    async fn append_reflection(&mut self, reflection: StoredReflection) -> Result<(), AppError> {
        self.pending.reflections.push(reflection);
        Ok(())
    }

    async fn append_trigger_ledger(
        &mut self,
        entry: StoredTriggerLedgerEntry,
    ) -> Result<(), AppError> {
        self.pending.trigger_ledger.push(entry);
        Ok(())
    }

    async fn load_identity(&mut self) -> Result<IdentityCore, AppError> {
        if let Some(identity) = &self.pending.identity {
            return Ok(identity.clone());
        }

        self.deps
            .state
            .lock()
            .unwrap()
            .committed
            .identity
            .clone()
            .ok_or_else(|| AppError::Message("missing identity".to_string()))
    }

    async fn replace_identity(&mut self, identity: IdentityCore) -> Result<(), AppError> {
        self.pending.identity = Some(identity);
        Ok(())
    }

    async fn load_commitments(&mut self) -> Result<Vec<Commitment>, AppError> {
        if let Some(commitments) = &self.pending.commitments {
            return Ok(commitments.clone());
        }

        Ok(self
            .deps
            .state
            .lock()
            .unwrap()
            .committed
            .commitments
            .clone())
    }

    async fn replace_commitments(&mut self, commitments: Vec<Commitment>) -> Result<(), AppError> {
        self.pending.commitments = Some(commitments);
        Ok(())
    }

    async fn update_claim_status(
        &mut self,
        claim_id: &str,
        status: ClaimStatus,
    ) -> Result<(), AppError> {
        self.pending
            .status_updates
            .push((claim_id.to_string(), status));
        Ok(())
    }

    async fn commit(self: Box<Self>) -> Result<(), AppError> {
        let mut state = self.deps.state.lock().unwrap();
        state.committed.receipts.extend(self.pending.receipts);
        for claim in self.pending.claims {
            upsert_claim(&mut state.committed.claims, claim);
        }
        state
            .committed
            .evidence_links
            .extend(self.pending.evidence_links);
        state.committed.reflections.extend(self.pending.reflections);
        state
            .committed
            .trigger_ledger
            .extend(self.pending.trigger_ledger);
        if let Some(identity) = self.pending.identity {
            state.committed.identity = Some(identity);
        }
        if let Some(commitments) = self.pending.commitments {
            state.committed.commitments = commitments;
        }
        for (claim_id, status) in self.pending.status_updates {
            if let Some(claim) = state
                .committed
                .claims
                .iter_mut()
                .find(|claim| claim.claim_id == claim_id)
            {
                claim.status = status;
            }
        }
        Ok(())
    }
}

#[tokio::test]
async fn direct_reflection_revalidates_target_and_rejects_terminal_or_missing_claims() {
    for status in [Some(ClaimStatus::Superseded), None] {
        let deps = test_support::reflection_deps();
        {
            let mut state = deps.state.lock().unwrap();
            if let Some(status) = status {
                state.committed.claims[0].status = status;
            } else {
                state.committed.claims.clear();
            }
        }
        let result = execute_reflection(&deps, test_support::reflection_input()).await;
        assert!(matches!(result, Err(AppError::InvalidParams(_))));
        assert!(deps.state.lock().unwrap().committed.reflections.is_empty());
        assert!(deps.claim("id-1:replacement").is_none());
    }
}

#[tokio::test]
async fn direct_reflection_rejects_replacement_scope_escape() {
    let deps = test_support::reflection_deps();
    let input = ReflectionInput::new(
        Reflection::new("Attempt to move a self claim into user memory"),
        "claim-old",
        Some(ClaimDraft::new(
            Owner::User,
            "user.role",
            "is",
            "architect",
            Mode::Observed,
        )),
        vec!["evt-reflection-1".to_string()],
    );
    let result = execute_reflection(&deps, input).await;
    assert!(matches!(result, Err(AppError::InvalidParams(_))));
    assert_eq!(deps.claim("claim-old").unwrap().status, ClaimStatus::Active);
    assert!(deps.state.lock().unwrap().committed.reflections.is_empty());
}

#[tokio::test]
async fn disputed_claim_can_be_corrected_only_once() {
    let deps = test_support::reflection_deps();
    {
        let mut state = deps.state.lock().unwrap();
        state.committed.claims[0].status = ClaimStatus::Disputed;
        state.committed.events = vec![StoredEvent::new(
            "evt-reflection-1".to_string(),
            state.now,
            Event::new(Owner::Self_, EventKind::Observation, "updated role"),
        )];
    }
    execute_reflection(&deps, test_support::reflection_input())
        .await
        .unwrap();
    let result = execute_reflection(&deps, test_support::reflection_input()).await;
    assert!(matches!(result, Err(AppError::InvalidParams(_))));
    assert_eq!(deps.state.lock().unwrap().committed.reflections.len(), 1);
    assert_eq!(
        deps.claim("claim-old").unwrap().status,
        ClaimStatus::Superseded
    );
}

#[tokio::test]
async fn scoped_self_correction_rejects_legacy_world_evidence() {
    let deps = test_support::reflection_deps();
    let result = execute_reflection(
        &deps,
        test_support::reflection_input().with_strict_evidence_scope(),
    )
    .await;
    assert!(matches!(result, Err(AppError::InvalidParams(_))));
    assert_eq!(deps.claim("claim-old").unwrap().status, ClaimStatus::Active);
    assert!(deps.state.lock().unwrap().committed.reflections.is_empty());
}
