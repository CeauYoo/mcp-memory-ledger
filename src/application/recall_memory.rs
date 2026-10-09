//! Offline lexical recall: ASCII-insensitive, literal substring OR matching, not embeddings.
use super::search_memory::SearchMemoryRecord;
use crate::{
    domain::{
        claim::ClaimReference,
        event::EventReference,
        types::{MemoryScope, Namespace, Owner},
    },
    error::AppError,
    ports::{
        ClaimRecordQuery, ClaimStatus, EventRecordQuery, MemoryReadStore,
        text_memory_store::{
            MAX_RECALL_LIMIT, MAX_RECALL_QUERY_BYTES, MAX_RECALL_TERMS, TextMemoryQuery,
            TextMemoryReference, TextMemoryStore,
        },
    },
};
use serde::Serialize;

pub const DEFAULT_RECALL_LIMIT: usize = 20;
#[derive(Debug, Clone)]
pub struct RecallMemoryInput {
    pub namespace: Namespace,
    pub query: String,
    pub limit: usize,
}
impl RecallMemoryInput {
    pub fn terms(&self) -> Result<Vec<String>, AppError> {
        if self.query.len() > MAX_RECALL_QUERY_BYTES
            || self.query.contains('\0')
            || self.limit == 0
            || self.limit > MAX_RECALL_LIMIT
        {
            return Err(AppError::InvalidParams(format!(
                "recall requires query <= {MAX_RECALL_QUERY_BYTES} UTF-8 bytes without NUL and limit 1..={MAX_RECALL_LIMIT}"
            )));
        }
        let mut terms = Vec::new();
        for word in self.query.split_whitespace() {
            let word = word.to_ascii_lowercase();
            if !terms.contains(&word) {
                terms.push(word);
            }
        }
        if terms.is_empty() || terms.len() > MAX_RECALL_TERMS {
            return Err(AppError::InvalidParams(format!(
                "recall requires 1..={MAX_RECALL_TERMS} distinct whitespace-separated terms"
            )));
        }
        Ok(terms)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecallMatch {
    pub matched_terms: usize,
    pub explanation: RecallExplanation,
    pub record: SearchMemoryRecord,
}
/// Selection evidence, not a claim that the retrieved content is objectively true.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecallExplanation {
    pub matched_query_terms: Vec<String>,
    pub validity: &'static str,
    pub time_basis: &'static str,
}

pub const RECALL_SELECTION_POLICY: &str =
    "balanced_claim_event_quotas_claim_odd_slot_then_term_count_recency_id_v2";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecallMemoryResult {
    pub owner: Owner,
    pub namespace: String,
    pub query: String,
    pub strategy: &'static str,
    pub selection_policy: &'static str,
    pub index_warning: Option<String>,
    pub unavailable_after_retrieval: usize,
    pub limit: usize,
    pub has_more: bool,
    pub records: Vec<RecallMatch>,
}
pub async fn execute<D: MemoryReadStore + TextMemoryStore + Sync>(
    deps: &D,
    input: RecallMemoryInput,
) -> Result<RecallMemoryResult, AppError> {
    let terms = input.terms()?;
    let scope = MemoryScope::for_namespace(input.namespace.clone());
    let page = deps
        .recall_text(TextMemoryQuery {
            scope: scope.clone(),
            terms: terms.clone(),
            limit: input.limit,
        })
        .await?;
    let retrieved_count = page.hits.len();
    let mut records = Vec::new();
    for hit in page.hits {
        let record = match hit.reference {
            TextMemoryReference::Claim(id) => deps
                .query_claim_records(ClaimRecordQuery {
                    scope: scope.clone(),
                    claim_reference: Some(ClaimReference::from_claim_id(id)),
                    status: Some(ClaimStatus::Active),
                    mode: None,
                    limit: 1,
                })
                .await?
                .into_iter()
                .next()
                .map(SearchMemoryRecord::from),
            TextMemoryReference::Event(id) => deps
                .query_event_records(EventRecordQuery {
                    scope: scope.clone(),
                    event_reference: Some(EventReference::from_event_id(id)),
                    kind: None,
                    recorded_after: None,
                    recorded_before: None,
                    limit: 1,
                })
                .await?
                .into_iter()
                .next()
                .map(SearchMemoryRecord::from),
        };
        if let Some(record) = record {
            // Hydration rechecks scope/status in the ledger. A legacy same-ID update can
            // also change text since candidate selection; never return a now-unrelated hit.
            let explanation = explain(&record, &terms);
            if explanation.matched_query_terms.is_empty() {
                continue;
            }
            records.push(RecallMatch {
                matched_terms: explanation.matched_query_terms.len(),
                explanation,
                record,
            });
        }
    }
    Ok(RecallMemoryResult {
        owner: scope.owner().expect("namespace scope"),
        namespace: input.namespace.as_str().to_string(),
        query: input.query,
        strategy: page.strategy,
        selection_policy: RECALL_SELECTION_POLICY,
        index_warning: page.index_warning,
        unavailable_after_retrieval: retrieved_count - records.len(),
        limit: input.limit,
        has_more: page.has_more,
        records,
    })
}

fn explain(record: &SearchMemoryRecord, terms: &[String]) -> RecallExplanation {
    let (fields, validity, time_basis) = match record {
        SearchMemoryRecord::Claim {
            subject,
            predicate,
            object,
            recorded_at,
            ..
        } => (
            vec![subject.as_str(), predicate.as_str(), object.as_str()],
            "active_claim_not_independently_verified",
            if recorded_at.is_some() {
                "claim_recorded_at_desc_after_term_count"
            } else {
                "claim_creation_time_unknown_no_recency_assumed"
            },
        ),
        SearchMemoryRecord::Event { summary, .. } => (
            vec![summary.as_str()],
            "historical_event_not_a_current_conclusion",
            "event_recorded_at_desc_after_term_count",
        ),
        _ => unreachable!("text recall supports only claims and events"),
    };
    RecallExplanation {
        matched_query_terms: terms
            .iter()
            .filter(|term| {
                fields
                    .iter()
                    .any(|field| field.to_ascii_lowercase().contains(term.as_str()))
            })
            .cloned()
            .collect(),
        validity,
        time_basis,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_bytes_terms_and_limit_without_reinterpreting_punctuation() {
        let input = |query: &str, limit| RecallMemoryInput {
            namespace: Namespace::self_(),
            query: query.into(),
            limit,
        };
        assert_eq!(
            input("COFFEE coffee 北京 %_*", 1).terms().unwrap(),
            ["coffee", "北京", "%_*"]
        );
        for invalid in ["", " \t\n", "a b c d e f g h i", "bad\0query"] {
            assert!(input(invalid, 1).terms().is_err());
        }
        assert!(input(&"京".repeat(171), 1).terms().is_err());
        assert!(input("coffee", 0).terms().is_err());
        assert!(input("coffee", 101).terms().is_err());
        assert_eq!(input("ÉCOLE", 1).terms().unwrap(), ["École"]);
    }
}
