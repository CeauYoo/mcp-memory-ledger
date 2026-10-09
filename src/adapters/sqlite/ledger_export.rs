//! No schema, PRAGMA-setting, index maintenance, receipt, or operation-log writes.
//! Size preflights execute before materializing TEXT/BLOB data in the SAME read
//! transaction as every subsequent fetch. Even corrupt large rows fail bounded.
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use sqlx::{Row, SqliteConnection, sqlite::SqliteRow};

use super::SqliteStore;
use crate::{
    domain::{
        claim::ClaimDraft,
        event::{Event, EventReference},
        experience::{ExperienceCandidate, PersistedEpisode},
        ledger_export::*,
        reflection_scope::{ReflectionScopeMetadata, ReflectionScopeStatus},
        types::{EventKind, MemoryScope, Mode, Namespace, Owner},
    },
    error::AppError,
    ports::{ClaimStatus, StoredClaim, StoredEvent, ledger_export_store::LedgerExportStore},
};

fn db<T>(value: Result<T, sqlx::Error>) -> Result<T, AppError> {
    // Never echo corrupt source values, raw SQL parameters, or foreign IDs.
    value.map_err(|_| AppError::Message("scoped export database read failed".into()))
}
fn decode<T: DeserializeOwned>(value: &str) -> Result<T, AppError> {
    serde_json::from_str(value).map_err(|_| invalid_graph())
}
fn timestamp(value: &str) -> Result<DateTime<Utc>, AppError> {
    DateTime::parse_from_rfc3339(value)
        .map(|v| v.with_timezone(&Utc))
        .map_err(|_| invalid_graph())
}
fn owner_name(owner: Owner) -> &'static str {
    match owner {
        Owner::Self_ => "self",
        Owner::User => "user",
        Owner::World => "world",
        Owner::Unknown => "unknown",
    }
}
fn parse_owner(value: &str) -> Result<Owner, AppError> {
    match value {
        "self" => Ok(Owner::Self_),
        "user" => Ok(Owner::User),
        "world" => Ok(Owner::World),
        _ => Err(invalid_graph()),
    }
}

struct Budget<'a> {
    request: &'a ExportMemoryRequest,
    records: usize,
    relations: usize,
    raw_bytes: usize,
}
impl<'a> Budget<'a> {
    fn new(request: &'a ExportMemoryRequest) -> Self {
        Self {
            request,
            records: 0,
            relations: 0,
            raw_bytes: 0,
        }
    }
}

/// `columns`/`from`/`order` are internal SQL fragments only. ?1/?2 always bind
/// the exact namespace and its derived canonical owner. No caller SQL is used.
async fn rows(
    conn: &mut SqliteConnection,
    budget: &mut Budget<'_>,
    columns: &[&str],
    from: &str,
    order: &str,
    relation: bool,
) -> Result<Vec<SqliteRow>, AppError> {
    let lengths = columns
        .iter()
        .map(|c| format!("coalesce(length(CAST({c} AS BLOB)),0)"))
        .collect::<Vec<_>>()
        .join("+");
    let sql = format!("SELECT count(*) AS n, total({lengths}) AS bytes FROM {from}");
    let owner = owner_name(budget.request.validate()?.derived_owner());
    let mut query = sqlx::query(&sql).bind(&budget.request.namespace);
    if from.contains("?2") {
        query = query.bind(owner);
    }
    let size = db(query.fetch_one(&mut *conn).await)?;
    let count: i64 = size.get("n");
    let bytes: f64 = size.get("bytes");
    if count < 0 || !bytes.is_finite() || bytes < 0.0 || bytes > budget.request.max_bytes as f64 {
        return Err(limit_error());
    }
    let count = usize::try_from(count).map_err(|_| limit_error())?;
    let used = if relation {
        &mut budget.relations
    } else {
        &mut budget.records
    };
    *used = used.checked_add(count).ok_or_else(limit_error)?;
    budget.raw_bytes = budget
        .raw_bytes
        .checked_add(bytes as usize)
        .ok_or_else(limit_error)?;
    if budget.records > budget.request.max_records
        || budget.relations > budget.request.max_relations
        || budget.raw_bytes > budget.request.max_bytes
    {
        return Err(limit_error());
    }
    let sql = format!("SELECT {} FROM {from} ORDER BY {order}", columns.join(","));
    let mut query = sqlx::query(&sql).bind(&budget.request.namespace);
    if from.contains("?2") {
        query = query.bind(owner);
    }
    db(query.fetch_all(&mut *conn).await)
}

