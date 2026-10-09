//! Inspectable, scope-bounded interchange. This is deliberately not an import or
//! replay contract: a verified whole-database backup remains the recovery path.
use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    domain::{
        experience::{ExperienceCandidate, PersistedEpisode, validate_namespace},
        reflection_scope::ReflectionScopeMetadata,
        types::{MemoryScope, Namespace, Owner},
    },
    error::AppError,
    ports::{StoredClaim, StoredEvent},
};

pub const EXPORT_FORMAT: &str = "memory-ledger-scoped-interchange";
pub const EXPORT_FORMAT_VERSION: u32 = 1;
pub const MAX_EXPORT_RECORDS: usize = 10_000;
pub const MAX_EXPORT_RELATIONS: usize = 100_000;
pub const MAX_EXPORT_BYTES: usize = 16 * 1024 * 1024;
pub const EXPORT_OMISSION_POLICY: &str = "Unknown or mixed scope and records with incomplete original source closure are omitted. Claims retain all original evidence links or are omitted; source-less Claims remain source-less. Global identity/commitments, operation logs, receipts, trigger keys, feedback candidates, and derived indexes are excluded. Caller-authored scoped content is preserved; this is not a secret-redaction service or a replayable backup.";

fn default_records() -> usize {
    1000
}
fn default_relations() -> usize {
    4000
}
fn default_bytes() -> usize {
    1024 * 1024
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportMemoryRequest {
    /// Exact namespace; owner is derived and cannot be widened by the caller.
    pub namespace: String,
    #[serde(default = "default_records")]
    pub max_records: usize,
    #[serde(default = "default_relations")]
    pub max_relations: usize,
    #[serde(default = "default_bytes")]
    pub max_bytes: usize,
}
impl ExportMemoryRequest {
    pub fn new(namespace: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
            max_records: default_records(),
            max_relations: default_relations(),
            max_bytes: default_bytes(),
        }
    }
    pub fn validate(&self) -> Result<Namespace, AppError> {
        let ns = validate_namespace(&self.namespace)?;
        if !(1..=MAX_EXPORT_RECORDS).contains(&self.max_records)
            || !(1..=MAX_EXPORT_RELATIONS).contains(&self.max_relations)
            || !(1024..=MAX_EXPORT_BYTES).contains(&self.max_bytes)
        {
            return Err(AppError::InvalidParams("export limits: max_records 1..10000, max_relations 1..100000, max_bytes 1024..16777216".into()));
        }
        Ok(ns)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportEvidenceLink {
    pub claim_id: String,
    pub event_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportEpisodeMembership {
    pub episode_reference: String,
    pub event_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportReflection {
    pub reflection_id: String,
    pub recorded_at: DateTime<Utc>,
    pub scope: ReflectionScopeMetadata,
    pub summary: String,
    pub superseded_claim_id: Option<String>,
    pub replacement_claim_id: Option<String>,
    pub supporting_evidence_event_ids: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportExperienceHead {
    pub candidate_id: String,
    pub current_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LedgerExport {
    pub format: String,
    pub format_version: u32,
    pub database_schema_version: i64,
    pub namespace: Namespace,
    pub owner: Owner,
    pub snapshot_consistency: String,
    pub replayable_backup: bool,
    pub omission_policy: String,
    pub events: Vec<StoredEvent>,
    pub claims: Vec<StoredClaim>,
    pub evidence_links: Vec<ExportEvidenceLink>,
    pub episode_memberships: Vec<ExportEpisodeMembership>,
    pub reflections: Vec<ExportReflection>,
    pub experience_episodes: Vec<PersistedEpisode>,
    pub experience_heads: Vec<ExportExperienceHead>,
    pub experience_versions: Vec<ExperienceCandidate>,
    pub record_count: usize,
    pub relation_count: usize,
    /// Exact size of this entire compact JSON document, including this field.
    pub used_bytes: usize,
}
impl LedgerExport {
    pub(crate) fn empty(namespace: Namespace, database_schema_version: i64) -> Self {
        Self {
            format: EXPORT_FORMAT.into(),
            format_version: EXPORT_FORMAT_VERSION,
            database_schema_version,
            owner: namespace.derived_owner(),
            namespace,
            snapshot_consistency: "single_read_transaction".into(),
            replayable_backup: false,
            omission_policy: EXPORT_OMISSION_POLICY.into(),
            events: vec![],
            claims: vec![],
            evidence_links: vec![],
            episode_memberships: vec![],
            reflections: vec![],
            experience_episodes: vec![],
            experience_heads: vec![],
            experience_versions: vec![],
            record_count: 0,
            relation_count: 0,
            used_bytes: 0,
        }
    }
    pub fn counts(&self) -> (usize, usize) {
        let records = self.events.len()
            + self.claims.len()
            + self.reflections.len()
            + self.experience_episodes.len()
            + self.experience_heads.len()
            + self.experience_versions.len();
        let relations = self.evidence_links.len()
            + self.episode_memberships.len()
            + self
                .events
                .iter()
                .map(|e| {
                    e.event.feedback().map_or(0, |f| {
                        f.evidence_refs.len()
                            + usize::from(
                                f.observed_target.starts_with("claim:")
                                    || f.observed_target.starts_with("event:"),
                            )
                    })
                })
                .sum::<usize>()
            + self
                .reflections
                .iter()
                .map(|r| {
                    r.supporting_evidence_event_ids.len()
                        + usize::from(r.superseded_claim_id.is_some())
                        + usize::from(r.replacement_claim_id.is_some())
                })
                .sum::<usize>()
            + self
                .experience_episodes
                .iter()
                .map(|e| e.content.source_event_refs.len())
                .sum::<usize>()
            + self.experience_heads.len()
            + self
                .experience_versions
                .iter()
                .map(|v| {
                    v.content.source_episode_ids.len()
                        + usize::from(v.previous_version.is_some())
                        + usize::from(v.rollback_target_version.is_some())
                })
                .sum::<usize>();
        (records, relations)
    }
    pub(crate) fn finish(&mut self, request: &ExportMemoryRequest) -> Result<(), AppError> {
        (self.record_count, self.relation_count) = self.counts();
        if self.record_count > request.max_records || self.relation_count > request.max_relations {
            return Err(limit_error());
        }
        loop {
            let size = serde_json::to_vec(self).map_err(|_| invalid_graph())?.len();
            if size > request.max_bytes {
                return Err(limit_error());
            }
            if size == self.used_bytes {
                break;
            }
            self.used_bytes = size;
        }
        validate_export(self)
    }
}

pub(crate) fn limit_error() -> AppError {
    AppError::InvalidParams("export exceeds the requested record, relation, or full-response byte budget; narrow the namespace or explicitly raise the bounded limits".into())
}
pub(crate) fn invalid_graph() -> AppError {
    AppError::InvalidParams(
        "invalid scoped export graph or unsupported interchange metadata".into(),
    )
}

/// Validate an already decoded interchange graph without importing or changing a
/// database. Structural integrity is not authentication, truth, or authority.
pub fn validate_export(value: &LedgerExport) -> Result<(), AppError> {
    let ns = &value.namespace;
    validate_namespace(ns.as_str()).map_err(|_| invalid_graph())?;
    let scope = MemoryScope::for_namespace(ns.clone());
    let owner = ns.derived_owner();
    if value.format != EXPORT_FORMAT
        || value.format_version != EXPORT_FORMAT_VERSION
        || value.database_schema_version < 1
        || value.owner != owner
        || value.replayable_backup
        || value.snapshot_consistency != "single_read_transaction"
        || value.omission_policy != EXPORT_OMISSION_POLICY
        || value.counts() != (value.record_count, value.relation_count)
        || value.record_count > MAX_EXPORT_RECORDS
        || value.relation_count > MAX_EXPORT_RELATIONS
        || value.used_bytes > MAX_EXPORT_BYTES
        || serde_json::to_vec(value)
            .map_err(|_| invalid_graph())?
            .len()
            != value.used_bytes
    {
        return Err(invalid_graph());
    }
    let events: BTreeSet<_> = value.events.iter().map(|v| v.event_id.as_str()).collect();
    let claims: BTreeSet<_> = value.claims.iter().map(|v| v.claim_id.as_str()).collect();
    let episodes: BTreeSet<_> = value
        .experience_episodes
        .iter()
        .map(|v| v.episode_id.as_str())
        .collect();
    let reflections: BTreeSet<_> = value
        .reflections
        .iter()
        .map(|v| v.reflection_id.as_str())
        .collect();
    if events.len() != value.events.len()
        || claims.len() != value.claims.len()
        || episodes.len() != value.experience_episodes.len()
        || reflections.len() != value.reflections.len()
        || value
            .events
            .iter()
            .any(|v| v.event.namespace() != ns || v.event.owner() != owner || v.event_id.is_empty())
        || value
            .claims
            .iter()
            .any(|v| v.claim.namespace() != ns || v.claim.owner() != owner || v.claim_id.is_empty())
    {
        return Err(invalid_graph());
    }
    for event in &value.events {
        if let Some(feedback) = event.event.feedback() {
            feedback.validate().map_err(|_| invalid_graph())?;
            if feedback
                .evidence_refs
                .iter()
                .any(|r| !events.contains(r.event_id()))
                || feedback
                    .observed_target
                    .strip_prefix("claim:")
                    .is_some_and(|id| !claims.contains(id))
                || feedback
                    .observed_target
                    .strip_prefix("event:")
                    .is_some_and(|id| !events.contains(id))
            {
                return Err(invalid_graph());
            }
        }
    }
    let mut links = BTreeSet::new();
    for link in &value.evidence_links {
        if !claims.contains(link.claim_id.as_str())
            || !events.contains(link.event_id.as_str())
            || !links.insert((&link.claim_id, &link.event_id))
        {
            return Err(invalid_graph());
        }
    }
    let mut memberships = BTreeSet::new();
    for link in &value.episode_memberships {
        if link.episode_reference.is_empty()
            || !events.contains(link.event_id.as_str())
            || !memberships.insert((&link.episode_reference, &link.event_id))
        {
            return Err(invalid_graph());
        }
    }
    for reflection in &value.reflections {
        if !reflection.scope.permits_targetless_read(&scope)
            || reflection
                .superseded_claim_id
                .as_deref()
                .is_some_and(|id| !claims.contains(id))
            || reflection
                .replacement_claim_id
                .as_deref()
                .is_some_and(|id| !claims.contains(id))
            || reflection
                .supporting_evidence_event_ids
                .iter()
                .any(|id| !events.contains(id.as_str()))
            || reflection
                .supporting_evidence_event_ids
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                != reflection.supporting_evidence_event_ids.len()
        {
            return Err(invalid_graph());
        }
    }
    for episode in &value.experience_episodes {
        episode.content.validate().map_err(|_| invalid_graph())?;
        if &episode.namespace != ns
            || episode.content.source_event_refs.iter().any(|r| {
                crate::domain::event::EventReference::parse(r)
                    .map_or(true, |r| !events.contains(r.event_id()))
            })
        {
            return Err(invalid_graph());
        }
    }
    let mut versions = BTreeMap::<&str, BTreeSet<i64>>::new();
    for version in &value.experience_versions {
        version.content.validate().map_err(|_| invalid_graph())?;
        if &version.namespace != ns
            || version.version < 1
            || !versions
                .entry(&version.candidate_id)
                .or_default()
                .insert(version.version)
            || version
                .content
                .source_episode_ids
                .iter()
                .any(|id| !episodes.contains(id.as_str()))
        {
            return Err(invalid_graph());
        }
    }
    for version in &value.experience_versions {
        let history = &versions[version.candidate_id.as_str()];
        if version.previous_version != (version.version > 1).then_some(version.version - 1)
            || version
                .previous_version
                .is_some_and(|v| !history.contains(&v))
            || version
                .rollback_target_version
                .is_some_and(|v| v >= version.version || !history.contains(&v))
        {
            return Err(invalid_graph());
        }
    }
    let mut heads = BTreeSet::new();
    for head in &value.experience_heads {
        let history = versions
            .get(head.candidate_id.as_str())
            .ok_or_else(invalid_graph)?;
        if !heads.insert(head.candidate_id.as_str())
            || history.last() != Some(&head.current_version)
            || history.len() as i64 != head.current_version
        {
            return Err(invalid_graph());
        }
    }
    if heads.len() != versions.len() {
        return Err(invalid_graph());
    }
    Ok(())
}
