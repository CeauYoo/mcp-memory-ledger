use std::collections::{BTreeSet, HashSet};

use crate::ports::{ClaimStatus, EvidenceQuery, SelfModelHistoryKind};
use crate::{
    application::{
        auto_reflect_if_needed::AutoReflectInput,
        build_self_snapshot::BuildSelfSnapshotInput,
        decide_with_snapshot::DecideWithSnapshotInput,
        get_evidence_relation::GetEvidenceRelationInput,
        get_memory::{GetMemoryInput, MemoryRecordReference},
        get_reflection_history::{DEFAULT_REFLECTION_HISTORY_LIMIT, GetReflectionHistoryInput},
        get_self_model_history::{DEFAULT_SELF_MODEL_HISTORY_LIMIT, GetSelfModelHistoryInput},
        ingest_interaction::IngestInput,
        run_reflection::ReflectionInput,
        search_memory::{DEFAULT_SEARCH_MEMORY_LIMIT, MemoryRecordType, SearchMemoryInput},
        supersede_memory::SupersedeMemoryInput,
    },
    domain::{
        claim::{ClaimDraft, ClaimReference},
        commitment::Commitment,
        event::{Event, EventReference, MAX_EVIDENCE_MANIFEST_ITEMS},
        reflection::{Reflection, ReflectionIdentityUpdate},
        self_revision::TriggerType,
        snapshot::{SelfSnapshot, SnapshotBudget, SnapshotTimeWindow},
        types::{EventKind, MemoryScope, Mode, Namespace, Owner},
    },
    error::AppError,
};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum OwnerDto {
    Self_,
    User,
    World,
    Unknown,
}