const REFLECTION_SCOPE: &str = "reflections r WHERE r.scope_status IN ('verified','legacy_unambiguous') AND EXISTS (SELECT 1 FROM reflection_scopes s WHERE s.reflection_id=r.reflection_id AND s.role='origin' AND s.namespace=?1 AND s.owner=?2) AND NOT EXISTS (SELECT 1 FROM reflection_scopes s WHERE s.reflection_id=r.reflection_id AND (s.namespace<>?1 OR s.owner<>?2))";

#[async_trait]
impl LedgerExportStore for SqliteStore {
    async fn export_memory(&self, request: ExportMemoryRequest) -> Result<LedgerExport, AppError> {
        let namespace = request.validate()?;
        let mut tx = db(self.pool.begin().await)?;
        // The first schema read pins the SQLite snapshot; every preflight/fetch
        // below uses the same connection, even during a concurrent WAL write.
        let version: i64 = db(
            sqlx::query_scalar("SELECT max(version) FROM schema_migrations")
                .fetch_one(&mut *tx)
                .await,
        )?;
        let mut result = LedgerExport::empty(namespace.clone(), version);
        let mut budget = Budget::new(&request);
        let event_rows = rows(
            &mut tx,
            &mut budget,
            &[
                "event_id",
                "recorded_at",
                "observed_at",
                "owner",
                "namespace",
                "kind",
                "summary",
                "feedback_json",
            ],
            "events WHERE namespace=?1 AND owner=?2",
            "event_id",
            false,
        )
        .await?;
        for row in event_rows {
            let kind = match row.get::<&str, _>("kind") {
                "observation" => EventKind::Observation,
                "conversation" => EventKind::Conversation,
                "action" => EventKind::Action,
                "reflection" => EventKind::Reflection,
                _ => return Err(invalid_graph()),
            };
            let mut event = Event::new_with_namespace(
                result.owner,
                namespace.clone(),
                kind,
                row.get::<String, _>("summary"),
            )?;
            if let Some(json) = row.get::<Option<String>, _>("feedback_json") {
                event = event
                    .with_feedback(decode(&json)?)
                    .map_err(|_| invalid_graph())?;
            }
            result.events.push(
                StoredEvent::new(
                    row.get("event_id"),
                    timestamp(row.get("recorded_at"))?,
                    event,
                )
                .with_observed_at(row.get("observed_at")),
            );
        }
        let claim_rows = rows(
            &mut tx,
            &mut budget,
            &[
                "claim_id",
                "owner",
                "namespace",
                "subject",
                "predicate",
                "object",
                "mode",
                "status",
                "recorded_at",
                "observed_at",
            ],
            "claims WHERE namespace=?1 AND owner=?2",
            "claim_id",
            false,
        )
        .await?;
        for row in claim_rows {
            let mode = match row.get::<&str, _>("mode") {
                "observed" => Mode::Observed,
                "said" => Mode::Said,
                "acted" => Mode::Acted,
                "inferred" => Mode::Inferred,
                "draft" => Mode::Draft,
                _ => return Err(invalid_graph()),
            };
            let status = match row.get::<&str, _>("status") {
                "active" => ClaimStatus::Active,
                "disputed" => ClaimStatus::Disputed,
                "superseded" => ClaimStatus::Superseded,
                _ => return Err(invalid_graph()),
            };
            let claim = ClaimDraft::new_with_namespace(
                result.owner,
                namespace.clone(),
                row.get::<String, _>("subject"),
                row.get::<String, _>("predicate"),
                row.get::<String, _>("object"),
                mode,
            );
            result.claims.push(
                StoredClaim::new(row.get("claim_id"), claim, status).with_temporal_metadata(
                    row.get::<Option<String>, _>("recorded_at")
                        .as_deref()
                        .map(timestamp)
                        .transpose()?,
                    row.get("observed_at"),
                ),
            );
        }
        // Preflight every original evidence edge for scoped Claims, including
        // unsafe endpoints. Filtering before this fetch would both hide missing
        // provenance and let arbitrarily many unsafe edges bypass the budget.
        for row in rows(&mut tx, &mut budget, &["l.claim_id", "l.event_id"],
            "evidence_links l JOIN claims c ON c.claim_id=l.claim_id WHERE c.namespace=?1 AND c.owner=?2", "l.claim_id,l.event_id", true).await? {
            result.evidence_links.push(ExportEvidenceLink {
                claim_id: row.get("claim_id"), event_id: row.get("event_id"),
            });
        }
        close_claim_event_sources(&mut result);
        let claim_ids: BTreeSet<_> = result.claims.iter().map(|c| c.claim_id.clone()).collect();
        let event_ids: BTreeSet<_> = result.events.iter().map(|e| e.event_id.clone()).collect();

        for row in rows(&mut tx, &mut budget, &["l.episode_reference", "l.event_id"],
            "episode_events l JOIN events e ON e.event_id=l.event_id WHERE e.namespace=?1 AND e.owner=?2 AND NOT EXISTS (SELECT 1 FROM episode_events x LEFT JOIN events y ON y.event_id=x.event_id WHERE x.episode_reference=l.episode_reference AND (y.event_id IS NULL OR y.namespace<>?1 OR y.owner<>?2))", "l.episode_reference,l.event_id", true).await? {
            result.episode_memberships.push(ExportEpisodeMembership { episode_reference: row.get("episode_reference"), event_id: row.get("event_id") });
        }
        let unsafe_episode_refs: BTreeSet<_> = result
            .episode_memberships
            .iter()
            .filter(|m| !event_ids.contains(&m.event_id))
            .map(|m| m.episode_reference.clone())
            .collect();
        result
            .episode_memberships
            .retain(|m| !unsafe_episode_refs.contains(&m.episode_reference));

        let reflection_rows = rows(
            &mut tx,
            &mut budget,
            &[
                "r.reflection_id",
                "r.recorded_at",
                "r.summary",
                "r.superseded_claim_id",
                "r.replacement_claim_id",
                "r.supporting_evidence_event_ids",
                "r.scope_status",
                "r.evidence_normalized",
            ],
            REFLECTION_SCOPE,
            "r.reflection_id",
            false,
        )
        .await?;
        let mut scopes = BTreeMap::<String, ReflectionScopeMetadata>::new();
        let scope_from = format!(
            "reflection_scopes s JOIN (SELECT r.reflection_id FROM {REFLECTION_SCOPE}) visible ON visible.reflection_id=s.reflection_id WHERE s.namespace=?1 AND s.owner=?2"
        );
        for row in rows(
            &mut tx,
            &mut budget,
            &["s.reflection_id", "s.role", "s.owner", "s.namespace"],
            &scope_from,
            "s.reflection_id,s.role,s.owner,s.namespace",
            true,
        )
        .await?
        {
            let scope =
                MemoryScope::for_namespace(Namespace::parse(row.get::<String, _>("namespace"))?);
            if scope.owner() != Some(parse_owner(row.get("owner"))?) {
                return Err(invalid_graph());
            }
            let metadata = scopes.entry(row.get("reflection_id")).or_default();
            match row.get::<&str, _>("role") {
                "origin" => metadata.origin_scopes.push(scope),
                "affected" => metadata.affected_scopes.push(scope),
                _ => return Err(invalid_graph()),
            }
        }
        let mut normalized = BTreeMap::<String, Vec<String>>::new();
        let evidence_from = format!(
            "reflection_evidence re JOIN (SELECT r.reflection_id FROM {REFLECTION_SCOPE}) visible ON visible.reflection_id=re.reflection_id WHERE ?1=?1 AND ?2=?2"
        );
        for row in rows(
            &mut tx,
            &mut budget,
            &["re.reflection_id", "re.event_id"],
            &evidence_from,
            "re.reflection_id,re.event_id",
            true,
        )
        .await?
        {
            normalized
                .entry(row.get("reflection_id"))
                .or_default()
                .push(row.get("event_id"));
        }
        for row in reflection_rows {
            let id: String = row.get("reflection_id");
            let Some(mut scope) = scopes.remove(&id) else {
                continue;
            };
            scope.status = match row.get::<&str, _>("scope_status") {
                "verified" => ReflectionScopeStatus::Verified,
                "legacy_unambiguous" => ReflectionScopeStatus::LegacyUnambiguous,
                _ => continue,
            };
            let mut evidence: Vec<String> = decode(row.get("supporting_evidence_event_ids"))?;
            evidence.sort();
            if evidence.windows(2).any(|w| w[0] == w[1]) {
                continue;
            }
            // Relational provenance must agree with durable JSON; neither silently
            // overwrites the other if the database was externally corrupted.
            if row.get::<i64, _>("evidence_normalized") != 1
                || normalized.remove(&id).unwrap_or_default() != evidence
            {
                continue;
            }
            let old: Option<String> = row.get("superseded_claim_id");
            let new: Option<String> = row.get("replacement_claim_id");
            if !scope.permits_targetless_read(&MemoryScope::for_namespace(namespace.clone()))
                || evidence.iter().any(|e| !event_ids.contains(e))
                || old.as_ref().is_some_and(|c| !claim_ids.contains(c))
                || new.as_ref().is_some_and(|c| !claim_ids.contains(c))
            {
                continue;
            }
            result.reflections.push(ExportReflection {
                reflection_id: id,
                recorded_at: timestamp(row.get("recorded_at"))?,
                scope,
                summary: row.get("summary"),
                superseded_claim_id: old,
                replacement_claim_id: new,
                supporting_evidence_event_ids: evidence,
            });
        }
        load_experience(&mut tx, &mut budget, &mut result, &event_ids).await?;
        result.finish(&request)?;
        // A read transaction is explicitly rolled back; no COMMIT hook or write
        // adapter can accidentally append a receipt for this operation.
        db(tx.rollback().await)?;
        Ok(result)
    }
}

