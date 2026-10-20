//! Host-selected, bounded semantic selection. Knowledge only; no adoption or execution.
use crate::{
    adapters::sqlite::semantic_index::{SemanticIndex, SemanticIndexHit, SemanticSourceKind},
    application::{
        build_task_context::BuildTaskContextInput, selection_source::SelectionSource,
        task::OptionalTaskSelection,
    },
    domain::recall_applicability::{EnvironmentFacts, EnvironmentMatch, match_environment},
    error::AppError,
};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Serialize)]
struct SelectionEvidence {
    reference: String,
    content_digest: String,
    version: Option<i64>,
    score: f64,
    applicability: &'static str,
    environment_match: EnvironmentMatch,
    declared_applicability: Option<crate::domain::recall_applicability::RecallApplicability>,
}
#[derive(Serialize)]
struct KindCoverage {
    examined: usize,
    unexamined_remaining: bool,
}
#[derive(Serialize)]
struct Report {
    strategy: &'static str,
    ranking: &'static str,
    sampling: &'static str,
    authority: &'static str,
    index_generation: Option<String>,
    source_generation: Option<String>,
    examined: usize,
    index_omissions: u64,
    kind_coverage: BTreeMap<&'static str, KindCoverage>,
    unexamined_remaining: bool,
    not_selected: BTreeMap<String, usize>,
    selected: Vec<SelectionEvidence>,
}
fn json<T: Serialize>(value: &T) -> Result<serde_json::Value, AppError> {
    serde_json::to_value(value)
        .map_err(|_| AppError::Message("unified selection encoding failed".into()))
}
fn measure(value: &OptionalTaskSelection) -> Result<usize, AppError> {
    serde_json::to_vec(value)
        .map(|v| v.len())
        .map_err(|_| AppError::Message("unified selection encoding failed".into()))
}
fn count(report: &mut Report, reason: &str, n: usize) {
    *report.not_selected.entry(reason.to_owned()).or_default() += n;
}
/// No rebuild or document embedding occurs here. Query embedding is prepared once.
/// At most 64 vectors per kind (256 total) are inspected; coverage is explicit.
pub async fn execute(
    index: &SemanticIndex,
    input: BuildTaskContextInput,
    environment: &EnvironmentFacts,
) -> Result<OptionalTaskSelection, AppError> {
    crate::domain::recall_applicability::validate_environment(environment)?;
    if !(1..=32).contains(&input.limit) || !(1024..=262144).contains(&input.max_bytes) {
        return Err(AppError::InvalidParams(
            "unified selection requires limit 1..32 and bytes 1024..262144".into(),
        ));
    }
    let mut result = OptionalTaskSelection {
        related_memory: None,
        experiences: vec![],
        episodes: vec![],
        report: None,
    };
    let mut report = Report {
        strategy: "unified_semantic_index_v2",
        ranking: "bounded_partial_index_scan",
        sampling: "up_to_64_reference_ordered_vectors_per_source_kind",
        authority: "knowledge_only_no_execution_authority",
        index_generation: None,
        source_generation: None,
        examined: 0,
        index_omissions: 0,
        kind_coverage: BTreeMap::new(),
        unexamined_remaining: false,
        not_selected: BTreeMap::new(),
        selected: vec![],
    };
    let prepared = match index.prepare_query(&input.namespace, &input.query).await {
        Ok(query) => query,
        Err(_) => {
            report.unexamined_remaining = true;
            count(
                &mut report,
                "index_unavailable_stale_incomplete_or_query_failed",
                1,
            );
            result.report = Some(json(&report)?);
            return Ok(result);
        }
    };
    let mut hits: Vec<SemanticIndexHit> = vec![];
    for kind in SemanticSourceKind::ALL {
        let page = index
            .search_prepared_kind(&input.namespace, &prepared, kind, 64)
            .await?;
        report.index_generation = Some(page.status.generation);
        report.source_generation = Some(page.status.source_generation);
        report.index_omissions = page.status.omitted;
        report.examined += page.scanned;
        report.unexamined_remaining |= page.has_more;
        report.kind_coverage.insert(
            kind.name(),
            KindCoverage {
                examined: page.scanned,
                unexamined_remaining: page.has_more,
            },
        );
        for (reason, amount) in page.omission_reasons {
            count(&mut report, &reason, amount);
        }
        hits.extend(page.hits);
        hits.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then(a.reference.cmp(&b.reference))
        });
        let mut retained_bytes = 0usize;
        let mut retained = vec![];
        for hit in hits {
            let bytes = serde_json::to_vec(&hit)
                .map_err(|_| AppError::Message("selection encoding failed".into()))?
                .len();
            if retained_bytes.saturating_add(bytes) <= 262144 {
                retained_bytes += bytes;
                retained.push(hit);
            } else {
                count(&mut report, "candidate_retention_byte_budget", 1);
            }
        }
        hits = retained;
    }
    if !report.unexamined_remaining
        && !report
            .not_selected
            .contains_key("candidate_retention_byte_budget")
    {
        report.ranking = "global_cosine_over_indexed_eligible_sources";
    }
    hits.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(a.reference.cmp(&b.reference))
    });
    let mut memory = vec![];
    for hit in hits {
        let environment_match = match &hit.declared_applicability {
            Some(declaration) if declaration.content_digest == hit.content_digest => {
                match_environment(&declaration.typed_environment, environment)?
            }
            _ => EnvironmentMatch::Unknown,
        };
        if environment_match == EnvironmentMatch::Mismatch {
            count(&mut report, "environment_mismatch", 1);
            continue;
        }
        if report.selected.len() >= input.limit {
            count(&mut report, "result_limit", 1);
            continue;
        }
        let evidence = SelectionEvidence {
            reference: hit.reference,
            content_digest: hit.content_digest,
            version: hit.record.version(),
            score: hit.score,
            applicability: hit.applicability,
            environment_match,
            declared_applicability: hit.declared_applicability,
        };
        report.selected.push(evidence);
        let kind = match hit.record {
            SelectionSource::Memory(record) => {
                memory.push(record);
                0
            }
            SelectionSource::Episode(episode) => {
                result.episodes.push(episode);
                1
            }
            SelectionSource::Experience(experience) => {
                result.experiences.push(experience);
                2
            }
        };
        result.related_memory = (!memory.is_empty()).then(|| json(&memory)).transpose()?;
        result.report = Some(json(&report)?);
        // Leave room for reason counters and decimal-width growth, not partial records.
        if measure(&result)?.saturating_add(256) > input.max_bytes {
            report.selected.pop();
            match kind {
                0 => {
                    memory.pop();
                }
                1 => {
                    result.episodes.pop();
                }
                _ => {
                    result.experiences.pop();
                }
            }
            count(&mut report, "byte_budget", 1);
        }
    }
    let final_status = index.status(&input.namespace).await?;
    if final_status.stale
        || !final_status.complete
        || report.index_generation.as_deref() != Some(final_status.generation.as_str())
    {
        return Err(AppError::Message(
            "unified index changed during selection".into(),
        ));
    }
    for selected in &report.selected {
        if let Some(declaration) = &selected.declared_applicability
            && declaration.content_digest == selected.content_digest
            && matches!(
                declaration.time_state(chrono::Utc::now())?,
                "expired" | "not_yet_valid"
            )
        {
            return Err(AppError::Message(
                "selected applicability changed during selection".into(),
            ));
        }
    }
    result.related_memory = (!memory.is_empty()).then(|| json(&memory)).transpose()?;
    result.report = Some(json(&report)?);
    if measure(&result)? > input.max_bytes {
        return Err(AppError::Message(
            "unified diagnostics cannot fit optional budget".into(),
        ));
    }
    Ok(result)
}