impl From<OwnerDto> for Owner {
    fn from(value: OwnerDto) -> Self {
        match value {
            OwnerDto::Self_ => Owner::Self_,
            OwnerDto::User => Owner::User,
            OwnerDto::World => Owner::World,
            OwnerDto::Unknown => Owner::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ModeDto {
    Observed,
    Said,
    Acted,
    Inferred,
    Draft,
}

impl From<ModeDto> for Mode {
    fn from(value: ModeDto) -> Self {
        match value {
            ModeDto::Observed => Mode::Observed,
            ModeDto::Said => Mode::Said,
            ModeDto::Acted => Mode::Acted,
            ModeDto::Inferred => Mode::Inferred,
            ModeDto::Draft => Mode::Draft,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum MemoryRecordTypeDto {
    Event,
    Claim,
    Episode,
    Reflection,
}

impl From<MemoryRecordTypeDto> for MemoryRecordType {
    fn from(value: MemoryRecordTypeDto) -> Self {
        match value {
            MemoryRecordTypeDto::Event => Self::Event,
            MemoryRecordTypeDto::Claim => Self::Claim,
            MemoryRecordTypeDto::Episode => Self::Episode,
            MemoryRecordTypeDto::Reflection => Self::Reflection,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum SearchMemoryRecordTypeDto {
    Event,
    Claim,
    Episode,
    Reflection,
}

impl From<SearchMemoryRecordTypeDto> for MemoryRecordType {
    fn from(value: SearchMemoryRecordTypeDto) -> Self {
        match value {
            SearchMemoryRecordTypeDto::Event => Self::Event,
            SearchMemoryRecordTypeDto::Claim => Self::Claim,
            SearchMemoryRecordTypeDto::Episode => Self::Episode,
            SearchMemoryRecordTypeDto::Reflection => Self::Reflection,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ClaimStatusDto {
    Active,
    Disputed,
    Superseded,
}

impl From<ClaimStatusDto> for ClaimStatus {
    fn from(value: ClaimStatusDto) -> Self {
        match value {
            ClaimStatusDto::Active => Self::Active,
            ClaimStatusDto::Disputed => Self::Disputed,
            ClaimStatusDto::Superseded => Self::Superseded,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum EventKindDto {
    Observation,
    Conversation,
    Action,
    Reflection,
}

impl From<EventKindDto> for EventKind {
    fn from(value: EventKindDto) -> Self {
        match value {
            EventKindDto::Observation => EventKind::Observation,
            EventKindDto::Conversation => EventKind::Conversation,
            EventKindDto::Action => EventKind::Action,
            EventKindDto::Reflection => EventKind::Reflection,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EventDto {
    #[serde(default)]
    pub feedback: Option<crate::domain::feedback::FeedbackMetadata>,
    pub owner: OwnerDto,
    #[serde(default)]
    pub namespace: Option<String>,
    pub kind: EventKindDto,
    pub summary: String,
}

impl TryFrom<EventDto> for Event {
    type Error = crate::domain::DomainError;

    fn try_from(value: EventDto) -> Result<Self, Self::Error> {
        let owner = Owner::from(value.owner);
        if !owner.is_accepted_for_new_writes() {
            return Err(crate::domain::DomainError::UnknownOwnerNotWritable);
        }
        let kind = EventKind::from(value.kind);
        let namespace = value.namespace.map(Namespace::parse).transpose()?;

        let event = match namespace {
            Some(namespace) => Event::new_with_namespace(owner, namespace, kind, value.summary)?,
            None => Event::new(owner, kind, value.summary),
        };
        match value.feedback {
            Some(feedback) => event.with_feedback(feedback),
            None => Ok(event),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ClaimDraftDto {
    pub owner: OwnerDto,
    pub namespace: Option<String>,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub mode: ModeDto,
}

impl TryFrom<ClaimDraftDto> for ClaimDraft {
    type Error = crate::domain::DomainError;

    fn try_from(value: ClaimDraftDto) -> Result<Self, Self::Error> {
        let owner = Owner::from(value.owner);
        if !owner.is_accepted_for_new_writes() {
            return Err(crate::domain::DomainError::UnknownOwnerNotWritable);
        }
        let mode = Mode::from(value.mode);
        let namespace = value.namespace.map(Namespace::parse).transpose()?;

        Ok(match namespace {
            Some(namespace) => ClaimDraft::new_with_namespace(
                owner,
                namespace,
                value.subject,
                value.predicate,
                value.object,
                mode,
            ),
            None => ClaimDraft::new(owner, value.subject, value.predicate, value.object, mode),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CommitmentDto {
    pub owner: OwnerDto,
    pub description: String,
}

impl From<CommitmentDto> for Commitment {
    fn from(value: CommitmentDto) -> Self {
        Commitment::new(value.owner.into(), value.description)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IngestInteractionParams {
    #[serde(default)]
    pub request_id: Option<String>,
    pub event: EventDto,
    pub claim_drafts: Vec<ClaimDraftDto>,
    pub episode_reference: Option<String>,
    #[serde(default)]
    pub trigger_hints: Vec<String>,
}

impl TryFrom<IngestInteractionParams> for IngestInput {
    type Error = crate::domain::DomainError;

    fn try_from(value: IngestInteractionParams) -> Result<Self, Self::Error> {
        let input = IngestInput::new(
            Event::try_from(value.event)?,
            value
                .claim_drafts
                .into_iter()
                .map(ClaimDraft::try_from)
                .collect::<Result<Vec<_>, _>>()?,
            value.episode_reference,
        )
        .with_trigger_hints(value.trigger_hints);
        Ok(match value.request_id {
            Some(key) => input.with_request_id(key),
            None => input,
        })
    }
}

impl IngestInteractionParams {
    fn auto_reflect_namespace(&self) -> Result<Namespace, AppError> {
        if self.claim_drafts.is_empty() {
            if let Some(namespace) = &self.event.namespace {
                return Namespace::parse(namespace.clone()).map_err(AppError::from);
            }

            return Ok(Namespace::for_owner(self.event.owner.into()));
        }

        let mut namespaces = BTreeSet::new();
        for draft in &self.claim_drafts {
            let namespace = match draft.namespace.as_deref() {
                Some(namespace) => {
                    Namespace::parse(namespace.to_string()).map_err(AppError::from)?
                }
                None => Namespace::for_owner(Owner::from(draft.owner)),
            };
            namespaces.insert(namespace.as_str().to_string());
        }

        match namespaces.len() {
            1 => Namespace::parse(
                namespaces
                    .into_iter()
                    .next()
                    .expect("single namespace must exist"),
            )
            .map_err(AppError::from),
            _ => Err(AppError::InvalidParams(format!(
                "ambiguous auto-reflection namespace derived from claim drafts: {}",
                namespaces.into_iter().collect::<Vec<_>>().join(", ")
            ))),
        }
    }
}

impl AutoReflectInput {
    pub fn from_ingest(params: &IngestInteractionParams) -> Result<Self, AppError> {
        Ok(Self::new(
            params.auto_reflect_namespace()?,
            ingest_trigger_type_from_hints(&params.trigger_hints),
            params.trigger_hints.clone(),
        ))
    }
}

// Ingest only upgrades to the conflict hook for explicit conflict/identity hints.
// Rollback-only hints stay on the existing failure path.
fn ingest_trigger_type_from_hints(trigger_hints: &[String]) -> TriggerType {
    if trigger_hints
        .iter()
        .any(|hint| matches!(hint.to_ascii_lowercase().as_str(), "conflict" | "identity"))
    {
        TriggerType::Conflict
    } else {
        TriggerType::Failure
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BuildSelfSnapshotParams {
    pub budget: usize,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    #[schemars(length(max = MAX_EVIDENCE_MANIFEST_ITEMS))]
    pub evidence_manifest: Option<Vec<String>>,
    #[serde(default)]
    pub recorded_after: Option<String>,
    #[serde(default)]
    pub recorded_before: Option<String>,
    #[serde(default)]
    pub auto_reflect_namespace: Option<String>,
}

impl TryFrom<BuildSelfSnapshotParams> for BuildSelfSnapshotInput {
    type Error = AppError;

    fn try_from(value: BuildSelfSnapshotParams) -> Result<Self, Self::Error> {
        if value.evidence_manifest.is_some() && value.namespace.is_none() {
            return Err(AppError::InvalidParams(
                "evidence_manifest requires an explicit namespace".to_string(),
            ));
        }
        if (value.recorded_after.is_some() || value.recorded_before.is_some())
            && value.namespace.is_none()
        {
            return Err(AppError::InvalidParams(
                "snapshot time window requires an explicit namespace".to_string(),
            ));
        }

        let scope = value
            .namespace
            .map(Namespace::parse)
            .transpose()
            .map_err(AppError::from)?
            .map(MemoryScope::for_namespace)
            .unwrap_or_else(MemoryScope::legacy_unscoped);
        let evidence_manifest = value
            .evidence_manifest
            .map(|manifest| -> Result<Vec<EventReference>, AppError> {
                if manifest.len() > MAX_EVIDENCE_MANIFEST_ITEMS {
                    return Err(AppError::InvalidParams(format!(
                        "evidence_manifest must contain at most {MAX_EVIDENCE_MANIFEST_ITEMS} entries"
                    )));
                }

                let mut canonical = Vec::new();
                let mut seen = HashSet::with_capacity(manifest.len());
                for value in manifest {
                    let reference = EventReference::parse(value).map_err(AppError::from)?;
                    if seen.insert(reference.clone()) {
                        canonical.push(reference);
                    }
                }
                Ok(canonical)
            })
            .transpose()?;
        let time_window = SnapshotTimeWindow::new(
            parse_optional_timestamp("recorded_after", value.recorded_after)?,
            parse_optional_timestamp("recorded_before", value.recorded_before)?,
        )
        .map_err(|_| {
            AppError::InvalidParams(
                "recorded_after must be less than or equal to recorded_before".to_string(),
            )
        })?;

        Ok(BuildSelfSnapshotInput {
            scope,
            evidence_manifest,
            time_window,
            budget: SnapshotBudget::new(value.budget),
        })
    }
}

impl BuildSelfSnapshotParams {
    fn auto_reflect_namespace(&self) -> Result<Option<Namespace>, AppError> {
        self.auto_reflect_namespace
            .as_deref()
            .map(|namespace| Namespace::parse(namespace.to_string()).map_err(AppError::from))
            .transpose()
    }
}

impl AutoReflectInput {
    pub fn from_build_snapshot(params: &BuildSelfSnapshotParams) -> Result<Option<Self>, AppError> {
        params
            .auto_reflect_namespace()?
            .map(|namespace| Ok(Self::for_periodic(namespace, vec!["periodic".to_string()])))
            .transpose()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SelfSnapshotDto {
    pub identity: Vec<String>,
    pub commitments: Vec<String>,
    pub claims: Vec<String>,
    pub evidence: Vec<String>,
    pub episodes: Vec<String>,
}

impl From<SelfSnapshotDto> for SelfSnapshot {
    fn from(value: SelfSnapshotDto) -> Self {
        SelfSnapshot {
            identity: value.identity,
            commitments: value.commitments,
            claims: value.claims,
            evidence: value.evidence,
            episodes: value.episodes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DecideWithSnapshotParams {
    pub task: String,
    pub action: String,
    pub snapshot: SelfSnapshotDto,
    #[serde(default)]
    pub trigger_hints: Vec<String>,
    #[serde(default)]
    pub auto_reflect_namespace: Option<String>,
}

impl From<DecideWithSnapshotParams> for DecideWithSnapshotInput {
    fn from(value: DecideWithSnapshotParams) -> Self {
        DecideWithSnapshotInput {
            task: value.task,
            action: value.action,
            snapshot: value.snapshot.into(),
        }
    }
}

impl DecideWithSnapshotParams {
    fn auto_reflect_namespace(&self) -> Result<Option<Namespace>, AppError> {
        self.auto_reflect_namespace
            .as_deref()
            .map(|namespace| Namespace::parse(namespace.to_string()).map_err(AppError::from))
            .transpose()
    }
}

impl AutoReflectInput {
    pub fn from_decide(params: &DecideWithSnapshotParams) -> Result<Option<Self>, AppError> {
        params
            .auto_reflect_namespace()?
            .map(|namespace| Ok(Self::for_conflict(namespace, params.trigger_hints.clone())))
            .transpose()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReflectionDto {
    pub summary: String,
}

impl From<ReflectionDto> for Reflection {
    fn from(value: ReflectionDto) -> Self {
        Reflection::new(value.summary)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReflectionIdentityUpdateDto {
    pub canonical_claims: Vec<String>,
}

impl From<ReflectionIdentityUpdateDto> for ReflectionIdentityUpdate {
    fn from(value: ReflectionIdentityUpdateDto) -> Self {
        ReflectionIdentityUpdate::new(value.canonical_claims)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EvidenceQueryDto {
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub owner: Option<OwnerDto>,
    #[serde(default)]
    pub kind: Option<EventKindDto>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub recorded_after: Option<String>,
    #[serde(default)]
    pub recorded_before: Option<String>,
    #[serde(default)]
    pub event_id_prefix: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SearchMemoryParams {
    pub namespace: String,
    #[serde(default)]
    pub record_type: Option<SearchMemoryRecordTypeDto>,
    #[serde(default)]
    pub record_types: Option<Vec<SearchMemoryRecordTypeDto>>,
    #[serde(default)]
    pub event_reference: Option<String>,
    #[serde(default)]
    pub kind: Option<EventKindDto>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub recorded_after: Option<String>,
    #[serde(default)]
    pub recorded_before: Option<String>,
    #[serde(default)]
    pub claim_reference: Option<String>,
    #[serde(default)]
    pub claim_status: Option<ClaimStatusDto>,
    #[serde(default)]
    pub mode: Option<ModeDto>,
    #[serde(default)]
    pub episode_reference: Option<String>,
    #[serde(default)]
    pub reflection_reference: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GetMemoryParams {
    pub namespace: String,
    pub id: String,
    #[serde(default)]
    pub record_type: Option<MemoryRecordTypeDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GetEvidenceRelationParams {
    pub namespace: String,
    #[schemars(length(max = MAX_EVIDENCE_MANIFEST_ITEMS))]
    pub trigger_window_event_ids: Vec<String>,
    #[serde(default)]
    #[schemars(length(max = MAX_EVIDENCE_MANIFEST_ITEMS))]
    pub selected_evidence_event_ids: Option<Vec<String>>,
    #[serde(default)]
    pub selection_basis: Option<String>,
}

impl TryFrom<GetEvidenceRelationParams> for GetEvidenceRelationInput {
    type Error = AppError;

    fn try_from(value: GetEvidenceRelationParams) -> Result<Self, Self::Error> {
        if value.trigger_window_event_ids.len() > MAX_EVIDENCE_MANIFEST_ITEMS {
            return Err(AppError::InvalidParams(format!(
                "trigger_window_event_ids must contain at most {MAX_EVIDENCE_MANIFEST_ITEMS} entries"
            )));
        }
        let selected_evidence_event_ids = value.selected_evidence_event_ids.unwrap_or_default();
        if selected_evidence_event_ids.len() > MAX_EVIDENCE_MANIFEST_ITEMS {
            return Err(AppError::InvalidParams(format!(
                "selected_evidence_event_ids must contain at most {MAX_EVIDENCE_MANIFEST_ITEMS} entries"
            )));
        }
        let input = Self {
            namespace: Namespace::parse(value.namespace).map_err(AppError::from)?,
            trigger_window: parse_event_references(value.trigger_window_event_ids)?,
            selected_evidence: parse_event_references(selected_evidence_event_ids)?,
            selection_basis: value.selection_basis,
        };
        input.validate()?;
        Ok(input)
    }
}

fn parse_event_references(values: Vec<String>) -> Result<Vec<EventReference>, AppError> {
    let mut parsed = Vec::new();
    for value in values {
        let reference = EventReference::parse(value).map_err(AppError::from)?;
        if parsed
            .iter()
            .all(|existing: &EventReference| existing.event_id() != reference.event_id())
        {
            parsed.push(reference);
        }
    }
    Ok(parsed)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GetReflectionHistoryParams {
    pub namespace: String,
    pub claim_reference: String,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum SelfModelHistoryTypeDto {
    Identity,
    Commitment,
}

impl From<SelfModelHistoryTypeDto> for SelfModelHistoryKind {
    fn from(value: SelfModelHistoryTypeDto) -> Self {
        match value {
            SelfModelHistoryTypeDto::Identity => Self::Identity,
            SelfModelHistoryTypeDto::Commitment => Self::Commitment,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GetSelfModelHistoryParams {
    pub namespace: String,
    pub history_type: SelfModelHistoryTypeDto,
    #[serde(default)]
    pub limit: Option<usize>,
}

impl TryFrom<GetSelfModelHistoryParams> for GetSelfModelHistoryInput {
    type Error = AppError;

    fn try_from(value: GetSelfModelHistoryParams) -> Result<Self, Self::Error> {
        let input = Self {
            namespace: Namespace::parse(value.namespace).map_err(AppError::from)?,
            history_kind: SelfModelHistoryKind::from(value.history_type),
            limit: value.limit.unwrap_or(DEFAULT_SELF_MODEL_HISTORY_LIMIT),
        };
        input.validate()?;
        Ok(input)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SupersedeMemoryParams {
    #[serde(default)]
    pub request_id: Option<String>,
    pub namespace: String,
    pub claim_reference: String,
    pub replacement_claim: ClaimDraftDto,
    #[schemars(length(max = MAX_EVIDENCE_MANIFEST_ITEMS))]
    pub replacement_evidence_event_ids: Vec<String>,
    pub summary: String,
}

impl TryFrom<SupersedeMemoryParams> for SupersedeMemoryInput {
    type Error = AppError;

    fn try_from(value: SupersedeMemoryParams) -> Result<Self, Self::Error> {
        if value.replacement_evidence_event_ids.len() > MAX_EVIDENCE_MANIFEST_ITEMS {
            return Err(AppError::InvalidParams(format!(
                "replacement_evidence_event_ids must contain at most {MAX_EVIDENCE_MANIFEST_ITEMS} entries"
            )));
        }
        let input = Self {
            request_id: value.request_id,
            namespace: Namespace::parse(value.namespace).map_err(AppError::from)?,
            claim_reference: ClaimReference::parse(value.claim_reference)
                .map_err(AppError::from)?,
            replacement_claim: ClaimDraft::try_from(value.replacement_claim)
                .map_err(AppError::from)?,
            evidence_event_ids: parse_event_references(value.replacement_evidence_event_ids)?,
            summary: value.summary,
        };
        input.validate()?;
        Ok(input)
    }
}

impl TryFrom<GetReflectionHistoryParams> for GetReflectionHistoryInput {
    type Error = AppError;

    fn try_from(value: GetReflectionHistoryParams) -> Result<Self, Self::Error> {
        let input = Self {
            namespace: Namespace::parse(value.namespace).map_err(AppError::from)?,
            claim_reference: ClaimReference::parse(value.claim_reference)
                .map_err(AppError::from)?,
            limit: value.limit.unwrap_or(DEFAULT_REFLECTION_HISTORY_LIMIT),
        };
        input.validate()?;
        Ok(input)
    }
}

impl TryFrom<GetMemoryParams> for GetMemoryInput {
    type Error = AppError;

    fn try_from(value: GetMemoryParams) -> Result<Self, Self::Error> {
        let record_type = value.record_type.unwrap_or(MemoryRecordTypeDto::Event);
        let id = match record_type {
            MemoryRecordTypeDto::Event => MemoryRecordReference::Event(
                EventReference::parse(value.id).map_err(AppError::from)?,
            ),
            MemoryRecordTypeDto::Claim => MemoryRecordReference::Claim(
                ClaimReference::parse(value.id).map_err(AppError::from)?,
            ),
            MemoryRecordTypeDto::Episode => {
                MemoryRecordReference::Episode(parse_opaque_episode_reference(value.id)?)
            }
            MemoryRecordTypeDto::Reflection => {
                MemoryRecordReference::Reflection(parse_opaque_reflection_reference(value.id)?)
            }
        };
        Ok(Self {
            namespace: Namespace::parse(value.namespace).map_err(AppError::from)?,
            id,
        })
    }
}

fn parse_opaque_episode_reference(value: String) -> Result<String, AppError> {
    parse_opaque_exact_reference(value, "episode_reference")
}

fn parse_opaque_reflection_reference(value: String) -> Result<String, AppError> {
    parse_opaque_exact_reference(value, "reflection_reference")
}

fn parse_opaque_exact_reference(value: String, field: &str) -> Result<String, AppError> {
    if value.is_empty() || value.trim() != value {
        return Err(AppError::InvalidParams(format!(
            "{field} must be non-empty and have no leading or trailing whitespace"
        )));
    }
    Ok(value)
}

impl TryFrom<SearchMemoryParams> for SearchMemoryInput {
    type Error = AppError;

    fn try_from(value: SearchMemoryParams) -> Result<Self, Self::Error> {
        let record_types = resolve_search_record_types(value.record_type, value.record_types)?;
        let input = Self {
            namespace: Namespace::parse(value.namespace).map_err(AppError::from)?,
            record_types: record_types.clone(),
            event_reference: value
                .event_reference
                .map(EventReference::parse)
                .transpose()
                .map_err(AppError::from)?,
            kind: value.kind.map(EventKind::from),
            recorded_after: parse_optional_timestamp("recorded_after", value.recorded_after)?,
            recorded_before: parse_optional_timestamp("recorded_before", value.recorded_before)?,
            claim_reference: value
                .claim_reference
                .map(ClaimReference::parse)
                .transpose()
                .map_err(AppError::from)?,
            claim_status: value.claim_status.map(ClaimStatus::from).or_else(|| {
                (record_types.as_slice() == [MemoryRecordType::Claim])
                    .then_some(ClaimStatus::Active)
            }),
            mode: value.mode.map(Mode::from),
            episode_reference: value.episode_reference,
            reflection_reference: value.reflection_reference,
            limit: value.limit.unwrap_or(DEFAULT_SEARCH_MEMORY_LIMIT),
        };
        input.validate()?;
        Ok(input)
    }
}

fn resolve_search_record_types(
    record_type: Option<SearchMemoryRecordTypeDto>,
    record_types: Option<Vec<SearchMemoryRecordTypeDto>>,
) -> Result<Vec<MemoryRecordType>, AppError> {
    match (record_type, record_types) {
        (Some(_), Some(_)) => Err(AppError::InvalidParams(
            "record_type and record_types cannot be set together".to_string(),
        )),
        (None, Some(types)) if types.is_empty() => Err(AppError::InvalidParams(
            "record_types must contain at least one record type".to_string(),
        )),
        (None, Some(types)) => Ok(types.into_iter().map(MemoryRecordType::from).collect()),
        (Some(record_type), None) => Ok(vec![MemoryRecordType::from(record_type)]),
        (None, None) => Ok(vec![MemoryRecordType::Event]),
    }
}

impl TryFrom<EvidenceQueryDto> for EvidenceQuery {
    type Error = AppError;

    fn try_from(value: EvidenceQueryDto) -> Result<Self, Self::Error> {
        if let Some(limit) = value.limit {
            if limit == 0 {
                return Err(AppError::InvalidParams(
                    "replacement evidence query limit must be at least 1".to_string(),
                ));
            }
            i64::try_from(limit).map_err(|_| {
                AppError::InvalidParams(
                    "replacement evidence query limit exceeds the supported maximum".to_string(),
                )
            })?;
        }

        if let Some(prefix) = value.event_id_prefix.as_ref()
            && prefix.trim().is_empty()
        {
            return Err(AppError::InvalidParams(
                "evidence query event_id_prefix must not be empty".to_string(),
            ));
        }

        Ok(Self {
            namespace: value
                .namespace
                .map(Namespace::parse)
                .transpose()
                .map_err(AppError::from)?,
            owner: value.owner.map(Owner::from),
            kind: value.kind.map(EventKind::from),
            limit: value.limit,
            recorded_after: parse_optional_timestamp("recorded_after", value.recorded_after)?,
            recorded_before: parse_optional_timestamp("recorded_before", value.recorded_before)?,
            event_id_prefix: value.event_id_prefix,
        })
    }
}

fn parse_optional_timestamp(
    field: &str,
    value: Option<String>,
) -> Result<Option<DateTime<Utc>>, AppError> {
    value
        .map(|timestamp| {
            DateTime::parse_from_rfc3339(&timestamp)
                .map(|timestamp| timestamp.with_timezone(&Utc))
                .map_err(|error| {
                    AppError::InvalidParams(format!("invalid {field} timestamp: {error}"))
                })
        })
        .transpose()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunReflectionParams {
    pub reflection: ReflectionDto,
    pub supersede_claim_id: String,
    pub replacement_claim: Option<ClaimDraftDto>,
    #[serde(default)]
    pub replacement_evidence_event_ids: Vec<String>,
    #[serde(default)]
    pub replacement_evidence_query: Option<EvidenceQueryDto>,
    #[serde(default)]
    pub identity_update: Option<ReflectionIdentityUpdateDto>,
    #[serde(default)]
    pub commitment_updates: Option<Vec<CommitmentDto>>,
}

impl TryFrom<RunReflectionParams> for ReflectionInput {
    type Error = AppError;

    fn try_from(value: RunReflectionParams) -> Result<Self, Self::Error> {
        let mut input = ReflectionInput::new(
            value.reflection.into(),
            value.supersede_claim_id,
            value
                .replacement_claim
                .map(ClaimDraft::try_from)
                .transpose()
                .map_err(AppError::from)?,
            value.replacement_evidence_event_ids,
        );

        if let Some(replacement_evidence_query) = value.replacement_evidence_query {
            input = input.with_replacement_evidence_query(replacement_evidence_query.try_into()?);
        }

        if let Some(identity_update) = value.identity_update {
            input = input.with_identity_update(identity_update.canonical_claims);
        }

        if let Some(commitment_updates) = value.commitment_updates {
            input = input.with_commitment_updates(
                commitment_updates
                    .into_iter()
                    .map(Commitment::from)
                    .collect(),
            );
        }

        Ok(input)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RecallMemoryParams {
    pub namespace: String,
    pub query: String,
    #[serde(default)]
    pub limit: Option<usize>,
}

impl TryFrom<RecallMemoryParams> for crate::application::recall_memory::RecallMemoryInput {
    type Error = AppError;
    fn try_from(value: RecallMemoryParams) -> Result<Self, Self::Error> {
        Ok(Self {
            namespace: Namespace::parse(value.namespace).map_err(AppError::from)?,
            query: value.query,
            limit: value
                .limit
                .unwrap_or(crate::application::recall_memory::DEFAULT_RECALL_LIMIT),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BuildTaskContextParams {
    pub namespace: String,
    pub query: String,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub max_bytes: Option<usize>,
}

impl TryFrom<BuildTaskContextParams>
    for crate::application::build_task_context::BuildTaskContextInput
{
    type Error = AppError;
    fn try_from(value: BuildTaskContextParams) -> Result<Self, Self::Error> {
        Ok(Self {
            namespace: Namespace::parse(value.namespace).map_err(AppError::from)?,
            query: value.query,
            limit: value
                .limit
                .unwrap_or(crate::application::recall_memory::DEFAULT_RECALL_LIMIT),
            max_bytes: value
                .max_bytes
                .unwrap_or(crate::application::build_task_context::DEFAULT_CONTEXT_MAX_BYTES),
        })
    }
}
