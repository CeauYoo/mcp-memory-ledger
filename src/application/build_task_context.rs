//! Packs whole, provenance-bearing records under an exact compact-JSON UTF-8 byte budget.
//! The budget excludes the enclosing MCP/JSON-RPC transport envelope. No token estimate.
use super::recall_memory::{self, RecallMatch, RecallMemoryInput};
use crate::{
    domain::types::{Namespace, Owner},
    error::AppError,
    ports::{MemoryReadStore, text_memory_store::TextMemoryStore},
};
use serde::Serialize;

pub const DEFAULT_CONTEXT_MAX_BYTES: usize = 16_384;
pub const MAX_CONTEXT_BYTES: usize = 262_144;
#[derive(Debug, Clone)]
pub struct BuildTaskContextInput {
    pub namespace: Namespace,
    pub query: String,
    pub limit: usize,
    pub max_bytes: usize,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContextOmissions {
    /// Number of retrieved records excluded because their complete payload did not fit.
    pub byte_budget: usize,
    /// Further matching records exist beyond the bounded retrieval candidate limit.
    pub candidate_limit: bool,
    /// Candidate changed/disappeared while its full scoped record was loaded.
    pub unavailable_after_retrieval: usize,
    /// This policy excludes obsolete claims; historical events remain explicitly labelled.
    pub claim_status_policy: &'static str,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BuildTaskContextResult {
    pub owner: Owner,
    pub namespace: String,
    pub query: String,
    pub budget_unit: &'static str,
    pub max_bytes: usize,
    pub serialized_bytes: usize,
    pub candidate_limit: usize,
    pub retrieval_strategy: &'static str,
    pub selection_policy: &'static str,
    pub index_warning: Option<String>,
    pub omissions: ContextOmissions,
    pub records: Vec<RecallMatch>,
}

impl BuildTaskContextResult {
    /// Stabilize the size field, including its own decimal digits and every metadata field.
    fn measure(&mut self) -> Result<usize, AppError> {
        loop {
            let size = serde_json::to_vec(self)
                .map_err(|error| AppError::Message(error.to_string()))?
                .len();
            if size == self.serialized_bytes {
                return Ok(size);
            }
            self.serialized_bytes = size;
        }
    }
}

pub async fn execute<D: MemoryReadStore + TextMemoryStore + Sync>(
    deps: &D,
    input: BuildTaskContextInput,
) -> Result<BuildTaskContextResult, AppError> {
    if input.max_bytes == 0 || input.max_bytes > MAX_CONTEXT_BYTES {
        return Err(AppError::InvalidParams(format!(
            "max_bytes must be 1..={MAX_CONTEXT_BYTES}"
        )));
    }
    let recalled = recall_memory::execute(
        deps,
        RecallMemoryInput {
            namespace: input.namespace,
            query: input.query,
            limit: input.limit,
        },
    )
    .await?;
    pack(recalled, input.max_bytes)
}

fn pack(
    recalled: recall_memory::RecallMemoryResult,
    max_bytes: usize,
) -> Result<BuildTaskContextResult, AppError> {
    let mut result = BuildTaskContextResult {
        owner: recalled.owner,
        namespace: recalled.namespace,
        query: recalled.query,
        budget_unit: "compact_json_utf8_bytes",
        max_bytes,
        serialized_bytes: 0,
        candidate_limit: recalled.limit,
        retrieval_strategy: recalled.strategy,
        selection_policy: recalled.selection_policy,
        index_warning: recalled.index_warning,
        omissions: ContextOmissions {
            byte_budget: recalled.records.len(),
            candidate_limit: recalled.has_more,
            unavailable_after_retrieval: recalled.unavailable_after_retrieval,
            claim_status_policy: "active_only",
        },
        records: Vec::new(),
    };
    let minimum = result.measure()?;
    if minimum > max_bytes {
        return Err(AppError::InvalidParams(format!(
            "max_bytes cannot fit context metadata: requires at least {minimum} bytes"
        )));
    }
    // Try later, smaller records if a large one fails; never truncate UTF-8 or provenance.
    for record in recalled.records {
        result.records.push(record);
        result.omissions.byte_budget -= 1;
        if result.measure()? > max_bytes {
            result.records.pop();
            result.omissions.byte_budget += 1;
            result.measure()?;
        }
    }
    debug_assert!(result.serialized_bytes <= max_bytes);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        application::search_memory::{EventProvenance, SearchMemoryRecord},
        domain::types::EventKind,
    };
    fn fixture() -> recall_memory::RecallMemoryResult {
        recall_memory::RecallMemoryResult {
            owner: Owner::User,
            namespace: "user/test".into(),
            query: "北京 咖啡 记忆 \"\\".into(),
            strategy: "test",
            selection_policy: recall_memory::RECALL_SELECTION_POLICY,
            index_warning: None,
            unavailable_after_retrieval: 0,
            limit: 20,
            has_more: true,
            records: (0..12)
                .map(|i| RecallMatch {
                    matched_terms: 1,
                    explanation: recall_memory::RecallExplanation {
                        matched_query_terms: vec!["北京".into()],
                        validity: "historical_event_not_a_current_conclusion",
                        time_basis: "event_recorded_at_desc_after_term_count",
                    },
                    record: SearchMemoryRecord::Event {
                        id: format!("event:{i}"),
                        recorded_at: chrono::DateTime::from_timestamp(0, 0).unwrap(),
                        owner: Owner::User,
                        namespace: "user/test".into(),
                        kind: EventKind::Observation,
                        feedback: None,
                        summary: "北京咖啡记忆🧠\n\"\\".repeat(i + 1),
                        provenance: EventProvenance {
                            evidence_event_reference: format!("event:{i}"),
                            claim_ids: vec!["claim:retained".into()],
                            episode_references: vec!["episode:retained".into()],
                        },
                    },
                })
                .collect(),
        }
    }
    #[test]
    fn every_budget_counts_exact_serialized_utf8_and_metadata() {
        for budget in 1..7000 {
            if let Ok(context) = pack(fixture(), budget) {
                assert_eq!(
                    serde_json::to_vec(&context).unwrap().len(),
                    context.serialized_bytes
                );
                assert!(context.serialized_bytes <= budget);
                assert_eq!(context.records.len() + context.omissions.byte_budget, 12);
                assert!(context.omissions.candidate_limit);
                for hit in context.records {
                    if let SearchMemoryRecord::Event { provenance, .. } = hit.record {
                        assert_eq!(provenance.claim_ids, ["claim:retained"]);
                    }
                }
            }
        }
    }
    #[test]
    fn oversized_first_record_does_not_block_later_small_record() {
        let mut recalled = fixture();
        recalled.records.truncate(2);
        if let SearchMemoryRecord::Event { summary, .. } = &mut recalled.records[0].record {
            *summary = "北京".repeat(10000);
        }
        let context = pack(recalled, 1600).unwrap();
        assert_eq!(context.records.len(), 1);
        assert_eq!(context.omissions.byte_budget, 1);
        assert!(
            matches!(&context.records[0].record, SearchMemoryRecord::Event { id, .. } if id == "event:1")
        );
        assert_eq!(
            serde_json::to_vec(&context).unwrap().len(),
            context.serialized_bytes
        );
    }
    #[test]
    fn rejects_tiny_budget_and_includes_all_at_exact_boundary() {
        assert!(pack(fixture(), 1).is_err());
        let large = pack(fixture(), 100_000).unwrap();
        let exact = pack(fixture(), large.serialized_bytes).unwrap();
        assert_eq!(exact.records.len(), 12);
        assert_eq!(exact.omissions.byte_budget, 0);
    }
}
