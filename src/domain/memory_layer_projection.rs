use crate::domain::{self_revision::SELF_REVISION_DURABLE_WRITE_PATH, snapshot::SelfSnapshot};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryLayerProjectionInput {
    pub snapshot: SelfSnapshot,
    pub episode_projection_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MemoryLayerProjection {
    pub read_only: bool,
    pub capability_scope: String,
    pub writes_performed: bool,
    pub durable_self_model_write_path: String,
    pub layers: Vec<MemoryLayerStatus>,
}

impl MemoryLayerProjection {
    pub fn layer_status(&self, layer: &str) -> Option<String> {
        self.layers
            .iter()
            .find(|entry| entry.layer == layer)
            .map(|entry| entry.status.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MemoryLayerStatus {
    pub layer: String,
    pub status: String,
    pub evidence: Vec<String>,
    pub writes_allowed: bool,
}

pub fn build_memory_layer_projection(input: MemoryLayerProjectionInput) -> MemoryLayerProjection {
    let snapshot = input.snapshot;
    MemoryLayerProjection {
        read_only: true,
        capability_scope: "legacy_snapshot_projection_only_not_v5_experience_inventory".into(),
        writes_performed: false,
        durable_self_model_write_path: SELF_REVISION_DURABLE_WRITE_PATH.to_string(),
        layers: vec![
            layer(
                "working",
                status_for_count(snapshot.evidence.len()),
                snapshot.evidence,
            ),
            layer(
                "episodic",
                status_for_count(snapshot.episodes.len().max(input.episode_projection_count)),
                snapshot.episodes,
            ),
            layer(
                "semantic",
                status_for_count(snapshot.claims.len()),
                snapshot.claims,
            ),
            layer("procedural", "not_implemented", Vec::new()),
            layer(
                "self_model",
                status_for_count(snapshot.identity.len() + snapshot.commitments.len()),
                snapshot
                    .identity
                    .into_iter()
                    .chain(snapshot.commitments)
                    .collect(),
            ),
        ],
    }
}

fn layer(layer: &str, status: &str, evidence: Vec<String>) -> MemoryLayerStatus {
    MemoryLayerStatus {
        layer: layer.to_string(),
        status: status.to_string(),
        evidence,
        writes_allowed: false,
    }
}

fn status_for_count(count: usize) -> &'static str {
    if count == 0 {
        "not_implemented"
    } else {
        "partial"
    }
}