/// Remove every record whose original structural source closure leaves the
/// selected graph. Reverse integer-index edges keep long chains linear and do
/// not clone a potentially large record identifier once per dependency.
fn close_claim_event_sources(result: &mut LedgerExport) {
    let event_count = result.events.len();
    let events: BTreeMap<_, _> = result
        .events
        .iter()
        .enumerate()
        .map(|(i, event)| (event.event_id.as_str(), i))
        .collect();
    let claims: BTreeMap<_, _> = result
        .claims
        .iter()
        .enumerate()
        .map(|(i, claim)| (claim.claim_id.as_str(), event_count + i))
        .collect();
    let mut retained = vec![true; event_count + result.claims.len()];
    let mut dependents = vec![Vec::new(); retained.len()];
    let mut pending = VecDeque::new();
    let mut require = |record: usize, source: Option<usize>| {
        if let Some(source) = source {
            dependents[source].push(record);
        } else {
            pending.push_back(record);
        }
    };
    for (i, event) in result.events.iter().enumerate() {
        if let Some(feedback) = event.event.feedback() {
            for source in &feedback.evidence_refs {
                require(i, events.get(source.event_id()).copied());
            }
            if let Some(id) = feedback.observed_target.strip_prefix("claim:") {
                require(i, claims.get(id).copied());
            }
            if let Some(id) = feedback.observed_target.strip_prefix("event:") {
                require(i, events.get(id).copied());
            }
        }
    }
    for link in &result.evidence_links {
        // These rows came from the exact scoped-Claim join above.
        if let Some(&claim) = claims.get(link.claim_id.as_str()) {
            require(claim, events.get(link.event_id.as_str()).copied());
        }
    }
    while let Some(record) = pending.pop_front() {
        if std::mem::replace(&mut retained[record], false) {
            pending.extend(dependents[record].iter().copied());
        }
    }
    // A genuine source-less Claim has no missing dependency and remains intact.
    let mut i = 0;
    result.events.retain(|_| {
        let keep = retained[i];
        i += 1;
        keep
    });
    result.claims.retain(|_| {
        let keep = retained[i];
        i += 1;
        keep
    });
    let retained_claims: BTreeSet<_> = result
        .claims
        .iter()
        .map(|claim| claim.claim_id.as_str())
        .collect();
    result
        .evidence_links
        .retain(|link| retained_claims.contains(link.claim_id.as_str()));
}

