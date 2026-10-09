//! Packs whole, provenance-bearing records under an exact compact-JSON UTF-8 byte budget.
//! The budget excludes the enclosing MCP/JSON-RPC transport envelope. No token estimate.
use super::recall_memory::{self, RecallMatch, RecallMemoryInput};
use super::search_memory::SearchMemoryRecord;
use crate::{
    domain::{
        caller_budget::CallerBudgetDecision,
        event::EventReference,
        experience::PersistedEpisode,
        types::{MemoryScope, Namespace, Owner},
    },
    error::AppError,
    ports::{
        ClaimStatus, MemoryReadStore,
        text_memory_store::{
            LINKED_EPISODE_LIMIT, LINKED_EPISODE_SOURCE_LIMIT, LinkedEpisodeQuery,
            TextClaimStatusDiagnostics, TextMemoryQuery, TextMemoryStore,
        },
    },
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

pub const DEFAULT_CONTEXT_MAX_BYTES: usize = 16_384;
pub const MAX_CONTEXT_BYTES: usize = 262_144;
const DIAGNOSTIC_REFERENCE_LIMIT: usize = 8;
const DIFFERING_VALUE_GROUP_LIMIT: usize = 4;
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
    /// Optional diagnostics never displace whole primary records.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub episodes: Option<ContextEpisodeOmissions>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContextEpisodeOmissions {
    pub byte_budget: usize,
    pub candidate_limit: usize,
    pub unavailable: usize,
    /// Lower bound of one when the selected source window has a distinct sentinel.
    pub source_limit: usize,
    pub rich_lookup_supported: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContextClaimReferences {
    /// Exact count within returned records; references are bounded separately.
    pub count: usize,
    pub claim_references: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PossibleDifferingValues {
    pub interpretation: &'static str,
    pub group_count: usize,
    pub groups: Vec<ContextClaimReferences>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReturnedRecordDiagnostics {
    pub no_records: bool,
    pub query_terms_without_returned_match: Vec<String>,
    pub active_unverified_claim_count: usize,
    pub unknown_recorded_at: ContextClaimReferences,
    pub claims_without_evidence_refs: ContextClaimReferences,
    pub possible_differing_values: PossibleDifferingValues,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContextDiagnostics {
    pub contract: &'static str,
    pub expiry: &'static str,
    pub status_count_basis: &'static str,
    /// None means this store does not implement the optional status inspection port.
    pub matching_claim_status: Option<TextClaimStatusDiagnostics>,
    pub returned_records: ReturnedRecordDiagnostics,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_warning: Option<String>,
    pub omissions: ContextOmissions,
    pub records: Vec<RecallMatch>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rich_episodes: Vec<PersistedEpisode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caller_budget: Option<CallerBudgetDecision>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<ContextDiagnostics>,
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
    execute_with_caller_budget(deps, input, None).await
}

pub async fn execute_with_caller_budget<D: MemoryReadStore + TextMemoryStore + Sync>(
    deps: &D,
    input: BuildTaskContextInput,
    caller_budget: Option<CallerBudgetDecision>,
) -> Result<BuildTaskContextResult, AppError> {
    if input.max_bytes == 0 || input.max_bytes > MAX_CONTEXT_BYTES {
        return Err(AppError::InvalidParams(format!(
            "max_bytes must be 1..={MAX_CONTEXT_BYTES}"
        )));
    }
    let recall_input = RecallMemoryInput {
        namespace: input.namespace,
        query: input.query,
        limit: input.limit,
    };
    let query = TextMemoryQuery {
        scope: MemoryScope::for_namespace(recall_input.namespace.clone()),
        terms: recall_input.terms()?,
        limit: recall_input.limit,
    };
    let recalled = recall_memory::execute(deps, recall_input).await?;
    let mut result = pack_with_caller_budget(recalled, input.max_bytes, caller_budget)?;
    pack_episodes(deps, &query.scope, &mut result).await?;
    let status = deps.inspect_text_claim_status(query).await?;
    pack_diagnostics(&mut result, status)?;
    Ok(result)
}

#[cfg(test)]
fn pack(
    recalled: recall_memory::RecallMemoryResult,
    max_bytes: usize,
) -> Result<BuildTaskContextResult, AppError> {
    pack_with_caller_budget(recalled, max_bytes, None)
}

fn pack_with_caller_budget(
    recalled: recall_memory::RecallMemoryResult,
    max_bytes: usize,
    caller_budget: Option<CallerBudgetDecision>,
) -> Result<BuildTaskContextResult, AppError> {
    // Reserve omission metadata before primary packing, using upper bounds that can
    // only shrink once the actually selected primary records are known.
    let source_count = source_event_references(&recalled.records).len();
    let mut result = BuildTaskContextResult {
        owner: recalled.owner,
        namespace: recalled.namespace,
        query: recalled.query,
        budget_unit: "json_utf8_v1",
        max_bytes,
        serialized_bytes: 0,
        candidate_limit: recalled.limit,
        retrieval_strategy: recalled.strategy,
        selection_policy: if recalled.selection_policy == recall_memory::RECALL_SELECTION_POLICY {
            "balanced_v2"
        } else {
            recalled.selection_policy
        },
        index_warning: recalled.index_warning,
        omissions: ContextOmissions {
            byte_budget: recalled.records.len(),
            candidate_limit: recalled.has_more,
            unavailable_after_retrieval: recalled.unavailable_after_retrieval,
            claim_status_policy: "active_only",
            diagnostics: Some("byte_budget"),
            episodes: (source_count > 0).then_some(ContextEpisodeOmissions {
                byte_budget: LINKED_EPISODE_LIMIT,
                candidate_limit: 1,
                unavailable: 0,
                source_limit: source_count.saturating_sub(LINKED_EPISODE_SOURCE_LIMIT),
                rich_lookup_supported: false,
            }),
        },
        records: Vec::new(),
        rich_episodes: Vec::new(),
        caller_budget,
        diagnostics: None,
    };
    let minimum = result.measure()?;
    if minimum > max_bytes {
        return Err(AppError::InvalidParams(format!(
            "max_bytes cannot fit context metadata: requires at least {minimum} bytes"
        )));
    }
    // Try later, smaller records if a large one fails; never truncate UTF-8 or provenance.
    for mut record in recalled.records {
        record.explanation.compact_for_context();
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

fn source_event_references(records: &[RecallMatch]) -> BTreeSet<String> {
    // Stop at one distinct sentinel instead of cloning an arbitrary provenance graph.
    // Primary/provenance order is deterministic; sort only this bounded source window.
    let mut references = BTreeSet::new();
    for hit in records {
        let sources = match &hit.record {
            SearchMemoryRecord::Claim { provenance, .. } => {
                provenance.evidence_event_references.as_slice()
            }
            SearchMemoryRecord::Event { id, .. } => std::slice::from_ref(id),
            _ => unreachable!("primary text recall only returns Claim/Event records"),
        };
        for source in sources {
            references.insert(source.clone());
            if references.len() > LINKED_EPISODE_SOURCE_LIMIT {
                return references;
            }
        }
    }
    references
}

async fn pack_episodes<D: MemoryReadStore + TextMemoryStore + Sync>(
    deps: &D,
    scope: &MemoryScope,
    context: &mut BuildTaskContextResult,
) -> Result<(), AppError> {
    let sources = source_event_references(&context.records);
    if sources.is_empty() {
        context.omissions.episodes = None;
        context.measure()?;
        return Ok(());
    }
    let page = deps
        .linked_episodes(LinkedEpisodeQuery {
            scope: scope.clone(),
            event_references: sources
                .iter()
                .take(LINKED_EPISODE_SOURCE_LIMIT)
                .map(|r| EventReference::from_event_id(r.strip_prefix("event:").unwrap_or(r)))
                .collect(),
        })
        .await?;
    let mut rich_records = Vec::new();
    let mut omissions = ContextEpisodeOmissions {
        byte_budget: 0,
        candidate_limit: 0,
        unavailable: 0,
        source_limit: sources.len().saturating_sub(LINKED_EPISODE_SOURCE_LIMIT),
        rich_lookup_supported: page.is_some(),
    };
    if let Some(page) = page {
        omissions.byte_budget += page.records.len();
        // A rich candidate sentinel contributes a lower bound of one omitted row.
        omissions.candidate_limit += usize::from(page.has_more);
        omissions.unavailable += page.unavailable;
        rich_records = page.records;
    }
    context.omissions.episodes = Some(omissions);
    context.measure()?;
    for record in rich_records {
        context.rich_episodes.push(record);
        context
            .omissions
            .episodes
            .as_mut()
            .expect("episode candidates")
            .byte_budget -= 1;
        if context.measure()? > context.max_bytes {
            context.rich_episodes.pop();
            context
                .omissions
                .episodes
                .as_mut()
                .expect("episode candidates")
                .byte_budget += 1;
            context.measure()?;
        }
    }
    debug_assert!(context.serialized_bytes <= context.max_bytes);
    Ok(())
}

fn pack_diagnostics(
    result: &mut BuildTaskContextResult,
    status: Option<TextClaimStatusDiagnostics>,
) -> Result<(), AppError> {
    // Diagnostics describe the records that actually fit, not the larger candidate set.
    result.diagnostics = Some(context_diagnostics(result, status));
    result.omissions.diagnostics = None;
    if result.measure()? > result.max_bytes {
        result.diagnostics = None;
        result.omissions.diagnostics = Some("byte_budget");
        result.measure()?;
    }
    debug_assert!(result.serialized_bytes <= result.max_bytes);
    Ok(())
}

fn context_diagnostics(
    context: &BuildTaskContextResult,
    matching_claim_status: Option<TextClaimStatusDiagnostics>,
) -> ContextDiagnostics {
    let mut terms = BTreeSet::new();
    for term in context.query.split_whitespace() {
        terms.insert(term.to_ascii_lowercase());
    }
    let mut active_count = 0;
    let mut unknown_times = Vec::new();
    let mut without_evidence = Vec::new();
    let mut groups: BTreeMap<(&str, &str), Vec<(&str, &str)>> = BTreeMap::new();
    for hit in &context.records {
        for term in &hit.explanation.matched_query_terms {
            terms.remove(term);
        }
        if let SearchMemoryRecord::Claim {
            id,
            subject,
            predicate,
            object,
            recorded_at,
            provenance,
            status: ClaimStatus::Active,
            ..
        } = &hit.record
        {
            active_count += 1;
            if recorded_at.is_none() {
                unknown_times.push(id.as_str());
            }
            if provenance.evidence_event_references.is_empty() {
                without_evidence.push(id.as_str());
            }
            groups
                .entry((subject, predicate))
                .or_default()
                .push((id, object));
        }
    }
    // Exact text grouping is observable data, not semantic contradiction inference.
    // At most MAX_RECALL_LIMIT returned records enter this comparison.
    let differing: Vec<_> = groups
        .values()
        .filter(|claims| {
            claims
                .first()
                .is_some_and(|first| claims.iter().any(|claim| claim.1 != first.1))
        })
        .collect();
    ContextDiagnostics {
        contract: "observable_v1",
        expiry: "not_evaluated_no_expiry_contract",
        status_count_basis: "sampled_lower_bounds_unless_scope_scan_complete",
        matching_claim_status,
        returned_records: ReturnedRecordDiagnostics {
            no_records: context.records.is_empty(),
            query_terms_without_returned_match: terms.into_iter().collect(),
            active_unverified_claim_count: active_count,
            unknown_recorded_at: reference_summary(unknown_times),
            claims_without_evidence_refs: reference_summary(without_evidence),
            possible_differing_values: PossibleDifferingValues {
                interpretation: "possible_multi_value_not_inferred_contradiction",
                group_count: differing.len(),
                groups: differing
                    .into_iter()
                    .take(DIFFERING_VALUE_GROUP_LIMIT)
                    .map(|claims| reference_summary(claims.iter().map(|claim| claim.0).collect()))
                    .collect(),
            },
        },
    }
}

fn reference_summary(mut references: Vec<&str>) -> ContextClaimReferences {
    references.sort_unstable();
    ContextClaimReferences {
        count: references.len(),
        claim_references: references
            .into_iter()
            .take(DIAGNOSTIC_REFERENCE_LIMIT)
            .map(str::to_owned)
            .collect(),
    }
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
                        observed_at: None,
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
