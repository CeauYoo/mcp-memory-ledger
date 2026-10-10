use std::sync::{Arc, Mutex};

use agent_llm_mm::{
    application::{
        auto_reflect_if_needed::{self, AutoReflectInput},
        build_self_snapshot::{BuildSelfSnapshotInput, execute as execute_build_snapshot},
        ingest_interaction::{IngestInput, execute as execute_ingest},
        run_reflection::{ReflectionInput, execute as execute_reflection},
    },
    domain::{
        claim::ClaimDraft,
        commitment::Commitment,
        event::Event,
        identity_core::IdentityCore,
        reflection::Reflection,
        self_model_version::{SelfModelVersion, SelfModelVersionKind},
        self_revision::TriggerType,
        snapshot::{SnapshotBudget, SnapshotTimeWindow},
        types::{EventKind, MemoryScope, Mode, Namespace, Owner},
    },
    error::AppError,
    ports::{
        ClaimStatus, ClaimStore, Clock, CommitmentStore, EpisodeStore, EventStore, EvidenceQuery,
        IdGenerator, IdentityStore, IngestTransaction, IngestTransactionRunner, ModelDecision,
        ModelInput, ModelPort, ReflectionTransaction, ReflectionTransactionRunner, StoredClaim,
        StoredEvent, StoredReflection, StoredTriggerLedgerEntry, TriggerLedgerStatus,
        TriggerLedgerStore,
    },
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};

#[tokio::test]
async fn inferred_claim_without_evidence_is_rejected() {
    let deps = test_support::deps_for_failure_modes();

    let result = execute_ingest(&deps, test_support::inferred_without_evidence()).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn reflection_marks_conflict_as_disputed_instead_of_deleting_history() {
    let deps = test_support::deps_for_failure_modes();

    let result = execute_reflection(&deps, test_support::conflicting_reflection())
        .await
        .unwrap();

    let original_claim = deps
        .claim("claim-conflict")
        .expect("original conflicting claim should remain in history");
    assert_eq!(original_claim.status, ClaimStatus::Disputed);
    assert_ne!(original_claim.status, ClaimStatus::Superseded);
    assert_eq!(result.replacement_claim_id, None);
    assert!(
        deps.claim("id-1:replacement").is_none(),
        "conflict-only reflections must not create a replacement claim"
    );
    assert_eq!(
        deps.claim_count(),
        1,
        "marking a conflict should preserve the original claim instead of replacing it"
    );
}

#[tokio::test]
async fn reflection_rejects_identity_or_commitment_updates_without_resolved_evidence() {
    let deps = test_support::deps_for_failure_modes();

    let result =
        execute_reflection(&deps, test_support::reflection_updates_without_evidence()).await;

    assert!(matches!(result, Err(AppError::InvalidParams(_))));
    assert_eq!(
        deps.identity().canonical_claims(),
        &["identity:self=architect".to_string()]
    );
    assert_eq!(
        deps.commitments(),
        vec![Commitment::new(
            Owner::Self_,
            "forbid:write_identity_core_directly",
        )]
    );
    assert!(deps.reflection("id-1").is_none());
}

#[tokio::test]
async fn reflection_rejects_empty_identity_update_even_with_supporting_evidence() {
    let deps = test_support::deps_for_failure_modes();

    let result = execute_reflection(
        &deps,
        test_support::reflection_updates_with_empty_identity(),
    )
    .await;

    assert!(matches!(result, Err(AppError::InvalidParams(_))));
    assert_eq!(
        deps.identity().canonical_claims(),
        &["identity:self=architect".to_string()]
    );
    assert!(deps.reflection("id-1").is_none());
}

#[tokio::test]
async fn reflection_commit_failure_rolls_back_claim_identity_commitment_and_audit_updates() {
    let deps = test_support::deps_with_fail_point(FailPoint::CommitReflection);

    let result = execute_reflection(&deps, test_support::reflection_updates_with_evidence()).await;

    assert!(
        matches!(result, Err(AppError::Message(message)) if message == "injected reflection commit failure")
    );
    assert_eq!(
        deps.claim("claim-conflict")
            .expect("original claim should remain")
            .status,
        ClaimStatus::Active
    );
    assert!(deps.claim("id-1:replacement").is_none());
    assert_eq!(
        deps.state
            .lock()
            .unwrap()
            .committed
            .self_model_versions
            .len(),
        1
    );
    assert!(deps.state.lock().unwrap().committed.receipts.is_empty());
    assert_eq!(
        deps.identity().canonical_claims(),
        &["identity:self=architect".to_string()]
    );
    assert_eq!(
        deps.commitments(),
        vec![Commitment::new(
            Owner::Self_,
            "forbid:write_identity_core_directly",
        )]
    );
    assert!(deps.reflection("id-1").is_none());
    assert!(deps.evidence_links().is_empty());
}

#[tokio::test]
async fn auto_reflection_returns_structured_diagnostics_for_recursion_guard_skip() {
    let deps = test_support::deps_for_failure_modes();

    let result = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_periodic(
            Namespace::for_project("agent-llm-mm"),
            vec!["periodic".to_string()],
        )
        .with_recursion_guard(
            agent_llm_mm::application::auto_reflect_if_needed::RecursionGuard::SkipAutoReflection,
        ),
    )
    .await
    .unwrap();

    assert!(!result.triggered);
    assert_eq!(result.trigger_type, Some(TriggerType::Periodic));
    assert_eq!(result.namespace.as_deref(), Some("project/agent-llm-mm"));
    assert_eq!(result.reflection_id, None);
    assert_eq!(result.ledger_status, None);
    assert_eq!(result.reason.as_deref(), Some("recursion guard enabled"));
    assert_eq!(
        result.trigger_key.as_deref(),
        Some("project/agent-llm-mm:periodic")
    );
    assert!(result.evidence_event_ids.is_empty());
    assert_eq!(result.cooldown_until, None);
    assert_eq!(result.suppression_reason, None);
    assert_eq!(result.diagnostics.trigger_type, TriggerType::Periodic);
    assert_eq!(result.diagnostics.namespace, "project/agent-llm-mm");
    assert_eq!(
        result.diagnostics.trigger_key,
        "project/agent-llm-mm:periodic"
    );
    assert_eq!(
        result.diagnostics.outcome,
        agent_llm_mm::domain::self_revision::AutoReflectOutcome::Skipped
    );
    assert_eq!(result.diagnostics.cooldown_state, "none");
    assert_eq!(result.diagnostics.rejection_reason, None);
    assert_eq!(result.diagnostics.suppression_reason, None);
    assert_eq!(result.diagnostics.cooldown_boundary, None);
    assert_eq!(result.diagnostics.evidence_window_size, 0);
    assert!(result.diagnostics.selected_evidence_event_ids.is_empty());
    assert_eq!(result.diagnostics.durable_write_path, "run_reflection");
    let diagnostics_json = serde_json::to_value(&result.diagnostics).unwrap();
    assert_eq!(diagnostics_json["trigger_type"], "periodic");
    assert_eq!(diagnostics_json["namespace"], "project/agent-llm-mm");
    assert_eq!(
        diagnostics_json["trigger_key"],
        "project/agent-llm-mm:periodic"
    );
    assert_eq!(diagnostics_json["outcome"], "skipped");
    assert_eq!(diagnostics_json["cooldown_state"], "none");
}

#[tokio::test]
async fn auto_reflection_returns_structured_diagnostics_for_not_triggered_case() {
    let deps = test_support::deps_for_failure_modes();

    let result = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_failure(
            Namespace::for_project("agent-llm-mm"),
            vec!["failure".to_string()],
        ),
    )
    .await
    .unwrap();

    assert!(!result.triggered);
    assert_eq!(result.trigger_type, Some(TriggerType::Failure));
    assert_eq!(result.namespace.as_deref(), Some("project/agent-llm-mm"));
    assert_eq!(result.reflection_id, None);
    assert_eq!(result.ledger_status, None);
    assert_eq!(result.reason, None);
    assert_eq!(
        result.trigger_key.as_deref(),
        Some("project/agent-llm-mm:failure")
    );
    assert!(result.evidence_event_ids.is_empty());
    assert_eq!(result.cooldown_until, None);
    assert_eq!(result.suppression_reason, None);
    assert_eq!(result.diagnostics.trigger_type, TriggerType::Failure);
    assert_eq!(result.diagnostics.namespace, "project/agent-llm-mm");
    assert_eq!(
        result.diagnostics.trigger_key,
        "project/agent-llm-mm:failure"
    );
    assert_eq!(
        result.diagnostics.outcome,
        agent_llm_mm::domain::self_revision::AutoReflectOutcome::NotTriggered
    );
    assert_eq!(result.diagnostics.cooldown_state, "none");
    assert_eq!(result.diagnostics.rejection_reason, None);
    assert_eq!(result.diagnostics.suppression_reason, None);
    assert_eq!(result.diagnostics.cooldown_boundary, None);
    assert_eq!(result.diagnostics.evidence_window_size, 0);
    assert!(result.diagnostics.selected_evidence_event_ids.is_empty());
    assert_eq!(result.diagnostics.durable_write_path, "run_reflection");
    let diagnostics_json = serde_json::to_value(&result.diagnostics).unwrap();
    assert_eq!(diagnostics_json["trigger_type"], "failure");
    assert_eq!(diagnostics_json["namespace"], "project/agent-llm-mm");
    assert_eq!(
        diagnostics_json["trigger_key"],
        "project/agent-llm-mm:failure"
    );
    assert_eq!(diagnostics_json["outcome"], "not_triggered");
    assert_eq!(diagnostics_json["cooldown_state"], "none");
}

#[tokio::test]
async fn auto_reflection_returns_structured_diagnostics_for_rejected_proposal() {
    let deps = test_support::deps_for_failure_modes();
    deps.seed_project_reflection_event();

    let result = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_periodic(
            Namespace::for_project("agent-llm-mm"),
            vec!["periodic".to_string()],
        ),
    )
    .await
    .unwrap();

    assert!(!result.triggered);
    assert_eq!(result.trigger_type, Some(TriggerType::Periodic));
    assert_eq!(result.namespace.as_deref(), Some("project/agent-llm-mm"));
    assert_eq!(result.reflection_id, None);
    assert_eq!(result.ledger_status, Some(TriggerLedgerStatus::Rejected));
    assert_eq!(
        result.reason.as_deref(),
        Some("mock model did not detect a valid Periodic revision")
    );
    assert_eq!(result.diagnostics.trigger_type, TriggerType::Periodic);
    assert_eq!(result.diagnostics.namespace, "project/agent-llm-mm");
    assert_eq!(
        result.diagnostics.trigger_key,
        "project/agent-llm-mm:periodic"
    );
    assert_eq!(
        result.diagnostics.outcome,
        agent_llm_mm::domain::self_revision::AutoReflectOutcome::Rejected
    );
    assert_eq!(result.diagnostics.cooldown_state, "none");
    assert_eq!(
        result.diagnostics.rejection_reason.as_deref(),
        Some("mock model did not detect a valid Periodic revision")
    );
    assert_eq!(result.diagnostics.suppression_reason, None);
    assert_eq!(result.diagnostics.cooldown_boundary, None);
    assert_eq!(result.diagnostics.evidence_window_size, 1);
    assert!(result.diagnostics.selected_evidence_event_ids.is_empty());
    assert_eq!(result.diagnostics.durable_write_path, "run_reflection");
    let diagnostics_json = serde_json::to_value(&result.diagnostics).unwrap();
    assert_eq!(diagnostics_json["trigger_type"], "periodic");
    assert_eq!(diagnostics_json["namespace"], "project/agent-llm-mm");
    assert_eq!(
        diagnostics_json["trigger_key"],
        "project/agent-llm-mm:periodic"
    );
    assert_eq!(diagnostics_json["outcome"], "rejected");
    assert_eq!(diagnostics_json["cooldown_state"], "none");
    assert_eq!(
        result.trigger_key.as_deref(),
        Some("project/agent-llm-mm:periodic")
    );
    assert_eq!(
        result.evidence_event_ids,
        vec!["evt-reflection-1".to_string()]
    );
    assert_eq!(result.cooldown_until, None);
    assert_eq!(result.suppression_reason, None);
}

#[tokio::test]
async fn snapshot_budget_deduplicates_recent_duplicate_evidence_before_truncating() {
    let deps = test_support::deps_for_failure_modes();

    let snapshot = execute_build_snapshot(&deps, test_support::budgeted_snapshot())
        .await
        .unwrap()
        .snapshot;

    assert_eq!(
        snapshot.evidence,
        vec![
            "event:recent-duplicate".to_string(),
            "event:anchor".to_string(),
            "event:baseline".to_string(),
        ],
        "the current data model only tracks ordered event references, so this regression models a recent-event hijack as duplicate recent references exhausting the budget"
    );
}

