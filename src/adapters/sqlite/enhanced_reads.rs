//! Opt-in retrieval reads. All payload and provenance plans are preflighted in
//! the same SQLite snapshot before any text payload is materialized. Limits are
//! input-allocation guards, independent of the caller's output JSON budget.
use super::{
    SqliteStore,
    store::rows::{owner_as_str, stored_claim_from_row, stored_event_from_row},
};
use crate::{
    application::search_memory::SearchMemoryRecord,
    domain::{claim::ClaimReference, event::EventReference, types::MemoryScope},
    error::AppError,
    ports::{
        ClaimReadRecord, ClaimRevisionLinks, EventReadRecord,
        enhanced_memory_store::{EnhancedMemoryStore, EnhancedRecordPage},
    },
};
use async_trait::async_trait;
use sqlx::{Row, SqliteConnection, sqlite::SqliteRow};
const RECORD_BYTES: usize = 32 * 1024;
const SCAN_BYTES: usize = 256 * 1024;
const EDGE_LIMIT: usize = 128;
fn error(e: impl std::fmt::Display) -> AppError {
    AppError::Message(format!("enhanced authoritative read: {e}"))
}
fn scope_parts(scope: &MemoryScope) -> Result<(&str, &str), AppError> {
    match (scope.owner(), scope.namespace()) {
        (Some(owner), Some(namespace)) => Ok((owner_as_str(owner), namespace.as_str())),
        _ => Err(AppError::InvalidParams(
            "enhanced reads require explicit scope".into(),
        )),
    }
}
struct Plan {
    sql: String,
    fields: Vec<(&'static str, bool)>,
}
fn plan(sql: String, fields: &[(&'static str, bool)]) -> Plan {
    Plan {
        sql,
        fields: fields.to_vec(),
    }
}
fn plans(claim: bool) -> Vec<Plan> {
    let table = if claim { "claims" } else { "events" };
    let active = if claim { " AND status='active'" } else { "" };
    let root = format!(
        "WITH target AS (SELECT * FROM {table} WHERE rowid=?1 AND owner=?2 AND namespace=?3{active}) "
    );
    if claim {
        vec![
            plan(
                format!(
                    "{root}SELECT claim_id,owner,namespace,subject,predicate,object,mode,status,recorded_at,observed_at FROM target"
                ),
                &[
                    ("claim_id", false),
                    ("owner", false),
                    ("namespace", false),
                    ("subject", false),
                    ("predicate", false),
                    ("object", false),
                    ("mode", false),
                    ("status", false),
                    ("recorded_at", true),
                    ("observed_at", true),
                ],
            ),
            plan(
                format!(
                    "{root}SELECT e.event_id FROM target t JOIN evidence_links el ON el.claim_id=t.claim_id JOIN events e ON e.event_id=el.event_id WHERE e.owner=?2 AND e.namespace=?3 ORDER BY e.recorded_at_sort_key DESC,e.rowid DESC"
                ),
                &[("event_id", false)],
            ),
            plan(
                format!(
                    "{root}SELECT DISTINCT ee.episode_reference FROM target t JOIN evidence_links el ON el.claim_id=t.claim_id JOIN events e ON e.event_id=el.event_id JOIN episode_events ee ON ee.event_id=e.event_id WHERE e.owner=?2 AND e.namespace=?3 ORDER BY ee.episode_reference"
                ),
                &[("episode_reference", false)],
            ),
            plan(
                format!(
                    "{root}SELECT r.reflection_id,r.superseded_claim_id,r.replacement_claim_id,CASE WHEN s.owner=?2 AND s.namespace=?3 THEN r.superseded_claim_id END AS scoped_superseded_claim_id,CASE WHEN p.owner=?2 AND p.namespace=?3 THEN r.replacement_claim_id END AS scoped_replacement_claim_id FROM target t JOIN reflections r ON r.superseded_claim_id=t.claim_id OR r.replacement_claim_id=t.claim_id LEFT JOIN claims s ON s.claim_id=r.superseded_claim_id LEFT JOIN claims p ON p.claim_id=r.replacement_claim_id ORDER BY r.recorded_at_sort_key DESC,r.rowid DESC"
                ),
                &[
                    ("reflection_id", false),
                    ("superseded_claim_id", true),
                    ("replacement_claim_id", true),
                    ("scoped_superseded_claim_id", true),
                    ("scoped_replacement_claim_id", true),
                ],
            ),
        ]
    } else {
        vec![
            plan(
                format!(
                    "{root}SELECT event_id,recorded_at,owner,namespace,kind,summary,feedback_json,observed_at FROM target"
                ),
                &[
                    ("event_id", false),
                    ("recorded_at", false),
                    ("owner", false),
                    ("namespace", false),
                    ("kind", false),
                    ("summary", false),
                    ("feedback_json", true),
                    ("observed_at", true),
                ],
            ),
            plan(
                format!(
                    "{root}SELECT el.claim_id FROM target t JOIN evidence_links el ON el.event_id=t.event_id JOIN claims c ON c.claim_id=el.claim_id WHERE c.owner=?2 AND c.namespace=?3 ORDER BY el.claim_id"
                ),
                &[("claim_id", false)],
            ),
            plan(
                format!(
                    "{root}SELECT ee.episode_reference FROM target t JOIN episode_events ee ON ee.event_id=t.event_id ORDER BY ee.episode_reference"
                ),
                &[("episode_reference", false)],
            ),
        ]
    }
}
// Only fixed-size type labels and integers cross the SQL boundary here. LIMIT
// bounds metadata row count; it is NOT the text-byte bound.
async fn preflight(
    conn: &mut SqliteConnection,
    plans: &[Plan],
    rowid: i64,
    owner: &str,
    namespace: &str,
    budget: usize,
) -> Result<Option<usize>, AppError> {
    let mut bytes = 0usize;
    for plan in plans {
        let columns = plan
            .fields
            .iter()
            .enumerate()
            .map(|(i, (field, _))| {
                format!("typeof({field}) AS t{i},length(CAST({field} AS BLOB)) AS n{i}")
            })
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT {columns} FROM ({}) LIMIT {}",
            plan.sql,
            EDGE_LIMIT + 1
        );
        let rows = sqlx::query(&sql)
            .bind(rowid)
            .bind(owner)
            .bind(namespace)
            .fetch_all(&mut *conn)
            .await
            .map_err(error)?;
        let mut oversized = rows.len() > EDGE_LIMIT;
        for row in rows {
            bytes = bytes.saturating_add(128);
            for (i, (_, nullable)) in plan.fields.iter().enumerate() {
                let kind: String = row.try_get(format!("t{i}").as_str()).map_err(error)?;
                if kind != "text" && !(kind == "null" && *nullable) {
                    return Err(error("non-text authoritative field"));
                }
                let length: Option<i64> = row.try_get(format!("n{i}").as_str()).map_err(error)?;
                let n = usize::try_from(length.unwrap_or(0)).map_err(error)?;
                bytes = bytes.saturating_add(n).saturating_add(16);
                oversized |= n > RECORD_BYTES;
            }
        }
        if oversized || bytes > budget {
            return Ok(None);
        }
    }
    Ok(Some(bytes))
}
pub(super) async fn hydrate(
    conn: &mut SqliteConnection,
    claim: bool,
    rowid: i64,
    owner: &str,
    namespace: &str,
    budget: usize,
) -> Result<Option<(SearchMemoryRecord, usize)>, AppError> {
    let plans = plans(claim);
    let Some(bytes) = preflight(
        conn,
        &plans,
        rowid,
        owner,
        namespace,
        budget.min(RECORD_BYTES),
    )
    .await?
    else {
        return Ok(None);
    };
    let mut groups: Vec<Vec<SqliteRow>> = Vec::new();
    for plan in plans {
        let rows = sqlx::query(&format!("{} LIMIT {}", plan.sql, EDGE_LIMIT + 1))
            .bind(rowid)
            .bind(owner)
            .bind(namespace)
            .fetch_all(&mut *conn)
            .await
            .map_err(error)?;
        // The legacy converters use Row::get. Validate decoding first so corrupt
        // UTF-8 is an explicit storage error rather than a converter panic.
        for row in &rows {
            for (field, nullable) in &plan.fields {
                if *nullable {
                    let _: Option<&str> = row.try_get(*field).map_err(error)?;
                } else {
                    let _: &str = row.try_get(*field).map_err(error)?;
                }
            }
        }
        groups.push(rows);
    }
    let Some(base) = groups[0].first() else {
        return Ok(None);
    };
    let record = if claim {
        let stored = stored_claim_from_row(base)?;
        ClaimReference::parse(&stored.claim_id)
            .map_err(|_| error("invalid authoritative claim reference"))?;
        let evidence = groups[1]
            .iter()
            .map(|r| {
                let id: String = r.try_get("event_id").map_err(error)?;
                EventReference::parse(id)
                    .map_err(|_| error("invalid authoritative evidence reference"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let episodes = strings(&groups[2], "episode_reference")?;
        let mut revision = ClaimRevisionLinks::default();
        for row in &groups[3] {
            let reflection: String = row.try_get("reflection_id").map_err(error)?;
            let old: Option<String> = row.try_get("superseded_claim_id").map_err(error)?;
            let new: Option<String> = row.try_get("replacement_claim_id").map_err(error)?;
            let scoped_old: Option<String> =
                row.try_get("scoped_superseded_claim_id").map_err(error)?;
            let scoped_new: Option<String> =
                row.try_get("scoped_replacement_claim_id").map_err(error)?;
            for endpoint in [scoped_old.as_deref(), scoped_new.as_deref()]
                .into_iter()
                .flatten()
            {
                ClaimReference::parse(endpoint)
                    .map_err(|_| error("invalid authoritative revision reference"))?;
            }
            if new.as_deref() == Some(stored.claim_id.as_str())
                && !(old.is_some() && scoped_old.is_none())
            {
                revision
                    .source_reflection_id
                    .get_or_insert(reflection.clone());
                if revision.supersedes_claim_reference.is_none() {
                    revision.supersedes_claim_reference =
                        scoped_old.map(ClaimReference::from_claim_id);
                }
            }
            if old.as_deref() == Some(stored.claim_id.as_str())
                && !(new.is_some() && scoped_new.is_none())
            {
                revision
                    .superseded_by_reflection_id
                    .get_or_insert(reflection);
                if revision.replacement_claim_reference.is_none() {
                    revision.replacement_claim_reference =
                        scoped_new.map(ClaimReference::from_claim_id);
                }
            }
        }
        SearchMemoryRecord::from(ClaimReadRecord::new(stored, evidence, episodes, revision))
    } else {
        let stored = stored_event_from_row(base)?;
        EventReference::parse(&stored.event_id)
            .map_err(|_| error("invalid authoritative event reference"))?;
        SearchMemoryRecord::from(EventReadRecord::new(
            stored,
            strings(&groups[1], "claim_id")?,
            strings(&groups[2], "episode_reference")?,
        ))
    };
    Ok(Some((record, bytes)))
}
fn strings(rows: &[SqliteRow], field: &str) -> Result<Vec<String>, AppError> {
    rows.iter()
        .map(|row| row.try_get(field).map_err(error))
        .collect()
}
#[async_trait]
impl EnhancedMemoryStore for SqliteStore {
    async fn scan_enhanced_records(
        &self,
        scope: MemoryScope,
        limit: usize,
    ) -> Result<EnhancedRecordPage, AppError> {
        if !(1..=100).contains(&limit) {
            return Err(AppError::InvalidParams(
                "enhanced scan limit must be 1..100".into(),
            ));
        }
        let (owner, namespace) = scope_parts(&scope)?;
        let mut tx = self.pool.begin().await.map_err(error)?;
        // Independent scope-first source window. Only integer rowids are loaded
        // before the complete payload and edge preflight, even for oversized IDs.
        let claims:Vec<i64>=sqlx::query_scalar("SELECT rowid FROM claims WHERE owner=? AND namespace=? AND status='active' ORDER BY claim_id LIMIT ?").bind(owner).bind(namespace).bind((limit+1) as i64).fetch_all(&mut *tx).await.map_err(error)?;
        let events: Vec<i64> = sqlx::query_scalar(
            "SELECT rowid FROM events WHERE owner=? AND namespace=? ORDER BY event_id LIMIT ?",
        )
        .bind(owner)
        .bind(namespace)
        .bind((limit + 1) as i64)
        .fetch_all(&mut *tx)
        .await
        .map_err(error)?;
        let has_more = claims.len() + events.len() > limit;
        let mut window = Vec::new();
        for i in 0..claims.len().max(events.len()) {
            if let Some(id) = claims.get(i) {
                window.push((true, *id));
            }
            if let Some(id) = events.get(i) {
                window.push((false, *id));
            }
        }
        let mut records = Vec::new();
        let mut oversized = 0;
        let mut remaining = SCAN_BYTES;
        for (claim, rowid) in window.into_iter().take(limit) {
            match hydrate(&mut tx, claim, rowid, owner, namespace, remaining).await? {
                Some((record, bytes)) => {
                    remaining -= bytes;
                    records.push(record);
                }
                None => oversized += 1,
            }
        }
        tx.rollback().await.map_err(error)?;
        Ok(EnhancedRecordPage {
            records,
            has_more,
            oversized,
        })
    }
    async fn hydrate_enhanced_record(
        &self,
        scope: MemoryScope,
        reference: &str,
    ) -> Result<Option<SearchMemoryRecord>, AppError> {
        let (owner, namespace) = scope_parts(&scope)?;
        if reference.len() > 1024 {
            return Ok(None);
        }
        let (claim, id) = if let Some(id) = reference.strip_prefix("claim:") {
            (true, id)
        } else if let Some(id) = reference.strip_prefix("event:") {
            (false, id)
        } else {
            return Err(AppError::InvalidParams(
                "enhanced reference must be claim: or event:".into(),
            ));
        };
        let table = if claim { "claims" } else { "events" };
        let key = if claim { "claim_id" } else { "event_id" };
        let active = if claim { " AND status='active'" } else { "" };
        let mut tx = self.pool.begin().await.map_err(error)?;
        let rowid: Option<i64> = sqlx::query_scalar(&format!(
            "SELECT rowid FROM {table} WHERE {key}=? AND owner=? AND namespace=?{active}"
        ))
        .bind(id)
        .bind(owner)
        .bind(namespace)
        .fetch_optional(&mut *tx)
        .await
        .map_err(error)?;
        let result = match rowid {
            Some(id) => hydrate(&mut tx, claim, id, owner, namespace, RECORD_BYTES)
                .await?
                .map(|(record, _)| record),
            None => None,
        };
        tx.rollback().await.map_err(error)?;
        Ok(result)
    }
    async fn hydrate_selection_source(
        &self,
        scope: MemoryScope,
        reference: &str,
    ) -> Result<Option<crate::application::selection_source::SelectionSource>, AppError> {
        let (_, namespace) = scope_parts(&scope)?;
        let namespace = crate::domain::types::Namespace::parse(namespace.to_owned())?;
        let mut tx = self.pool.begin().await.map_err(error)?;
        let result = super::selection_reads::hydrate_by_reference(
            &mut tx,
            &namespace,
            reference,
            RECORD_BYTES,
        )
        .await?;
        tx.rollback().await.map_err(error)?;
        Ok(result)
    }
    async fn selection_source_generation(&self, scope: MemoryScope) -> Result<String, AppError> {
        scope_parts(&scope)?;
        let mut tx = self.pool.begin().await.map_err(error)?;
        let result = super::semantic_index::source_generation(&mut tx).await?;
        tx.rollback().await.map_err(error)?;
        Ok(result)
    }
}
