//! Bounded authoritative hydration for the unified knowledge projection.
//!
//! All text and all normalized provenance edges are preflighted before any
//! payload is materialized. A nested transaction makes the check and hydration
//! one snapshot, including when called outside an existing read transaction.
use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use sqlx::{Connection, Row, SqliteConnection, sqlite::SqliteRow};

use super::{enhanced_reads, store::rows::owner_as_str};
use crate::{
    application::selection_source::SelectionSource,
    domain::{
        event::EventReference,
        experience::{
            ExperienceCandidate, ExperienceChangeKind, ExperienceStatus, PersistedEpisode,
            identifier, validate_version,
        },
        types::Namespace,
    },
    error::AppError,
};

const RECORD_BYTES: usize = 32 * 1024;
const EDGE_LIMIT: usize = 64;

fn error(value: impl std::fmt::Display) -> AppError {
    AppError::Message(format!("selection authoritative read: {value}"))
}

#[derive(Clone, Copy)]
enum Field {
    Text(&'static str),
    Integer(&'static str),
    OptionalInteger(&'static str),
}
impl Field {
    fn name(self) -> &'static str {
        match self {
            Self::Text(name) | Self::Integer(name) | Self::OptionalInteger(name) => name,
        }
    }
    fn accepts(self, kind: &str) -> bool {
        match self {
            Self::Text(_) => kind == "text",
            Self::Integer(_) => kind == "integer",
            Self::OptionalInteger(_) => kind == "integer" || kind == "null",
        }
    }
}
struct Plan {
    sql: String,
    fields: Vec<Field>,
    edges: bool,
}
impl Plan {
    fn new(sql: String, fields: &[Field], edges: bool) -> Self {
        Self {
            sql,
            fields: fields.to_vec(),
            edges,
        }
    }
    fn limit(&self) -> usize {
        if self.edges { EDGE_LIMIT + 1 } else { 1 }
    }
}

fn plans(candidate: bool) -> Vec<Plan> {
    use Field::{Integer as I, OptionalInteger as O, Text as T};
    let episode_fields = [
        T("episode_id"),
        T("namespace"),
        T("recorded_at"),
        T("payload_json"),
    ];
    let edge_fields = [T("episode_id"), T("event_id"), I("valid_event")];
    if !candidate {
        let root =
            "WITH target AS (SELECT * FROM experience_episodes WHERE rowid=?1 AND namespace=?2) ";
        return vec![
            Plan::new(
                format!("{root}SELECT episode_id,namespace,recorded_at,payload_json FROM target"),
                &episode_fields,
                false,
            ),
            Plan::new(
                format!(
                    "{root}SELECT s.episode_id,s.event_id,CASE WHEN e.event_id IS NOT NULL AND e.namespace=?2 AND e.owner=?3 THEN 1 ELSE 0 END AS valid_event FROM target t JOIN experience_episode_sources s ON s.episode_id=t.episode_id LEFT JOIN events e ON e.event_id=s.event_id"
                ),
                &edge_fields,
                true,
            ),
        ];
    }
    let root = "WITH target AS (SELECT c.candidate_id,c.namespace,c.current_version,v.version,v.recorded_at,v.status,v.kind,v.payload_json,v.search_text,v.change_kind,v.rollback_target_version FROM experience_candidates c JOIN experience_candidate_versions v ON v.candidate_id=c.candidate_id AND v.version=c.current_version WHERE c.rowid=?1 AND c.namespace=?2 AND v.status='active') ";
    vec![
        Plan::new(
            format!("{root}SELECT * FROM target"),
            &[
                T("candidate_id"),
                T("namespace"),
                I("current_version"),
                I("version"),
                T("recorded_at"),
                T("status"),
                T("kind"),
                T("payload_json"),
                T("search_text"),
                T("change_kind"),
                O("rollback_target_version"),
            ],
            false,
        ),
        Plan::new(
            format!(
                "{root}SELECT s.episode_id,e.namespace,e.recorded_at,e.payload_json FROM target t JOIN experience_candidate_sources s ON s.candidate_id=t.candidate_id AND s.version=t.current_version LEFT JOIN experience_episodes e ON e.episode_id=s.episode_id AND e.namespace=?2"
            ),
            &episode_fields,
            true,
        ),
        Plan::new(
            format!(
                "{root}SELECT s.episode_id,s.event_id,CASE WHEN e.event_id IS NOT NULL AND e.namespace=?2 AND e.owner=?3 THEN 1 ELSE 0 END AS valid_event FROM target t JOIN experience_candidate_sources cs ON cs.candidate_id=t.candidate_id AND cs.version=t.current_version JOIN experience_episode_sources s ON s.episode_id=cs.episode_id LEFT JOIN events e ON e.event_id=s.event_id"
            ),
            &edge_fields,
            true,
        ),
    ]
}

/// Returns false for a whole oversized graph. Only fixed-size type strings and
/// integers cross the SQLite boundary during this phase.
async fn preflight(
    conn: &mut SqliteConnection,
    plans: &[Plan],
    namespace: &Namespace,
    rowid: i64,
    budget: usize,
) -> Result<bool, AppError> {
    let mut bytes = 0usize;
    let mut edges = 0usize;
    for plan in plans {
        let columns = plan
            .fields
            .iter()
            .enumerate()
            .map(|(i, field)| {
                let name = field.name();
                format!("typeof({name}) AS t{i},length(CAST({name} AS BLOB)) AS n{i}")
            })
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT {columns} FROM ({}) LIMIT {}",
            plan.sql,
            plan.limit()
        );
        let mut query = sqlx::query(&sql).bind(rowid).bind(namespace.as_str());
        if plan.sql.contains("?3") {
            query = query.bind(owner_as_str(namespace.derived_owner()));
        }
        let rows = query.fetch_all(&mut *conn).await.map_err(error)?;
        if plan.edges {
            edges = edges.saturating_add(rows.len());
            if edges > EDGE_LIMIT {
                return Ok(false);
            }
        }
        for row in rows {
            bytes = bytes.saturating_add(128);
            for (i, field) in plan.fields.iter().enumerate() {
                let kind: String = row.try_get(format!("t{i}").as_str()).map_err(error)?;
                if !field.accepts(&kind) {
                    return Err(error(
                        "invalid authoritative field type or missing scoped source",
                    ));
                }
                let length: Option<i64> = row.try_get(format!("n{i}").as_str()).map_err(error)?;
                let length = usize::try_from(length.unwrap_or(0)).map_err(error)?;
                bytes = bytes.saturating_add(length).saturating_add(16);
                if length > RECORD_BYTES || bytes > budget {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}

async fn materialize(
    conn: &mut SqliteConnection,
    plans: &[Plan],
    namespace: &Namespace,
    rowid: i64,
) -> Result<Vec<Vec<SqliteRow>>, AppError> {
    let mut groups = Vec::with_capacity(plans.len());
    for plan in plans {
        let sql = format!("{} LIMIT {}", plan.sql, plan.limit());
        let mut query = sqlx::query(&sql).bind(rowid).bind(namespace.as_str());
        if plan.sql.contains("?3") {
            query = query.bind(owner_as_str(namespace.derived_owner()));
        }
        let rows = query.fetch_all(&mut *conn).await.map_err(error)?;
        // Validate UTF-8 and numeric decoding explicitly; never use Row::get.
        for row in &rows {
            for field in &plan.fields {
                match field {
                    Field::Text(name) => {
                        let _: &str = row.try_get(*name).map_err(error)?;
                    }
                    Field::Integer(name) => {
                        let _: i64 = row.try_get(*name).map_err(error)?;
                    }
                    Field::OptionalInteger(name) => {
                        let _: Option<i64> = row.try_get(*name).map_err(error)?;
                    }
                }
            }
        }
        groups.push(rows);
    }
    Ok(groups)
}

fn same_time(recorded_at: DateTime<Utc>, row: &SqliteRow) -> Result<bool, AppError> {
    let timestamp: &str = row.try_get("recorded_at").map_err(error)?;
    Ok(DateTime::parse_from_rfc3339(timestamp)
        .map_err(error)?
        .with_timezone(&Utc)
        == recorded_at)
}

fn episode(row: &SqliteRow, namespace: &Namespace) -> Result<PersistedEpisode, AppError> {
    let payload: &str = row.try_get("payload_json").map_err(error)?;
    let record: PersistedEpisode = serde_json::from_str(payload).map_err(error)?;
    let id: &str = row.try_get("episode_id").map_err(error)?;
    let sql_namespace: &str = row.try_get("namespace").map_err(error)?;
    if record.episode_id != id
        || record.namespace != *namespace
        || sql_namespace != namespace.as_str()
        || !same_time(record.recorded_at, row)?
    {
        return Err(error("episode SQL/payload identity mismatch"));
    }
    identifier(&record.episode_id, "episode_id").map_err(error)?;
    record.content.validate().map_err(error)?;
    Ok(record)
}

fn event_edges(rows: &[SqliteRow]) -> Result<BTreeMap<String, BTreeSet<String>>, AppError> {
    let mut result: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for row in rows {
        if row.try_get::<i64, _>("valid_event").map_err(error)? != 1 {
            return Err(error(
                "source event does not have the canonical owner and exact namespace",
            ));
        }
        let episode_id: String = row.try_get("episode_id").map_err(error)?;
        let event_id: String = row.try_get("event_id").map_err(error)?;
        identifier(&event_id, "source_event_id").map_err(error)?;
        let reference = EventReference::parse(&event_id).map_err(|e| error(format!("{e:?}")))?;
        if reference.event_id() != event_id {
            return Err(error("non-normalized source event edge"));
        }
        if !result.entry(episode_id).or_default().insert(event_id) {
            return Err(error("duplicate normalized source event edge"));
        }
    }
    Ok(result)
}

fn validate_episode_edges(
    record: &PersistedEpisode,
    edges: &mut BTreeMap<String, BTreeSet<String>>,
) -> Result<(), AppError> {
    let expected = record
        .content
        .source_event_refs
        .iter()
        .map(|reference| {
            EventReference::parse(reference)
                .map(|reference| reference.event_id().to_owned())
                .map_err(|e| error(format!("{e:?}")))
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    if edges.remove(&record.episode_id).unwrap_or_default() != expected {
        return Err(error(
            "episode payload and normalized source edges disagree",
        ));
    }
    Ok(())
}

fn candidate(row: &SqliteRow, namespace: &Namespace) -> Result<ExperienceCandidate, AppError> {
    let payload: &str = row.try_get("payload_json").map_err(error)?;
    let record: ExperienceCandidate = serde_json::from_str(payload).map_err(error)?;
    let id: &str = row.try_get("candidate_id").map_err(error)?;
    let sql_namespace: &str = row.try_get("namespace").map_err(error)?;
    let current_version: i64 = row.try_get("current_version").map_err(error)?;
    let version: i64 = row.try_get("version").map_err(error)?;
    let status: &str = row.try_get("status").map_err(error)?;
    let kind: &str = row.try_get("kind").map_err(error)?;
    let search_text: &str = row.try_get("search_text").map_err(error)?;
    let change_kind: &str = row.try_get("change_kind").map_err(error)?;
    let rollback_target: Option<i64> = row.try_get("rollback_target_version").map_err(error)?;
    if record.candidate_id != id
        || record.namespace != *namespace
        || sql_namespace != namespace.as_str()
        || record.version != current_version
        || record.version != version
        || record.status != ExperienceStatus::Active
        || status != record.status.as_str()
        || kind != record.content.kind.as_str()
        || change_kind != record.change_kind.as_str()
        || rollback_target != record.rollback_target_version
        || !same_time(record.recorded_at, row)?
    {
        return Err(error(
            "candidate SQL/payload identity, status or version mismatch",
        ));
    }
    identifier(&record.candidate_id, "candidate_id").map_err(error)?;
    validate_version(record.version).map_err(error)?;
    record.content.validate().map_err(error)?;
    if search_text != record.content.search_text().to_lowercase()
        || record.previous_version != record.version.checked_sub(1).filter(|v| *v > 0)
        || (record.change_kind == ExperienceChangeKind::Created) != (record.version == 1)
        || record.rollback_target_version.is_some()
            != (record.change_kind == ExperienceChangeKind::RolledBack)
        || record
            .rollback_target_version
            .is_some_and(|target| target <= 0 || target >= record.previous_version.unwrap_or(0))
    {
        return Err(error(
            "candidate content or version metadata is inconsistent",
        ));
    }
    Ok(record)
}

// Preserve legacy memory codecs while applying the same 64-edge graph ceiling.
async fn memory_edges_fit(
    conn: &mut SqliteConnection,
    namespace: &Namespace,
    claim: bool,
    rowid: i64,
) -> Result<bool, AppError> {
    let parts = if claim {
        vec![
            "SELECT 1 FROM target t JOIN evidence_links l ON l.claim_id=t.claim_id JOIN events e ON e.event_id=l.event_id WHERE e.owner=?3 AND e.namespace=?2 LIMIT 65",
            "SELECT DISTINCT ee.episode_reference FROM target t JOIN evidence_links l ON l.claim_id=t.claim_id JOIN events e ON e.event_id=l.event_id JOIN episode_events ee ON ee.event_id=e.event_id WHERE e.owner=?3 AND e.namespace=?2 LIMIT 65",
            "SELECT 1 FROM target t JOIN reflections r ON r.superseded_claim_id=t.claim_id OR r.replacement_claim_id=t.claim_id LIMIT 65",
        ]
    } else {
        vec![
            "SELECT 1 FROM target t JOIN evidence_links l ON l.event_id=t.event_id JOIN claims c ON c.claim_id=l.claim_id WHERE c.owner=?3 AND c.namespace=?2 LIMIT 65",
            "SELECT 1 FROM target t JOIN episode_events ee ON ee.event_id=t.event_id LIMIT 65",
        ]
    };
    let table = if claim { "claims" } else { "events" };
    let active = if claim { " AND status='active'" } else { "" };
    let mut count = 0i64;
    for part in parts {
        let sql = format!(
            "WITH target AS (SELECT * FROM {table} WHERE rowid=?1 AND namespace=?2 AND owner=?3{active}) SELECT count(*) FROM ({part})"
        );
        count += sqlx::query_scalar::<_, i64>(&sql)
            .bind(rowid)
            .bind(namespace.as_str())
            .bind(owner_as_str(namespace.derived_owner()))
            .fetch_one(&mut *conn)
            .await
            .map_err(error)?;
        if count > EDGE_LIMIT as i64 {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn hydrate_snapshot(
    conn: &mut SqliteConnection,
    namespace: &Namespace,
    kind: i64,
    rowid: i64,
    budget: usize,
) -> Result<Option<SelectionSource>, AppError> {
    let budget = budget.min(RECORD_BYTES);
    if budget == 0 {
        return Ok(None);
    }
    if kind == 0 || kind == 1 {
        if !memory_edges_fit(conn, namespace, kind == 0, rowid).await? {
            return Ok(None);
        }
        return Ok(enhanced_reads::hydrate(
            conn,
            kind == 0,
            rowid,
            owner_as_str(namespace.derived_owner()),
            namespace.as_str(),
            budget,
        )
        .await?
        .map(|(record, _)| SelectionSource::Memory(record)));
    }
    if kind != 2 && kind != 3 {
        return Err(AppError::InvalidParams(
            "selection source kind must be 0..3".into(),
        ));
    }
    let candidate_source = kind == 3;
    let plans = plans(candidate_source);
    if !preflight(conn, &plans, namespace, rowid, budget).await? {
        return Ok(None);
    }
    let groups = materialize(conn, &plans, namespace, rowid).await?;
    let Some(root) = groups[0].first() else {
        return Ok(None);
    };
    if !candidate_source {
        let record = episode(root, namespace)?;
        let mut edges = event_edges(&groups[1])?;
        validate_episode_edges(&record, &mut edges)?;
        if !edges.is_empty() {
            return Err(error("unexpected episode source edges"));
        }
        return Ok(Some(SelectionSource::Episode(record)));
    }
    let record = candidate(root, namespace)?;
    let mut episodes = BTreeSet::new();
    let mut edges = event_edges(&groups[2])?;
    for row in &groups[1] {
        let source = episode(row, namespace)?;
        validate_episode_edges(&source, &mut edges)?;
        if !episodes.insert(source.episode_id) {
            return Err(error("duplicate candidate source episode"));
        }
    }
    let expected: BTreeSet<_> = record.content.source_episode_ids.iter().cloned().collect();
    if episodes != expected || !edges.is_empty() {
        return Err(error(
            "candidate payload and normalized source edges disagree",
        ));
    }
    Ok(Some(SelectionSource::Experience(record)))
}

/// Kinds: 0 = active claim, 1 = event, 2 = persisted experience episode,
/// 3 = current active experience candidate. Rowids belong to their root tables,
/// not historical candidate versions. Oversized records are omitted whole.
pub async fn hydrate_selection(
    conn: &mut SqliteConnection,
    namespace: &Namespace,
    kind: i64,
    rowid: i64,
    budget: usize,
) -> Result<Option<SelectionSource>, AppError> {
    let mut tx = conn.begin().await.map_err(error)?;
    let result = hydrate_snapshot(&mut tx, namespace, kind, rowid, budget).await;
    tx.rollback().await.map_err(error)?;
    result
}

/// Resolves only explicit typed references in the exact requested namespace.
/// It never consults historical active snapshots or widens owner/namespace scope.
pub async fn hydrate_by_reference(
    conn: &mut SqliteConnection,
    namespace: &Namespace,
    reference: &str,
    budget: usize,
) -> Result<Option<SelectionSource>, AppError> {
    if reference.len() > 1024 {
        return Ok(None);
    }
    let (kind, table, key, id) = if let Some(id) = reference.strip_prefix("claim:") {
        (0, "claims", "claim_id", id)
    } else if let Some(id) = reference.strip_prefix("event:") {
        (1, "events", "event_id", id)
    } else if let Some(id) = reference.strip_prefix("experience_episode:") {
        (2, "experience_episodes", "episode_id", id)
    } else if let Some(id) = reference.strip_prefix("experience_candidate:") {
        (3, "experience_candidates", "candidate_id", id)
    } else {
        return Err(AppError::InvalidParams(
            "selection reference must be claim:, event:, experience_episode: or experience_candidate:".into(),
        ));
    };
    if id.is_empty() {
        return Ok(None);
    }
    let mut tx = conn.begin().await.map_err(error)?;
    let scope = if kind < 2 { " AND owner=?3" } else { "" };
    let active = if kind == 0 {
        " AND status='active'"
    } else {
        ""
    };
    let sql = format!("SELECT rowid FROM {table} WHERE {key}=?1 AND namespace=?2{scope}{active}");
    let mut query = sqlx::query_scalar::<_, i64>(&sql)
        .bind(id)
        .bind(namespace.as_str());
    if kind < 2 {
        query = query.bind(owner_as_str(namespace.derived_owner()));
    }
    let result = match query.fetch_optional(&mut *tx).await.map_err(error)? {
        Some(rowid) => hydrate_snapshot(&mut tx, namespace, kind, rowid, budget).await,
        None => Ok(None),
    };
    tx.rollback().await.map_err(error)?;
    result
}

#[cfg(test)]
mod bounded_plan_tests {
    use super::*;

    #[tokio::test]
    async fn experience_preflight_limits_are_not_behind_transitive_sorts() {
        let dir = tempfile::tempdir().unwrap();
        let url = crate::support::sqlite_url::sqlite_file_url(
            &dir.path().canonicalize().unwrap().join("plans.sqlite"),
        );
        let store = crate::adapters::sqlite::SqliteStore::bootstrap(&url)
            .await
            .unwrap();
        let mut conn = store.pool.acquire().await.unwrap();
        for candidate in [false, true] {
            for plan in plans(candidate) {
                let columns = plan
                    .fields
                    .iter()
                    .enumerate()
                    .map(|(i, field)| {
                        let field = field.name();
                        format!("typeof({field}) AS t{i},length(CAST({field} AS BLOB)) AS n{i}")
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                for sql in [
                    format!(
                        "SELECT {columns} FROM ({}) LIMIT {}",
                        plan.sql,
                        plan.limit()
                    ),
                    format!("{} LIMIT {}", plan.sql, plan.limit()),
                ] {
                    // Bind the same scoped parameters as production preflight/materialization.
                    let explanation = format!("EXPLAIN QUERY PLAN {sql}");
                    let mut query = sqlx::query(&explanation).bind(1_i64).bind("project/a");
                    if plan.sql.contains("?3") {
                        query = query.bind("world");
                    }
                    let details = query
                        .fetch_all(&mut *conn)
                        .await
                        .unwrap()
                        .into_iter()
                        .map(|row| row.get::<String, _>("detail"))
                        .collect::<Vec<_>>();
                    assert!(
                        !details.iter().any(|s| s.contains("USE TEMP B-TREE")),
                        "{details:?}"
                    );
                    assert!(
                        !details.iter().any(|s| s.starts_with("SCAN ")),
                        "{details:?}"
                    );
                }
            }
        }
    }
}