#[tokio::test]
async fn auto_reflection_rejects_identity_patch_without_minimum_support_and_records_ledger() {
    let deps = test_support::deps_for_failure_modes();
    deps.set_self_revision_proposal(test_support::identity_only_auto_reflection_proposal());
    deps.seed_identity_support_context(
        vec!["episode-001".to_string()],
        vec![
            StoredClaim::new(
                "claim-supporting-1".to_string(),
                ClaimDraft::new(
                    Owner::World,
                    "self.role",
                    "is",
                    "principal_architect",
                    Mode::Observed,
                ),
                ClaimStatus::Active,
            ),
            StoredClaim::new(
                "claim-conflicting-1".to_string(),
                ClaimDraft::new(Owner::Self_, "self.role", "is", "architect", Mode::Observed),
                ClaimStatus::Active,
            ),
        ],
    );

    let result = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_conflict(
            Namespace::world(),
            vec!["conflict".to_string(), "identity".to_string()],
        ),
    )
    .await;

    assert!(matches!(result, Err(AppError::InvalidParams(_))));
    assert_eq!(
        deps.latest_trigger_status(),
        Some(TriggerLedgerStatus::Rejected)
    );
    assert!(deps.reflection("id-2").is_none());
}

#[tokio::test]
async fn auto_reflection_ignores_unrelated_episodes_for_identity_support() {
    let deps = test_support::deps_for_failure_modes();
    deps.set_self_revision_proposal(test_support::identity_only_auto_reflection_proposal());
    deps.seed_identity_support_context_without_provenance(
        vec![
            "episode-unrelated-001".to_string(),
            "episode-unrelated-002".to_string(),
            "episode-unrelated-003".to_string(),
        ],
        (1..=3)
            .map(|index| {
                StoredClaim::new(
                    format!("claim-supporting-{index}"),
                    ClaimDraft::new(
                        Owner::World,
                        "self.role",
                        "is",
                        "principal_architect",
                        Mode::Observed,
                    )
                    .with_namespace(Namespace::world()),
                    ClaimStatus::Active,
                )
            })
            .collect(),
    );

    let error = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_conflict(
            Namespace::world(),
            vec!["conflict".to_string(), "identity".to_string()],
        ),
    )
    .await
    .expect_err("unrelated episodes must not satisfy cross-episode identity support");

    assert!(
        error
            .to_string()
            .contains("requires support across at least 2 episodes"),
        "unexpected rejection: {error}"
    );
    assert_eq!(
        deps.latest_trigger_status(),
        Some(TriggerLedgerStatus::Rejected)
    );
    assert!(deps.reflections().is_empty());
    assert_eq!(
        deps.identity().canonical_claims(),
        &["identity:self=architect".to_string()]
    );
}

#[tokio::test]
async fn auto_reflection_ignores_cross_scope_evidence_links_for_identity_support() {
    let deps = test_support::deps_for_failure_modes();
    deps.set_self_revision_proposal(test_support::identity_only_auto_reflection_proposal());
    deps.seed_identity_support_context_with_cross_scope_evidence_links();

    let error = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_conflict(
            Namespace::world(),
            vec!["conflict".to_string(), "identity".to_string()],
        ),
    )
    .await
    .expect_err("cross-scope evidence links must not satisfy identity episode support");

    assert!(
        error
            .to_string()
            .contains("requires support across at least 2 episodes"),
        "unexpected rejection: {error}"
    );
    assert_eq!(
        deps.latest_trigger_status(),
        Some(TriggerLedgerStatus::Rejected)
    );
    assert!(deps.reflections().is_empty());
    assert_eq!(
        deps.identity().canonical_claims(),
        &["identity:self=architect".to_string()]
    );
}

#[tokio::test]
async fn auto_reflection_rejects_model_proposed_evidence_outside_trigger_window() {
    let deps = test_support::deps_for_failure_modes();
    deps.seed_failure_window(vec![
        (
            "evt-failure-1",
            "rollback after violating a hard commitment",
        ),
        (
            "evt-failure-2",
            "second rollback after violating the same hard commitment",
        ),
    ]);
    deps.set_self_revision_proposal(
        test_support::commitment_only_auto_reflection_proposal_with_policy(
            vec!["evt-outside-1".to_string(), "evt-outside-2".to_string()],
            None,
        ),
    );

    let result = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_failure(
            Namespace::for_project("agent-llm-mm"),
            vec!["failure".to_string(), "rollback".to_string()],
        ),
    )
    .await;

    assert!(matches!(result, Err(AppError::InvalidParams(_))));
    assert_eq!(
        deps.latest_trigger_status(),
        Some(TriggerLedgerStatus::Rejected)
    );
}

#[tokio::test]
async fn auto_reflection_records_rejection_reason_when_governed_evidence_validation_fails() {
    // 校验失败的统一拒绝语义：治理校验拒绝 model 越界证据时，
    // 既要落 Rejected ledger，也要把可解释原因原样向上传播。
    let deps = test_support::deps_for_failure_modes();
    deps.seed_failure_window(vec![
        (
            "evt-failure-1",
            "rollback after violating a hard commitment",
        ),
        (
            "evt-failure-2",
            "second rollback after violating the same hard commitment",
        ),
    ]);
    deps.set_self_revision_proposal(
        test_support::commitment_only_auto_reflection_proposal_with_policy(
            vec!["evt-outside-1".to_string()],
            None,
        ),
    );

    let result = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_failure(
            Namespace::for_project("agent-llm-mm"),
            vec!["failure".to_string(), "rollback".to_string()],
        ),
    )
    .await;

    match result {
        Err(AppError::InvalidParams(reason)) => {
            assert!(
                reason.contains("outside the current trigger window"),
                "拒绝原因应携带可解释文案，实际为：{reason}"
            );
        }
        other => panic!("expected governed-validation rejection, got {other:?}"),
    }
    assert_eq!(
        deps.latest_trigger_status(),
        Some(TriggerLedgerStatus::Rejected)
    );
    assert!(deps.latest_reflection().is_none());
}

#[tokio::test]
async fn auto_reflection_rejects_mixed_valid_and_invalid_model_proposed_evidence_ids() {
    let deps = test_support::deps_for_failure_modes();
    deps.seed_failure_window(vec![
        (
            "evt-failure-1",
            "rollback after violating a hard commitment",
        ),
        (
            "evt-failure-2",
            "second rollback after violating the same hard commitment",
        ),
    ]);
    deps.set_self_revision_proposal(
        test_support::commitment_only_auto_reflection_proposal_with_policy(
            vec!["evt-failure-2".to_string(), "evt-outside-1".to_string()],
            None,
        ),
    );

    let result = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_failure(
            Namespace::for_project("agent-llm-mm"),
            vec!["failure".to_string(), "rollback".to_string()],
        ),
    )
    .await;

    assert!(matches!(result, Err(AppError::InvalidParams(_))));
    assert_eq!(
        deps.latest_trigger_status(),
        Some(TriggerLedgerStatus::Rejected)
    );
    assert!(deps.latest_reflection().is_none());
}

#[tokio::test]
async fn auto_reflection_rejects_model_proposed_evidence_ids_that_do_not_match_query_policy() {
    let deps = test_support::deps_for_failure_modes();
    deps.seed_events(vec![
        StoredEvent::new(
            "evt-conflict-outside-window".to_string(),
            chrono::DateTime::parse_from_rfc3339("2026-03-23T09:55:00Z")
                .unwrap()
                .with_timezone(&Utc),
            Event::new(
                Owner::World,
                EventKind::Observation,
                "older conflicting observation outside the current trigger window",
            ),
        ),
        StoredEvent::new(
            "evt-conflict-1".to_string(),
            chrono::DateTime::parse_from_rfc3339("2026-03-23T10:01:00Z")
                .unwrap()
                .with_timezone(&Utc),
            Event::new(
                Owner::User,
                EventKind::Conversation,
                "user raised a possible commitment conflict",
            ),
        ),
        StoredEvent::new(
            "evt-conflict-2".to_string(),
            chrono::DateTime::parse_from_rfc3339("2026-03-23T10:02:00Z")
                .unwrap()
                .with_timezone(&Utc),
            Event::new(
                Owner::World,
                EventKind::Observation,
                "current conflicting observation inside the trigger window",
            ),
        ),
        StoredEvent::new(
            "evt-conflict-3".to_string(),
            chrono::DateTime::parse_from_rfc3339("2026-03-23T10:03:00Z")
                .unwrap()
                .with_timezone(&Utc),
            Event::new_with_namespace(
                Owner::World,
                Namespace::for_project("agent-llm-mm"),
                EventKind::Action,
                "self attempted a conflicting overwrite",
            )
            .unwrap(),
        ),
        StoredEvent::new(
            "evt-conflict-4".to_string(),
            chrono::DateTime::parse_from_rfc3339("2026-03-23T10:04:00Z")
                .unwrap()
                .with_timezone(&Utc),
            Event::new(
                Owner::User,
                EventKind::Conversation,
                "user requested commitment clarification",
            ),
        ),
        StoredEvent::new(
            "evt-conflict-5".to_string(),
            chrono::DateTime::parse_from_rfc3339("2026-03-23T10:05:00Z")
                .unwrap()
                .with_timezone(&Utc),
            Event::new_with_namespace(
                Owner::World,
                Namespace::for_project("agent-llm-mm"),
                EventKind::Action,
                "self retried the conflicting overwrite",
            )
            .unwrap(),
        ),
    ]);
    deps.set_self_revision_proposal(
        test_support::commitment_only_auto_reflection_proposal_with_policy(
            vec!["evt-conflict-3".to_string()],
            Some(EvidenceQuery {
                namespace: None,
                owner: Some(Owner::World),
                kind: Some(EventKind::Observation),
                limit: Some(5),
                recorded_after: None,
                recorded_before: None,
                event_id_prefix: None,
            }),
        ),
    );

    let result = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_conflict(
            Namespace::world(),
            vec!["conflict".to_string(), "commitment".to_string()],
        ),
    )
    .await;

    assert!(matches!(result, Err(AppError::InvalidParams(_))));
    assert_eq!(
        deps.latest_trigger_status(),
        Some(TriggerLedgerStatus::Rejected)
    );
    assert!(deps.latest_reflection().is_none());
}

#[tokio::test]
async fn auto_reflection_keeps_explicit_ids_authoritative_when_query_limit_only_matches_outside_window()
 {
    let deps = test_support::deps_for_failure_modes();
    deps.seed_events(vec![
        StoredEvent::new(
            "evt-outside-newest".to_string(),
            chrono::DateTime::parse_from_rfc3339("2026-03-23T10:06:00Z")
                .unwrap()
                .with_timezone(&Utc),
            Event::new(
                Owner::User,
                EventKind::Conversation,
                "newest non-failure event outside the failure trigger window",
            ),
        ),
        StoredEvent::new(
            "evt-failure-1".to_string(),
            chrono::DateTime::parse_from_rfc3339("2026-03-23T10:01:00Z")
                .unwrap()
                .with_timezone(&Utc),
            Event::new_with_namespace(
                Owner::World,
                Namespace::for_project("agent-llm-mm"),
                EventKind::Action,
                "rollback after violating a hard commitment",
            )
            .unwrap(),
        ),
        StoredEvent::new(
            "evt-failure-2".to_string(),
            chrono::DateTime::parse_from_rfc3339("2026-03-23T10:02:00Z")
                .unwrap()
                .with_timezone(&Utc),
            Event::new_with_namespace(
                Owner::World,
                Namespace::for_project("agent-llm-mm"),
                EventKind::Action,
                "second rollback after violating the same hard commitment",
            )
            .unwrap(),
        ),
    ]);
    deps.set_self_revision_proposal(
        test_support::commitment_only_auto_reflection_proposal_with_policy(
            vec!["evt-failure-2".to_string()],
            Some(EvidenceQuery {
                namespace: None,
                owner: None,
                kind: None,
                limit: Some(1),
                recorded_after: None,
                recorded_before: None,
                event_id_prefix: None,
            }),
        ),
    );

    let result = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_failure(
            Namespace::for_project("agent-llm-mm"),
            vec!["failure".to_string(), "rollback".to_string()],
        ),
    )
    .await
    .unwrap();

    assert!(result.triggered);
    assert_eq!(result.namespace.as_deref(), Some("project/agent-llm-mm"));
    assert_eq!(result.evidence_event_ids, vec!["evt-failure-2".to_string()]);
}

