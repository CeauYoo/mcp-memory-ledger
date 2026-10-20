//! Explicit opt-in retrieval. Ranking is selection evidence, never truth or authority.
use super::{recall_memory::RecallMemoryInput, search_memory::SearchMemoryRecord};
use crate::{
    domain::types::{MemoryScope, Namespace},
    error::AppError,
    ports::{
        embedding::{EmbeddingPort, EmbeddingPurpose},
        enhanced_memory_store::EnhancedMemoryStore,
        text_memory_store::TextMemoryStore,
    },
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnhancedRecallConfig {
    /// Exact namespace keys. Expansion is single-hop from original tokens only.
    pub aliases: BTreeMap<String, BTreeMap<String, Vec<String>>>,
}
#[derive(Debug, Clone)]
pub struct EnhancedRecallInput {
    pub namespace: Namespace,
    pub query: String,
    pub limit: usize,
    pub max_bytes: usize,
}
#[derive(Debug, Clone, Serialize)]
pub struct EnhancedRecallHit {
    pub reference: String,
    pub content_digest: String,
    pub routes: BTreeMap<String, usize>,
    pub rrf_score: f64,
    pub applicability: &'static str,
    pub declared_applicability: Option<crate::domain::recall_applicability::RecallApplicability>,
    pub record: SearchMemoryRecord,
}
#[derive(Debug, Clone, Serialize)]
pub struct EnhancedRecallResult {
    pub namespace: String,
    pub query: String,
    pub tokens: Vec<String>,
    pub expanded_terms: Vec<String>,
    pub strategy: &'static str,
    pub interpretation: &'static str,
    pub records: Vec<EnhancedRecallHit>,
    pub candidate_count: usize,
    pub candidate_limit_reached: bool,
    pub unavailable_after_retrieval: usize,
    pub applicability_omissions: usize,
    pub byte_budget_omissions: usize,
    pub result_limit_omissions: usize,
    pub warnings: Vec<String>,
    pub serialized_bytes: usize,
    pub max_bytes: usize,
}
impl EnhancedRecallResult {
    fn measure(&mut self) -> Result<usize, AppError> {
        loop {
            let n = serde_json::to_vec(self)
                .map_err(|e| AppError::Message(e.to_string()))?
                .len();
            if n == self.serialized_bytes {
                return Ok(n);
            }
            self.serialized_bytes = n;
        }
    }
}
pub fn record_reference(record: &SearchMemoryRecord) -> &str {
    match record {
        SearchMemoryRecord::Claim { id, .. }
        | SearchMemoryRecord::Event { id, .. }
        | SearchMemoryRecord::Episode { id, .. }
        | SearchMemoryRecord::Reflection { id, .. } => id,
    }
}
pub fn content_digest(record: &SearchMemoryRecord) -> Result<String, AppError> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(record).map_err(|e| AppError::Message(e.to_string()))?)
    ))
}
fn record_text(record: &SearchMemoryRecord) -> String {
    match record {
        SearchMemoryRecord::Claim {
            subject,
            predicate,
            object,
            ..
        } => format!("{subject}\n{predicate}\n{object}"),
        SearchMemoryRecord::Event { summary, .. } => summary.clone(),
        _ => String::new(),
    }
}
/// Bundled deterministic dictionary; punctuation discarded, original query retained separately.
pub fn tokenize(query: &str) -> Vec<String> {
    let jieba = jieba_rs::Jieba::new();
    let mut result = Vec::new();
    for token in jieba.cut(query, false) {
        let word = token.word.trim().to_ascii_lowercase();
        if !word.is_empty() && word.chars().any(char::is_alphanumeric) && !result.contains(&word) {
            result.push(word);
        }
    }
    result
}
fn terms(
    config: &EnhancedRecallConfig,
    namespace: &str,
    tokens: &[String],
) -> Result<Vec<String>, AppError> {
    let mut expanded = Vec::new();
    if let Some(aliases) = config.aliases.get(namespace) {
        if aliases.len() > 128 {
            return Err(AppError::InvalidParams("at most 128 scoped aliases".into()));
        }
        for token in tokens {
            if let Some(values) = aliases.get(token) {
                if values.len() > 8 {
                    return Err(AppError::InvalidParams(
                        "at most 8 expansions per alias".into(),
                    ));
                }
                for value in values {
                    if value.is_empty()
                        || value.len() > 128
                        || value.contains('\0')
                        || value.split_whitespace().count() != 1
                    {
                        return Err(AppError::InvalidParams(
                            "alias must be one bounded token".into(),
                        ));
                    }
                    let value = value.to_ascii_lowercase();
                    if !tokens.contains(&value) && !expanded.contains(&value) {
                        expanded.push(value);
                    }
                }
            }
        }
    }
    if expanded.len() > 16 || expanded.iter().map(String::len).sum::<usize>() > 512 {
        return Err(AppError::InvalidParams(
            "alias expansion budget exceeded".into(),
        ));
    }
    Ok(expanded)
}
pub async fn execute<S: TextMemoryStore + EnhancedMemoryStore + Sync>(
    store: &S,
    input: EnhancedRecallInput,
    config: &EnhancedRecallConfig,
) -> Result<EnhancedRecallResult, AppError> {
    execute_with_embedding(store, input, config, None).await
}
pub async fn execute_with_embedding<S: TextMemoryStore + EnhancedMemoryStore + Sync>(
    store: &S,
    input: EnhancedRecallInput,
    config: &EnhancedRecallConfig,
    embedding: Option<&dyn EmbeddingPort>,
) -> Result<EnhancedRecallResult, AppError> {
    let literal = RecallMemoryInput {
        namespace: input.namespace.clone(),
        query: input.query.clone(),
        limit: input.limit,
    };
    literal.terms()?;
    if !(1024..=262144).contains(&input.max_bytes) {
        return Err(AppError::InvalidParams(
            "enhanced max_bytes must be 1024..262144".into(),
        ));
    }
    let tokens = tokenize(&input.query);
    if tokens.len() > 32 {
        return Err(AppError::InvalidParams(
            "enhanced query exceeds 32 tokens".into(),
        ));
    }
    let expanded = terms(config, input.namespace.as_str(), &tokens)?;
    let mut routes = vec![("literal".to_owned(), input.query.clone())];
    for (i, chunk) in tokens.chunks(8).enumerate() {
        routes.push((format!("tokens_{i}"), chunk.join(" ")));
    }
    for (i, chunk) in expanded.chunks(8).enumerate() {
        routes.push((format!("aliases_{i}"), chunk.join(" ")));
    }
    let mut candidates: BTreeMap<String, EnhancedRecallHit> = BTreeMap::new();
    let mut result = EnhancedRecallResult {
        namespace: input.namespace.as_str().into(),
        query: input.query.clone(),
        tokens,
        expanded_terms: expanded,
        strategy: "scoped_lexical_rrf_v1",
        interpretation: "ranking_not_truth_conditions_unknown_no_execution_authority",
        records: vec![],
        candidate_count: 0,
        candidate_limit_reached: false,
        unavailable_after_retrieval: 0,
        applicability_omissions: 0,
        byte_budget_omissions: 0,
        result_limit_omissions: 0,
        warnings: vec![],
        serialized_bytes: 0,
        max_bytes: input.max_bytes,
    };
    for (route, query) in routes {
        let route_input = RecallMemoryInput {
            namespace: input.namespace.clone(),
            query,
            limit: 100,
        };
        let route_terms = route_input.terms()?;
        let recalled = store
            .recall_text_exact(crate::ports::text_memory_store::TextMemoryQuery {
                scope: MemoryScope::for_namespace(input.namespace.clone()),
                terms: route_terms.clone(),
                limit: 100,
            })
            .await?;
        result.candidate_limit_reached |= recalled.has_more;
        if let Some(warning) = recalled.index_warning
            && !result.warnings.contains(&warning)
        {
            result.warnings.push(warning);
        }
        for (rank, hit) in recalled.hits.into_iter().enumerate() {
            let reference = match hit.reference {
                crate::ports::text_memory_store::TextMemoryReference::Claim(id) => {
                    format!("claim:{id}")
                }
                crate::ports::text_memory_store::TextMemoryReference::Event(id) => {
                    format!("event:{id}")
                }
            };
            let Some(record) = store
                .hydrate_enhanced_record(
                    MemoryScope::for_namespace(input.namespace.clone()),
                    &reference,
                )
                .await?
            else {
                result.unavailable_after_retrieval += 1;
                continue;
            };
            // Selection and hydration are separate snapshots: do not assign stale lexical evidence.
            if !matches_route(&record, &route_terms) {
                result.unavailable_after_retrieval += 1;
                continue;
            }
            let digest = content_digest(&record)?;
            let item = candidates
                .entry(reference.clone())
                .or_insert(EnhancedRecallHit {
                    reference,
                    content_digest: digest.clone(),
                    routes: BTreeMap::new(),
                    rrf_score: 0.0,
                    applicability: "unknown_no_declared_conditions",
                    declared_applicability: None,
                    record,
                });
            if item.content_digest != digest {
                result
                    .warnings
                    .push("candidate_changed_between_routes".into());
                continue;
            }
            item.routes.insert(route.clone(), rank + 1);
        }
    }
    // Independent scope-first authoritative window enables semantic-only matches.
    // Window exhaustion is reported; this is never a global top-k guarantee.
    if embedding.is_some() {
        let page = store
            .scan_enhanced_records(MemoryScope::for_namespace(input.namespace.clone()), 64)
            .await?;
        result.candidate_limit_reached |= page.has_more;
        if page.oversized > 0 {
            result.warnings.push(format!(
                "semantic_scan_oversized_records:{}",
                page.oversized
            ));
        }
        for record in page.records {
            let reference = record_reference(&record).to_owned();
            let digest = content_digest(&record)?;
            candidates
                .entry(reference.clone())
                .or_insert(EnhancedRecallHit {
                    reference,
                    content_digest: digest,
                    routes: BTreeMap::new(),
                    rrf_score: 0.0,
                    applicability: "unknown_no_declared_conditions",
                    declared_applicability: None,
                    record,
                });
        }
    }
    // Reject outside declared intervals before any optional embedding transmission.
    let mut applicable = BTreeMap::new();
    for (id, mut hit) in candidates {
        if apply_metadata(store, &input.namespace, &mut hit).await? {
            applicable.insert(id, hit);
        } else {
            result.applicability_omissions += 1;
        }
    }
    let mut candidates = applicable;
    if let Some(port) = embedding {
        let dimensions = port.profile().dimensions;
        let mut refs = Vec::new();
        let mut total = 0usize;
        // Stable bounded selection; preflight borrowed source lengths before allocating text.
        for (id, hit) in &candidates {
            let bytes = record_text_len(&hit.record);
            if refs.len() == 64 {
                result.candidate_limit_reached = true;
                break;
            }
            if bytes > 32768 || total.saturating_add(bytes) > 131072 {
                result
                    .warnings
                    .push("embedding_text_budget_skipped_record".into());
                continue;
            }
            refs.push(id.clone());
            total += bytes;
        }
        if !refs.is_empty() && (1..=8192).contains(&dimensions) {
            let texts: Vec<_> = refs
                .iter()
                .map(|id| record_text(&candidates[id].record))
                .collect();
            let vectors = tokio::time::timeout(std::time::Duration::from_millis(5000), async {
                let docs = port
                    .embed(&input.namespace, EmbeddingPurpose::Document, &texts)
                    .await?;
                let query = port
                    .embed(
                        &input.namespace,
                        EmbeddingPurpose::Query,
                        std::slice::from_ref(&input.query),
                    )
                    .await?;
                Ok::<_, AppError>((docs, query))
            })
            .await;
            match vectors {
                Ok(Ok((docs, queries)))
                    if docs.len() == refs.len()
                        && queries.len() == 1
                        && docs
                            .iter()
                            .chain(&queries)
                            .all(|v| v.len() == dimensions && cosine(v, v).is_some()) =>
                {
                    let query = &queries[0];
                    let mut ranks = Vec::new();
                    for (i, doc) in docs.iter().enumerate() {
                        if let Some(score) = cosine(doc, query)
                            && score > 0.0
                        {
                            ranks.push((score, refs[i].clone()));
                        }
                    }
                    ranks.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
                    for (rank, (_, id)) in ranks.into_iter().enumerate() {
                        candidates
                            .get_mut(&id)
                            .unwrap()
                            .routes
                            .insert("local_embedding".into(), rank + 1);
                    }
                }
                _ => result
                    .warnings
                    .push("embedding_unavailable_lexical_results_retained".into()),
            }
        } else if dimensions == 0 || dimensions > 8192 {
            result
                .warnings
                .push("embedding_profile_dimensions_invalid_lexical_results_retained".into());
        }
    }
    // Scan-only records have no retrieval evidence when the embedding route failed.
    candidates.retain(|_, hit| !hit.routes.is_empty());
    if let Some(port) = embedding {
        result.warnings.extend(port.take_warnings());
    }
    result.warnings.sort();
    result.warnings.dedup();
    result.candidate_count = candidates.len();
    let mut hydrated = Vec::new();
    for (_, mut candidate) in candidates {
        if let Some(current) = store
            .hydrate_enhanced_record(
                MemoryScope::for_namespace(input.namespace.clone()),
                &candidate.reference,
            )
            .await?
            && content_digest(&current)? == candidate.content_digest
            && apply_metadata(store, &input.namespace, &mut candidate).await?
        {
            candidate.record = current;
            candidate.rrf_score = candidate
                .routes
                .values()
                .map(|rank| 1.0 / (60.0 + *rank as f64))
                .sum();
            hydrated.push(candidate);
            continue;
        }
        result.unavailable_after_retrieval += 1;
    }
    hydrated.sort_by(|a, b| {
        b.rrf_score
            .total_cmp(&a.rrf_score)
            .then(a.reference.cmp(&b.reference))
    });
    result.byte_budget_omissions = hydrated.len();
    result.result_limit_omissions = hydrated.len();
    if result.measure()? > input.max_bytes {
        return Err(AppError::InvalidParams(
            "enhanced metadata exceeds byte budget".into(),
        ));
    }
    for candidate in hydrated {
        if result.records.len() >= input.limit {
            result.byte_budget_omissions -= 1;
            continue;
        }
        result.result_limit_omissions -= 1;
        result.records.push(candidate);
        result.byte_budget_omissions -= 1;
        if result.measure()? > input.max_bytes {
            result.records.pop();
            result.byte_budget_omissions += 1;
            result.measure()?;
        }
    }
    result.measure()?;
    Ok(result)
}
fn cosine(a: &[f32], b: &[f32]) -> Option<f64> {
    if a.is_empty() || a.len() != b.len() || a.iter().chain(b).any(|v| !v.is_finite()) {
        return None;
    }
    let dot: f64 = a.iter().zip(b).map(|(x, y)| *x as f64 * *y as f64).sum();
    let norm = |x: &[f32]| x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>().sqrt();
    let denom = norm(a) * norm(b);
    (denom > 0.0).then_some(dot / denom)
}
async fn apply_metadata<S: TextMemoryStore + Sync>(
    store: &S,
    namespace: &Namespace,
    hit: &mut EnhancedRecallHit,
) -> Result<bool, AppError> {
    hit.declared_applicability = store
        .recall_applicability(namespace.as_str(), &hit.reference)
        .await?;
    if let Some(metadata) = &hit.declared_applicability {
        if metadata.content_digest != hit.content_digest {
            hit.applicability = "declaration_for_other_content_digest_unknown";
            return Ok(true);
        }
        let state = metadata.time_state(chrono::Utc::now())?;
        if state == "expired" || state == "not_yet_valid" {
            return Ok(false);
        }
        hit.applicability = if metadata.conditions.is_empty() && metadata.environment.is_empty() {
            state
        } else {
            "declared_conditions_not_evaluated"
        };
    }
    Ok(true)
}

