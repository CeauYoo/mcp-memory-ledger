use crate::{
    domain::{
        claim::ClaimDraft,
        commitment::Commitment,
        event::EventReference,
        identity_core::IdentityCore,
        reflection::{Reflection, ReflectionIdentityUpdate},
        rules::reflection_policy::{ReflectionDecision, ReflectionTrigger, classify_reflection},
        types::Owner,
    },
    error::AppError,
    ports::{
        ClaimStatus, Clock, EventStore, EvidenceQuery, IdGenerator, ReflectionTransactionRunner,
        StoredClaim, StoredReflection, StoredTriggerLedgerEntry,
    },
};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReflectionInput {
    reflection: Reflection,
    target_claim_id: Option<String>,
    replacement_claim: Option<ClaimDraft>,
    replacement_evidence_event_ids: Vec<String>,
    replacement_evidence_query: Option<EvidenceQuery>,
    identity_update: Option<ReflectionIdentityUpdate>,
    commitment_updates: Option<Vec<Commitment>>,
    handled_trigger_ledger_entry: Option<StoredTriggerLedgerEntry>,
    #[serde(default)]
    strict_evidence_scope: bool,
    #[serde(skip)]
    write_receipt: Option<crate::ports::WriteReceiptRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    feedback_candidate: Option<(crate::domain::types::Namespace, String)>,
}

impl ReflectionInput {
    pub fn new(
        reflection: Reflection,
        supersede_claim_id: impl Into<String>,
        replacement_claim: Option<ClaimDraft>,
        replacement_evidence_event_ids: Vec<String>,
    ) -> Self {
        Self {
            reflection,
            target_claim_id: Some(supersede_claim_id.into()),
            replacement_claim,
            replacement_evidence_event_ids,
            replacement_evidence_query: None,
            identity_update: None,
            commitment_updates: None,
            handled_trigger_ledger_entry: None,
            strict_evidence_scope: false,
            write_receipt: None,
            feedback_candidate: None,
        }
    }

    pub fn record_only(
        reflection: Reflection,
        replacement_evidence_event_ids: Vec<String>,
    ) -> Self {
        Self {
            reflection,
            target_claim_id: None,
            replacement_claim: None,
            replacement_evidence_event_ids,
            replacement_evidence_query: None,
            identity_update: None,
            commitment_updates: None,
            handled_trigger_ledger_entry: None,
            strict_evidence_scope: false,
            write_receipt: None,
            feedback_candidate: None,
        }
    }

    pub(crate) fn with_feedback_candidate(
        mut self,
        namespace: crate::domain::types::Namespace,
        candidate_id: String,
    ) -> Self {
        self.feedback_candidate = Some((namespace, candidate_id));
        self
    }

    pub(crate) fn with_write_receipt(mut self, request: crate::ports::WriteReceiptRequest) -> Self {
        self.write_receipt = Some(request);
        self
    }

    /// Scoped correction entrypoints opt into strict evidence isolation, including self memory.
    pub fn with_strict_evidence_scope(mut self) -> Self {
        self.strict_evidence_scope = true;
        self
    }

    pub fn with_replacement_evidence_query(
        mut self,
        replacement_evidence_query: EvidenceQuery,
    ) -> Self {
        self.replacement_evidence_query = Some(replacement_evidence_query);
        self
    }

    pub fn with_optional_replacement_evidence_query(
        mut self,
        replacement_evidence_query: Option<EvidenceQuery>,
    ) -> Self {
        self.replacement_evidence_query = replacement_evidence_query;
        self
    }

    pub fn with_identity_update(mut self, canonical_claims: Vec<String>) -> Self {
        self.identity_update = Some(ReflectionIdentityUpdate::new(canonical_claims));
        self
    }

    pub fn with_commitment_updates(mut self, commitments: Vec<Commitment>) -> Self {
        self.commitment_updates = Some(commitments);
        self
    }

