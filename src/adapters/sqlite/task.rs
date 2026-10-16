//! Atomic scoped task revisions/checkpoints and durable retry receipts. No executor.
use super::SqliteStore;
use crate::domain::experience::identifier;
use crate::{
    domain::{event::EventReference, task::*},
    error::AppError,
    ports::{StoredWriteReceipt, WriteReceiptRequest, task_store::TaskStore},
};
use async_trait::async_trait;
use chrono::Utc;
use serde::{Serialize, de::DeserializeOwned};
use sqlx::{Row, SqliteConnection};

pub(super) const TASK_SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS task_heads (
 namespace TEXT NOT NULL CHECK (namespace LIKE 'project/%' AND length(namespace)>8),
 task_id TEXT NOT NULL, current_version INTEGER NOT NULL CHECK(current_version>0),
 PRIMARY KEY(namespace,task_id)
);
CREATE TABLE IF NOT EXISTS task_revisions (
 namespace TEXT NOT NULL, task_id TEXT NOT NULL, version INTEGER NOT NULL CHECK(version>0),
 title TEXT NOT NULL, status TEXT NOT NULL CHECK(status IN ('active','paused','completed','cancelled')),
 source_json TEXT NOT NULL, source_event_id TEXT, reason TEXT NOT NULL, recorded_at TEXT NOT NULL,
 PRIMARY KEY(namespace,task_id,version),
 FOREIGN KEY(namespace,task_id) REFERENCES task_heads(namespace,task_id),
 FOREIGN KEY(source_event_id) REFERENCES events(event_id)
);
CREATE TABLE IF NOT EXISTS task_requirements (
 namespace TEXT NOT NULL,task_id TEXT NOT NULL,version INTEGER NOT NULL,requirement_id TEXT NOT NULL,
 position INTEGER NOT NULL, state TEXT NOT NULL CHECK(state IN ('reported','candidate','withdrawn')),
 source_event_id TEXT, payload_json TEXT NOT NULL CHECK(length(CAST(payload_json AS BLOB))<=131072),
 PRIMARY KEY(namespace,task_id,version,requirement_id),UNIQUE(namespace,task_id,version,position),
 FOREIGN KEY(namespace,task_id,version) REFERENCES task_revisions(namespace,task_id,version),
 FOREIGN KEY(source_event_id) REFERENCES events(event_id)
);
CREATE TABLE IF NOT EXISTS task_checkpoints (
 namespace TEXT NOT NULL,task_id TEXT NOT NULL,sequence INTEGER NOT NULL CHECK(sequence>0),checkpoint_id TEXT NOT NULL,
 task_version INTEGER NOT NULL,payload_json TEXT NOT NULL CHECK(length(CAST(payload_json AS BLOB))<=131072),
 PRIMARY KEY(namespace,task_id,sequence),UNIQUE(namespace,task_id,checkpoint_id),
 FOREIGN KEY(namespace,task_id,task_version) REFERENCES task_revisions(namespace,task_id,version)
);
CREATE TABLE IF NOT EXISTS task_checkpoint_sources (
 namespace TEXT NOT NULL,task_id TEXT NOT NULL,sequence INTEGER NOT NULL,event_id TEXT NOT NULL,
 PRIMARY KEY(namespace,task_id,sequence,event_id),
 FOREIGN KEY(namespace,task_id,sequence) REFERENCES task_checkpoints(namespace,task_id,sequence),
 FOREIGN KEY(event_id) REFERENCES events(event_id)
);
CREATE INDEX IF NOT EXISTS idx_task_checkpoint_version ON task_checkpoints(namespace,task_id,task_version);
"#;
pub(super) async fn install(c: &mut SqliteConnection) -> Result<(), AppError> {
    for statement in TASK_SCHEMA_SQL.split(';').filter(|s| !s.trim().is_empty()) {
        db(sqlx::query(statement).execute(&mut *c).await)?;
    }
    Ok(())
}
fn db<T>(r: Result<T, sqlx::Error>) -> Result<T, AppError> {
    r.map_err(|_| AppError::Message("task database operation failed".into()))
}
fn json<T: Serialize>(r: &T) -> Result<String, AppError> {
    serde_json::to_string(r).map_err(|e| AppError::Message(e.to_string()))
}
fn decode<T: DeserializeOwned>(r: &str) -> Result<T, AppError> {
    if r.len() > MAX_TASK_BYTES {
        return invalid("persisted task payload exceeds bounds");
    }
    serde_json::from_str(r).map_err(|_| AppError::Message("invalid persisted task data".into()))
}
fn status(s: TaskStatus) -> &'static str {
    match s {
        TaskStatus::Active => "active",
        TaskStatus::Paused => "paused",
        TaskStatus::Completed => "completed",
        TaskStatus::Cancelled => "cancelled",
    }
}
fn state(s: RequirementState) -> &'static str {
    match s {
        RequirementState::Reported => "reported",
        RequirementState::Candidate => "candidate",
        RequirementState::Withdrawn => "withdrawn",
    }
}
async fn replay<T: DeserializeOwned>(
    c: &mut SqliteConnection,
    r: &WriteReceiptRequest,
) -> Result<Option<TaskWriteResult<T>>, AppError> {
    let row=db(sqlx::query("SELECT request_summary_json,response_summary_json FROM operation_log WHERE operation_id=? AND namespace=? AND actor_id='durable_write_receipt_v1' AND status='ok'").bind(&r.operation_id).bind(&r.namespace).fetch_optional(c).await)?;
    row.map(|row| {
        let metadata: serde_json::Value =
            serde_json::from_str(&row.get::<String, _>("request_summary_json"))
                .map_err(|_| AppError::Message("invalid task receipt".into()))?;
        let receipt = StoredWriteReceipt {
            request_hash: metadata
                .get("request_hash")
                .and_then(|v| v.as_str())
                .ok_or_else(|| AppError::Message("missing task receipt hash".into()))?
                .into(),
            result_json: row.get("response_summary_json"),
        };
        let mut result: TaskWriteResult<T> = receipt.replay(r)?;
        result.replayed = true;
        Ok(result)
    })
    .transpose()
}
async fn audit<T: Serialize>(
    c: &mut SqliteConnection,
    r: &WriteReceiptRequest,
    result: &TaskWriteResult<T>,
) -> Result<(), AppError> {
    db(sqlx::query("INSERT INTO operation_log(operation_id,occurred_at,namespace,actor_kind,actor_id,entrypoint,operation_kind,status,request_summary_json,response_summary_json,redaction_version) VALUES(?,?,?,'system','durable_write_receipt_v1',?,'tool','ok',?,?,1)").bind(&r.operation_id).bind(Utc::now().to_rfc3339()).bind(&r.namespace).bind(&r.operation).bind(json(&serde_json::json!({"request_hash":r.request_hash,"knowledge_only":true}))?).bind(json(result)?).execute(c).await)?;
    Ok(())
}
async fn validate_reference(
    c: &mut SqliteConnection,
    ns: &str,
    reference: &str,
) -> Result<String, AppError> {
    let event = EventReference::parse(reference)?;
    let n: i64 = db(sqlx::query_scalar(
        "SELECT count(*) FROM events WHERE namespace=? AND event_id=? AND owner='world'",
    )
    .bind(ns)
    .bind(event.event_id())
    .fetch_one(c)
    .await)?;
    if n != 1 {
        return invalid("task evidence must exist in the exact project namespace");
    }
    Ok(event.event_id().into())
}
async fn validate_sources(c: &mut SqliteConnection, record: &TaskRevision) -> Result<(), AppError> {
    if let Some(r) = &record.source.reference {
        validate_reference(c, &record.namespace, r).await?;
    }
    for q in &record.requirements {
        if let Some(r) = &q.source.reference {
            validate_reference(c, &record.namespace, r).await?;
        }
    }
    Ok(())
}
async fn save_revision(c: &mut SqliteConnection, r: &TaskRevision) -> Result<(), AppError> {
    validate_sources(c, r).await?;
    let source_event = r
        .source
        .reference
        .as_ref()
        .map(EventReference::parse)
        .transpose()?;
    db(sqlx::query("INSERT INTO task_revisions(namespace,task_id,version,title,status,source_json,source_event_id,reason,recorded_at) VALUES(?,?,?,?,?,?,?,?,?)").bind(&r.namespace).bind(&r.task_id).bind(r.version).bind(&r.title).bind(status(r.status)).bind(json(&r.source)?).bind(source_event.as_ref().map(|e|e.event_id())).bind(&r.reason).bind(&r.recorded_at).execute(&mut *c).await)?;
    for (position, q) in r.requirements.iter().enumerate() {
        let event = q
            .source
            .reference
            .as_ref()
            .map(EventReference::parse)
            .transpose()?;
        db(sqlx::query("INSERT INTO task_requirements(namespace,task_id,version,requirement_id,position,state,source_event_id,payload_json) VALUES(?,?,?,?,?,?,?,?)").bind(&r.namespace).bind(&r.task_id).bind(r.version).bind(&q.requirement_id).bind(position as i64).bind(state(q.state)).bind(event.as_ref().map(|e|e.event_id())).bind(json(q)?).execute(&mut *c).await)?;
    }
    Ok(())
}
// Caller parameters are validated at the service/store boundary. Any failure while
// hydrating persisted objects is a storage failure, never a user query error.
fn corrupt_task_data(_: AppError) -> AppError {
    AppError::Message("stored task data is unavailable or inconsistent".into())
}
async fn load_revision(
    c: &mut SqliteConnection,
    ns: &str,
    id: &str,
    version: Option<i64>,
) -> Result<Option<TaskRevision>, AppError> {
    load_revision_inner(c, ns, id, version)
        .await
        .map_err(corrupt_task_data)
}
async fn load_revision_inner(
    c: &mut SqliteConnection,
    ns: &str,
    id: &str,
    version: Option<i64>,
) -> Result<Option<TaskRevision>, AppError> {
    let row=db(sqlx::query("SELECT r.* FROM task_heads h JOIN task_revisions r ON r.namespace=h.namespace AND r.task_id=h.task_id AND r.version=COALESCE(?,h.current_version) WHERE h.namespace=? AND h.task_id=?").bind(version).bind(ns).bind(id).fetch_optional(&mut *c).await)?;
    let Some(row) = row else {
        if version.is_none() {
            let heads: i64 = db(sqlx::query_scalar(
                "SELECT count(*) FROM task_heads WHERE namespace=? AND task_id=?",
            )
            .bind(ns)
            .bind(id)
            .fetch_one(&mut *c)
            .await)?;
            if heads != 0 {
                return invalid("task head references missing current revision");
            }
        }
        return Ok(None);
    };
    let version: i64 = row.get("version");
    let bytes:i64=db(sqlx::query_scalar("SELECT COALESCE(sum(length(CAST(payload_json AS BLOB))),0) FROM task_requirements WHERE namespace=? AND task_id=? AND version=?").bind(ns).bind(id).bind(version).fetch_one(&mut *c).await)?;
    if bytes > MAX_TASK_BYTES as i64 {
        return invalid("persisted task requirements exceed bounds");
    }
    let rows=db(sqlx::query("SELECT requirement_id,state,source_event_id,payload_json FROM task_requirements WHERE namespace=? AND task_id=? AND version=? ORDER BY position LIMIT 129").bind(ns).bind(id).bind(version).fetch_all(&mut *c).await)?;
    if rows.len() > MAX_TASK_ITEMS {
        return invalid("persisted task requirement count exceeds bounds");
    }
    let mut requirements = Vec::new();
    for q in rows {
        let requirement: TaskRequirement = decode(&q.get::<String, _>("payload_json"))?;
        let source = requirement
            .source
            .reference
            .as_ref()
            .map(EventReference::parse)
            .transpose()?;
        if q.get::<String, _>("requirement_id") != requirement.requirement_id
            || q.get::<String, _>("state") != state(requirement.state)
            || q.get::<Option<String>, _>("source_event_id").as_deref()
                != source.as_ref().map(|s| s.event_id())
        {
            return invalid("task requirement relational/payload mismatch");
        }
        requirements.push(requirement);
    }
    let record = TaskRevision {
        namespace: ns.into(),
        task_id: id.into(),
        version,
        title: row.get("title"),
        status: decode(&format!("\"{}\"", row.get::<String, _>("status")))?,
        source: decode(&row.get::<String, _>("source_json"))?,
        reason: row.get("reason"),
        recorded_at: row.get("recorded_at"),
        requirements,
    };
    validate_persisted_revision(&record)?;
    let source = record
        .source
        .reference
        .as_ref()
        .map(EventReference::parse)
        .transpose()?;
    if row.get::<Option<String>, _>("source_event_id").as_deref()
        != source.as_ref().map(|e| e.event_id())
    {
        return invalid("task revision source relational/payload mismatch");
    }
    validate_sources(c, &record).await?;
    Ok(Some(record))
}
async fn latest_checkpoint(
    c: &mut SqliteConnection,
    ns: &str,
    id: &str,
) -> Result<Option<TaskCheckpoint>, AppError> {
    latest_checkpoint_inner(c, ns, id)
        .await
        .map_err(corrupt_task_data)
}
async fn latest_checkpoint_inner(
    c: &mut SqliteConnection,
    ns: &str,
    id: &str,
) -> Result<Option<TaskCheckpoint>, AppError> {
    let row=db(sqlx::query("SELECT checkpoint_id,sequence,task_version,CASE WHEN length(CAST(payload_json AS BLOB))<=131072 THEN payload_json ELSE NULL END AS payload_json FROM task_checkpoints WHERE namespace=? AND task_id=? ORDER BY sequence DESC LIMIT 1").bind(ns).bind(id).fetch_optional(&mut *c).await)?;
    let Some(row) = row else { return Ok(None) };
    let payload: Option<String> = row.get("payload_json");
    let cp: TaskCheckpoint = decode(
        payload
            .as_deref()
            .ok_or_else(|| AppError::Message("checkpoint exceeds read bounds".into()))?,
    )?;
    if cp.namespace != ns
        || cp.task_id != id
        || cp.checkpoint_id != row.get::<String, _>("checkpoint_id")
        || cp.sequence != row.get::<i64, _>("sequence")
        || cp.task_version != row.get::<i64, _>("task_version")
    {
        return invalid("checkpoint relational/payload mismatch");
    }
    validate_persisted_checkpoint(&cp)?;
    let mut expected = std::collections::BTreeSet::new();
    if let Some(reference) = &cp.source.reference {
        expected.insert(validate_reference(c, ns, reference).await?);
    }
    for step in &cp.steps {
        for reference in &step.evidence_refs {
            expected.insert(validate_reference(c, ns, reference).await?);
        }
    }
    let stored:Vec<String>=db(sqlx::query_scalar("SELECT event_id FROM task_checkpoint_sources WHERE namespace=? AND task_id=? AND sequence=? LIMIT 4098").bind(ns).bind(id).bind(cp.sequence).fetch_all(&mut *c).await)?;
    if stored
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
        != expected
    {
        return invalid("checkpoint source edges do not match payload");
    }
    Ok(Some(cp))
}
#[async_trait]
impl TaskStore for SqliteStore {
    async fn create_task(
        &self,
        r: CreateTaskRequest,
    ) -> Result<TaskWriteResult<TaskRevision>, AppError> {
        validate_create(&r)?;
        let receipt = WriteReceiptRequest::new("create_task", &r.namespace, &r.request_id, &r)?;
        let mut tx = db(self.pool.begin_with("BEGIN IMMEDIATE").await)?;
        if let Some(result) = replay(&mut tx, &receipt).await? {
            db(tx.commit().await)?;
            return Ok(result);
        }
        let record = TaskRevision {
            namespace: r.namespace,
            task_id: r.task_id,
            version: 1,
            title: r.title,
            status: TaskStatus::Active,
            requirements: r.requirements,
            source: r.source,
            reason: r.reason,
            recorded_at: Utc::now().to_rfc3339(),
        };
        bounded(&record)?;
        db(
            sqlx::query("INSERT INTO task_heads(namespace,task_id,current_version) VALUES(?,?,1)")
                .bind(&record.namespace)
                .bind(&record.task_id)
                .execute(&mut *tx)
                .await,
        )?;
        save_revision(&mut tx, &record).await?;
        let result = TaskWriteResult {
            record,
            replayed: false,
            operation_id: receipt.operation_id.clone(),
        };
        audit(&mut tx, &receipt, &result).await?;
        db(tx.commit().await)?;
        Ok(result)
    }
    async fn revise_task(
        &self,
        r: ReviseTaskRequest,
    ) -> Result<TaskWriteResult<TaskRevision>, AppError> {
        scope(&r.namespace)?;
        let receipt = WriteReceiptRequest::new("revise_task", &r.namespace, &r.request_id, &r)?;
        let mut tx = db(self.pool.begin_with("BEGIN IMMEDIATE").await)?;
        if let Some(result) = replay(&mut tx, &receipt).await? {
            db(tx.commit().await)?;
            return Ok(result);
        }
        let current = load_revision(&mut tx, &r.namespace, &r.task_id, None)
            .await?
            .ok_or_else(|| {
                AppError::InvalidParams("task not found in requested namespace".into())
            })?;
        let record = apply_revision(&current, &r, Utc::now().to_rfc3339())?;
        save_revision(&mut tx, &record).await?;
        let changed=db(sqlx::query("UPDATE task_heads SET current_version=? WHERE namespace=? AND task_id=? AND current_version=?").bind(record.version).bind(&r.namespace).bind(&r.task_id).bind(r.expected_version).execute(&mut *tx).await)?;
        if changed.rows_affected() != 1 {
            return invalid("task version conflict");
        }
        let result = TaskWriteResult {
            record,
            replayed: false,
            operation_id: receipt.operation_id.clone(),
        };
        audit(&mut tx, &receipt, &result).await?;
        db(tx.commit().await)?;
        Ok(result)
    }
    async fn get_task(&self, r: GetTaskRequest) -> Result<Option<TaskRevision>, AppError> {
        scope(&r.namespace)?;
        identifier(&r.task_id, "task_id")?;
        if let Some(v) = r.version {
            crate::domain::experience::validate_version(v)?;
        }
        let mut tx = db(self.pool.begin().await)?;
        let result = load_revision(&mut tx, &r.namespace, &r.task_id, r.version).await?;
        db(tx.commit().await)?;
        Ok(result)
    }
    async fn list_tasks(&self, r: ListTasksRequest) -> Result<Vec<TaskRevision>, AppError> {
        scope(&r.namespace)?;
        crate::domain::experience::validate_limit(r.limit)?;
        if let Some(id) = &r.after_id {
            identifier(id, "after_id")?;
        }
        let mut tx = db(self.pool.begin().await)?;
        let ids:Vec<String>=db(sqlx::query_scalar("SELECT task_id FROM task_heads WHERE namespace=? AND (? IS NULL OR task_id>?) ORDER BY task_id LIMIT ?").bind(&r.namespace).bind(&r.after_id).bind(&r.after_id).bind(r.limit as i64).fetch_all(&mut *tx).await)?;
        let mut result = Vec::new();
        for id in ids {
            if let Some(record) = load_revision(&mut tx, &r.namespace, &id, None).await? {
                result.push(record);
            }
        }
        db(tx.commit().await)?;
        Ok(result)
    }
    async fn record_task_checkpoint(
        &self,
        r: RecordTaskCheckpointRequest,
    ) -> Result<TaskWriteResult<TaskCheckpoint>, AppError> {
        validate_checkpoint(&r)?;
        let receipt =
            WriteReceiptRequest::new("record_task_checkpoint", &r.namespace, &r.request_id, &r)?;
        let mut tx = db(self.pool.begin_with("BEGIN IMMEDIATE").await)?;
        if let Some(result) = replay(&mut tx, &receipt).await? {
            db(tx.commit().await)?;
            return Ok(result);
        }
        let task = load_revision(&mut tx, &r.namespace, &r.task_id, None)
            .await?
            .ok_or_else(|| {
                AppError::InvalidParams("task not found in requested namespace".into())
            })?;
        let sequence = latest_checkpoint(&mut tx, &r.namespace, &r.task_id)
            .await?
            .map_or(0, |c| c.sequence);
        if task.version != r.expected_version || sequence != r.expected_checkpoint_sequence {
            return invalid("checkpoint task version or sequence conflict");
        }
        let mut sources = std::collections::BTreeSet::new();
        if let Some(e) = &r.source.reference {
            sources.insert(validate_reference(&mut tx, &r.namespace, e).await?);
        }
        for step in &r.steps {
            for e in &step.evidence_refs {
                sources.insert(validate_reference(&mut tx, &r.namespace, e).await?);
            }
        }
        let record = TaskCheckpoint {
            namespace: r.namespace,
            task_id: r.task_id,
            checkpoint_id: r.checkpoint_id,
            task_version: task.version,
            sequence: sequence + 1,
            recorded_at: Utc::now().to_rfc3339(),
            steps: r.steps,
            blockers: r.blockers,
            next_steps: r.next_steps,
            environment: r.environment,
            repository_revision: r.repository_revision,
            artifacts: r.artifacts,
            source: r.source,
        };
        bounded(&record)?;
        db(sqlx::query("INSERT INTO task_checkpoints(namespace,task_id,sequence,checkpoint_id,task_version,payload_json) VALUES(?,?,?,?,?,?)").bind(&record.namespace).bind(&record.task_id).bind(record.sequence).bind(&record.checkpoint_id).bind(record.task_version).bind(json(&record)?).execute(&mut *tx).await)?;
        for event in sources {
            db(sqlx::query("INSERT INTO task_checkpoint_sources(namespace,task_id,sequence,event_id) VALUES(?,?,?,?)").bind(&record.namespace).bind(&record.task_id).bind(record.sequence).bind(event).execute(&mut *tx).await)?;
        }
        let result = TaskWriteResult {
            record,
            replayed: false,
            operation_id: receipt.operation_id.clone(),
        };
        audit(&mut tx, &receipt, &result).await?;
        db(tx.commit().await)?;
        Ok(result)
    }
    async fn get_task_snapshot(
        &self,
        namespace: &str,
        task_id: &str,
    ) -> Result<Option<TaskSnapshot>, AppError> {
        scope(namespace)?;
        identifier(task_id, "task_id")?;
        let mut tx = db(self.pool.begin().await)?;
        let task = load_revision(&mut tx, namespace, task_id, None).await?;
        let result = if let Some(task) = task {
            Some(TaskSnapshot {
                task,
                checkpoint: latest_checkpoint(&mut tx, namespace, task_id).await?,
            })
        } else {
            None
        };
        db(tx.commit().await)?;
        Ok(result)
    }
}
