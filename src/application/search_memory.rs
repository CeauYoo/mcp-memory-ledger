use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::{
    domain::{
        claim::ClaimReference,
        event::EventReference,
        feedback::FeedbackMetadata,
        types::{EventKind, MemoryScope, Mode, Namespace, Owner},
    },
    error::AppError,
    ports::{
        ClaimReadRecord, ClaimRecordQuery, ClaimStatus, EpisodeReadRecord, EpisodeRecordQuery,
        EventReadRecord, EventRecordQuery, MAX_EVENT_RECORD_QUERY_LIMIT, MemoryReadStore,
        ReflectionReadRecord, ReflectionRecordQuery,
    },
};

pub const DEFAULT_SEARCH_MEMORY_LIMIT: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryRecordType {
    Event,
    Claim,
    Episode,
    Reflection,
}

impl MemoryRecordType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Event => "event",
            Self::Claim => "claim",
            Self::Episode => "episode",
            Self::Reflection => "reflection",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchMemoryInput {
    pub namespace: Namespace,
    pub record_types: Vec<MemoryRecordType>,
    pub event_reference: Option<EventReference>,
    pub kind: Option<EventKind>,
    pub recorded_after: Option<DateTime<Utc>>,
    pub recorded_before: Option<DateTime<Utc>>,
    pub claim_reference: Option<ClaimReference>,
    pub claim_status: Option<ClaimStatus>,
    pub mode: Option<Mode>,
    pub episode_reference: Option<String>,
    pub reflection_reference: Option<String>,
    pub limit: usize,
}

impl SearchMemoryInput {
    pub fn is_union(&self) -> bool {
        self.record_types.len() > 1
    }

    pub fn single_record_type(&self) -> Option<MemoryRecordType> {
        (self.record_types.len() == 1).then_some(self.record_types[0])
    }

    pub fn validate(&self) -> Result<(), AppError> {
        if self.limit == 0 {
            return Err(AppError::InvalidParams(
                "search_memory limit must be at least 1".to_string(),
            ));
        }
        if self.limit > MAX_EVENT_RECORD_QUERY_LIMIT {
            return Err(AppError::InvalidParams(format!(
                "search_memory limit must be at most {MAX_EVENT_RECORD_QUERY_LIMIT}"
            )));
        }
        if self.record_types.is_empty() {
            return Err(AppError::InvalidParams(
                "search_memory record_types must contain at least one record type".to_string(),
            ));
        }
        if self.record_types.iter().any(|record_type| {
            self.record_types
                .iter()
                .filter(|candidate| *candidate == record_type)
                .count()
                > 1
        }) {
            return Err(AppError::InvalidParams(
                "search_memory record_types must not contain duplicates".to_string(),
            ));
        }
        if self
            .recorded_after
            .zip(self.recorded_before)
            .is_some_and(|(after, before)| after > before)
        {
            return Err(AppError::InvalidParams(
                "recorded_after must be less than or equal to recorded_before".to_string(),
            ));
        }
        if let Some(reference) = self.episode_reference.as_deref()
            && (reference.is_empty() || reference.trim() != reference)
        {
            return Err(AppError::InvalidParams(
                "episode_reference must be non-empty and have no leading or trailing whitespace"
                    .to_string(),
            ));
        }
        if let Some(reference) = self.reflection_reference.as_deref()
            && (reference.is_empty() || reference.trim() != reference)
        {
            return Err(AppError::InvalidParams(
                "reflection_reference must be non-empty and have no leading or trailing whitespace"
                    .to_string(),
            ));
        }
        if self.is_union() {
            return self.validate_union_filters();
        }
        self.validate_single_type_filters(self.record_types[0])
    }

    fn validate_union_filters(&self) -> Result<(), AppError> {
        if self.event_reference.is_some()
            || self.kind.is_some()
            || self.recorded_after.is_some()
            || self.recorded_before.is_some()
            || self.claim_reference.is_some()
            || self.claim_status.is_some()
            || self.mode.is_some()
            || self.episode_reference.is_some()
            || self.reflection_reference.is_some()
        {
            return Err(AppError::InvalidParams(
                "union searches support only namespace, record_types, and limit filters"
                    .to_string(),
            ));
        }
        Ok(())
    }