    pub fn with_handled_trigger_ledger_entry(
        mut self,
        handled_trigger_ledger_entry: StoredTriggerLedgerEntry,
    ) -> Self {
        self.handled_trigger_ledger_entry = Some(handled_trigger_ledger_entry);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReflectionResult {
    pub reflection_id: String,
    pub replacement_claim_id: Option<String>,
}

pub async fn execute<D>(deps: &D, input: ReflectionInput) -> Result<ReflectionResult, AppError>
where
    D: ReflectionTransactionRunner + EventStore + IdGenerator + Clock + Sync,
{
    let receipt_payload =
        serde_json::to_value(&input).map_err(|e| AppError::Message(e.to_string()))?;
    let ReflectionInput {
        reflection,
        target_claim_id,
        replacement_claim,
        replacement_evidence_event_ids,
        replacement_evidence_query,
        identity_update,
        commitment_updates,
        handled_trigger_ledger_entry,
        strict_evidence_scope,
        write_receipt,
        feedback_candidate,
    } = input;
    let replacement_evidence_event_ids = normalize_event_ids(replacement_evidence_event_ids)?;

    let reflection_id = deps.next_id().await?;
    let recorded_at = deps.now().await?;
    let decision = classify_reflection(match (&target_claim_id, replacement_claim.as_ref()) {
        (Some(_), Some(_)) => ReflectionTrigger::Failure,
        (Some(_), None) => ReflectionTrigger::Conflict,
        (None, None) => ReflectionTrigger::Manual,
        (None, Some(_)) => {
            return Err(AppError::InvalidParams(
                "replacement claim reflections require a target claim id".to_string(),
            ));
        }
    });
    let requires_supporting_evidence =
        replacement_claim.is_some() || identity_update.is_some() || commitment_updates.is_some();
    let supporting_evidence_event_ids = if requires_supporting_evidence {
        resolve_evidence_event_ids(
            deps,
            replacement_evidence_query,
            replacement_evidence_event_ids,
        )
        .await?
    } else {
        Vec::new()
    };

    // Resolve optional query candidates before reserving the writer connection.
    // Their existence and allowed scope are revalidated below in this transaction.
    let mut transaction = deps.begin_reflection_transaction().await?;
    if let Some(request) = &write_receipt
        && let Some(receipt) = transaction
            .load_write_receipt(&request.operation_id)
            .await?
    {
        let result = receipt.replay(request)?;
        transaction.commit().await?;
        return Ok(result);
    }
    // Candidate lifecycle and Claim correction share this writer transaction.
    // Revalidation is mandatory even after an earlier successful validation.
    let mut guarded_candidate = if let Some((namespace, candidate_id)) = &feedback_candidate {
        let candidate = super::feedback_candidate::load_candidate(
            transaction.as_mut(),
            namespace,
            candidate_id,
        )
        .await?;
        if identity_update.is_some()
            || commitment_updates.is_some()
            || handled_trigger_ledger_entry.is_some()
            || target_claim_id.as_deref()
                != Some(candidate.proposal.target_claim_reference.claim_id())
            || replacement_claim.as_ref() != Some(&candidate.proposal.replacement_claim)
            || reflection.summary() != candidate.proposal.summary
            || supporting_evidence_event_ids
                != candidate
                    .proposal
                    .evidence_event_ids
                    .iter()
                    .map(|id| id.event_id().to_string())
                    .collect::<Vec<_>>()
        {
            return Err(AppError::InvalidParams("feedback candidate commit does not match its immutable proposal or attempts a global patch".into()));
        }
        use crate::domain::feedback_candidate::FeedbackCandidateState;
        if candidate.state == FeedbackCandidateState::Committed {
            let result = ReflectionResult {
                reflection_id: candidate.reflection_id.clone().ok_or_else(|| {
                    AppError::Message("committed candidate has no reflection".into())
                })?,
                replacement_claim_id: candidate.replacement_claim_id.clone(),
            };
            let request = write_receipt.as_ref().ok_or_else(|| {
                AppError::InvalidParams("feedback candidate commit requires request_id".into())
            })?;
            transaction
                .append_write_receipt(
                    request,
                    crate::ports::write_receipt::receipt_result(request, &result)?,
                    recorded_at,
                )
                .await?;
            transaction.commit().await?;
            return Ok(result);
        }
        if candidate.state != FeedbackCandidateState::Validated {
            return Err(AppError::InvalidParams("feedback candidate must be explicitly validated before commit; rejected candidates cannot commit".into()));
        }
        let validation = super::feedback_candidate::validate_in_transaction(
            transaction.as_mut(),
            &candidate.proposal,
        )
        .await?;
        if !validation.passed {
            return Err(AppError::InvalidParams(format!(
                "feedback candidate commit revalidation failed: {}",
                serde_json::to_string(&validation.reasons)
                    .map_err(|e| AppError::Message(e.to_string()))?
            )));
        }
        Some(candidate)
    } else {
        None
    };
    if (identity_update.is_some() || commitment_updates.is_some())
        && supporting_evidence_event_ids.is_empty()
    {
        return Err(AppError::InvalidParams(
            "identity or commitment reflection updates require at least one resolved evidence event id"
                .to_string(),
        ));
    }

    if identity_update
        .as_ref()
        .is_some_and(|update| update.canonical_claims.is_empty())
    {
        return Err(AppError::InvalidParams(
            "identity reflection updates must include at least one canonical claim".to_string(),
        ));
    }

    // 收紧服务端校验：identity 更新的任一 canonical claim 不得为纯空白串，
    // 避免 " " 这类空白内容绕过非空检查后写入 durable identity。
    if identity_update.as_ref().is_some_and(|update| {
        update
            .canonical_claims
            .iter()
            .any(|claim| claim.trim().is_empty())
    }) {
        return Err(AppError::InvalidParams(
            "identity reflection updates must not contain blank canonical claims".to_string(),
        ));
    }

    // 与 identity canonical claim 对称：commitment 更新的任一 description 不得为纯空白串，
    // 避免 " " 这类空白内容绕过非空检查后写入 durable commitments。
    if commitment_updates.as_ref().is_some_and(|updates| {
        updates
            .iter()
            .any(|commitment| commitment.description().trim().is_empty())
    }) {
        return Err(AppError::InvalidParams(
            "commitment reflection updates must not contain blank descriptions".to_string(),
        ));
    }

    // Authoritative validation belongs inside the write transaction. The scoped
    // preparation path is only a convenience and must not be a security boundary.
    let target = if let Some(target_claim_id) = &target_claim_id {
        let target = transaction
            .load_claim_for_reflection(target_claim_id)
            .await?
            .ok_or_else(|| {
                AppError::InvalidParams("reflection target claim does not exist".to_string())
            })?;
        if target.status == ClaimStatus::Superseded {
            return Err(AppError::InvalidParams(
                "reflection target claim is already superseded".to_string(),
            ));
        }
        if replacement_claim.as_ref().is_some_and(|replacement| {
            replacement.owner() != target.claim.owner()
                || replacement.namespace() != target.claim.namespace()
        }) {
            return Err(AppError::InvalidParams(
                "replacement claim must remain in the target claim scope".to_string(),
            ));
        }
        Some(target)
    } else {
        None
    };
    for event_id in &supporting_evidence_event_ids {
        let event = transaction
            .load_event_for_reflection(event_id)
            .await?
            .ok_or_else(|| {
                AppError::InvalidParams(format!(
                    "unknown replacement evidence event id: {event_id}"
                ))
            })?;
        if target.as_ref().is_some_and(|target| {
            let same_scope = event.event.owner() == target.claim.owner()
                && event.event.namespace() == target.claim.namespace();
            // Legacy self-governance may cite global world observations. Scoped
            // corrections never inherit that exception, and project evidence is isolated.
            let legacy_world_observation = !strict_evidence_scope
                && target.claim.owner() == Owner::Self_
                && event.event.owner() == Owner::World
                && event.event.namespace()
                    == &crate::domain::types::Namespace::for_owner(Owner::World);
            !same_scope && !legacy_world_observation
        }) {
            return Err(AppError::InvalidParams(
                "reflection evidence must remain in the target claim scope".to_string(),
            ));
        }
    }

    let commitment_updates = if let Some(commitment_updates) = commitment_updates {
        let existing_commitments = transaction.load_commitments().await?;
        Some(preserve_baseline_commitments(
            commitment_updates,
            existing_commitments,
        ))
    } else {
        None
    };
    let replacement_claim_id = match (decision, replacement_claim) {
        (ReflectionDecision::SupersedeWithReplacement, Some(claim)) => {
            claim.validate(supporting_evidence_event_ids.len())?;
            let claim_id = format!("{reflection_id}:replacement");
            transaction
                .upsert_claim(StoredClaim::new(
                    claim_id.clone(),
                    claim,
                    ClaimStatus::Active,
                ))
                .await?;
            for event_id in &supporting_evidence_event_ids {
                transaction
                    .link_evidence(claim_id.clone(), event_id.clone())
                    .await?;
            }
            Some(claim_id)
        }
        (ReflectionDecision::SupersedeWithReplacement, None) => {
            return Err(AppError::Message(
                "superseding reflections require a replacement claim".to_string(),
            ));
        }
        _ => None,
    };

    if let Some(identity_update) = &identity_update {
        let _ = transaction.load_identity().await?;
        transaction
            .replace_identity(IdentityCore::new(identity_update.canonical_claims.clone()))
            .await?;
    }

    if let Some(commitment_updates) = &commitment_updates {
        transaction
            .replace_commitments(commitment_updates.clone())
            .await?;
    }

    transaction
        .append_reflection(
            StoredReflection::new(
                reflection_id.clone(),
                recorded_at,
                reflection,
                target_claim_id.clone(),
                replacement_claim_id.clone(),
            )
            .with_supporting_evidence_event_ids(supporting_evidence_event_ids)
            .with_requested_identity_update(identity_update)
            .with_requested_commitment_updates(commitment_updates),
        )
        .await?;

    if let Some(mut handled_trigger_ledger_entry) = handled_trigger_ledger_entry {
        handled_trigger_ledger_entry.reflection_id = Some(reflection_id.clone());
        transaction
            .append_trigger_ledger(handled_trigger_ledger_entry)
            .await?;
    }

    match decision {
        ReflectionDecision::MarkDisputed => {
            let target_claim_id = target_claim_id.ok_or_else(|| {
                AppError::Message("disputing reflections require a target claim id".to_string())
            })?;
            transaction
                .compare_and_set_claim_status(
                    &target_claim_id,
                    target.as_ref().expect("validated target").status,
                    ClaimStatus::Disputed,
                )
                .await?;
        }
        ReflectionDecision::SupersedeWithReplacement => {
            let target_claim_id = target_claim_id.ok_or_else(|| {
                AppError::Message("superseding reflections require a target claim id".to_string())
            })?;
            transaction
                .compare_and_set_claim_status(
                    &target_claim_id,
                    target.as_ref().expect("validated target").status,
                    ClaimStatus::Superseded,
                )
                .await?;
        }
        ReflectionDecision::RecordOnly => {}
    }

    let result = ReflectionResult {
        reflection_id,
        replacement_claim_id,
    };
    if let Some(candidate) = &mut guarded_candidate {
        let previous_revision = candidate.revision;
        candidate.revision += 1;
        candidate.updated_at = recorded_at;
        candidate.state = crate::domain::feedback_candidate::FeedbackCandidateState::Committed;
        candidate.reflection_id = Some(result.reflection_id.clone());
        candidate.replacement_claim_id = result.replacement_claim_id.clone();
        transaction
            .update_feedback_candidate(candidate, previous_revision)
            .await?;
    }
    let request = match write_receipt {
        Some(request) => request,
        None => crate::ports::WriteReceiptRequest::new(
            "run_reflection",
            target
                .as_ref()
                .map(|target| target.claim.namespace().as_str())
                .unwrap_or("self"),
            &result.reflection_id,
            &receipt_payload,
        )?,
    };
    transaction
        .append_write_receipt(
            &request,
            crate::ports::write_receipt::receipt_result(&request, &result)?,
            recorded_at,
        )
        .await?;
    transaction.commit().await?;
    Ok(result)
}

fn preserve_baseline_commitments(
    mut requested_commitments: Vec<Commitment>,
    existing_commitments: Vec<Commitment>,
) -> Vec<Commitment> {
    for commitment in existing_commitments {
        if is_baseline_commitment(&commitment)
            && !requested_commitments
                .iter()
                .any(|candidate| candidate == &commitment)
        {
            requested_commitments.push(commitment);
        }
    }

    requested_commitments
}

fn is_baseline_commitment(commitment: &Commitment) -> bool {
    commitment.description() == "forbid:write_identity_core_directly"
}

async fn resolve_evidence_event_ids<D>(
    deps: &D,
    query: Option<EvidenceQuery>,
    explicit: Vec<String>,
) -> Result<Vec<String>, AppError>
where
    D: EventStore + Sync,
{
    let mut evidence_event_ids = explicit;

    if let Some(query) = query {
        let mut queried_ids = normalize_event_ids(deps.query_evidence_event_ids(query).await?)?;
        if queried_ids.is_empty() && evidence_event_ids.is_empty() {
            return Err(AppError::InvalidParams(
                "no replacement evidence found for the provided query".to_string(),
            ));
        }
        evidence_event_ids.append(&mut queried_ids);
    }

    normalize_event_ids(evidence_event_ids)
}

/// Reflection APIs accept either a raw event id or its `event:<id>` reference.
/// Persistence and store lookups intentionally retain raw ids for the existing
/// `*_event_ids` compatibility contract.
fn normalize_event_ids(event_ids: Vec<String>) -> Result<Vec<String>, AppError> {
    let mut normalized = Vec::new();
    for event_id in event_ids {
        let event_id = EventReference::parse(event_id)
            .map_err(AppError::from)?
            .event_id()
            .to_string();
        if !normalized.contains(&event_id) {
            normalized.push(event_id);
        }
    }
    Ok(normalized)
}
