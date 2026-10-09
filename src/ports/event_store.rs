use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::{
    domain::{
        event::{Event, EventReference, MAX_EVIDENCE_MANIFEST_ITEMS},
        snapshot::SnapshotTimeWindow,
        types::{EventKind, MemoryScope, Namespace, Owner},
    },
    error::AppError,
};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StoredEvent {
    pub event_id: String,
    pub recorded_at: DateTime<Utc>,
    /// Exact caller-supplied observation time, independent of recorded_at.
    #[serde(default)]
    pub observed_at: Option<String>,
    pub event: Event,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EvidenceQuery {
    pub namespace: Option<Namespace>,
    pub owner: Option<Owner>,
    pub kind: Option<EventKind>,
    pub limit: Option<usize>,
    pub recorded_after: Option<DateTime<Utc>>,
    pub recorded_before: Option<DateTime<Utc>>,
    /// 有界收窄过滤：按 event_id 前缀精确匹配，仍 intersect-only / no-widening，不引入排序或打分。
    pub event_id_prefix: Option<String>,
}

impl StoredEvent {
    pub fn new(event_id: String, recorded_at: DateTime<Utc>, event: Event) -> Self {
        Self {
            event_id,
            recorded_at,
            observed_at: None,
            event,
        }
    }

    pub fn with_observed_at(mut self, observed_at: Option<String>) -> Self {
        self.observed_at = observed_at;
        self
    }

    pub fn event_reference(&self) -> String {
        EventReference::from_event_id(self.event_id.clone()).canonical()
    }
}

#[async_trait]
pub trait EventStore {
    async fn append_event(&self, event: StoredEvent) -> Result<(), AppError>;
    async fn list_event_references(&self) -> Result<Vec<String>, AppError>;
    async fn list_event_references_in_scope(
        &self,
        scope: &MemoryScope,
        evidence_manifest: Option<&[EventReference]>,
    ) -> Result<Vec<String>, AppError> {
        if evidence_manifest.is_some_and(|manifest| manifest.len() > MAX_EVIDENCE_MANIFEST_ITEMS) {
            return Err(AppError::InvalidParams(format!(
                "evidence_manifest must contain at most {MAX_EVIDENCE_MANIFEST_ITEMS} entries"
            )));
        }
        if evidence_manifest.is_some() && !scope.is_explicitly_scoped() {
            return Err(AppError::InvalidParams(
                "evidence_manifest requires an explicit namespace".to_string(),
            ));
        }
        if evidence_manifest.is_some_and(|manifest| manifest.is_empty()) {
            return Ok(Vec::new());
        }
        if scope.is_legacy_unscoped() && evidence_manifest.is_none() {
            return self.list_event_references().await;
        }
        let mut event_ids = self
            .query_evidence_event_ids_unbounded(EvidenceQuery {
                namespace: scope.namespace().cloned(),
                owner: scope.owner(),
                kind: None,
                limit: None,
                recorded_after: None,
                recorded_before: None,
                event_id_prefix: None,
            })
            .await?;
        if let Some(manifest) = evidence_manifest {
            event_ids.retain(|event_id| {
                manifest
                    .iter()
                    .any(|reference| reference.event_id() == event_id)
            });
        }
        Ok(event_ids
            .into_iter()
            .map(|event_id| EventReference::from_event_id(event_id).canonical())
            .collect())
    }
    async fn list_event_references_for_snapshot(
        &self,
        scope: &MemoryScope,
        evidence_manifest: Option<&[EventReference]>,
        time_window: &SnapshotTimeWindow,
    ) -> Result<Vec<String>, AppError> {
        time_window.validate().map_err(|_| {
            AppError::InvalidParams(
                "recorded_after must be less than or equal to recorded_before".to_string(),
            )
        })?;
        if evidence_manifest.is_some_and(|manifest| manifest.len() > MAX_EVIDENCE_MANIFEST_ITEMS) {
            return Err(AppError::InvalidParams(format!(
                "evidence_manifest must contain at most {MAX_EVIDENCE_MANIFEST_ITEMS} entries"
            )));
        }
        if evidence_manifest.is_some() && !scope.is_explicitly_scoped() {
            return Err(AppError::InvalidParams(
                "evidence_manifest requires an explicit namespace".to_string(),
            ));
        }
        if !time_window.is_unbounded() && !scope.is_explicitly_scoped() {
            return Err(AppError::InvalidParams(
                "snapshot time window requires an explicit namespace".to_string(),
            ));
        }
        if time_window.is_unbounded() {
            return self
                .list_event_references_in_scope(scope, evidence_manifest)
                .await;
        }
        if evidence_manifest.is_some_and(|manifest| manifest.is_empty()) {
            return Ok(Vec::new());
        }
        let mut event_ids = self
            .query_evidence_event_ids_unbounded(EvidenceQuery {
                namespace: scope.namespace().cloned(),
                owner: scope.owner(),
                kind: None,
                limit: None,
                recorded_after: time_window.recorded_after,
                recorded_before: time_window.recorded_before,
                event_id_prefix: None,
            })
            .await?;
        if let Some(manifest) = evidence_manifest {
            event_ids.retain(|event_id| {
                manifest
                    .iter()
                    .any(|reference| reference.event_id() == event_id)
            });
        }
        Ok(event_ids
            .into_iter()
            .map(|event_id| EventReference::from_event_id(event_id).canonical())
            .collect())
    }
    /// Returns the exact recorded timestamps for an already-authorized snapshot
    /// manifest. Callers use these bounds to keep dependent episode reads in the
    /// same trigger window; a missing manifest entry is never substituted with a
    /// wider scope query.
    async fn list_recorded_at_for_snapshot_manifest(
        &self,
        scope: &MemoryScope,
        evidence_manifest: &[EventReference],
    ) -> Result<Vec<DateTime<Utc>>, AppError>;
    async fn query_evidence_event_ids(&self, query: EvidenceQuery)
    -> Result<Vec<String>, AppError>;
    async fn query_evidence_event_ids_unbounded(
        &self,
        query: EvidenceQuery,
    ) -> Result<Vec<String>, AppError>;
    async fn has_event(&self, event_id: &str) -> Result<bool, AppError>;
}