#[tokio::test]
async fn auto_reflection_applies_query_limit_within_current_trigger_window_when_ids_are_empty() {
    let deps = test_support::deps_for_failure_modes();
    deps.seed_events(vec![
        StoredEvent::new(
            "evt-outside-newest-1".to_string(),
            chrono::DateTime::parse_from_rfc3339("2026-03-23T10:06:00Z")
                .unwrap()
                .with_timezone(&Utc),
            Event::new(
                Owner::User,
                EventKind::Conversation,
                "newest non-failure event outside the failure trigger window",
            ),
        ),
        StoredEvent::new(
            "evt-outside-newest-2".to_string(),
            chrono::DateTime::parse_from_rfc3339("2026-03-23T10:05:30Z")
                .unwrap()
                .with_timezone(&Utc),
            Event::new(
                Owner::World,
                EventKind::Observation,
                "second newest non-failure event outside the failure trigger window",
            ),
        ),
        StoredEvent::new(
            "evt-failure-1".to_string(),
            chrono::DateTime::parse_from_rfc3339("2026-03-23T10:01:00Z")
                .unwrap()
                .with_timezone(&Utc),
            Event::new_with_namespace(
                Owner::World,
                Namespace::for_project("agent-llm-mm"),
                EventKind::Action,
                "rollback after violating a hard commitment",
            )
            .unwrap(),
        ),
        StoredEvent::new(
            "evt-failure-2".to_string(),
            chrono::DateTime::parse_from_rfc3339("2026-03-23T10:02:00Z")
                .unwrap()
                .with_timezone(&Utc),
            Event::new_with_namespace(
                Owner::World,
                Namespace::for_project("agent-llm-mm"),
                EventKind::Action,
                "second rollback after violating the same hard commitment",
            )
            .unwrap(),
        ),
    ]);
    deps.set_self_revision_proposal(
        test_support::commitment_only_auto_reflection_proposal_with_policy(
            Vec::new(),
            Some(EvidenceQuery {
                namespace: None,
                owner: None,
                kind: None,
                limit: Some(1),
                recorded_after: None,
                recorded_before: None,
                event_id_prefix: None,
            }),
        ),
    );

    let result = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_failure(
            Namespace::for_project("agent-llm-mm"),
            vec!["failure".to_string(), "rollback".to_string()],
        ),
    )
    .await
    .unwrap();

    assert!(result.triggered);
    assert_eq!(result.evidence_event_ids, vec!["evt-failure-2".to_string()]);
    let reflection = deps
        .latest_reflection()
        .expect("query-limited auto-reflection should persist a reflection");
    assert_eq!(
        reflection.supporting_evidence_event_ids,
        vec!["evt-failure-2".to_string()]
    );
    let handled_entry = deps
        .latest_trigger_entry()
        .expect("handled auto-reflection should persist a trigger ledger entry");
    assert_eq!(
        handled_entry.evidence_window,
        vec!["evt-failure-2".to_string(), "evt-failure-1".to_string()]
    );
}

#[tokio::test]
async fn auto_reflection_applies_model_proposed_evidence_subset_but_preserves_full_trigger_window_in_handled_ledger()
 {
    let deps = test_support::deps_for_failure_modes();
    deps.seed_failure_window(vec![
        (
            "evt-failure-1",
            "rollback after violating a hard commitment",
        ),
        (
            "evt-failure-2",
            "second rollback after violating the same hard commitment",
        ),
    ]);
    deps.set_self_revision_proposal(
        test_support::commitment_only_auto_reflection_proposal_with_policy(
            vec!["evt-failure-2".to_string()],
            None,
        ),
    );

    let result = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_failure(
            Namespace::for_project("agent-llm-mm"),
            vec!["failure".to_string(), "rollback".to_string()],
        ),
    )
    .await
    .unwrap();

    assert!(result.triggered);
    assert_eq!(result.evidence_event_ids, vec!["evt-failure-2".to_string()]);
    assert_eq!(result.diagnostics.trigger_type, TriggerType::Failure);
    assert_eq!(result.diagnostics.namespace, "project/agent-llm-mm");
    assert_eq!(
        result.diagnostics.trigger_key,
        "project/agent-llm-mm:failure"
    );
    assert_eq!(
        result.diagnostics.outcome,
        agent_llm_mm::domain::self_revision::AutoReflectOutcome::Handled
    );
    assert_eq!(result.diagnostics.cooldown_state, "set");
    assert_eq!(result.diagnostics.rejection_reason, None);
    assert_eq!(result.diagnostics.suppression_reason, None);
    assert_eq!(result.diagnostics.evidence_window_size, 2);
    assert_eq!(
        result.diagnostics.selected_evidence_event_ids,
        vec!["evt-failure-2".to_string()]
    );
    assert_eq!(result.diagnostics.durable_write_path, "run_reflection");
    let diagnostics_json = serde_json::to_value(&result.diagnostics).unwrap();
    assert_eq!(diagnostics_json["trigger_type"], "failure");
    assert_eq!(diagnostics_json["namespace"], "project/agent-llm-mm");
    assert_eq!(
        diagnostics_json["trigger_key"],
        "project/agent-llm-mm:failure"
    );
    assert_eq!(diagnostics_json["outcome"], "handled");
    assert_eq!(diagnostics_json["cooldown_state"], "set");
    let reflection = deps
        .latest_reflection()
        .expect("handled auto-reflection should persist a reflection");
    assert_eq!(
        reflection.supporting_evidence_event_ids,
        vec!["evt-failure-2".to_string()]
    );
    let handled_entry = deps
        .latest_trigger_entry()
        .expect("handled auto-reflection should persist a trigger ledger entry");
    assert_eq!(
        handled_entry.evidence_window,
        vec!["evt-failure-2".to_string(), "evt-failure-1".to_string()]
    );
}

#[tokio::test]
async fn auto_reflection_scopes_trigger_window_to_input_namespace() {
    let deps = test_support::deps_for_failure_modes();
    let project_namespace = Namespace::for_project("agent-llm-mm");
    deps.seed_events(vec![
        StoredEvent::new(
            "evt-project-a-1".to_string(),
            chrono::DateTime::parse_from_rfc3339("2026-03-23T10:01:00Z")
                .unwrap()
                .with_timezone(&Utc),
            Event::new_with_namespace(
                Owner::World,
                project_namespace.clone(),
                EventKind::Observation,
                "project A evidence should remain in the trigger window",
            )
            .unwrap(),
        ),
        StoredEvent::new(
            "evt-project-a-2".to_string(),
            chrono::DateTime::parse_from_rfc3339("2026-03-23T10:02:00Z")
                .unwrap()
                .with_timezone(&Utc),
            Event::new_with_namespace(
                Owner::World,
                project_namespace.clone(),
                EventKind::Observation,
                "newer project A evidence should remain in the trigger window",
            )
            .unwrap(),
        ),
        StoredEvent::new(
            "evt-project-b-newer".to_string(),
            chrono::DateTime::parse_from_rfc3339("2026-03-23T10:03:00Z")
                .unwrap()
                .with_timezone(&Utc),
            Event::new_with_namespace(
                Owner::World,
                Namespace::for_project("other"),
                EventKind::Observation,
                "newer project B evidence must not enter project A trigger window",
            )
            .unwrap(),
        ),
        StoredEvent::new(
            "evt-world-newest".to_string(),
            chrono::DateTime::parse_from_rfc3339("2026-03-23T10:04:00Z")
                .unwrap()
                .with_timezone(&Utc),
            Event::new(
                Owner::World,
                EventKind::Observation,
                "newest world evidence must not enter project A trigger window",
            ),
        ),
    ]);
    deps.set_self_revision_proposal(test_support::commitment_only_auto_reflection_proposal());

    let result = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_conflict(project_namespace, vec!["conflict".to_string()]),
    )
    .await
    .unwrap();

    assert!(result.triggered);
    assert_eq!(
        result.evidence_event_ids,
        vec!["evt-project-a-2".to_string(), "evt-project-a-1".to_string()]
    );
    let reflection = deps
        .latest_reflection()
        .expect("namespace-scoped auto-reflection should persist a reflection");
    assert_eq!(
        reflection.supporting_evidence_event_ids,
        vec!["evt-project-a-2".to_string(), "evt-project-a-1".to_string()]
    );
    let handled_entry = deps
        .latest_trigger_entry()
        .expect("namespace-scoped auto-reflection should persist a trigger ledger entry");
    assert_eq!(
        handled_entry.evidence_window,
        vec!["evt-project-a-2".to_string(), "evt-project-a-1".to_string()]
    );
}

#[tokio::test]
async fn auto_reflection_suppresses_unchanged_failure_window_after_cooldown_when_prior_reflection_used_subset()
 {
    let deps = test_support::deps_for_failure_modes();
    deps.seed_failure_window(vec![
        (
            "evt-failure-1",
            "rollback after violating a hard commitment",
        ),
        (
            "evt-failure-2",
            "second rollback after violating the same hard commitment",
        ),
    ]);
    deps.set_self_revision_proposal(
        test_support::commitment_only_auto_reflection_proposal_with_policy(
            vec!["evt-failure-2".to_string()],
            None,
        ),
    );

    let first = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_failure(
            Namespace::for_project("agent-llm-mm"),
            vec!["failure".to_string(), "rollback".to_string()],
        ),
    )
    .await
    .unwrap();
    deps.advance_now_by_hours(25);
    let second = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_failure(
            Namespace::for_project("agent-llm-mm"),
            vec!["failure".to_string(), "rollback".to_string()],
        ),
    )
    .await
    .unwrap();

    assert!(first.triggered);
    assert!(!second.triggered);
    assert_eq!(second.ledger_status, Some(TriggerLedgerStatus::Suppressed));
    assert_eq!(
        second.suppression_reason.as_deref(),
        Some("evidence_window_unchanged")
    );
    assert_eq!(deps.reflections().len(), 1);
}

#[tokio::test]
async fn auto_reflection_preserves_handled_baseline_across_cooldown_suppression_for_unchanged_failure_window()
 {
    let deps = test_support::deps_for_failure_modes();
    deps.seed_failure_window(vec![
        (
            "evt-failure-1",
            "rollback after violating a hard commitment",
        ),
        (
            "evt-failure-2",
            "second rollback after violating the same hard commitment",
        ),
    ]);
    deps.set_self_revision_proposal(
        test_support::commitment_only_auto_reflection_proposal_with_policy(
            vec!["evt-failure-2".to_string()],
            None,
        ),
    );

    let first = auto_reflect_if_needed::execute(
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
    deps.advance_now_by_hours(25);
    let third = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_failure(
            Namespace::for_project("agent-llm-mm"),
            vec!["failure".to_string(), "rollback".to_string()],
        ),
    )
    .await
    .unwrap();

    assert!(first.triggered);
    assert_eq!(first.ledger_status, Some(TriggerLedgerStatus::Handled));

    assert!(!second.triggered);
    assert_eq!(second.ledger_status, Some(TriggerLedgerStatus::Suppressed));
    assert_eq!(
        second.suppression_reason.as_deref(),
        Some("cooldown_active")
    );

    assert!(!third.triggered);
    assert_eq!(third.ledger_status, Some(TriggerLedgerStatus::Suppressed));
    assert_eq!(
        third.suppression_reason.as_deref(),
        Some("evidence_window_unchanged")
    );
    assert_eq!(
        deps.reflections().len(),
        1,
        "cooldown-expired unchanged failure windows should stay suppressed against the last handled baseline"
    );
}

/// B3：抑制结果除了自由文本 `suppression_reason`，还要带机器可读的 `suppression_category`。
/// 这里复用「handled 后立即再触发 → cooldown_active」的路径，断言枚举值与其 JSON 字面量
/// 同既有字符串口径一致，保证下游可直接按分类匹配而无需解析自由文本。
#[tokio::test]
async fn auto_reflection_suppression_exposes_machine_readable_category() {
    use agent_llm_mm::domain::self_revision::SuppressionCategory;

    let deps = test_support::deps_for_failure_modes();
    deps.seed_failure_window(vec![
        (
            "evt-failure-1",
            "rollback after violating a hard commitment",
        ),
        (
            "evt-failure-2",
            "second rollback after violating the same hard commitment",
        ),
    ]);
    deps.set_self_revision_proposal(
        test_support::commitment_only_auto_reflection_proposal_with_policy(
            vec!["evt-failure-2".to_string()],
            None,
        ),
    );

    let first = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_failure(
            Namespace::for_project("agent-llm-mm"),
            vec!["failure".to_string(), "rollback".to_string()],
        ),
    )
    .await
    .unwrap();
    let suppressed = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_failure(
            Namespace::for_project("agent-llm-mm"),
            vec!["failure".to_string(), "rollback".to_string()],
        ),
    )
    .await
    .unwrap();

    assert!(first.triggered);
    assert_eq!(
        suppressed.ledger_status,
        Some(TriggerLedgerStatus::Suppressed)
    );
    // 自由文本与机器可读分类必须同源，互为冗余而非歧义。
    assert_eq!(
        suppressed.suppression_reason.as_deref(),
        Some("cooldown_active")
    );
    assert_eq!(
        suppressed.diagnostics.suppression_category,
        Some(SuppressionCategory::CooldownActive)
    );
    assert_eq!(
        SuppressionCategory::CooldownActive.as_str(),
        "cooldown_active"
    );

    let diagnostics_json = serde_json::to_value(&suppressed.diagnostics).unwrap();
    assert_eq!(diagnostics_json["suppression_category"], "cooldown_active");
    assert_eq!(diagnostics_json["suppression_reason"], "cooldown_active");

    // 非抑制结果不应携带分类。
    let handled_json = serde_json::to_value(&first.diagnostics).unwrap();
    assert!(handled_json["suppression_category"].is_null());
}

