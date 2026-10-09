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
    pub record: SearchMemoryRecord,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecallMemoryResult {
    pub owner: Owner,
    pub namespace: String,
    pub query: String,
    pub strategy: &'static str,
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
            terms,
            limit: input.limit,
        })
        .await?;
    let mut records = Vec::new();
    for hit in page.hits {
        let record = match hit.reference {
            TextMemoryReference::Claim(id) => deps
                .query_claim_records(ClaimRecordQuery {
                    scope: scope.clone(),
                    claim_reference: Some(ClaimReference::parse(id)?),
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
                    event_reference: Some(EventReference::parse(id)?),
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
            records.push(RecallMatch {
                matched_terms: hit.matched_terms,
                record,
            });
        }
    }
    Ok(RecallMemoryResult {
        owner: scope.owner().expect("namespace scope"),
        namespace: input.namespace.as_str().to_string(),
        query: input.query,
        strategy: "literal_substring_ascii_case_insensitive_or_claims_first_v1",
        limit: input.limit,
        has_more: page.has_more,
        records,
    })
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
