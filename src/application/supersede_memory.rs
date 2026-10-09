use serde::Serialize;

use crate::{
    domain::{
        claim::{ClaimDraft, ClaimReference},
        event::{EventReference, MAX_EVIDENCE_MANIFEST_ITEMS},
        reflection::Reflection,
        self_revision::SELF_REVISION_DURABLE_WRITE_PATH,
        types::{MemoryScope, Namespace, Owner},
    },
    error::AppError,
    ports::{
        ClaimRecordQuery, ClaimStatus, Clock, EventStore, IdGenerator, MemoryReadStore,
        ReflectionTransactionRunner, ScopedEventIdQuery,
    },
};

use super::run_reflection::{self, ReflectionInput};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupersedeMemoryInput {
    pub namespace: Namespace,
    pub claim_reference: ClaimReference,
    pub replacement_claim: ClaimDraft,
    pub evidence_event_ids: Vec<EventReference>,
    pub summary: String,
    pub request_id: Option<String>,
}

impl SupersedeMemoryInput {
    pub fn validate(&self) -> Result<(), AppError> {
        if self.summary.is_empty() || self.summary.trim() != self.summary {
            return Err(AppError::InvalidParams(
                "supersede_memory summary must be non-empty and have no leading or trailing whitespace"
                    .to_string(),
            ));
        }
        if self.replacement_claim.namespace() != &self.namespace {
            return Err(AppError::InvalidParams(
                "supersede_memory replacement claim must stay in the requested namespace"
                    .to_string(),
            ));
        }
        if self.evidence_event_ids.is_empty() {
            return Err(AppError::InvalidParams(
                "supersede_memory requires at least one replacement evidence event id".to_string(),
            ));
        }
        if self.evidence_event_ids.len() > MAX_EVIDENCE_MANIFEST_ITEMS {
            return Err(AppError::InvalidParams(format!(
                "supersede_memory replacement_evidence_event_ids must contain at most {MAX_EVIDENCE_MANIFEST_ITEMS} entries"
            )));
        }
        self.replacement_claim
            .validate(self.evidence_event_ids.len())
            .map_err(AppError::from)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SupersedeMemoryResult {
    pub owner: Owner,
    pub namespace: String,
    pub superseded_claim_reference: String,
    pub replacement_claim_id: Option<String>,
    pub reflection_id: String,
    pub durable_write_path: String,
}

pub async fn execute<D>(
    deps: &D,
    input: SupersedeMemoryInput,
) -> Result<SupersedeMemoryResult, AppError>
where
    D: ReflectionTransactionRunner + EventStore + IdGenerator + Clock + Sync,
{
    let namespace = input.namespace.clone();
    let claim_reference = input.claim_reference.clone();
    let owner = MemoryScope::for_namespace(namespace.clone())
        .owner()
        .expect("namespace-derived memory scope must have an owner");
    input.validate()?;
    // Preflight reads are intentionally bypassed here: replay and authoritative
    // validation must happen in the same transaction, even for terminal targets.
    let mut reflection_input = ReflectionInput::new(
        Reflection::new(input.summary.clone()),
        input.claim_reference.claim_id(),
        Some(input.replacement_claim.clone()),
        input
            .evidence_event_ids
            .iter()
            .map(|id| id.event_id().to_string())
            .collect(),
    )
    .with_strict_evidence_scope();
    if let Some(key) = &input.request_id {
        let request = crate::ports::WriteReceiptRequest::new(
            "supersede_memory",
            input.namespace.as_str(),
            key,
            &(
                &input.claim_reference,
                &input.replacement_claim,
                &input.evidence_event_ids,
                &input.summary,
            ),
        )?;
        reflection_input = reflection_input.with_write_receipt(request);
    }
    let result = run_reflection::execute(deps, reflection_input).await?;
    Ok(SupersedeMemoryResult {
        owner,
        namespace: namespace.as_str().to_string(),
        superseded_claim_reference: claim_reference.canonical(),
        replacement_claim_id: result.replacement_claim_id,
        reflection_id: result.reflection_id,
        durable_write_path: SELF_REVISION_DURABLE_WRITE_PATH.to_string(),
    })
}

pub async fn prepare_scoped_supersede<D>(
    deps: &D,
    input: &SupersedeMemoryInput,
) -> Result<ReflectionInput, AppError>
where
    D: MemoryReadStore + Sync,
{
    input.validate()?;
    let scope = MemoryScope::for_namespace(input.namespace.clone());
    let target_claim_id = require_scoped_claim(deps, &scope, &input.claim_reference).await?;
    let evidence_event_ids =
        require_scoped_evidence(deps, &scope, &input.evidence_event_ids).await?;
    Ok(ReflectionInput::new(
        Reflection::new(input.summary.clone()),
        target_claim_id,
        Some(input.replacement_claim.clone()),
        evidence_event_ids,
    )
    .with_strict_evidence_scope())
}

async fn require_scoped_claim<D>(
    deps: &D,
    scope: &MemoryScope,
    claim_reference: &ClaimReference,
) -> Result<String, AppError>
where
    D: MemoryReadStore + Sync,
{
    let records = deps
        .query_claim_records(ClaimRecordQuery {
            scope: scope.clone(),
            claim_reference: Some(claim_reference.clone()),
            status: None,
            mode: None,
            limit: 1,
        })
        .await?;
    records
        .into_iter()
        .next()
        .filter(|record| record.claim.status != ClaimStatus::Superseded)
        .map(|record| record.claim.claim_id)
        .ok_or_else(|| {
            AppError::InvalidParams(
                "supersede_memory target claim was not found or is already superseded in the requested namespace"
                    .to_string(),
            )
        })
}

async fn require_scoped_evidence<D>(
    deps: &D,
    scope: &MemoryScope,
    evidence_event_ids: &[EventReference],
) -> Result<Vec<String>, AppError>
where
    D: MemoryReadStore + Sync,
{
    let evidence_ids = evidence_event_ids
        .iter()
        .map(|reference| reference.event_id().to_string())
        .collect::<Vec<_>>();
    let scoped_ids = deps
        .query_scoped_event_ids(ScopedEventIdQuery {
            scope: scope.clone(),
            event_ids: evidence_ids.clone(),
        })
        .await?;
    if evidence_ids
        .iter()
        .any(|event_id| !scoped_ids.contains(event_id))
    {
        return Err(AppError::InvalidParams(
            "supersede_memory evidence must exist in the requested namespace".to_string(),
        ));
    }
    Ok(evidence_ids)
}