#[tokio::test]
async fn auto_reflection_rejects_empty_proposed_evidence_query_instead_of_widening() {
    let deps = test_support::deps_for_failure_modes();
    deps.seed_failure_window(vec![
        (
            "evt-failure-1",
            "rollback after violating a hard commitment",
        ),
        (
            "evt-failure-2",
            "second rollback after violating the same hard commitment",
        ),
    ]);
    deps.set_self_revision_proposal(
        test_support::commitment_only_auto_reflection_proposal_with_policy(
            Vec::new(),
            Some(EvidenceQuery {
                namespace: None,
                owner: Some(Owner::World),
                kind: Some(EventKind::Observation),
                limit: Some(5),
                recorded_after: None,
                recorded_before: None,
                event_id_prefix: None,
            }),
        ),
    );

    let result = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_failure(
            Namespace::for_project("agent-llm-mm"),
            vec!["failure".to_string(), "rollback".to_string()],
        ),
    )
    .await;

    assert!(matches!(
        result,
        Err(AppError::InvalidParams(message))
            if message.contains("proposed evidence query did not match the current trigger window")
    ));
    assert_eq!(
        deps.latest_trigger_status(),
        Some(TriggerLedgerStatus::Rejected)
    );
    assert!(deps.latest_reflection().is_none());
}

#[tokio::test]
async fn auto_reflection_rejects_namespace_filter_with_no_trigger_window_intersection() {
    let deps = test_support::deps_for_failure_modes();
    deps.seed_events(vec![
        StoredEvent::new(
            "evt-conflict-1".to_string(),
            chrono::DateTime::parse_from_rfc3339("2026-03-23T10:01:00Z")
                .unwrap()
                .with_timezone(&Utc),
            Event::new_with_namespace(
                Owner::World,
                Namespace::for_project("agent-llm-mm"),
                EventKind::Observation,
                "current conflicting observation inside the trigger window",
            )
            .unwrap(),
        ),
        StoredEvent::new(
            "evt-conflict-2".to_string(),
            chrono::DateTime::parse_from_rfc3339("2026-03-23T10:02:00Z")
                .unwrap()
                .with_timezone(&Utc),
            Event::new_with_namespace(
                Owner::World,
                Namespace::for_project("agent-llm-mm"),
                EventKind::Observation,
                "second current conflicting observation inside the trigger window",
            )
            .unwrap(),
        ),
    ]);
    deps.set_self_revision_proposal(
        test_support::commitment_only_auto_reflection_proposal_with_policy(
            Vec::new(),
            Some(EvidenceQuery {
                namespace: Some(Namespace::for_project("other")),
                owner: Some(Owner::World),
                kind: Some(EventKind::Observation),
                limit: Some(5),
                recorded_after: None,
                recorded_before: None,
                event_id_prefix: None,
            }),
        ),
    );

    let result = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_conflict(
            Namespace::for_project("agent-llm-mm"),
            vec!["conflict".to_string(), "commitment".to_string()],
        ),
    )
    .await;

    assert!(matches!(
        result,
        Err(AppError::InvalidParams(message))
            if message.contains("proposed evidence query did not match the current trigger window")
    ));
    assert_eq!(
        deps.latest_trigger_status(),
        Some(TriggerLedgerStatus::Rejected)
    );
    assert!(deps.latest_reflection().is_none());
}

#[tokio::test]
async fn auto_reflection_rejects_noop_proposal_when_query_has_no_trigger_window_intersection() {
    let deps = test_support::deps_for_failure_modes();
    deps.seed_events(vec![StoredEvent::new(
        "evt-conflict-1".to_string(),
        chrono::DateTime::parse_from_rfc3339("2026-03-23T10:01:00Z")
            .unwrap()
            .with_timezone(&Utc),
        Event::new_with_namespace(
            Owner::World,
            Namespace::for_project("agent-llm-mm"),
            EventKind::Observation,
            "current conflicting observation inside the trigger window",
        )
        .unwrap(),
    )]);
    deps.set_self_revision_proposal(test_support::noop_auto_reflection_proposal_with_policy(
        Some(EvidenceQuery {
            namespace: Some(Namespace::for_project("other")),
            owner: Some(Owner::World),
            kind: Some(EventKind::Observation),
            limit: Some(5),
            recorded_after: None,
            recorded_before: None,
            event_id_prefix: None,
        }),
    ));

    let result = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_conflict(
            Namespace::for_project("agent-llm-mm"),
            vec!["conflict".to_string(), "commitment".to_string()],
        ),
    )
    .await;

    assert!(matches!(
        result,
        Err(AppError::InvalidParams(message))
            if message.contains("proposed evidence query did not match the current trigger window")
    ));
    assert_eq!(
        deps.latest_trigger_status(),
        Some(TriggerLedgerStatus::Rejected)
    );
    assert!(deps.latest_reflection().is_none());
}

#[tokio::test]
async fn auto_reflection_rejects_proposed_evidence_query_zero_limit_instead_of_masking_as_empty() {
    // C1 第三路径覆盖：model 提议的 evidence query limit==0 必须在 governed 解析入口
    // 被拒绝为 InvalidParams，而非经后置 take(0) 静默收窄为空集、当成确定性空匹配掩盖。
    // 刻意 seed 与 query 匹配的 World/Observation 事件，使候选集非空——若缺少早拒，
    // take(0) 会产出空集并 Ok 返回，本测试将假绿；早拒到位时则为干净的 Err。
    let deps = test_support::deps_for_failure_modes();
    deps.seed_events(vec![
        StoredEvent::new(
            "evt-conflict-1".to_string(),
            chrono::DateTime::parse_from_rfc3339("2026-03-23T10:01:00Z")
                .unwrap()
                .with_timezone(&Utc),
            Event::new(
                Owner::World,
                EventKind::Observation,
                "matching observation inside the trigger window",
            ),
        ),
        StoredEvent::new(
            "evt-conflict-2".to_string(),
            chrono::DateTime::parse_from_rfc3339("2026-03-23T10:02:00Z")
                .unwrap()
                .with_timezone(&Utc),
            Event::new(
                Owner::World,
                EventKind::Observation,
                "another matching observation inside the trigger window",
            ),
        ),
    ]);
    deps.set_self_revision_proposal(
        test_support::commitment_only_auto_reflection_proposal_with_policy(
            Vec::new(),
            Some(EvidenceQuery {
                namespace: None,
                owner: Some(Owner::World),
                kind: Some(EventKind::Observation),
                limit: Some(0),
                recorded_after: None,
                recorded_before: None,
                event_id_prefix: None,
            }),
        ),
    );

    let result = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_conflict(
            Namespace::world(),
            vec!["conflict".to_string(), "commitment".to_string()],
        ),
    )
    .await;

    assert!(matches!(
        result,
        Err(AppError::InvalidParams(message))
            if message.contains("limit must be at least 1")
    ));
    assert_eq!(
        deps.latest_trigger_status(),
        Some(TriggerLedgerStatus::Rejected)
    );
    assert!(deps.latest_reflection().is_none());
}

#[tokio::test]
async fn fake_event_store_rejects_zero_limit_for_bounded_and_unbounded_evidence_queries() {
    let deps = test_support::deps_for_failure_modes();
    let query = EvidenceQuery {
        namespace: None,
        owner: Some(Owner::World),
        kind: Some(EventKind::Observation),
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
async fn auto_reflection_intersects_proposed_evidence_query_with_current_trigger_window_when_ids_are_empty()
 {
    let deps = test_support::deps_for_failure_modes();
    deps.seed_events(vec![
        StoredEvent::new(
            "evt-conflict-outside-window".to_string(),
            chrono::DateTime::parse_from_rfc3339("2026-03-23T09:55:00Z")
                .unwrap()
                .with_timezone(&Utc),
            Event::new(
                Owner::World,
                EventKind::Observation,
                "older conflicting observation outside the current trigger window",
            ),
        ),
        StoredEvent::new(
            "evt-conflict-1".to_string(),
            chrono::DateTime::parse_from_rfc3339("2026-03-23T10:01:00Z")
                .unwrap()
                .with_timezone(&Utc),
            Event::new(
                Owner::User,
                EventKind::Conversation,
                "user raised a possible commitment conflict",
            ),
        ),
        StoredEvent::new(
            "evt-conflict-2".to_string(),
            chrono::DateTime::parse_from_rfc3339("2026-03-23T10:02:00Z")
                .unwrap()
                .with_timezone(&Utc),
            Event::new(
                Owner::World,
                EventKind::Observation,
                "current conflicting observation inside the trigger window",
            ),
        ),
        StoredEvent::new(
            "evt-conflict-3".to_string(),
            chrono::DateTime::parse_from_rfc3339("2026-03-23T10:03:00Z")
                .unwrap()
                .with_timezone(&Utc),
            Event::new(
                Owner::World,
                EventKind::Action,
                "self attempted a conflicting overwrite",
            ),
        ),
        StoredEvent::new(
            "evt-conflict-4".to_string(),
            chrono::DateTime::parse_from_rfc3339("2026-03-23T10:04:00Z")
                .unwrap()
                .with_timezone(&Utc),
            Event::new(
                Owner::User,
                EventKind::Conversation,
                "user requested commitment clarification",
            ),
        ),
        StoredEvent::new(
            "evt-conflict-5".to_string(),
            chrono::DateTime::parse_from_rfc3339("2026-03-23T10:05:00Z")
                .unwrap()
                .with_timezone(&Utc),
            Event::new(
                Owner::World,
                EventKind::Action,
                "self retried the conflicting overwrite",
            ),
        ),
    ]);
    deps.set_self_revision_proposal(
        test_support::commitment_only_auto_reflection_proposal_with_policy(
            Vec::new(),
            Some(EvidenceQuery {
                namespace: None,
                owner: Some(Owner::World),
                kind: Some(EventKind::Observation),
                limit: Some(5),
                recorded_after: None,
                recorded_before: None,
                event_id_prefix: None,
            }),
        ),
    );

    let result = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_conflict(
            Namespace::world(),
            vec!["conflict".to_string(), "commitment".to_string()],
        ),
    )
    .await
    .unwrap();

    assert!(result.triggered);
    assert_eq!(
        result.evidence_event_ids,
        vec!["evt-conflict-2".to_string()]
    );
    let reflection = deps
        .latest_reflection()
        .expect("query-intersected auto-reflection should persist a reflection");
    assert_eq!(
        reflection.supporting_evidence_event_ids,
        vec!["evt-conflict-2".to_string()]
    );
    let handled_entry = deps
        .latest_trigger_entry()
        .expect("handled auto-reflection should persist a trigger ledger entry");
    assert_eq!(
        handled_entry.evidence_window,
        vec![
            "evt-conflict-5".to_string(),
            "evt-conflict-3".to_string(),
            "evt-conflict-2".to_string(),
        ]
    );
}

#[tokio::test]
async fn auto_reflection_normalizes_prefixed_proposal_evidence_ids_before_governance_and_audit() {
    let deps = test_support::deps_for_failure_modes();
    deps.seed_events(vec![
        StoredEvent::new(
            "evt-prefixed-conflict-1".to_string(),
            chrono::DateTime::parse_from_rfc3339("2026-03-23T10:01:00Z")
                .unwrap()
                .with_timezone(&Utc),
            Event::new(
                Owner::User,
                EventKind::Conversation,
                "user raised a possible commitment conflict",
            ),
        ),
        StoredEvent::new(
            "evt-prefixed-conflict-2".to_string(),
            chrono::DateTime::parse_from_rfc3339("2026-03-23T10:02:00Z")
                .unwrap()
                .with_timezone(&Utc),
            Event::new(
                Owner::World,
                EventKind::Observation,
                "current conflicting observation inside the trigger window",
            ),
        ),
    ]);
    deps.set_self_revision_proposal(
        test_support::commitment_only_auto_reflection_proposal_with_policy(
            vec![
                "event:evt-prefixed-conflict-2".to_string(),
                "evt-prefixed-conflict-2".to_string(),
            ],
            None,
        ),
    );

    let result = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_conflict(
            Namespace::world(),
            vec!["conflict".to_string(), "commitment".to_string()],
        ),
    )
    .await
    .unwrap();

    assert!(result.triggered);
    assert_eq!(
        result.evidence_event_ids,
        vec!["evt-prefixed-conflict-2".to_string()]
    );
    assert_eq!(
        result.diagnostics.selected_evidence_event_ids,
        vec!["evt-prefixed-conflict-2".to_string()]
    );
    assert_eq!(
        deps.latest_reflection()
            .expect("handled auto-reflection should persist audit evidence")
            .supporting_evidence_event_ids,
        vec!["evt-prefixed-conflict-2".to_string()]
    );
}