    fn validate_single_type_filters(&self, record_type: MemoryRecordType) -> Result<(), AppError> {
        match record_type {
            MemoryRecordType::Event
                if self.claim_reference.is_some()
                    || self.claim_status.is_some()
                    || self.mode.is_some()
                    || self.episode_reference.is_some()
                    || self.reflection_reference.is_some() =>
            {
                Err(AppError::InvalidParams(
                    "claim_reference, claim_status, mode, episode_reference, and reflection_reference require their matching record_type"
                        .to_string(),
                ))
            }
            MemoryRecordType::Claim
                if self.event_reference.is_some()
                    || self.kind.is_some()
                    || self.recorded_after.is_some()
                    || self.recorded_before.is_some()
                    || self.episode_reference.is_some()
                    || self.reflection_reference.is_some() =>
            {
                Err(AppError::InvalidParams(
                    "event_reference, kind, recorded time filters, episode_reference, and reflection_reference require their matching record_type; Claim recorded-time filtering is not supported; legacy recording times may be unknown"
                        .to_string(),
                ))
            }
            MemoryRecordType::Episode
                if self.event_reference.is_some()
                    || self.kind.is_some()
                    || self.recorded_after.is_some()
                    || self.recorded_before.is_some()
                    || self.claim_reference.is_some()
                    || self.claim_status.is_some()
                    || self.mode.is_some()
                    || self.reflection_reference.is_some() =>
            {
                Err(AppError::InvalidParams(
                    "Episode searches support only episode_reference and limit filters".to_string(),
                ))
            }
            MemoryRecordType::Reflection
                if self.event_reference.is_some()
                    || self.kind.is_some()
                    || self.recorded_after.is_some()
                    || self.recorded_before.is_some()
                    || self.claim_reference.is_some()
                    || self.claim_status.is_some()
                    || self.mode.is_some()
                    || self.episode_reference.is_some() =>
            {
                Err(AppError::InvalidParams(
                    "Reflection searches support only reflection_reference and limit filters"
                        .to_string(),
                ))
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchMemoryResult {
    pub owner: Owner,
    pub namespace: String,
    pub limit: usize,
    pub record_types: Vec<String>,
    pub records: Vec<SearchMemoryRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "record_type", rename_all = "snake_case")]
pub enum SearchMemoryRecord {
    Event {
        id: String,
        recorded_at: DateTime<Utc>,
        observed_at: Option<String>,
        owner: Owner,
        namespace: String,
        kind: EventKind,
        summary: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        feedback: Option<FeedbackMetadata>,
        provenance: EventProvenance,
    },
    Claim {
        id: String,
        recorded_at: Option<DateTime<Utc>>,
        observed_at: Option<String>,
        owner: Owner,
        namespace: String,
        subject: String,
        predicate: String,
        object: String,
        mode: Mode,
        status: ClaimStatus,
        provenance: ClaimProvenance,
    },
    Episode {
        id: String,
        recorded_at: DateTime<Utc>,
        owner: Owner,
        namespace: String,
        provenance: EpisodeProvenance,
    },
    Reflection {
        id: String,
        scope: crate::domain::reflection_scope::ReflectionScopeMetadata,
        recorded_at: DateTime<Utc>,
        owner: Owner,
        namespace: String,
        summary: String,
        provenance: ReflectionProvenance,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EventProvenance {
    pub evidence_event_reference: String,
    pub claim_ids: Vec<String>,
    pub episode_references: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClaimProvenance {
    pub evidence_event_references: Vec<String>,
    pub episode_references: Vec<String>,
    pub source_reflection_id: Option<String>,
    pub supersedes_claim_reference: Option<String>,
    pub superseded_by_reflection_id: Option<String>,
    pub replacement_claim_reference: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EpisodeProvenance {
    pub event_references: Vec<String>,
    pub claim_references: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReflectionProvenance {
    pub superseded_claim_reference: Option<String>,
    pub replacement_claim_reference: Option<String>,
    pub supporting_evidence_event_references: Vec<String>,
}

pub async fn execute<D>(deps: &D, input: SearchMemoryInput) -> Result<SearchMemoryResult, AppError>
where
    D: MemoryReadStore + Sync,
{
    input.validate()?;
    let scope = MemoryScope::for_namespace(input.namespace.clone());
    let owner = scope
        .owner()
        .expect("namespace-derived memory scope must have an owner");
    let record_type_labels = input
        .record_types
        .iter()
        .map(|record_type| record_type.as_str().to_string())
        .collect::<Vec<_>>();
    let mut records = Vec::new();
    for record_type in &input.record_types {
        records.extend(query_typed_records(deps, &scope, *record_type, &input).await?);
    }
    if input.is_union() {
        sort_union_records(&mut records);
        records.truncate(input.limit);
    }

    Ok(SearchMemoryResult {
        owner,
        namespace: input.namespace.as_str().to_string(),
        limit: input.limit,
        record_types: record_type_labels,
        records,
    })
}

async fn query_typed_records<D>(
    deps: &D,
    scope: &MemoryScope,
    record_type: MemoryRecordType,
    input: &SearchMemoryInput,
) -> Result<Vec<SearchMemoryRecord>, AppError>
where
    D: MemoryReadStore + Sync,
{
    let records = match record_type {
        MemoryRecordType::Event => {
            let query = EventRecordQuery {
                scope: scope.clone(),
                event_reference: input.event_reference.clone(),
                kind: input.kind,
                recorded_after: input.recorded_after,
                recorded_before: input.recorded_before,
                limit: input.limit,
            };
            let records = if input.is_union() {
                deps.query_event_records_for_union(query).await?
            } else {
                deps.query_event_records(query).await?
            };
            records.into_iter().map(SearchMemoryRecord::from).collect()
        }
        MemoryRecordType::Claim => {
            let query = ClaimRecordQuery {
                scope: scope.clone(),
                claim_reference: input.claim_reference.clone(),
                status: claim_query_status(input),
                mode: input.mode,
                limit: input.limit,
            };
            let records = if input.is_union() {
                deps.query_claim_records_for_union(query).await?
            } else {
                deps.query_claim_records(query).await?
            };
            records.into_iter().map(SearchMemoryRecord::from).collect()
        }
        MemoryRecordType::Episode => {
            let query = EpisodeRecordQuery {
                scope: scope.clone(),
                episode_reference: input.episode_reference.clone(),
                limit: input.limit,
            };
            let records = if input.is_union() {
                deps.query_episode_records_for_union(query).await?
            } else {
                deps.query_episode_records(query).await?
            };
            records.into_iter().map(SearchMemoryRecord::from).collect()
        }
        MemoryRecordType::Reflection => {
            let query = ReflectionRecordQuery {
                scope: scope.clone(),
                reflection_reference: input.reflection_reference.clone(),
                limit: input.limit,
            };
            let records = if input.is_union() {
                deps.query_reflection_records_for_union(query).await?
            } else {
                deps.query_reflection_records(query).await?
            };
            records.into_iter().map(SearchMemoryRecord::from).collect()
        }
    };
    Ok(records)
}

fn claim_query_status(input: &SearchMemoryInput) -> Option<ClaimStatus> {
    if input.is_union() {
        input.claim_status.or(Some(ClaimStatus::Active))
    } else {
        input.claim_status
    }
}

fn sort_union_records(records: &mut [SearchMemoryRecord]) {
    records.sort_by(|left, right| {
        union_recorded_at(right)
            .cmp(&union_recorded_at(left))
            .then_with(|| union_type_rank(left).cmp(&union_type_rank(right)))
            .then_with(|| union_record_id(right).cmp(union_record_id(left)))
    });
}

fn union_recorded_at(record: &SearchMemoryRecord) -> Option<DateTime<Utc>> {
    match record {
        SearchMemoryRecord::Event { recorded_at, .. }
        | SearchMemoryRecord::Episode { recorded_at, .. }
        | SearchMemoryRecord::Reflection { recorded_at, .. } => Some(*recorded_at),
        SearchMemoryRecord::Claim { recorded_at, .. } => *recorded_at,
    }
}

fn union_type_rank(record: &SearchMemoryRecord) -> u8 {
    match record {
        SearchMemoryRecord::Event { .. } => 0,
        SearchMemoryRecord::Episode { .. } => 1,
        SearchMemoryRecord::Reflection { .. } => 2,
        SearchMemoryRecord::Claim { .. } => 3,
    }
}

fn union_record_id(record: &SearchMemoryRecord) -> &str {
    match record {
        SearchMemoryRecord::Event { id, .. }
        | SearchMemoryRecord::Claim { id, .. }
        | SearchMemoryRecord::Episode { id, .. }
        | SearchMemoryRecord::Reflection { id, .. } => id,
    }
}

impl From<EventReadRecord> for SearchMemoryRecord {
    fn from(value: EventReadRecord) -> Self {
        let reference = value.event.event_reference();
        Self::Event {
            id: reference.clone(),
            recorded_at: value.event.recorded_at,
            observed_at: value.event.observed_at.clone(),
            owner: value.event.event.owner(),
            namespace: value.event.event.namespace().as_str().to_string(),
            kind: value.event.event.kind(),
            summary: value.event.event.summary().to_string(),
            feedback: value.event.event.feedback().cloned(),
            provenance: EventProvenance {
                evidence_event_reference: reference,
                claim_ids: value.claim_ids,
                episode_references: value.episode_references,
            },
        }
    }
}

impl From<ClaimReadRecord> for SearchMemoryRecord {
    fn from(value: ClaimReadRecord) -> Self {
        Self::Claim {
            id: ClaimReference::from_claim_id(&value.claim.claim_id).canonical(),
            recorded_at: value.claim.recorded_at,
            observed_at: value.claim.observed_at.clone(),
            owner: value.claim.claim.owner(),
            namespace: value.claim.claim.namespace().as_str().to_string(),
            subject: value.claim.claim.subject().to_string(),
            predicate: value.claim.claim.predicate().to_string(),
            object: value.claim.claim.object().to_string(),
            mode: value.claim.claim.mode(),
            status: value.claim.status,
            provenance: ClaimProvenance {
                evidence_event_references: value
                    .evidence_event_references
                    .into_iter()
                    .map(|reference| reference.canonical())
                    .collect(),
                episode_references: value.episode_references,
                source_reflection_id: value.revision.source_reflection_id,
                supersedes_claim_reference: value
                    .revision
                    .supersedes_claim_reference
                    .map(|reference| reference.canonical()),
                superseded_by_reflection_id: value.revision.superseded_by_reflection_id,
                replacement_claim_reference: value
                    .revision
                    .replacement_claim_reference
                    .map(|reference| reference.canonical()),
            },
        }
    }
}

impl From<EpisodeReadRecord> for SearchMemoryRecord {
    fn from(value: EpisodeReadRecord) -> Self {
        Self::Episode {
            id: value.episode_reference,
            recorded_at: value.recorded_at,
            owner: value.owner,
            namespace: value.namespace.as_str().to_string(),
            provenance: EpisodeProvenance {
                event_references: value
                    .event_references
                    .into_iter()
                    .map(|reference| reference.canonical())
                    .collect(),
                claim_references: value
                    .claim_references
                    .into_iter()
                    .map(|reference| reference.canonical())
                    .collect(),
            },
        }
    }
}

impl From<ReflectionReadRecord> for SearchMemoryRecord {
    fn from(value: ReflectionReadRecord) -> Self {
        Self::Reflection {
            id: value.reflection_id,
            scope: value.scope,
            recorded_at: value.recorded_at,
            owner: value.owner,
            namespace: value.namespace.as_str().to_string(),
            summary: value.summary,
            provenance: ReflectionProvenance {
                superseded_claim_reference: value
                    .provenance
                    .superseded_claim_reference
                    .map(|reference| reference.canonical()),
                replacement_claim_reference: value
                    .provenance
                    .replacement_claim_reference
                    .map(|reference| reference.canonical()),
                supporting_evidence_event_references: value
                    .provenance
                    .supporting_evidence_event_references
                    .into_iter()
                    .map(|reference| reference.canonical())
                    .collect(),
            },
        }
    }
}
