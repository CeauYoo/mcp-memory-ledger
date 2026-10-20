//! Local-only owner-channel attestation. Deliberately absent from TaskStore/MCP.
use super::{
    SqliteStore,
    task::{db, load_revision, save_revision},
};
use crate::{
    domain::{experience::identifier, feedback::FeedbackSourceKind, owner_attestation::*, task::*},
    error::AppError,
};
use sqlx::{Row, SqliteConnection};

pub(super) async fn install(c: &mut SqliteConnection) -> Result<(), AppError> {
    for statement in r#"
CREATE TABLE IF NOT EXISTS task_owner_events (
 import_event_id TEXT PRIMARY KEY NOT NULL,
 namespace TEXT NOT NULL, task_id TEXT NOT NULL, version INTEGER NOT NULL CHECK(version>1),
 input_json TEXT NOT NULL CHECK(length(CAST(input_json AS BLOB))<=131072),
 UNIQUE(namespace,task_id,version),
 FOREIGN KEY(namespace,task_id,version) REFERENCES task_revisions(namespace,task_id,version)
);
CREATE TABLE IF NOT EXISTS task_owner_provenance (
 namespace TEXT NOT NULL, task_id TEXT NOT NULL, requirement_id TEXT NOT NULL,
 import_event_id TEXT NOT NULL UNIQUE, imported_at_version INTEGER NOT NULL,
 kind_json TEXT NOT NULL, exact_text TEXT NOT NULL, evidence_text TEXT NOT NULL,
 withdrawn_at_version INTEGER, withdrawal_event_id TEXT UNIQUE,
 PRIMARY KEY(namespace,task_id,requirement_id),
 FOREIGN KEY(import_event_id) REFERENCES task_owner_events(import_event_id),
 FOREIGN KEY(withdrawal_event_id) REFERENCES task_owner_events(import_event_id),
 FOREIGN KEY(namespace,task_id,imported_at_version,requirement_id) REFERENCES task_requirements(namespace,task_id,version,requirement_id),
 CHECK((withdrawn_at_version IS NULL AND withdrawal_event_id IS NULL) OR (withdrawn_at_version>imported_at_version AND withdrawal_event_id IS NOT NULL))
);
"#.split(';').filter(|s| !s.trim().is_empty()) {
        db(sqlx::query(statement).execute(&mut *c).await)?;
    }
    Ok(())
}
pub(super) async fn load_provenance(
    c: &mut SqliteConnection,
    ns: &str,
    task: &str,
    version: i64,
) -> Result<Vec<OwnerRequirementProvenance>, AppError> {
    let future: i64 = db(sqlx::query_scalar("SELECT (SELECT count(*) FROM task_owner_events e JOIN task_heads h ON h.namespace=e.namespace AND h.task_id=e.task_id WHERE e.namespace=?1 AND e.task_id=?2 AND e.version>h.current_version) + (SELECT count(*) FROM task_owner_provenance p JOIN task_heads h ON h.namespace=p.namespace AND h.task_id=p.task_id WHERE p.namespace=?1 AND p.task_id=?2 AND (p.imported_at_version>h.current_version OR p.withdrawn_at_version>h.current_version))").bind(ns).bind(task).fetch_one(&mut *c).await)?;
    if future != 0 {
        return invalid("owner history is ahead of task head");
    }
    let bytes: f64 = db(sqlx::query_scalar("SELECT total(length(CAST(requirement_id AS BLOB))+length(CAST(import_event_id AS BLOB))+length(CAST(kind_json AS BLOB))+length(CAST(exact_text AS BLOB))+length(CAST(evidence_text AS BLOB))+COALESCE(length(CAST(withdrawal_event_id AS BLOB)),0)) FROM task_owner_provenance WHERE namespace=? AND task_id=? AND imported_at_version<=?").bind(ns).bind(task).bind(version).fetch_one(&mut *c).await)?;
    if bytes > MAX_TASK_BYTES as f64 {
        return invalid("owner provenance exceeds byte bounds");
    }
    let rows = db(sqlx::query("SELECT * FROM task_owner_provenance WHERE namespace=? AND task_id=? AND imported_at_version<=? ORDER BY imported_at_version LIMIT 129").bind(ns).bind(task).bind(version).fetch_all(&mut *c).await)?;
    if rows.len() > MAX_TASK_ITEMS {
        return invalid("owner provenance exceeds bounds");
    }
    let stored: Vec<_> = rows
        .into_iter()
        .map(|r| -> Result<OwnerRequirementProvenance, AppError> {
            let withdrawal: Option<i64> = r.get("withdrawn_at_version");
            let visible = withdrawal.is_some_and(|v| v <= version);
            Ok(OwnerRequirementProvenance {
                requirement_id: r.get("requirement_id"),
                kind: serde_json::from_str(&r.get::<String, _>("kind_json"))
                    .map_err(|_| AppError::Message("invalid owner requirement kind".into()))?,
                import_event_id: r.get("import_event_id"),
                exact_text: r.get("exact_text"),
                evidence_text: r.get("evidence_text"),
                imported_at_version: r.get("imported_at_version"),
                withdrawn_at_version: if visible { withdrawal } else { None },
                withdrawal_event_id: if visible {
                    r.get("withdrawal_event_id")
                } else {
                    None
                },
                attestation_basis:
                    "local_owner_channel_not_authenticated_human_no_execution_authority".into(),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let events = db(sqlx::query("SELECT import_event_id,version,CASE WHEN length(CAST(input_json AS BLOB))<=131072 THEN input_json ELSE NULL END AS input_json FROM task_owner_events WHERE namespace=? AND task_id=? AND version<=? ORDER BY version LIMIT 257").bind(ns).bind(task).bind(version).fetch_all(&mut *c).await)?;
    if events.len() > MAX_TASK_ITEMS * 2 {
        return invalid("owner event history exceeds bounds");
    }
    let mut derived: Vec<OwnerRequirementProvenance> = vec![];
    for event in events {
        let input: Option<String> = event.get("input_json");
        let request: OwnerAttestationRequest = serde_json::from_str(
            input
                .as_deref()
                .ok_or_else(|| AppError::Message("owner event exceeds bounds".into()))?,
        )
        .map_err(|_| AppError::Message("invalid owner event".into()))?;
        scope(&request.namespace)?;
        identifier(&request.task_id, "task_id")?;
        identifier(&request.import_event_id, "import_event_id")?;
        crate::domain::experience::validate_version(request.expected_version)?;
        text(&request.evidence_text, "owner evidence", 16384)?;
        let event_version: i64 = event.get("version");
        if request.namespace != ns
            || request.task_id != task
            || request.import_event_id != event.get::<String, _>("import_event_id")
            || request.expected_version.checked_add(1) != Some(event_version)
        {
            return invalid("owner event scope/version mismatch");
        }
        match &request.change {
            OwnerRequirementChange::Append {
                requirement_id,
                exact_text,
                ..
            }
            | OwnerRequirementChange::Withdraw {
                requirement_id,
                exact_text,
            } => {
                identifier(requirement_id, "requirement_id")?;
                text(exact_text, "exact owner text", 4096)?;
            }
        }
        match request.change {
            OwnerRequirementChange::Append {
                requirement_id,
                exact_text,
                kind,
            } => derived.push(OwnerRequirementProvenance {
                requirement_id,
                kind,
                import_event_id: request.import_event_id,
                exact_text,
                evidence_text: request.evidence_text,
                imported_at_version: event_version,
                withdrawn_at_version: None,
                withdrawal_event_id: None,
                attestation_basis:
                    "local_owner_channel_not_authenticated_human_no_execution_authority".into(),
            }),
            OwnerRequirementChange::Withdraw {
                requirement_id,
                exact_text,
            } => {
                let provenance = derived
                    .iter_mut()
                    .find(|p| p.requirement_id == requirement_id)
                    .ok_or_else(|| AppError::Message("owner withdrawal import missing".into()))?;
                if provenance.exact_text != exact_text || provenance.withdrawn_at_version.is_some()
                {
                    return invalid("owner withdrawal mismatch");
                }
                provenance.withdrawn_at_version = Some(event_version);
                provenance.withdrawal_event_id = Some(request.import_event_id);
            }
        }
    }
    if stored != derived {
        return invalid("owner event/provenance mismatch");
    }
    Ok(stored)
}
impl SqliteStore {
    /// Only the explicit local CLI invokes this path. Possession of same-user shell
    /// access suffices; this is not an identity claim or a security boundary.
    pub async fn attest_task_owner(
        &self,
        r: OwnerAttestationRequest,
    ) -> Result<TaskWriteResult<TaskRevision>, AppError> {
        scope(&r.namespace)?;
        identifier(&r.task_id, "task_id")?;
        identifier(&r.import_event_id, "import_event_id")?;
        crate::domain::experience::validate_version(r.expected_version)?;
        text(&r.evidence_text, "exact owner evidence", 16384)?;
        bounded(&r)?;
        let input = serde_json::to_string(&r).map_err(|e| AppError::Message(e.to_string()))?;
        let mut tx = db(self.pool.begin_with("BEGIN IMMEDIATE").await)?;
        if let Some(row) = db(sqlx::query("SELECT namespace,task_id,CASE WHEN length(CAST(input_json AS BLOB))<=131072 THEN input_json ELSE NULL END AS input_json,version FROM task_owner_events WHERE import_event_id=?").bind(&r.import_event_id).fetch_optional(&mut *tx).await)? {
            if row.get::<String,_>("namespace") != r.namespace || row.get::<String,_>("task_id") != r.task_id || row.get::<Option<String>,_>("input_json").as_deref() != Some(input.as_str()) {
                return invalid("owner attestation event conflict");
            }
            let record = load_revision(&mut tx, &r.namespace, &r.task_id, Some(row.get("version"))).await?.ok_or_else(|| AppError::Message("owner receipt revision missing".into()))?;
            let result = TaskWriteResult {record, replayed: true, operation_id: r.import_event_id.clone()};
            db(tx.commit().await)?;
            return Ok(result);
        }
        let mut record = load_revision(&mut tx, &r.namespace, &r.task_id, None)
            .await?
            .ok_or_else(|| {
                AppError::InvalidParams("task not found in requested namespace".into())
            })?;
        if record.version != r.expected_version
            || record.version >= i64::MAX - 1
            || record.status != TaskStatus::Active
        {
            return invalid("owner attestation requires current active task version");
        }
        let source = TaskSource {
            kind: FeedbackSourceKind::CallerReported,
            reference: None,
        };
        match &r.change {
            OwnerRequirementChange::Append {
                requirement_id,
                kind,
                exact_text,
            } => {
                identifier(requirement_id, "requirement_id")?;
                text(exact_text, "exact requirement text", 4096)?;
                if record
                    .requirements
                    .iter()
                    .any(|q| &q.requirement_id == requirement_id)
                {
                    return invalid("requirement ID already exists");
                }
                record.requirements.push(TaskRequirement {
                    requirement_id: requirement_id.clone(),
                    kind: *kind,
                    text: exact_text.clone(),
                    state: RequirementState::Reported,
                    source: source.clone(),
                    replaces: None,
                });
            }
            OwnerRequirementChange::Withdraw {
                requirement_id,
                exact_text,
            } => {
                if !record.owner_provenance.iter().any(|p| {
                    &p.requirement_id == requirement_id && p.withdrawn_at_version.is_none()
                }) {
                    return invalid("withdrawal requires live owner-attested requirement");
                }
                let q = record
                    .requirements
                    .iter_mut()
                    .find(|q| &q.requirement_id == requirement_id)
                    .ok_or_else(|| AppError::Message("owner provenance target missing".into()))?;
                if &q.text != exact_text {
                    return invalid("withdrawal exact text mismatch");
                }
                q.state = RequirementState::Withdrawn;
            }
        }
        record.version += 1;
        record.recorded_at = chrono::Utc::now().to_rfc3339();
        record.source = source;
        record.reason = "Explicit local owner-channel attestation; knowledge only".into();
        // Provenance is persisted separately; final validation occurs after hydration.
        if record.requirements.len() > MAX_TASK_ITEMS {
            return invalid("task requirement history exceeds bounds");
        }
        bounded(&record)?;
        save_revision(&mut tx, &record).await?;
        let changed = db(sqlx::query("UPDATE task_heads SET current_version=? WHERE namespace=? AND task_id=? AND current_version=?").bind(record.version).bind(&r.namespace).bind(&r.task_id).bind(r.expected_version).execute(&mut *tx).await)?;
        if changed.rows_affected() != 1 {
            return invalid("task version conflict");
        }
        db(sqlx::query("INSERT INTO task_owner_events(import_event_id,namespace,task_id,version,input_json) VALUES(?,?,?,?,?)").bind(&r.import_event_id).bind(&r.namespace).bind(&r.task_id).bind(record.version).bind(&input).execute(&mut *tx).await)?;
        match &r.change {
            OwnerRequirementChange::Append {
                requirement_id,
                exact_text,
                kind,
            } => {
                db(sqlx::query("INSERT INTO task_owner_provenance(namespace,task_id,requirement_id,import_event_id,imported_at_version,exact_text,evidence_text,kind_json) VALUES(?,?,?,?,?,?,?,?)").bind(&r.namespace).bind(&r.task_id).bind(requirement_id).bind(&r.import_event_id).bind(record.version).bind(exact_text).bind(&r.evidence_text).bind(serde_json::to_string(kind).map_err(|e| AppError::Message(e.to_string()))?).execute(&mut *tx).await)?;
            }
            OwnerRequirementChange::Withdraw { requirement_id, .. } => {
                db(sqlx::query("UPDATE task_owner_provenance SET withdrawn_at_version=?,withdrawal_event_id=? WHERE namespace=? AND task_id=? AND requirement_id=? AND withdrawn_at_version IS NULL").bind(record.version).bind(&r.import_event_id).bind(&r.namespace).bind(&r.task_id).bind(requirement_id).execute(&mut *tx).await)?;
            }
        }
        let record = load_revision(&mut tx, &r.namespace, &r.task_id, None)
            .await?
            .ok_or_else(|| AppError::Message("task vanished".into()))?;
        let result = TaskWriteResult {
            record,
            replayed: false,
            operation_id: r.import_event_id.clone(),
        };
        db(tx.commit().await)?;
        Ok(result)
    }
}

/// Used by doctor/open/restore verification, so inconsistent authority cannot be
/// reported as a current healthy database merely because its foreign keys pass.
pub(super) async fn validate_all(c: &mut SqliteConnection) -> Result<(), AppError> {
    let tasks = db(sqlx::query("SELECT namespace,task_id FROM task_owner_events UNION SELECT namespace,task_id FROM task_owner_provenance").fetch_all(&mut *c).await)?;
    for task in tasks {
        if load_revision(
            c,
            &task.get::<String, _>("namespace"),
            &task.get::<String, _>("task_id"),
            None,
        )
        .await?
        .is_none()
        {
            return invalid("owner attestation task missing");
        }
    }
    Ok(())
}