#[tokio::test]
async fn auto_reflection_preserves_handled_reflection_id_when_unchanged_suppression_follows_rejected_attempt()
 {
    let deps = test_support::deps_for_failure_modes();
    deps.seed_failure_window(vec![
        (
            "evt-failure-1",
            "rollback after violating a hard commitment",
        ),
        (
            "evt-failure-2",
            "second rollback after violating the same hard commitment",
        ),
    ]);
    deps.set_self_revision_proposal(
        test_support::commitment_only_auto_reflection_proposal_with_policy(
            vec!["evt-failure-2".to_string()],
            None,
        ),
    );

    let handled = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_failure(
            Namespace::for_project("agent-llm-mm"),
            vec!["failure".to_string(), "rollback".to_string()],
        ),
    )
    .await
    .unwrap();

    deps.advance_now_by_hours(25);
    deps.seed_failure_window(vec![
        (
            "evt-failure-2",
            "second rollback after violating a hard commitment",
        ),
        (
            "evt-failure-3",
            "third rollback after violating a different hard commitment",
        ),
    ]);
    deps.set_self_revision_proposal(
        agent_llm_mm::domain::self_revision::SelfRevisionProposal::no_revision(
            "mock model did not detect a valid Failure revision".to_string(),
        ),
    );

    let rejected = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_failure(
            Namespace::for_project("agent-llm-mm"),
            vec!["failure".to_string(), "rollback".to_string()],
        ),
    )
    .await
    .unwrap();

    deps.seed_failure_window(vec![
        (
            "evt-failure-1",
            "rollback after violating a hard commitment",
        ),
        (
            "evt-failure-2",
            "second rollback after violating the same hard commitment",
        ),
    ]);

    let suppressed = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_failure(
            Namespace::for_project("agent-llm-mm"),
            vec!["failure".to_string(), "rollback".to_string()],
        ),
    )
    .await
    .unwrap();

    assert!(handled.triggered);
    assert_eq!(handled.ledger_status, Some(TriggerLedgerStatus::Handled));

    assert!(!rejected.triggered);
    assert_eq!(rejected.ledger_status, Some(TriggerLedgerStatus::Rejected));

    assert!(!suppressed.triggered);
    assert_eq!(
        suppressed.ledger_status,
        Some(TriggerLedgerStatus::Suppressed)
    );
    assert_eq!(
        suppressed.suppression_reason.as_deref(),
        Some("evidence_window_unchanged")
    );
    assert_eq!(suppressed.reflection_id, handled.reflection_id);
    assert_eq!(deps.reflections().len(), 1);
}

#[tokio::test]
async fn auto_reflection_applies_recency_filters_from_proposed_evidence_query() {
    let deps = test_support::deps_for_failure_modes();
    let base_time = chrono::DateTime::parse_from_rfc3339("2026-03-23T10:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    deps.seed_events(vec![
        StoredEvent::new(
            "evt-conflict-old".to_string(),
            base_time + chrono::Duration::minutes(1),
            Event::new(
                Owner::World,
                EventKind::Observation,
                "older conflicting observation outside the proposed recency window",
            ),
        ),
        StoredEvent::new(
            "evt-conflict-mid".to_string(),
            base_time + chrono::Duration::minutes(2),
            Event::new(
                Owner::World,
                EventKind::Observation,
                "middle conflicting observation inside the proposed recency window",
            ),
        ),
        StoredEvent::new(
            "evt-conflict-new".to_string(),
            base_time + chrono::Duration::minutes(3),
            Event::new(
                Owner::World,
                EventKind::Observation,
                "newer conflicting observation inside the proposed recency window",
            ),
        ),
    ]);
    deps.set_self_revision_proposal(
        test_support::commitment_only_auto_reflection_proposal_with_policy(
            Vec::new(),
            Some(EvidenceQuery {
                namespace: None,
                owner: Some(Owner::World),
                kind: Some(EventKind::Observation),
                limit: None,
                recorded_after: Some(base_time + chrono::Duration::minutes(2)),
                recorded_before: Some(base_time + chrono::Duration::minutes(3)),
                event_id_prefix: None,
            }),
        ),
    );

    let result = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_conflict(
            Namespace::world(),
            vec!["conflict".to_string(), "commitment".to_string()],
        ),
    )
    .await
    .unwrap();

    assert_eq!(
        result.evidence_event_ids,
        vec![
            "evt-conflict-new".to_string(),
            "evt-conflict-mid".to_string()
        ]
    );
    assert_eq!(
        deps.evidence_query_trace()
            .last()
            .expect("auto-reflection should query evidence")
            .recorded_after,
        Some(base_time + chrono::Duration::minutes(2))
    );
    assert_eq!(
        deps.latest_reflection()
            .expect("recency-filtered auto-reflection should persist")
            .supporting_evidence_event_ids,
        vec![
            "evt-conflict-new".to_string(),
            "evt-conflict-mid".to_string()
        ]
    );
}

#[tokio::test]
async fn auto_reflection_intersects_proposed_event_id_prefix_with_trigger_window() {
    let deps = test_support::deps_for_failure_modes();
    let base_time = chrono::DateTime::parse_from_rfc3339("2026-03-23T10:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    deps.seed_events(vec![
        StoredEvent::new(
            "alpha-conflict".to_string(),
            base_time + chrono::Duration::minutes(1),
            Event::new(
                Owner::World,
                EventKind::Observation,
                "alpha conflicting observation in the trigger window",
            ),
        ),
        StoredEvent::new(
            "beta-conflict".to_string(),
            base_time + chrono::Duration::minutes(2),
            Event::new(
                Owner::World,
                EventKind::Observation,
                "beta conflicting observation in the trigger window",
            ),
        ),
    ]);
    deps.set_self_revision_proposal(
        test_support::commitment_only_auto_reflection_proposal_with_policy(
            Vec::new(),
            Some(EvidenceQuery {
                namespace: None,
                owner: None,
                kind: None,
                limit: None,
                recorded_after: None,
                recorded_before: None,
                event_id_prefix: Some("alpha-".to_string()),
            }),
        ),
    );

    let result = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_conflict(
            Namespace::world(),
            vec!["conflict".to_string(), "commitment".to_string()],
        ),
    )
    .await
    .unwrap();

    // 前缀过滤仍是与 trigger window 的交集（no-widening）：只保留命中前缀且本就在窗口内的事件。
    assert_eq!(
        result.evidence_event_ids,
        vec!["alpha-conflict".to_string()]
    );
    assert_eq!(
        deps.latest_reflection()
            .expect("prefix-filtered auto-reflection should persist")
            .supporting_evidence_event_ids,
        vec!["alpha-conflict".to_string()]
    );
}

#[tokio::test]
async fn auto_reflection_rejected_identity_attempt_does_not_start_cooldown_for_later_valid_retry() {
    let deps = test_support::deps_for_failure_modes();
    deps.set_self_revision_proposal(test_support::identity_only_auto_reflection_proposal());
    deps.seed_identity_support_context(
        vec!["episode-001".to_string()],
        vec![StoredClaim::new(
            "claim-supporting-1".to_string(),
            ClaimDraft::new(
                Owner::World,
                "self.role",
                "is",
                "principal_architect",
                Mode::Observed,
            )
            .with_namespace(Namespace::world()),
            ClaimStatus::Active,
        )],
    );

    let first = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_conflict(
            Namespace::world(),
            vec!["conflict".to_string(), "identity".to_string()],
        ),
    )
    .await;

    assert!(matches!(first, Err(AppError::InvalidParams(_))));
    assert_eq!(
        deps.latest_trigger_status(),
        Some(TriggerLedgerStatus::Rejected)
    );
    let rejected_entry = deps
        .latest_trigger_entry()
        .expect("rejected attempt should remain auditable");
    assert_eq!(rejected_entry.handled_at, None);
    assert_eq!(rejected_entry.cooldown_until, None);

    deps.seed_identity_support_context(
        vec![
            "episode-101".to_string(),
            "episode-202".to_string(),
            "episode-303".to_string(),
        ],
        vec![
            StoredClaim::new(
                "claim-supporting-1".to_string(),
                ClaimDraft::new(
                    Owner::World,
                    "self.role",
                    "is",
                    "principal_architect",
                    Mode::Observed,
                )
                .with_namespace(Namespace::world()),
                ClaimStatus::Active,
            ),
            StoredClaim::new(
                "claim-supporting-2".to_string(),
                ClaimDraft::new(
                    Owner::World,
                    "self.role",
                    "is",
                    "principal_architect",
                    Mode::Observed,
                )
                .with_namespace(Namespace::world()),
                ClaimStatus::Active,
            ),
            StoredClaim::new(
                "claim-supporting-3".to_string(),
                ClaimDraft::new(
                    Owner::Self_,
                    "self.role",
                    "is",
                    "principal_architect",
                    Mode::Observed,
                )
                .with_namespace(Namespace::world()),
                ClaimStatus::Active,
            ),
        ],
    );

    let second = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_conflict(
            Namespace::world(),
            vec!["conflict".to_string(), "identity".to_string()],
        ),
    )
    .await
    .unwrap();

    assert!(second.triggered);
    assert_eq!(second.trigger_type, Some(TriggerType::Conflict));
    assert_eq!(
        deps.latest_trigger_status(),
        Some(TriggerLedgerStatus::Handled)
    );
    let reflection = deps
        .latest_reflection()
        .expect("later valid retry should persist a reflection");
    assert_eq!(reflection.superseded_claim_id, None);
    assert_eq!(reflection.replacement_claim_id, None);
}

#[tokio::test]
async fn auto_reflection_handled_ledger_failure_rolls_back_reflection_updates() {
    let deps = test_support::deps_with_fail_point(FailPoint::AppendHandledTriggerLedger);
    deps.set_self_revision_proposal(test_support::commitment_only_auto_reflection_proposal());
    let evidence_links_before = deps.evidence_links();

    let result = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_conflict(
            Namespace::world(),
            vec!["conflict".to_string(), "commitment".to_string()],
        ),
    )
    .await;

    assert!(
        matches!(result, Err(AppError::Message(message)) if message == "injected handled trigger ledger failure")
    );
    assert!(deps.reflections().is_empty());
    assert_eq!(
        deps.commitments(),
        vec![Commitment::new(
            Owner::Self_,
            "forbid:write_identity_core_directly",
        )]
    );
    assert_eq!(
        deps.state
            .lock()
            .unwrap()
            .committed
            .self_model_versions
            .len(),
        1
    );
    assert!(deps.state.lock().unwrap().committed.receipts.is_empty());
    assert_eq!(
        deps.identity().canonical_claims(),
        &["identity:self=architect".to_string()]
    );
    assert_eq!(deps.evidence_links(), evidence_links_before);
    let failed_entries = deps.trigger_entries();
    assert_eq!(failed_entries.len(), 1);
    assert_eq!(failed_entries[0].status, TriggerLedgerStatus::Rejected);
    assert_eq!(failed_entries[0].reflection_id, None);
    assert_eq!(failed_entries[0].handled_at, None);
    assert_eq!(failed_entries[0].cooldown_until, None);

    deps.clear_fail_point();

    let retry = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_conflict(
            Namespace::world(),
            vec!["conflict".to_string(), "commitment".to_string()],
        ),
    )
    .await
    .unwrap();

    assert!(retry.triggered);
    assert_eq!(
        deps.latest_trigger_status(),
        Some(TriggerLedgerStatus::Handled)
    );
    assert_eq!(deps.reflections().len(), 1);
}

#[tokio::test]
async fn auto_reflection_commit_failure_records_only_rejected_audit_and_rolls_back_deeper_updates()
{
    let deps = test_support::deps_with_fail_point(FailPoint::CommitReflection);
    deps.set_self_revision_proposal(
        test_support::identity_and_commitment_auto_reflection_proposal(),
    );
    deps.seed_identity_support_context(
        vec![
            "episode-support-001".to_string(),
            "episode-support-002".to_string(),
        ],
        (1..=3)
            .map(|index| {
                StoredClaim::new(
                    format!("claim-supporting-{index}"),
                    ClaimDraft::new(
                        Owner::World,
                        "self.role",
                        "is",
                        "principal_architect",
                        Mode::Observed,
                    )
                    .with_namespace(Namespace::world()),
                    ClaimStatus::Active,
                )
            })
            .collect(),
    );
    let evidence_links_before = deps.evidence_links();

    let result = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_conflict(
            Namespace::world(),
            vec!["conflict".to_string(), "identity".to_string()],
        ),
    )
    .await;

    assert!(
        matches!(result, Err(AppError::Message(message)) if message == "injected reflection commit failure")
    );
    assert_eq!(
        deps.state
            .lock()
            .unwrap()
            .committed
            .self_model_versions
            .len(),
        1
    );
    assert!(deps.state.lock().unwrap().committed.receipts.is_empty());
    assert_eq!(
        deps.identity().canonical_claims(),
        &["identity:self=architect".to_string()]
    );
    assert_eq!(
        deps.commitments(),
        vec![Commitment::new(
            Owner::Self_,
            "forbid:write_identity_core_directly",
        )]
    );
    assert!(deps.reflections().is_empty());
    assert_eq!(deps.evidence_links(), evidence_links_before);
    for claim_id in [
        "claim-supporting-1",
        "claim-supporting-2",
        "claim-supporting-3",
    ] {
        assert_eq!(
            deps.claim(claim_id)
                .expect("supporting claim must remain")
                .status,
            ClaimStatus::Active
        );
    }
    let entries = deps.trigger_entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].status, TriggerLedgerStatus::Rejected);
    assert_eq!(entries[0].reflection_id, None);
    assert_eq!(entries[0].handled_at, None);
    assert_eq!(entries[0].cooldown_until, None);
}