fn record_text_len(record: &SearchMemoryRecord) -> usize {
    match record {
        SearchMemoryRecord::Claim {
            subject,
            predicate,
            object,
            ..
        } => subject
            .len()
            .saturating_add(predicate.len())
            .saturating_add(object.len())
            .saturating_add(2),
        SearchMemoryRecord::Event { summary, .. } => summary.len(),
        _ => 0,
    }
}

fn matches_route(record: &SearchMemoryRecord, terms: &[String]) -> bool {
    let fields = match record {
        SearchMemoryRecord::Claim {
            subject,
            predicate,
            object,
            ..
        } => vec![subject.as_str(), predicate.as_str(), object.as_str()],
        SearchMemoryRecord::Event { summary, .. } => vec![summary.as_str()],
        _ => vec![],
    };
    fields.iter().any(|field| {
        let lower = field.to_ascii_lowercase();
        terms.iter().any(|term| lower.contains(term))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aliases_are_scoped_single_hop() {
        let mut c = EnhancedRecallConfig::default();
        c.aliases.insert(
            "project/a".into(),
            BTreeMap::from([
                ("car".into(), vec!["auto".into()]),
                ("auto".into(), vec!["vehicle".into()]),
            ]),
        );
        assert_eq!(
            terms(&c, "project/a", &["car".into()]).unwrap(),
            vec!["auto"]
        );
        assert!(terms(&c, "project/b", &["car".into()]).unwrap().is_empty());
    }
    #[test]
    fn rejects_invalid_vectors() {
        assert_eq!(cosine(&[1.0, 0.0], &[1.0, 0.0]), Some(1.0));
        assert!(cosine(&[f32::NAN], &[1.0]).is_none());
        assert!(cosine(&[0.0], &[1.0]).is_none());
    }
    #[test]
    fn chinese_segmentation_is_not_whitespace_only() {
        let words = tokenize("北京咖啡与任务记忆");
        assert!(words.len() > 1);
        assert!(words.iter().any(|w| w == "咖啡"));
    }
}