async fn load_experience(
    conn: &mut SqliteConnection,
    budget: &mut Budget<'_>,
    result: &mut LedgerExport,
    event_ids: &BTreeSet<String>,
) -> Result<(), AppError> {
    let episode_rows = rows(
        conn,
        budget,
        &["episode_id", "namespace", "recorded_at", "payload_json"],
        "experience_episodes WHERE namespace=?1",
        "episode_id",
        false,
    )
    .await?;
    let mut episode_sources = BTreeMap::<String, BTreeSet<String>>::new();
    for row in rows(conn, budget, &["s.episode_id", "s.event_id"], "experience_episode_sources s JOIN experience_episodes e ON e.episode_id=s.episode_id WHERE e.namespace=?1", "s.episode_id,s.event_id", true).await? {
        episode_sources.entry(row.get("episode_id")).or_default().insert(row.get("event_id"));
    }
    for row in episode_rows {
        let episode: PersistedEpisode = decode(row.get("payload_json"))?;
        if episode.episode_id != row.get::<String, _>("episode_id")
            || episode.namespace != result.namespace
            || episode.recorded_at != timestamp(row.get("recorded_at"))?
        {
            continue;
        }
        if episode.content.validate().is_err() {
            continue;
        }
        let sources = episode
            .content
            .source_event_refs
            .iter()
            .map(|r| EventReference::parse(r).map(|r| r.event_id().to_owned()))
            .collect::<Result<BTreeSet<_>, _>>()?;
        if sources.iter().all(|e| event_ids.contains(e))
            && episode_sources
                .remove(&episode.episode_id)
                .unwrap_or_default()
                == sources
        {
            result.experience_episodes.push(episode);
        }
    }
    let episode_ids: BTreeSet<_> = result
        .experience_episodes
        .iter()
        .map(|e| e.episode_id.clone())
        .collect();
    let heads = rows(
        conn,
        budget,
        &["candidate_id", "current_version"],
        "experience_candidates WHERE namespace=?1",
        "candidate_id",
        false,
    )
    .await?;
    let version_rows = rows(conn, budget, &["v.candidate_id", "v.version", "v.recorded_at", "v.status", "v.kind", "v.payload_json", "v.change_kind", "v.rollback_target_version"], "experience_candidate_versions v JOIN experience_candidates c ON c.candidate_id=v.candidate_id WHERE c.namespace=?1", "v.candidate_id,v.version", false).await?;
    let mut sources = BTreeMap::<(String, i64), BTreeSet<String>>::new();
    for row in rows(conn, budget, &["s.candidate_id", "s.version", "s.episode_id"], "experience_candidate_sources s JOIN experience_candidates c ON c.candidate_id=s.candidate_id WHERE c.namespace=?1", "s.candidate_id,s.version,s.episode_id", true).await? {
        sources.entry((row.get("candidate_id"),row.get("version"))).or_default().insert(row.get("episode_id"));
    }
    let mut versions = BTreeMap::<String, Vec<ExperienceCandidate>>::new();
    let mut unsafe_candidates = BTreeSet::new();
    for row in version_rows {
        let id: String = row.get("candidate_id");
        let version: i64 = row.get("version");
        let candidate: ExperienceCandidate = decode(row.get("payload_json"))?;
        let declared: BTreeSet<_> = candidate
            .content
            .source_episode_ids
            .iter()
            .cloned()
            .collect();
        if candidate.candidate_id != id
            || candidate.version != version
            || candidate.namespace != result.namespace
            || candidate.recorded_at != timestamp(row.get("recorded_at"))?
            || candidate.status.as_str() != row.get::<&str, _>("status")
            || candidate.content.kind.as_str() != row.get::<&str, _>("kind")
            || candidate.change_kind.as_str() != row.get::<&str, _>("change_kind")
            || candidate.rollback_target_version
                != row.get::<Option<i64>, _>("rollback_target_version")
            || candidate.content.validate().is_err()
            || declared.iter().any(|e| !episode_ids.contains(e))
            || sources.remove(&(id.clone(), version)).unwrap_or_default() != declared
        {
            unsafe_candidates.insert(id.clone());
        }
        versions.entry(id).or_default().push(candidate);
    }
    for row in heads {
        let id: String = row.get("candidate_id");
        let current: i64 = row.get("current_version");
        let history = versions.remove(&id).unwrap_or_default();
        // Include the entire historical source closure or quarantine the entire
        // candidate; never call a fragment a complete version history.
        let complete = current > 0
            && history.len() as i64 == current
            && history.iter().enumerate().all(|(i, v)| {
                v.version == i as i64 + 1
                    && v.previous_version == (i > 0).then_some(i as i64)
                    && v.rollback_target_version
                        .is_none_or(|target| target > 0 && target < v.version)
            });
        if unsafe_candidates.contains(&id) || !complete {
            continue;
        }
        result.experience_heads.push(ExportExperienceHead {
            candidate_id: id,
            current_version: current,
        });
        result.experience_versions.extend(history);
    }
    Ok(())
}