#[tokio::test]
async fn auto_reflection_suppresses_periodic_trigger_during_cooldown() {
    let deps = test_support::deps_for_failure_modes();
    deps.seed_periodic_cooldown("project/agent-llm-mm:periodic");

    let result = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_periodic(
            Namespace::for_project("agent-llm-mm"),
            vec!["periodic".to_string()],
        ),
    )
    .await
    .unwrap();

    assert!(!result.triggered);
    assert_eq!(result.trigger_type, Some(TriggerType::Periodic));
    assert_eq!(
        deps.latest_trigger_status(),
        Some(TriggerLedgerStatus::Suppressed)
    );
    assert!(deps.reflection("id-2").is_none());
}

#[tokio::test]
async fn auto_reflection_returns_structured_diagnostics_for_suppressed_trigger() {
    let deps = test_support::deps_for_failure_modes();
    deps.seed_project_reflection_event();
    deps.seed_periodic_cooldown("project/agent-llm-mm:periodic");

    let result = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_periodic(
            Namespace::for_project("agent-llm-mm"),
            vec!["periodic".to_string()],
        ),
    )
    .await
    .unwrap();

    assert_eq!(result.ledger_status, Some(TriggerLedgerStatus::Suppressed));
    assert_eq!(result.namespace.as_deref(), Some("project/agent-llm-mm"));
    assert_eq!(result.reflection_id.as_deref(), Some("seeded-reflection"));
    assert_eq!(result.diagnostics.trigger_type, TriggerType::Periodic);
    assert_eq!(result.diagnostics.namespace, "project/agent-llm-mm");
    assert_eq!(
        result.diagnostics.trigger_key,
        "project/agent-llm-mm:periodic"
    );
    assert_eq!(
        result.diagnostics.outcome,
        agent_llm_mm::domain::self_revision::AutoReflectOutcome::Suppressed
    );
    assert_eq!(result.diagnostics.cooldown_state, "active");
    assert_eq!(result.diagnostics.rejection_reason, None);
    assert_eq!(
        result.trigger_key.as_deref(),
        Some("project/agent-llm-mm:periodic")
    );
    assert!(result.cooldown_until.is_some());
    assert_eq!(result.diagnostics.cooldown_boundary, result.cooldown_until);
    assert_eq!(
        result.suppression_reason.as_deref(),
        Some("cooldown_active")
    );
    assert_eq!(
        result.diagnostics.suppression_reason.as_deref(),
        Some("cooldown_active")
    );
    assert_eq!(result.diagnostics.evidence_window_size, 1);
    assert!(result.diagnostics.selected_evidence_event_ids.is_empty());
    assert_eq!(result.diagnostics.durable_write_path, "run_reflection");
    let diagnostics_json = serde_json::to_value(&result.diagnostics).unwrap();
    assert_eq!(diagnostics_json["trigger_type"], "periodic");
    assert_eq!(diagnostics_json["namespace"], "project/agent-llm-mm");
    assert_eq!(
        diagnostics_json["trigger_key"],
        "project/agent-llm-mm:periodic"
    );
    assert_eq!(diagnostics_json["outcome"], "suppressed");
    assert_eq!(diagnostics_json["cooldown_state"], "active");
}

#[tokio::test]
async fn auto_reflection_repeated_suppression_does_not_extend_existing_cooldown() {
    let deps = test_support::deps_for_failure_modes();
    deps.seed_periodic_cooldown("project/agent-llm-mm:periodic");

    let first = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_periodic(
            Namespace::for_project("agent-llm-mm"),
            vec!["periodic".to_string()],
        ),
    )
    .await
    .unwrap();
    let first_entry = deps
        .latest_trigger_entry()
        .expect("suppressed trigger should be recorded");

    let second = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_periodic(
            Namespace::for_project("agent-llm-mm"),
            vec!["periodic".to_string()],
        ),
    )
    .await
    .unwrap();
    let second_entry = deps
        .latest_trigger_entry()
        .expect("repeated suppressed trigger should be recorded");

    assert!(!first.triggered);
    assert!(!second.triggered);
    assert_eq!(first.ledger_status, Some(TriggerLedgerStatus::Suppressed));
    assert_eq!(second.ledger_status, Some(TriggerLedgerStatus::Suppressed));
    assert_eq!(first_entry.cooldown_until, second_entry.cooldown_until);
    assert_eq!(
        second_entry.cooldown_until,
        Some(
            chrono::DateTime::parse_from_rfc3339("2026-03-24T09:50:00Z")
                .unwrap()
                .with_timezone(&Utc)
        )
    );
}

#[tokio::test]
async fn auto_reflection_preserves_reflection_id_for_non_cooldown_suppression() {
    let deps = test_support::deps_for_failure_modes();
    deps.seed_periodic_watermark_suppression("project/agent-llm-mm:periodic");

    let result = auto_reflect_if_needed::execute(
        &deps,
        AutoReflectInput::for_periodic(
            Namespace::for_project("agent-llm-mm"),
            vec!["periodic".to_string()],
        ),
    )
    .await
    .unwrap();

    assert_eq!(result.ledger_status, Some(TriggerLedgerStatus::Suppressed));
    assert_eq!(result.reflection_id.as_deref(), Some("seeded-reflection"));
    assert_eq!(
        result.suppression_reason.as_deref(),
        Some("episode_watermark_unchanged")
    );
}

mod test_support {
    use super::*;

    pub fn deps_for_failure_modes() -> FailureModeDeps {
        FailureModeDeps::new(State::default())
    }

    pub fn deps_with_fail_point(fail_point: FailPoint) -> FailureModeDeps {
        FailureModeDeps::new(State {
            fail_point: Some(fail_point),
            ..State::default()
        })
    }

    pub fn inferred_without_evidence() -> IngestInput {
        IngestInput::new(
            Event::new(
                Owner::User,
                EventKind::Conversation,
                "The user guessed at an unsupported identity change.",
            ),
            vec![ClaimDraft::new(
                Owner::Self_,
                "self.role",
                "is",
                "principal_architect",
                Mode::Inferred,
            )],
            None,
        )
    }

    pub fn conflicting_reflection() -> ReflectionInput {
        ReflectionInput::new(
            Reflection::new("A conflicting reflection should mark the claim as disputed."),
            "claim-conflict",
            None,
            Vec::new(),
        )
    }

    pub fn reflection_updates_without_evidence() -> ReflectionInput {
        ReflectionInput::new(
            Reflection::new("Identity-only updates still require supporting evidence."),
            "claim-conflict",
            None,
            Vec::new(),
        )
        .with_identity_update(vec!["identity:self=principal_architect".to_string()])
        .with_commitment_updates(vec![Commitment::new(
            Owner::Self_,
            "prefer:reflect_before_identity_changes",
        )])
    }

    pub fn reflection_updates_with_evidence() -> ReflectionInput {
        ReflectionInput::new(
            Reflection::new("Reflection should replace claim, identity, and commitments together."),
            "claim-conflict",
            Some(ClaimDraft::new(
                Owner::Self_,
                "self.role",
                "is",
                "principal_architect",
                Mode::Observed,
            )),
            vec!["evt-reflection-1".to_string()],
        )
        .with_identity_update(vec!["identity:self=principal_architect".to_string()])
        .with_commitment_updates(vec![Commitment::new(
            Owner::Self_,
            "prefer:reflect_before_identity_changes",
        )])
    }

    pub fn reflection_updates_with_empty_identity() -> ReflectionInput {
        ReflectionInput::new(
            Reflection::new("Identity updates cannot clear the canonical identity list."),
            "claim-conflict",
            None,
            vec!["evt-reflection-1".to_string()],
        )
        .with_identity_update(Vec::new())
    }

    pub fn budgeted_snapshot() -> BuildSelfSnapshotInput {
        BuildSelfSnapshotInput {
            scope: MemoryScope::legacy_unscoped(),
            evidence_manifest: None,
            time_window: SnapshotTimeWindow::unbounded(),
            budget: SnapshotBudget::new(3),
        }
    }

    pub fn commitment_only_auto_reflection_proposal()
    -> agent_llm_mm::domain::self_revision::SelfRevisionProposal {
        agent_llm_mm::domain::self_revision::SelfRevisionProposal {
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
        }
    }

    pub fn commitment_only_auto_reflection_proposal_with_policy(
        proposed_evidence_event_ids: Vec<String>,
        proposed_evidence_query: Option<EvidenceQuery>,
    ) -> agent_llm_mm::domain::self_revision::SelfRevisionProposal {
        agent_llm_mm::domain::self_revision::SelfRevisionProposal {
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
            proposed_evidence_event_ids,
            proposed_evidence_query,
            confidence: Some("medium".to_string()),
        }
    }

    pub fn noop_auto_reflection_proposal_with_policy(
        proposed_evidence_query: Option<EvidenceQuery>,
    ) -> agent_llm_mm::domain::self_revision::SelfRevisionProposal {
        agent_llm_mm::domain::self_revision::SelfRevisionProposal {
            should_reflect: true,
            rationale: "record-only proposal should still respect evidence query governance"
                .to_string(),
            machine_patch: agent_llm_mm::domain::self_revision::SelfRevisionPatch::default(),
            proposed_evidence_event_ids: Vec::new(),
            proposed_evidence_query,
            confidence: Some("medium".to_string()),
        }
    }

    pub fn identity_only_auto_reflection_proposal()
    -> agent_llm_mm::domain::self_revision::SelfRevisionProposal {
        agent_llm_mm::domain::self_revision::SelfRevisionProposal {
            should_reflect: true,
            rationale: "conflict-backed identity rewrite".to_string(),
            machine_patch: agent_llm_mm::domain::self_revision::SelfRevisionPatch {
                identity_patch: Some(
                    agent_llm_mm::domain::self_revision::SelfRevisionIdentityPatch::new(vec![
                        "identity:self=principal_architect".to_string(),
                    ]),
                ),
                commitment_patch: None,
            },
            proposed_evidence_event_ids: Vec::new(),
            proposed_evidence_query: None,
            confidence: None,
        }
    }

    pub fn identity_and_commitment_auto_reflection_proposal()
    -> agent_llm_mm::domain::self_revision::SelfRevisionProposal {
        let mut proposal = identity_only_auto_reflection_proposal();
        proposal.machine_patch.commitment_patch = Some(
            agent_llm_mm::domain::self_revision::SelfRevisionCommitmentPatch::new(vec![
                "prefer:evidence_backed_identity_updates".to_string(),
            ]),
        );
        proposal
    }
}

#[derive(Clone)]
struct FailureModeDeps {
    state: Arc<Mutex<State>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailPoint {
    CommitReflection,
    AppendHandledTriggerLedger,
}

#[derive(Clone)]
struct State {
    committed: CommittedState,
    now: DateTime<Utc>,
    next_id: usize,
    fail_point: Option<FailPoint>,
    model_calls: Vec<ModelInput>,
    self_revision_proposal: agent_llm_mm::domain::self_revision::SelfRevisionProposal,
    evidence_query_trace: Vec<EvidenceQuery>,
}

#[derive(Clone)]
struct CommittedState {
    self_model_versions: Vec<SelfModelVersion>,
    receipts: Vec<(String, agent_llm_mm::ports::StoredWriteReceipt)>,
    claims: Vec<StoredClaim>,
    commitments: Vec<Commitment>,
    identity: IdentityCore,
    event_references: Vec<String>,
    episode_references: Vec<String>,
    episode_events: Vec<(String, String)>,
    reflections: Vec<StoredReflection>,
    trigger_ledger: Vec<StoredTriggerLedgerEntry>,
    evidence_links: Vec<(String, String)>,
    events: Vec<StoredEvent>,
}

#[derive(Default)]
struct PendingIngest {
    receipts: Vec<(String, agent_llm_mm::ports::StoredWriteReceipt)>,
    claims: Vec<StoredClaim>,
    evidence_links: Vec<(String, String)>,
    events: Vec<StoredEvent>,
}

#[derive(Default)]
struct PendingReflection {
    self_model_versions: Vec<SelfModelVersion>,
    expected_self_model_version: Option<u64>,
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
                self_model_versions: Vec::new(),
                receipts: Vec::new(),
                claims: vec![StoredClaim::new(
                    "claim-conflict".to_string(),
                    ClaimDraft::new(Owner::Self_, "self.role", "is", "architect", Mode::Observed),
                    ClaimStatus::Active,
                )],
                commitments: vec![Commitment::new(
                    Owner::Self_,
                    "forbid:write_identity_core_directly",
                )],
                identity: IdentityCore::new(vec!["identity:self=architect".to_string()]),
                event_references: vec![
                    "event:recent-duplicate".to_string(),
                    "event:recent-duplicate".to_string(),
                    "event:recent-duplicate".to_string(),
                    "event:anchor".to_string(),
                    "event:baseline".to_string(),
                ],
                episode_references: vec!["episode:failure-modes".to_string()],
                episode_events: vec![(
                    "episode:failure-modes".to_string(),
                    "evt-reflection-1".to_string(),
                )],
                reflections: Vec::new(),
                trigger_ledger: Vec::new(),
                evidence_links: Vec::new(),
                events: vec![StoredEvent::new(
                    "evt-reflection-1".to_string(),
                    chrono::DateTime::parse_from_rfc3339("2026-03-23T10:01:00Z")
                        .unwrap()
                        .with_timezone(&Utc),
                    Event::new(Owner::World, EventKind::Observation, "evt-reflection-1"),
                )],
            },
            now: chrono::DateTime::parse_from_rfc3339("2026-03-23T10:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            next_id: 1,
            fail_point: None,
            model_calls: Vec::new(),
            self_revision_proposal:
                agent_llm_mm::domain::self_revision::SelfRevisionProposal::no_revision(
                    "mock model did not detect a valid Failure revision".to_string(),
                ),
            evidence_query_trace: Vec::new(),
        }
    }
}

