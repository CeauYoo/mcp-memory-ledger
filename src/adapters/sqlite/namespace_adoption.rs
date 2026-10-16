//! Conservative, non-rewriting inventory for explicit legacy namespace adoption.
//! All queries use the caller's single transaction. Unsupported histories are
//! reported, never filtered away to turn an incomplete graph into an approval.
use super::DatabaseTableCount;
use crate::{domain::feedback::FeedbackMetadata, error::AppError};
use serde::Serialize;
use sqlx::{Row, SqliteConnection};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
pub struct AdoptionInventory {
    pub namespace: String,
    pub schema_status: String,
    pub supported: bool,
    pub blocked_reasons: Vec<String>,
    pub database_counts: Vec<DatabaseTableCount>,
    pub namespace_counts: BTreeMap<String, i64>,
    pub support_contract: &'static str,
}
const CONTRACT: &str = "basic-facts-v1: events, claims, evidence links, legacy episode memberships and unchanged operation logs/receipts. Reflection, experience, feedback-candidate and trigger histories are not supported; their presence anywhere blocks this conservative first slice because absence of cross-namespace incoming references is not proven. Global self-model baseline is retained; later versions are unsupported. No data is copied, rewritten or imported.";
fn db(e: sqlx::Error) -> AppError {
    AppError::Message(format!("adoption inventory read failed: {e}"))
}
async fn count(conn: &mut SqliteConnection, sql: &str, namespace: &str) -> Result<i64, AppError> {
    sqlx::query_scalar(sql)
        .bind(namespace)
        .fetch_one(conn)
        .await
        .map_err(db)
}
impl AdoptionInventory {
    pub(crate) fn block(&mut self, reason: impl Into<String>) {
        self.supported = false;
        self.blocked_reasons.push(reason.into());
    }
}
pub(crate) async fn inventory(
    conn: &mut SqliteConnection,
    namespace: &str,
) -> Result<AdoptionInventory, AppError> {
    let schema =
        super::lifecycle::inspect_connection(conn, "adoption_inventory", true, None).await?;
    let mut report = AdoptionInventory {
        namespace: namespace.into(),
        schema_status: schema.status.clone(),
        supported: true,
        blocked_reasons: Vec::new(),
        database_counts: schema.table_counts.clone(),
        namespace_counts: BTreeMap::new(),
        support_contract: CONTRACT,
    };
    if !schema.is_current() {
        report.block(format!(
            "database is not current: {}; explicit repair/migration is required",
            schema.status
        ));
        return Ok(report);
    }
    // An unknown extension table could carry provenance we cannot inventory.
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%'",
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(db)?;
    for name in names {
        if !report.database_counts.iter().any(|t| t.table == name)
            && !matches!(
                name.as_str(),
                "text_recall_documents"
                    | "text_recall_vocab"
                    | "text_recall_fts"
                    | "text_recall_fts_data"
                    | "text_recall_fts_idx"
                    | "text_recall_fts_docsize"
                    | "text_recall_fts_config"
            )
        {
            report.block("unsupported extension table: source closure cannot be established");
            break;
        }
    }
    for table in [
        "events",
        "claims",
        "operation_log",
        "reflection_scopes",
        "reflection_trigger_ledger",
        "feedback_candidates",
        "experience_episodes",
        "experience_candidates",
    ] {
        let n = count(
            conn,
            &format!("SELECT count(*) FROM {table} WHERE namespace=?"),
            namespace,
        )
        .await?;
        report.namespace_counts.insert(table.into(), n);
    }
    let receipts = count(conn, "SELECT count(*) FROM operation_log WHERE namespace=? AND actor_id='durable_write_receipt_v1'", namespace).await?;
    report
        .namespace_counts
        .insert("durable_write_receipts".into(), receipts);
    for (label, sql) in [
        (
            "evidence_links",
            "SELECT count(*) FROM evidence_links l JOIN claims c ON c.claim_id=l.claim_id WHERE c.namespace=?",
        ),
        (
            "episode_events",
            "SELECT count(*) FROM episode_events l JOIN events e ON e.event_id=l.event_id WHERE e.namespace=?",
        ),
    ] {
        report
            .namespace_counts
            .insert(label.into(), count(conn, sql, namespace).await?);
    }
    if report.namespace_counts["events"] + report.namespace_counts["claims"] == 0 {
        report.block("namespace has no basic fact records");
    }
    for table in ["events", "claims"] {
        if count(
            conn,
            &format!("SELECT count(*) FROM {table} WHERE namespace=? AND owner<>'world'"),
            namespace,
        )
        .await?
            > 0
        {
            report.block(format!(
                "{table}: namespace contains non-World/Unknown owner records"
            ));
        }
    }
    for table in [
        "reflections",
        "reflection_scopes",
        "reflection_evidence",
        "reflection_trigger_ledger",
        "feedback_candidates",
        "experience_episodes",
        "experience_episode_sources",
        "experience_candidates",
        "experience_candidate_versions",
        "experience_candidate_sources",
    ] {
        let n = report
            .database_counts
            .iter()
            .find(|c| c.table == table)
            .map_or(0, |c| c.rows);
        if n > 0 {
            report.block(format!("unsupported history {table} ({n} database rows): this first slice cannot prove absence of cross-namespace incoming references"));
        }
    }
    let later: i64 = sqlx::query_scalar("SELECT count(*) FROM self_model_versions WHERE version>0")
        .fetch_one(&mut *conn)
        .await
        .map_err(db)?;
    if later > 0 {
        report.block(format!(
            "unsupported self-model history ({later} later versions): source closure is not proven"
        ));
    }
    for (label, sql) in [
        (
            "evidence links cross namespace or have missing/non-World endpoints",
            "SELECT count(*) FROM evidence_links l LEFT JOIN claims c ON c.claim_id=l.claim_id LEFT JOIN events e ON e.event_id=l.event_id WHERE (c.namespace=?1 OR e.namespace=?1) AND (c.claim_id IS NULL OR e.event_id IS NULL OR c.namespace<>?1 OR e.namespace<>?1 OR c.owner<>'world' OR e.owner<>'world')",
        ),
        (
            "legacy episode has mixed, missing or non-World source members",
            "SELECT count(*) FROM episode_events l LEFT JOIN events e ON e.event_id=l.event_id WHERE EXISTS (SELECT 1 FROM episode_events x JOIN events y ON y.event_id=x.event_id WHERE x.episode_reference=l.episode_reference AND y.namespace=?1) AND (e.event_id IS NULL OR e.namespace<>?1 OR e.owner<>'world')",
        ),
    ] {
        if count(conn, sql, namespace).await? > 0 {
            report.block(label);
        }
    }
    validate_feedback(conn, namespace, &mut report).await?;
    Ok(report)
}

