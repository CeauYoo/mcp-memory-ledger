use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::{
    domain::{
        claim::ClaimReference,
        types::{MemoryScope, Namespace, Owner},
    },
    error::AppError,
    ports::{
        ClaimReflectionHistoryQuery, ClaimReflectionHistoryRecord, MAX_EVENT_RECORD_QUERY_LIMIT,
        MemoryReadStore,
    },
};

pub const DEFAULT_REFLECTION_HISTORY_LIMIT: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetReflectionHistoryInput {
    pub namespace: Namespace,
    pub claim_reference: ClaimReference,
    pub limit: usize,
}

impl GetReflectionHistoryInput {
    pub fn validate(&self) -> Result<(), AppError> {
        if self.limit == 0 {
            return Err(AppError::InvalidParams(
                "get_reflection_history limit must be at least 1".to_string(),
            ));
        }
        if self.limit > MAX_EVENT_RECORD_QUERY_LIMIT {
            return Err(AppError::InvalidParams(format!(
                "get_reflection_history limit must be at most {MAX_EVENT_RECORD_QUERY_LIMIT}"
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GetReflectionHistoryResult {
    pub owner: Owner,
    pub namespace: String,
    pub claim_reference: String,
    pub limit: usize,
    pub has_more: bool,
    pub reflections: Vec<ReflectionHistoryRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReflectionHistoryRecord {
    pub scope: crate::domain::reflection_scope::ReflectionScopeMetadata,
    pub reflection_id: String,
    pub recorded_at: DateTime<Utc>,
    pub summary: String,
    pub superseded_claim_reference: Option<String>,
    pub replacement_claim_reference: Option<String>,
    pub supporting_evidence_event_references: Vec<String>,
}

pub async fn execute<D>(
    deps: &D,
    input: GetReflectionHistoryInput,
) -> Result<GetReflectionHistoryResult, AppError>
where
    D: MemoryReadStore + Sync,
{
    input.validate()?;
    let scope = MemoryScope::for_namespace(input.namespace.clone());
    let owner = scope
        .owner()
        .expect("namespace-derived memory scope must have an owner");
    let canonical_claim_reference = input.claim_reference.canonical();
    let page = deps
        .query_claim_reflection_history(ClaimReflectionHistoryQuery {
            scope,
            claim_reference: input.claim_reference,
            limit: input.limit,
        })
        .await?;

    Ok(GetReflectionHistoryResult {
        owner,
        namespace: input.namespace.as_str().to_string(),
        claim_reference: canonical_claim_reference,
        limit: input.limit,
        has_more: page.has_more,
        reflections: page.records.into_iter().map(Into::into).collect(),
    })
}

impl From<ClaimReflectionHistoryRecord> for ReflectionHistoryRecord {
    fn from(value: ClaimReflectionHistoryRecord) -> Self {
        Self {
            scope: value.scope,
            reflection_id: value.reflection_id,
            recorded_at: value.recorded_at,
            summary: value.summary,
            superseded_claim_reference: value
                .superseded_claim_reference
                .map(|reference| reference.canonical()),
            replacement_claim_reference: value
                .replacement_claim_reference
                .map(|reference| reference.canonical()),
            supporting_evidence_event_references: value
                .supporting_evidence_event_references
                .into_iter()
                .map(|reference| reference.canonical())
                .collect(),
        }
    }
}
