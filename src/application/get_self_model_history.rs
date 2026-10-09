use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::{
    domain::{
        commitment::Commitment,
        reflection::ReflectionIdentityUpdate,
        types::{MemoryScope, Namespace, Owner},
    },
    error::AppError,
    ports::{
        MAX_EVENT_RECORD_QUERY_LIMIT, MemoryReadStore, SelfModelHistoryKind, SelfModelHistoryQuery,
    },
};

pub const DEFAULT_SELF_MODEL_HISTORY_LIMIT: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetSelfModelHistoryInput {
    pub namespace: Namespace,
    pub history_kind: SelfModelHistoryKind,
    pub limit: usize,
}

impl GetSelfModelHistoryInput {
    pub fn validate(&self) -> Result<(), AppError> {
        if self.limit == 0 {
            return Err(AppError::InvalidParams(
                "get_self_model_history limit must be at least 1".to_string(),
            ));
        }
        if self.limit > MAX_EVENT_RECORD_QUERY_LIMIT {
            return Err(AppError::InvalidParams(format!(
                "get_self_model_history limit must be at most {MAX_EVENT_RECORD_QUERY_LIMIT}"
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GetSelfModelHistoryResult {
    pub owner: Owner,
    pub namespace: String,
    pub history_type: String,
    pub limit: usize,
    pub has_more: bool,
    pub records: Vec<SelfModelHistoryRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SelfModelHistoryRecord {
    pub scope: crate::domain::reflection_scope::ReflectionScopeMetadata,
    pub reflection_id: String,
    pub recorded_at: DateTime<Utc>,
    pub summary: String,
    pub superseded_claim_reference: Option<String>,
    pub replacement_claim_reference: Option<String>,
    pub supporting_evidence_event_references: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_update: Option<ReflectionIdentityUpdate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commitment_updates: Option<Vec<Commitment>>,
}

pub async fn execute<D>(
    deps: &D,
    input: GetSelfModelHistoryInput,
) -> Result<GetSelfModelHistoryResult, AppError>
where
    D: MemoryReadStore + Sync,
{
    input.validate()?;
    let scope = MemoryScope::for_namespace(input.namespace.clone());
    let owner = scope
        .owner()
        .expect("namespace-derived memory scope must have an owner");
    let page = deps
        .query_self_model_history(SelfModelHistoryQuery {
            scope,
            history_kind: input.history_kind,
            limit: input.limit,
        })
        .await?;

    Ok(GetSelfModelHistoryResult {
        owner,
        namespace: input.namespace.as_str().to_string(),
        history_type: input.history_kind.as_str().to_string(),
        limit: input.limit,
        has_more: page.has_more,
        records: page
            .records
            .into_iter()
            .map(|record| SelfModelHistoryRecord {
                scope: record.scope,
                reflection_id: record.reflection_id,
                recorded_at: record.recorded_at,
                summary: record.summary,
                superseded_claim_reference: record
                    .superseded_claim_reference
                    .map(|reference| reference.canonical()),
                replacement_claim_reference: record
                    .replacement_claim_reference
                    .map(|reference| reference.canonical()),
                supporting_evidence_event_references: record
                    .supporting_evidence_event_references
                    .into_iter()
                    .map(|reference| reference.canonical())
                    .collect(),
                identity_update: record.identity_update,
                commitment_updates: record.commitment_updates,
            })
            .collect(),
    })
}
