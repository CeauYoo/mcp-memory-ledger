use chrono::Utc;

use crate::{
    domain::{
        self_revision::{
            AutoReflectDiagnosticInput, AutoReflectDiagnosticSummary, AutoReflectOutcome,
            SelfRevisionRequest, SuppressionCategory, TriggerType,
        },
        types::Namespace,
    },
    error::AppError,
    ports::{
        ClaimStore, Clock, CommitmentStore, EpisodeStore, EventStore, IdGenerator, IdentityStore,
        ModelPort, ReflectionTransactionRunner, StoredTriggerLedgerEntry, TriggerLedgerStatus,
        TriggerLedgerStore,
    },
};

const FAILURE_TRIGGER_THRESHOLD: usize = 2;
const AUTO_REFLECTION_COOLDOWN_HOURS: i64 = 24;
const DEFAULT_SNAPSHOT_BUDGET: usize = 3;

mod candidate;
mod commit;
mod evidence;
mod policy;

use candidate::{
    TriggerCandidate, build_revision_snapshot, canonical_trigger_key, detect_trigger_candidate,
};
use commit::{
    apply_validated_self_revision, record_rejected_trigger, record_rejection_then_propagate,
    record_suppressed_trigger,
};
use evidence::resolve_governed_evidence_window;
use policy::{evaluate_trigger_suppression, validate_self_revision};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum RecursionGuard {
    #[default]
    Allow,
    SkipAutoReflection,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AutoReflectInput {
    pub namespace: Namespace,
    pub trigger_type: TriggerType,
    #[serde(default)]
    pub trigger_hints: Vec<String>,
    #[serde(default)]
    pub recursion_guard: RecursionGuard,
}

impl AutoReflectInput {
    pub fn for_failure(namespace: Namespace, trigger_hints: Vec<String>) -> Self {
        Self::new(namespace, TriggerType::Failure, trigger_hints)
    }

    pub fn for_conflict(namespace: Namespace, trigger_hints: Vec<String>) -> Self {
        Self::new(namespace, TriggerType::Conflict, trigger_hints)
    }

    pub fn for_periodic(namespace: Namespace, trigger_hints: Vec<String>) -> Self {
        Self::new(namespace, TriggerType::Periodic, trigger_hints)
    }

    pub fn new(
        namespace: Namespace,
        trigger_type: TriggerType,
        trigger_hints: Vec<String>,
    ) -> Self {
        Self {
            namespace,
            trigger_type,
            trigger_hints,
            recursion_guard: RecursionGuard::Allow,
        }
    }

    pub fn with_recursion_guard(mut self, recursion_guard: RecursionGuard) -> Self {
        self.recursion_guard = recursion_guard;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AutoReflectResult {
    pub triggered: bool,
    pub trigger_type: Option<TriggerType>,
    pub namespace: Option<String>,
    pub reflection_id: Option<String>,
    pub ledger_status: Option<TriggerLedgerStatus>,
    pub reason: Option<String>,
    pub trigger_key: Option<String>,
    #[serde(default)]
    pub evidence_event_ids: Vec<String>,
    pub cooldown_until: Option<chrono::DateTime<Utc>>,
    pub suppression_reason: Option<String>,
    pub diagnostics: AutoReflectDiagnosticSummary,
}

impl AutoReflectResult {
    fn skipped(input: &AutoReflectInput, reason: impl Into<String>) -> Self {
        let reason = reason.into();
        let evidence_event_ids = Vec::new();
        Self {
            triggered: false,
            trigger_type: Some(input.trigger_type),
            namespace: Some(input.namespace.as_str().to_string()),
            reflection_id: None,
            ledger_status: None,
            reason: Some(reason.clone()),
            trigger_key: Some(input.trigger_key()),
            evidence_event_ids: evidence_event_ids.clone(),
            cooldown_until: None,
            suppression_reason: None,
            diagnostics: AutoReflectDiagnosticSummary::new(AutoReflectDiagnosticInput {
                trigger_type: input.trigger_type,
                namespace: input.namespace.as_str().to_string(),
                trigger_key: input.trigger_key(),
                outcome: AutoReflectOutcome::Skipped,
                suppression_reason: None,
                suppression_category: None,
                rejection_reason: None,
                cooldown_boundary: None,
                evidence_window_size: 0,
                selected_evidence_event_ids: evidence_event_ids,
            }),
        }
    }

    fn not_triggered(candidate: &TriggerCandidate) -> Self {
        let evidence_event_ids = candidate.evidence_event_ids.clone();
        Self {
            triggered: false,
            trigger_type: Some(candidate.trigger_type),
            namespace: Some(candidate.namespace.as_str().to_string()),
            reflection_id: None,
            ledger_status: None,
            reason: None,
            trigger_key: Some(candidate.trigger_key.clone()),
            evidence_event_ids: evidence_event_ids.clone(),
            cooldown_until: None,
            suppression_reason: None,
            diagnostics: AutoReflectDiagnosticSummary::new(AutoReflectDiagnosticInput {
                trigger_type: candidate.trigger_type,
                namespace: candidate.namespace.as_str().to_string(),
                trigger_key: candidate.trigger_key.clone(),
                outcome: AutoReflectOutcome::NotTriggered,
                suppression_reason: None,
                suppression_category: None,
                rejection_reason: None,
                cooldown_boundary: None,
                evidence_window_size: evidence_event_ids.len(),
                selected_evidence_event_ids: Vec::new(),
            }),
        }
    }

    fn rejected(candidate: &TriggerCandidate, reason: impl Into<String>) -> Self {
        let reason = reason.into();
        let evidence_event_ids = candidate.evidence_event_ids.clone();
        Self {
            triggered: false,
            trigger_type: Some(candidate.trigger_type),
            namespace: Some(candidate.namespace.as_str().to_string()),
            reflection_id: None,
            ledger_status: Some(TriggerLedgerStatus::Rejected),
            reason: Some(reason.clone()),
            trigger_key: Some(candidate.trigger_key.clone()),
            evidence_event_ids: evidence_event_ids.clone(),
            cooldown_until: None,
            suppression_reason: None,
            diagnostics: AutoReflectDiagnosticSummary::new(AutoReflectDiagnosticInput {
                trigger_type: candidate.trigger_type,
                namespace: candidate.namespace.as_str().to_string(),
                trigger_key: candidate.trigger_key.clone(),
                outcome: AutoReflectOutcome::Rejected,
                suppression_reason: None,
                suppression_category: None,
                rejection_reason: Some(reason),
                cooldown_boundary: None,
                evidence_window_size: evidence_event_ids.len(),
                selected_evidence_event_ids: Vec::new(),
            }),
        }
    }

    fn suppressed(
        candidate: &TriggerCandidate,
        entry: &StoredTriggerLedgerEntry,
        suppression_category: SuppressionCategory,
    ) -> Self {
        let suppression_reason = suppression_category.as_str().to_string();
        let evidence_event_ids = candidate.evidence_event_ids.clone();
        Self {
            triggered: false,
            trigger_type: Some(candidate.trigger_type),
            namespace: Some(candidate.namespace.as_str().to_string()),
            reflection_id: entry.reflection_id.clone(),
            ledger_status: Some(entry.status),
            reason: None,
            trigger_key: Some(entry.trigger_key.clone()),
            evidence_event_ids: evidence_event_ids.clone(),
            cooldown_until: entry.cooldown_until,
            suppression_reason: Some(suppression_reason.clone()),
            diagnostics: AutoReflectDiagnosticSummary::new(AutoReflectDiagnosticInput {
                trigger_type: candidate.trigger_type,
                namespace: candidate.namespace.as_str().to_string(),
                trigger_key: entry.trigger_key.clone(),
                outcome: AutoReflectOutcome::Suppressed,
                suppression_reason: Some(suppression_reason),
                suppression_category: Some(suppression_category),
                rejection_reason: None,
                cooldown_boundary: entry.cooldown_until,
                evidence_window_size: evidence_event_ids.len(),
                selected_evidence_event_ids: Vec::new(),
            }),
        }
    }

    fn handled(
        candidate: &TriggerCandidate,
        evidence_event_ids: Vec<String>,
        reflection_id: String,
        cooldown_until: Option<chrono::DateTime<Utc>>,
    ) -> Self {
        let selected_evidence_event_ids = evidence_event_ids.clone();
        let evidence_window_size = candidate.evidence_event_ids.len();
        Self {
            triggered: true,
            trigger_type: Some(candidate.trigger_type),
            namespace: Some(candidate.namespace.as_str().to_string()),
            reflection_id: Some(reflection_id),
            ledger_status: Some(TriggerLedgerStatus::Handled),
            reason: None,
            trigger_key: Some(candidate.trigger_key.clone()),
            evidence_event_ids,
            cooldown_until,
            suppression_reason: None,
            diagnostics: AutoReflectDiagnosticSummary::new(AutoReflectDiagnosticInput {
                trigger_type: candidate.trigger_type,
                namespace: candidate.namespace.as_str().to_string(),
                trigger_key: candidate.trigger_key.clone(),
                outcome: AutoReflectOutcome::Handled,
                suppression_reason: None,
                suppression_category: None,
                rejection_reason: None,
                cooldown_boundary: cooldown_until,
                evidence_window_size,
                selected_evidence_event_ids,
            }),
        }
    }
}

pub async fn execute<D>(deps: &D, input: AutoReflectInput) -> Result<AutoReflectResult, AppError>
where
    D: TriggerLedgerStore
        + EventStore
        + ClaimStore
        + CommitmentStore
        + IdentityStore
        + EpisodeStore
        + ReflectionTransactionRunner
        + ModelPort
        + Clock
        + IdGenerator
        + Sync,
{
    if input.recursion_guard == RecursionGuard::SkipAutoReflection {
        return Ok(AutoReflectResult::skipped(
            &input,
            "recursion guard enabled",
        ));
    }

    let candidate = detect_trigger_candidate(deps, &input).await?;
    if !candidate.should_consider {
        return Ok(AutoReflectResult::not_triggered(&candidate));
    }

    if let Some(suppression) = evaluate_trigger_suppression(deps, &candidate).await? {
        let entry = record_suppressed_trigger(deps, &candidate).await?;
        return Ok(AutoReflectResult::suppressed(
            &candidate,
            &entry,
            suppression.reason,
        ));
    }

    let snapshot = build_revision_snapshot(deps, &candidate).await?;
    let proposal = deps
        .propose_self_revision(SelfRevisionRequest::new(
            candidate.trigger_type,
            candidate.namespace.clone(),
            snapshot,
            candidate.evidence_event_ids.clone(),
            candidate.trigger_hints.clone(),
        ))
        .await?;

    if !proposal.should_reflect {
        record_rejected_trigger(deps, &candidate, None).await?;
        return Ok(AutoReflectResult::rejected(&candidate, proposal.rationale));
    }

    let governed_evidence_event_ids = match resolve_governed_evidence_window(
        deps,
        &candidate.evidence_event_ids,
        &proposal,
    )
    .await
    {
        Ok(governed_evidence_event_ids) => governed_evidence_event_ids,
        Err(error) => return record_rejection_then_propagate(deps, &candidate, error).await,
    };

    let validated =
        match validate_self_revision(deps, &candidate, &proposal, &governed_evidence_event_ids)
            .await
        {
            Ok(validated) => validated,
            Err(error) => return record_rejection_then_propagate(deps, &candidate, error).await,
        };

    match apply_validated_self_revision(
        deps,
        &candidate,
        &proposal,
        &governed_evidence_event_ids,
        validated,
    )
    .await
    {
        Ok(result) => Ok(result),
        Err(error) => record_rejection_then_propagate(deps, &candidate, error).await,
    }
}

impl AutoReflectInput {
    pub fn trigger_key(&self) -> String {
        canonical_trigger_key(&self.namespace, self.trigger_type)
    }
}