async fn validate_feedback(
    conn: &mut SqliteConnection,
    namespace: &str,
    report: &mut AdoptionInventory,
) -> Result<(), AppError> {
    // Inspect both directions, including foreign events with incoming references.
    // Bound materialization before reading JSON; invalid foreign JSON also blocks
    // because its references cannot safely be excluded from this inventory.
    let size = sqlx::query("SELECT count(*) AS n, total(length(CAST(feedback_json AS BLOB))) AS bytes FROM events WHERE feedback_json IS NOT NULL").fetch_one(&mut *conn).await.map_err(db)?;
    if size.get::<i64, _>("n") > 10_000 || size.get::<f64, _>("bytes") > 16_777_216.0 {
        report.block("feedback provenance exceeds bounded inventory limits (10000 rows/16 MiB)");
        return Ok(());
    }
    let rows = sqlx::query(
        "SELECT namespace=? AS in_scope, owner='world' AS canonical_owner, feedback_json FROM events WHERE feedback_json IS NOT NULL",
    )
    .bind(namespace)
    .fetch_all(&mut *conn)
    .await
    .map_err(db)?;
    for row in rows {
        let origin: bool = row.get("in_scope");
        let owner: bool = row.get("canonical_owner");
        let json: String = row.get("feedback_json");
        let metadata = match serde_json::from_str::<FeedbackMetadata>(&json) {
            Ok(value) if value.validate().is_ok() => value,
            _ => {
                report.block("invalid feedback provenance: absence of cross-namespace references cannot be proven");
                continue;
            }
        };
        let mut refs: Vec<(&str, &str)> = metadata
            .evidence_refs
            .iter()
            .map(|r| ("events", r.event_id()))
            .collect();
        if let Some(id) = metadata.observed_target.strip_prefix("event:") {
            refs.push(("events", id));
        }
        if let Some(id) = metadata.observed_target.strip_prefix("claim:") {
            refs.push(("claims", id));
        }
        for (table, id) in refs {
            let column = if table == "events" {
                "event_id"
            } else {
                "claim_id"
            };
            let target = sqlx::query(&format!(
                "SELECT namespace=? AS in_scope, owner='world' AS canonical_owner FROM {table} WHERE {column}=?"
            ))
            .bind(namespace)
            .bind(id)
            .fetch_optional(&mut *conn)
            .await
            .map_err(db)?;
            let target = target.map(|r| {
                (
                    r.get::<bool, _>("in_scope"),
                    r.get::<bool, _>("canonical_owner"),
                )
            });
            let touches = origin || target.as_ref().is_some_and(|(ns, _)| *ns);
            if touches && (!origin || !owner || !target.as_ref().is_some_and(|(ns, o)| *ns && *o)) {
                report
                    .block("feedback provenance crosses namespace or has missing/non-World source");
                return Ok(());
            }
        }
    }
    Ok(())
}