fn identity_support_events_for_claims(
    claims: &[StoredClaim],
    recorded_at: DateTime<Utc>,
) -> Vec<StoredEvent> {
    claims
        .iter()
        .enumerate()
        .map(|(index, claim)| {
            let scope = MemoryScope::for_namespace(claim.claim.namespace().clone());
            StoredEvent::new(
                format!("identity-support-event-{index}"),
                recorded_at,
                Event::new_with_namespace(
                    scope
                        .owner()
                        .expect("namespace-derived memory scope is explicit"),
                    claim.claim.namespace().clone(),
                    EventKind::Observation,
                    format!("identity support event {index}"),
                )
                .expect("claim namespace already matches a legal owner"),
            )
        })
        .collect()
}

impl FailureModeDeps {
    fn new(mut state: State) -> Self {
        // Seed the fixture baseline exactly once, after all intentional fixture setup.
        // Subsequent direct projection changes must be detected as drift.
        assert!(state.committed.self_model_versions.is_empty());
        state.committed.self_model_versions.push(SelfModelVersion {
            version: 0,
            previous_version: None,
            kind: SelfModelVersionKind::InitializationBaseline,
            reflection_id: None,
            recorded_at: state.now,
            effective_at: Some(state.now),
            identity: state.committed.identity.clone(),
            commitments: state.committed.commitments.clone(),
            identity_written: false,
            commitments_written: false,
            identity_source_version: 0,
            commitment_source_version: 0,
            rollback_target_version: None,
        });
        Self {
            state: Arc::new(Mutex::new(state)),
        }
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

    fn claim_count(&self) -> usize {
        self.state.lock().unwrap().committed.claims.len()
    }

    fn identity(&self) -> IdentityCore {
        self.state.lock().unwrap().committed.identity.clone()
    }

    fn commitments(&self) -> Vec<Commitment> {
        self.state.lock().unwrap().committed.commitments.clone()
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

    fn latest_reflection(&self) -> Option<StoredReflection> {
        self.state
            .lock()
            .unwrap()
            .committed
            .reflections
            .last()
            .cloned()
    }

    fn evidence_links(&self) -> Vec<(String, String)> {
        self.state.lock().unwrap().committed.evidence_links.clone()
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

    fn latest_trigger_entry(&self) -> Option<StoredTriggerLedgerEntry> {
        self.state
            .lock()
            .unwrap()
            .committed
            .trigger_ledger
            .last()
            .cloned()
    }

    fn trigger_entries(&self) -> Vec<StoredTriggerLedgerEntry> {
        self.state.lock().unwrap().committed.trigger_ledger.clone()
    }

    fn evidence_query_trace(&self) -> Vec<EvidenceQuery> {
        self.state.lock().unwrap().evidence_query_trace.clone()
    }

    fn clear_fail_point(&self) {
        self.state.lock().unwrap().fail_point = None;
    }

    fn set_self_revision_proposal(
        &self,
        proposal: agent_llm_mm::domain::self_revision::SelfRevisionProposal,
    ) {
        self.state.lock().unwrap().self_revision_proposal = proposal;
    }

    fn seed_identity_support_context(
        &self,
        episode_references: Vec<String>,
        claims: Vec<StoredClaim>,
    ) {
        let evidence_links = claims
            .iter()
            .enumerate()
            .map(|(index, claim)| {
                (
                    claim.claim_id.clone(),
                    format!("identity-support-event-{index}"),
                )
            })
            .collect::<Vec<_>>();
        let episode_events = if episode_references.is_empty() {
            Vec::new()
        } else {
            evidence_links
                .iter()
                .enumerate()
                .map(|(index, (_, event_id))| {
                    (
                        episode_references[index % episode_references.len()].clone(),
                        event_id.clone(),
                    )
                })
                .collect()
        };
        let mut state = self.state.lock().unwrap();
        let recorded_at = state.now;
        let evidence_events = identity_support_events_for_claims(&claims, recorded_at);
        state
            .committed
            .events
            .retain(|event| !event.event_id.starts_with("identity-support-event-"));
        state.committed.events.extend(evidence_events);
        state.committed.episode_references = episode_references;
        state.committed.episode_events = episode_events;
        state.committed.claims = claims;
        state.committed.evidence_links = evidence_links;
    }

    fn seed_identity_support_context_with_cross_scope_evidence_links(&self) {
        let world_claims = (1..=3)
            .map(|index| {
                StoredClaim::new(
                    format!("claim-supporting-{index}"),
                    ClaimDraft::new(
                        Owner::World,
                        "self.role",
                        "is",
                        "principal_architect",
                        Mode::Observed,
                    )
                    .with_namespace(Namespace::world()),
                    ClaimStatus::Active,
                )
            })
            .collect::<Vec<_>>();
        let foreign_namespace = Namespace::for_project("other");
        let mut state = self.state.lock().unwrap();
        let recorded_at = state.now;
        let in_scope_event = StoredEvent::new(
            "identity-support-event-0".to_string(),
            recorded_at,
            Event::new(
                Owner::World,
                EventKind::Observation,
                "in-scope identity support event",
            ),
        );
        let foreign_events = (1..=2)
            .map(|index| {
                StoredEvent::new(
                    format!("identity-support-event-{index}"),
                    recorded_at,
                    Event::new_with_namespace(
                        Owner::World,
                        foreign_namespace.clone(),
                        EventKind::Observation,
                        format!("foreign identity support event {index}"),
                    )
                    .unwrap(),
                )
            })
            .collect::<Vec<_>>();
        state
            .committed
            .events
            .retain(|event| !event.event_id.starts_with("identity-support-event-"));
        state.committed.events.push(in_scope_event);
        state.committed.events.extend(foreign_events);
        state.committed.claims = world_claims;
        state.committed.evidence_links = vec![
            (
                "claim-supporting-1".to_string(),
                "identity-support-event-0".to_string(),
            ),
            (
                "claim-supporting-2".to_string(),
                "identity-support-event-1".to_string(),
            ),
            (
                "claim-supporting-3".to_string(),
                "identity-support-event-2".to_string(),
            ),
        ];
        state.committed.episode_references = vec![
            "episode:world-a".to_string(),
            "episode:foreign-a".to_string(),
            "episode:foreign-b".to_string(),
        ];
        state.committed.episode_events = vec![
            (
                "episode:world-a".to_string(),
                "identity-support-event-0".to_string(),
            ),
            (
                "episode:foreign-a".to_string(),
                "identity-support-event-1".to_string(),
            ),
            (
                "episode:foreign-b".to_string(),
                "identity-support-event-2".to_string(),
            ),
        ];
    }

    fn seed_identity_support_context_without_provenance(
        &self,
        episode_references: Vec<String>,
        claims: Vec<StoredClaim>,
    ) {
        let mut state = self.state.lock().unwrap();
        state.committed.episode_references = episode_references;
        state.committed.episode_events.clear();
        state.committed.claims = claims;
        state.committed.evidence_links.clear();
    }

    fn seed_failure_window(&self, events: Vec<(&str, &str)>) {
        let mut state = self.state.lock().unwrap();
        state.committed.events = events
            .into_iter()
            .enumerate()
            .map(|(index, (event_id, summary))| {
                StoredEvent::new(
                    event_id.to_string(),
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

    fn seed_events(&self, events: Vec<StoredEvent>) {
        let mut event_references = events
            .iter()
            .map(|event| (event.recorded_at, format!("event:{}", event.event_id)))
            .collect::<Vec<_>>();
        event_references.sort_by(|lhs, rhs| rhs.0.cmp(&lhs.0).then_with(|| rhs.1.cmp(&lhs.1)));
        let event_references = event_references
            .into_iter()
            .map(|(_, event_reference)| event_reference)
            .collect();
        let mut state = self.state.lock().unwrap();
        state.committed.events = events;
        state.committed.event_references = event_references;
    }

    fn seed_project_reflection_event(&self) {
        self.seed_events(vec![StoredEvent::new(
            "evt-reflection-1".to_string(),
            chrono::DateTime::parse_from_rfc3339("2026-03-23T10:01:00Z")
                .unwrap()
                .with_timezone(&Utc),
            Event::new_with_namespace(
                Owner::World,
                Namespace::for_project("agent-llm-mm"),
                EventKind::Observation,
                "evt-reflection-1",
            )
            .unwrap(),
        )]);
    }

    fn advance_now_by_hours(&self, hours: i64) {
        let mut state = self.state.lock().unwrap();
        state.now += chrono::Duration::hours(hours);
    }

    fn seed_periodic_cooldown(&self, trigger_key: &str) {
        self.seed_project_reflection_event();
        self.state
            .lock()
            .unwrap()
            .committed
            .trigger_ledger
            .push(StoredTriggerLedgerEntry {
                ledger_id: "ledger-seeded-periodic".to_string(),
                trigger_type: TriggerType::Periodic,
                namespace: Namespace::for_project("agent-llm-mm"),
                trigger_key: trigger_key.to_string(),
                status: TriggerLedgerStatus::Handled,
                evidence_window: vec!["evt-reflection-1".to_string()],
                handled_at: Some(
                    chrono::DateTime::parse_from_rfc3339("2026-03-23T09:50:00Z")
                        .unwrap()
                        .with_timezone(&Utc),
                ),
                cooldown_until: Some(
                    chrono::DateTime::parse_from_rfc3339("2026-03-24T09:50:00Z")
                        .unwrap()
                        .with_timezone(&Utc),
                ),
                episode_watermark: Some(1),
                reflection_id: Some("seeded-reflection".to_string()),
            });
    }

    fn seed_periodic_watermark_suppression(&self, trigger_key: &str) {
        self.seed_project_reflection_event();
        self.state
            .lock()
            .unwrap()
            .committed
            .trigger_ledger
            .push(StoredTriggerLedgerEntry {
                ledger_id: "ledger-seeded-periodic-watermark".to_string(),
                trigger_type: TriggerType::Periodic,
                namespace: Namespace::for_project("agent-llm-mm"),
                trigger_key: trigger_key.to_string(),
                status: TriggerLedgerStatus::Handled,
                evidence_window: vec!["evt-unrelated".to_string()],
                handled_at: Some(
                    chrono::DateTime::parse_from_rfc3339("2026-03-22T09:50:00Z")
                        .unwrap()
                        .with_timezone(&Utc),
                ),
                cooldown_until: Some(
                    chrono::DateTime::parse_from_rfc3339("2026-03-23T09:30:00Z")
                        .unwrap()
                        .with_timezone(&Utc),
                ),
                episode_watermark: Some(1),
                reflection_id: Some("seeded-reflection".to_string()),
            });
    }
}

fn upsert_claim(target: &mut Vec<StoredClaim>, claim: StoredClaim) {
    if let Some(existing) = target
        .iter_mut()
        .find(|stored_claim| stored_claim.claim_id == claim.claim_id)
    {
        *existing = claim;
    } else {
        target.push(claim);
    }
}

#[async_trait]
impl Clock for FailureModeDeps {
    async fn now(&self) -> Result<DateTime<Utc>, AppError> {
        Ok(self.state.lock().unwrap().now)
    }
}

#[async_trait]
impl IdGenerator for FailureModeDeps {
    async fn next_id(&self) -> Result<String, AppError> {
        let mut state = self.state.lock().unwrap();
        let next_id = format!("id-{}", state.next_id);
        state.next_id += 1;
        Ok(next_id)
    }
}

#[async_trait]
impl ModelPort for FailureModeDeps {
    async fn decide(&self, input: ModelInput) -> Result<ModelDecision, AppError> {
        self.state.lock().unwrap().model_calls.push(input);
        Ok(ModelDecision::new("Proceed".to_string()))
    }

    async fn propose_self_revision(
        &self,
        request: agent_llm_mm::domain::self_revision::SelfRevisionRequest,
    ) -> Result<agent_llm_mm::domain::self_revision::SelfRevisionProposal, AppError> {
        let state = self.state.lock().unwrap();
        if state.self_revision_proposal.should_reflect {
            return Ok(state.self_revision_proposal.clone());
        }

        Ok(
            agent_llm_mm::domain::self_revision::SelfRevisionProposal::no_revision(format!(
                "mock model did not detect a valid {:?} revision",
                request.trigger_type
            )),
        )
    }
}

#[async_trait]
impl IdentityStore for FailureModeDeps {
    async fn load_identity(&self) -> Result<IdentityCore, AppError> {
        Ok(self.state.lock().unwrap().committed.identity.clone())
    }

    async fn save_identity(&self, identity: IdentityCore) -> Result<(), AppError> {
        self.state.lock().unwrap().committed.identity = identity;
        Ok(())
    }
}

#[async_trait]
impl CommitmentStore for FailureModeDeps {
    async fn list_commitments(&self) -> Result<Vec<Commitment>, AppError> {
        Ok(self.state.lock().unwrap().committed.commitments.clone())
    }
}

#[async_trait]
impl EventStore for FailureModeDeps {
    async fn append_event(&self, event: StoredEvent) -> Result<(), AppError> {
        self.state.lock().unwrap().committed.events.push(event);
        Ok(())
    }

    async fn list_event_references(&self) -> Result<Vec<String>, AppError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .committed
            .event_references
            .clone())
    }

    async fn list_recorded_at_for_snapshot_manifest(
        &self,
        scope: &MemoryScope,
        evidence_manifest: &[agent_llm_mm::domain::event::EventReference],
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
            query.event_id_prefix,
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
            query.event_id_prefix,
        );

        let events = if let Some(limit) = query.limit {
            events.into_iter().take(limit).collect()
        } else {
            events
        };

        Ok(events.into_iter().map(|event| event.event_id).collect())
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
}

fn filter_and_order_events(
    events: &mut Vec<StoredEvent>,
    namespace: Option<Namespace>,
    owner: Option<Owner>,
    kind: Option<EventKind>,
    recorded_after: Option<DateTime<Utc>>,
    recorded_before: Option<DateTime<Utc>>,
    event_id_prefix: Option<String>,
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

    if let Some(prefix) = event_id_prefix {
        events.retain(|event| event.event_id.starts_with(&prefix));
    }

    events.sort_by(|lhs, rhs| {
        rhs.recorded_at
            .cmp(&lhs.recorded_at)
            .then_with(|| rhs.event_id.cmp(&lhs.event_id))
    });
}

#[async_trait]
impl EpisodeStore for FailureModeDeps {
    async fn record_event_in_episode(
        &self,
        episode_reference: String,
        event_id: String,
    ) -> Result<(), AppError> {
        let mut state = self.state.lock().unwrap();
        state
            .committed
            .episode_references
            .push(episode_reference.clone());
        state
            .committed
            .episode_events
            .push((episode_reference, event_id));
        Ok(())
    }

    async fn list_episode_references(&self) -> Result<Vec<String>, AppError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .committed
            .episode_references
            .clone())
    }

    async fn list_episode_references_supporting_claims(
        &self,
        scope: &MemoryScope,
        claim_ids: &[String],
    ) -> Result<Vec<String>, AppError> {
        let (Some(owner), Some(namespace)) = (scope.owner(), scope.namespace()) else {
            return Err(AppError::InvalidParams(
                "claim-to-evidence-to-episode lookup requires an explicit namespace".to_string(),
            ));
        };
        let state = self.state.lock().unwrap();
        let scoped_claim_ids = state
            .committed
            .claims
            .iter()
            .filter(|claim| {
                claim_ids.contains(&claim.claim_id)
                    && claim.claim.owner() == owner
                    && claim.claim.namespace() == namespace
            })
            .map(|claim| claim.claim_id.clone())
            .collect::<Vec<_>>();
        let linked_event_ids = state
            .committed
            .evidence_links
            .iter()
            .filter(|(claim_id, event_id)| {
                scoped_claim_ids.contains(claim_id)
                    && state.committed.events.iter().any(|event| {
                        event.event_id == *event_id
                            && event.event.owner() == owner
                            && event.event.namespace() == namespace
                    })
            })
            .map(|(_, event_id)| event_id.clone())
            .collect::<Vec<_>>();
        Ok(state
            .committed
            .episode_events
            .iter()
            .filter(|(_, event_id)| linked_event_ids.contains(event_id))
            .fold(Vec::new(), |mut episodes, (episode_reference, _)| {
                if !episodes.contains(episode_reference) {
                    episodes.push(episode_reference.clone());
                }
                episodes
            }))
    }

    async fn list_episode_references_in_scope(
        &self,
        _scope: &MemoryScope,
    ) -> Result<Vec<String>, AppError> {
        self.list_episode_references().await
    }

    async fn list_episode_references_for_snapshot(
        &self,
        scope: &MemoryScope,
        time_window: &SnapshotTimeWindow,
    ) -> Result<Vec<String>, AppError> {
        time_window.validate().map_err(AppError::from)?;
        let state = self.state.lock().unwrap();
        let has_qualifying_event = state.committed.events.iter().any(|event| {
            scope
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
                    .is_none_or(|before| event.recorded_at <= before)
        });
        Ok(if has_qualifying_event {
            state.committed.episode_references.clone()
        } else {
            Vec::new()
        })
    }
}

#[async_trait]
impl ClaimStore for FailureModeDeps {
    async fn upsert_claim(&self, claim: StoredClaim) -> Result<(), AppError> {
        upsert_claim(&mut self.state.lock().unwrap().committed.claims, claim);
        Ok(())
    }

    async fn link_evidence(&self, claim_id: String, event_id: String) -> Result<(), AppError> {
        self.state
            .lock()
            .unwrap()
            .committed
            .evidence_links
            .push((claim_id, event_id));
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
        let claim = state
            .committed
            .claims
            .iter_mut()
            .find(|claim| claim.claim_id == claim_id)
            .ok_or_else(|| AppError::Message(format!("missing claim: {claim_id}")))?;
        claim.status = status;
        Ok(())
    }
}

#[async_trait]
impl IngestTransactionRunner for FailureModeDeps {
    async fn begin_ingest_transaction(
        &self,
    ) -> Result<Box<dyn IngestTransaction + Send + '_>, AppError> {
        Ok(Box::new(FailureModeIngestTransaction {
            deps: self.clone(),
            pending: PendingIngest::default(),
        }))
    }
}

#[async_trait]
impl ReflectionTransactionRunner for FailureModeDeps {
    async fn begin_reflection_transaction(
        &self,
    ) -> Result<Box<dyn ReflectionTransaction + Send + '_>, AppError> {
        Ok(Box::new(FailureModeReflectionTransaction {
            deps: self.clone(),
            pending: PendingReflection::default(),
        }))
    }
}

#[async_trait]
impl TriggerLedgerStore for FailureModeDeps {
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

struct FailureModeIngestTransaction {
    deps: FailureModeDeps,
    pending: PendingIngest,
}

#[async_trait]
impl IngestTransaction for FailureModeIngestTransaction {
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
        self.pending.events.push(event);
        Ok(())
    }

    async fn record_event_in_episode(
        &mut self,
        _episode_reference: String,
        _event_id: String,
    ) -> Result<(), AppError> {
        Ok(())
    }

    async fn upsert_claim(&mut self, claim: StoredClaim) -> Result<(), AppError> {
        self.pending.claims.push(claim);
        Ok(())
    }

    async fn link_evidence(&mut self, claim_id: String, event_id: String) -> Result<(), AppError> {
        self.pending.evidence_links.push((claim_id, event_id));
        Ok(())
    }

    async fn commit(self: Box<Self>) -> Result<(), AppError> {
        let mut state = self.deps.state.lock().unwrap();
        state.committed.receipts.extend(self.pending.receipts);
        state.committed.events.extend(self.pending.events);
        state
            .committed
            .evidence_links
            .extend(self.pending.evidence_links);
        for claim in self.pending.claims {
            upsert_claim(&mut state.committed.claims, claim);
        }
        Ok(())
    }
}

struct FailureModeReflectionTransaction {
    deps: FailureModeDeps,
    pending: PendingReflection,
}

#[async_trait]
impl ReflectionTransaction for FailureModeReflectionTransaction {
    async fn load_current_self_model_version(&mut self) -> Result<SelfModelVersion, AppError> {
        let head = self
            .pending
            .self_model_versions
            .last()
            .cloned()
            .or_else(|| {
                self.deps
                    .state
                    .lock()
                    .unwrap()
                    .committed
                    .self_model_versions
                    .last()
                    .cloned()
            })
            .ok_or_else(|| AppError::Message("missing fixture self-model baseline".into()))?;
        if self.load_identity().await? != head.identity
            || self.load_commitments().await? != head.commitments
        {
            return Err(AppError::Message("self-model projection drift".into()));
        }
        Ok(head)
    }
    async fn load_self_model_version(
        &mut self,
        version: u64,
    ) -> Result<Option<SelfModelVersion>, AppError> {
        Ok(self
            .pending
            .self_model_versions
            .iter()
            .find(|entry| entry.version == version)
            .cloned()
            .or_else(|| {
                self.deps
                    .state
                    .lock()
                    .unwrap()
                    .committed
                    .self_model_versions
                    .iter()
                    .find(|entry| entry.version == version)
                    .cloned()
            }))
    }
    async fn load_self_model_reflection(
        &mut self,
        reflection_id: &str,
    ) -> Result<Option<StoredReflection>, AppError> {
        Ok(self
            .pending
            .reflections
            .iter()
            .find(|entry| entry.reflection_id == reflection_id)
            .cloned()
            .or_else(|| {
                self.deps
                    .state
                    .lock()
                    .unwrap()
                    .committed
                    .reflections
                    .iter()
                    .find(|entry| entry.reflection_id == reflection_id)
                    .cloned()
            }))
    }
    async fn append_self_model_version(
        &mut self,
        expected_version: u64,
        version: SelfModelVersion,
    ) -> Result<(), AppError> {
        let head = self
            .pending
            .self_model_versions
            .last()
            .cloned()
            .or_else(|| {
                self.deps
                    .state
                    .lock()
                    .unwrap()
                    .committed
                    .self_model_versions
                    .last()
                    .cloned()
            })
            .ok_or_else(|| AppError::Message("missing fixture self-model baseline".into()))?;
        if head.version != expected_version
            || version.version != expected_version + 1
            || version.previous_version != Some(expected_version)
        {
            return Err(AppError::InvalidParams("stale self-model version".into()));
        }
        self.pending
            .expected_self_model_version
            .get_or_insert(expected_version);
        self.pending.self_model_versions.push(version);
        Ok(())
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
        if self.deps.state.lock().unwrap().fail_point == Some(FailPoint::AppendHandledTriggerLedger)
            && entry.status == TriggerLedgerStatus::Handled
        {
            return Err(AppError::Message(
                "injected handled trigger ledger failure".to_string(),
            ));
        }

        self.pending.trigger_ledger.push(entry);
        Ok(())
    }

    async fn load_identity(&mut self) -> Result<IdentityCore, AppError> {
        if let Some(identity) = &self.pending.identity {
            return Ok(identity.clone());
        }

        Ok(self.deps.state.lock().unwrap().committed.identity.clone())
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
        if self.deps.state.lock().unwrap().fail_point == Some(FailPoint::CommitReflection) {
            return Err(AppError::Message(
                "injected reflection commit failure".to_string(),
            ));
        }
        let mut state = self.deps.state.lock().unwrap();
        if let Some(expected) = self.pending.expected_self_model_version
            && state
                .committed
                .self_model_versions
                .last()
                .map(|head| head.version)
                != Some(expected)
        {
            return Err(AppError::InvalidParams(
                "stale self-model version at commit".into(),
            ));
        }
        state
            .committed
            .self_model_versions
            .extend(self.pending.self_model_versions);
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
            state.committed.identity = identity;
        }
        if let Some(commitments) = self.pending.commitments {
            state.committed.commitments = commitments;
        }
        for (claim_id, status) in self.pending.status_updates {
            let claim = state
                .committed
                .claims
                .iter_mut()
                .find(|claim| claim.claim_id == claim_id)
                .ok_or_else(|| AppError::Message(format!("missing claim: {claim_id}")))?;
            claim.status = status;
        }
        Ok(())
    }
}
