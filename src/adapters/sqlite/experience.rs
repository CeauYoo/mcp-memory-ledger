use async_trait::async_trait;
use chrono::Utc;
use serde::{Serialize, de::DeserializeOwned};
use sqlx::{QueryBuilder, Row, Sqlite, SqliteConnection};

use super::SqliteStore;
use crate::{
    domain::{
        event::EventReference,
        experience::*,
        types::{Namespace, Owner},
    },
    error::AppError,
    ports::{StoredWriteReceipt, WriteReceiptRequest, experience_store::ExperienceStore},
};

/// Additive schema v5. Historical projections and legacy episode_events are untouched.
pub(super) const EXPERIENCE_SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS experience_episodes (
    episode_id TEXT PRIMARY KEY,
    namespace TEXT NOT NULL CHECK (namespace IN ('self','world') OR (namespace LIKE 'user/%' AND length(namespace)>5) OR (namespace LIKE 'project/%' AND length(namespace)>8)),
    recorded_at TEXT NOT NULL,
    payload_json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS experience_episode_sources (
    episode_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    PRIMARY KEY (episode_id, event_id),
    FOREIGN KEY (episode_id) REFERENCES experience_episodes(episode_id),
    FOREIGN KEY (event_id) REFERENCES events(event_id)
);
CREATE TABLE IF NOT EXISTS experience_candidates (
    candidate_id TEXT PRIMARY KEY,
    namespace TEXT NOT NULL CHECK (namespace IN ('self','world') OR (namespace LIKE 'user/%' AND length(namespace)>5) OR (namespace LIKE 'project/%' AND length(namespace)>8)),
    current_version INTEGER NOT NULL CHECK (current_version > 0)
);
CREATE TABLE IF NOT EXISTS experience_candidate_versions (
    candidate_id TEXT NOT NULL,
    version INTEGER NOT NULL CHECK (version > 0),
    recorded_at TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending','active','rejected','superseded')),
    kind TEXT NOT NULL CHECK (kind IN ('semantic','procedural')),
    payload_json TEXT NOT NULL,
    search_text TEXT NOT NULL,
    change_kind TEXT NOT NULL CHECK (change_kind IN ('created','status_updated','revised','rolled_back')),
    rollback_target_version INTEGER,
    PRIMARY KEY (candidate_id, version),
    FOREIGN KEY (candidate_id) REFERENCES experience_candidates(candidate_id),
    FOREIGN KEY (candidate_id, rollback_target_version) REFERENCES experience_candidate_versions(candidate_id,version)
);
CREATE TABLE IF NOT EXISTS experience_candidate_sources (
    candidate_id TEXT NOT NULL,
    version INTEGER NOT NULL,
    episode_id TEXT NOT NULL,
    PRIMARY KEY (candidate_id, version, episode_id),
    FOREIGN KEY (candidate_id, version) REFERENCES experience_candidate_versions(candidate_id, version),
    FOREIGN KEY (episode_id) REFERENCES experience_episodes(episode_id)
);
CREATE INDEX IF NOT EXISTS idx_experience_episodes_namespace ON experience_episodes(namespace,episode_id);
CREATE INDEX IF NOT EXISTS idx_experience_candidates_namespace ON experience_candidates(namespace,candidate_id);
CREATE INDEX IF NOT EXISTS idx_experience_episode_sources_event ON experience_episode_sources(event_id,episode_id);
CREATE INDEX IF NOT EXISTS idx_experience_candidate_sources_episode ON experience_candidate_sources(episode_id,candidate_id,version);
"#;

fn db<T>(result: Result<T, sqlx::Error>) -> Result<T, AppError> {
    result.map_err(|e| AppError::Message(format!("experience database operation failed: {e}")))
}
fn json<T: Serialize>(value: &T) -> Result<String, AppError> {
    serde_json::to_string(value).map_err(|e| AppError::Message(e.to_string()))
}
fn decode<T: DeserializeOwned>(value: &str) -> Result<T, AppError> {
    serde_json::from_str(value)
        .map_err(|e| AppError::Message(format!("invalid persisted experience: {e}")))
}
fn missing_source() -> AppError {
    AppError::InvalidParams(
        "experience sources must exist in the exact namespace with its canonical owner".into(),
    )
}
fn conflict() -> AppError {
    AppError::InvalidParams(
        "experience version conflict: expected_version must match the current version".into(),
    )
}
fn not_found() -> AppError {
    AppError::InvalidParams("experience candidate was not found in the requested namespace".into())
}
fn owner_name(namespace: &Namespace) -> &'static str {
    match namespace.derived_owner() {
        Owner::Self_ => "self",
        Owner::User => "user",
        Owner::World => "world",
        Owner::Unknown => unreachable!(),
    }
}

async fn replay<T: DeserializeOwned>(
    conn: &mut SqliteConnection,
    request: &WriteReceiptRequest,
) -> Result<Option<ExperienceWriteResult<T>>, AppError> {
    let row = db(sqlx::query("SELECT request_summary_json,response_summary_json FROM operation_log WHERE operation_id=? AND namespace=? AND actor_id='durable_write_receipt_v1' AND status='ok'")
        .bind(&request.operation_id).bind(&request.namespace).fetch_optional(&mut *conn).await)?;
    row.map(|row| {
        let metadata: serde_json::Value = decode(&row.get::<String, _>("request_summary_json"))?;
        let hash = metadata
            .get("request_hash")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::Message("invalid experience receipt hash".into()))?;
        let receipt = StoredWriteReceipt {
            request_hash: hash.to_owned(),
            result_json: row.get("response_summary_json"),
        };
        let mut result: ExperienceWriteResult<T> = receipt.replay(request)?;
        result.replayed = true;
        Ok(result)
    })
    .transpose()
}
async fn audit<T: Serialize>(
    conn: &mut SqliteConnection,
    request: &WriteReceiptRequest,
    result: &ExperienceWriteResult<T>,
) -> Result<(), AppError> {
    db(sqlx::query("INSERT INTO operation_log (operation_id,occurred_at,namespace,actor_kind,actor_id,entrypoint,operation_kind,status,request_summary_json,response_summary_json,redaction_version) VALUES (?, ?, ?, 'system', 'durable_write_receipt_v1', ?, 'tool', 'ok', ?, ?, 1)")
        .bind(&request.operation_id).bind(Utc::now().to_rfc3339()).bind(&request.namespace).bind(&request.operation)
        .bind(json(&serde_json::json!({"request_hash":request.request_hash,"knowledge_only":true}))?)
        .bind(json(result)?).execute(&mut *conn).await)?;
    Ok(())
}
async fn validate_episode_sources(
    conn: &mut SqliteConnection,
    namespace: &Namespace,
    content: &EpisodeContent,
) -> Result<(), AppError> {
    for reference in &content.source_event_refs {
        let reference = EventReference::parse(reference)?;
        let found: i64 = db(sqlx::query_scalar(
            "SELECT count(*) FROM events WHERE event_id=? AND namespace=? AND owner=?",
        )
        .bind(reference.event_id())
        .bind(namespace.as_str())
        .bind(owner_name(namespace))
        .fetch_one(&mut *conn)
        .await)?;
        if found != 1 {
            return Err(missing_source());
        }
    }
    Ok(())
}
async fn validate_candidate_sources(
    conn: &mut SqliteConnection,
    namespace: &Namespace,
    content: &ExperienceContent,
) -> Result<(), AppError> {
    for id in &content.source_episode_ids {
        let row = db(sqlx::query(
            "SELECT payload_json FROM experience_episodes WHERE episode_id=? AND namespace=?",
        )
        .bind(id)
        .bind(namespace.as_str())
        .fetch_optional(&mut *conn)
        .await)?
        .ok_or_else(missing_source)?;
        let episode: PersistedEpisode = decode(&row.get::<String, _>("payload_json"))?;
        if episode.namespace != *namespace || episode.episode_id != *id {
            return Err(missing_source());
        }
        episode.content.validate()?;
        validate_episode_sources(conn, namespace, &episode.content).await?;
    }
    Ok(())
}
async fn current(
    conn: &mut SqliteConnection,
    namespace: &Namespace,
    id: &str,
) -> Result<Option<ExperienceCandidate>, AppError> {
    let row = db(sqlx::query("SELECT v.payload_json FROM experience_candidates c JOIN experience_candidate_versions v ON v.candidate_id=c.candidate_id AND v.version=c.current_version WHERE c.candidate_id=? AND c.namespace=?")
        .bind(id).bind(namespace.as_str()).fetch_optional(&mut *conn).await)?;
    row.map(|row| decode(&row.get::<String, _>("payload_json")))
        .transpose()
}
async fn insert_version(
    conn: &mut SqliteConnection,
    candidate: &ExperienceCandidate,
) -> Result<(), AppError> {
    db(sqlx::query("INSERT INTO experience_candidate_versions (candidate_id,version,recorded_at,status,kind,payload_json,search_text,change_kind,rollback_target_version) VALUES (?,?,?,?,?,?,?,?,?)")
        .bind(&candidate.candidate_id).bind(candidate.version).bind(candidate.recorded_at.to_rfc3339())
        .bind(candidate.status.as_str()).bind(candidate.content.kind.as_str()).bind(json(candidate)?)
        .bind(candidate.content.search_text().to_lowercase()).bind(candidate.change_kind.as_str()).bind(candidate.rollback_target_version)
        .execute(&mut *conn).await)?;
    for id in &candidate.content.source_episode_ids {
        db(sqlx::query("INSERT INTO experience_candidate_sources (candidate_id,version,episode_id) VALUES (?,?,?)")
            .bind(&candidate.candidate_id).bind(candidate.version).bind(id).execute(&mut *conn).await)?;
    }
    Ok(())
}

#[async_trait]
impl ExperienceStore for SqliteStore {
    async fn create_episode(
        &self,
        request: CreateEpisodeRequest,
    ) -> Result<ExperienceWriteResult<PersistedEpisode>, AppError> {
        let namespace = validate_namespace(&request.namespace)?;
        identifier(&request.episode_id, "episode_id")?;
        request.content.validate()?;
        let receipt = WriteReceiptRequest::new(
            "create_experience_episode",
            namespace.as_str(),
            &request.request_id,
            &request,
        )?;
        let mut tx = db(self.pool.begin_with("BEGIN IMMEDIATE").await)?;
        if let Some(result) = replay(&mut tx, &receipt).await? {
            db(tx.commit().await)?;
            return Ok(result);
        }
        let exists: i64 = db(sqlx::query_scalar(
            "SELECT count(*) FROM experience_episodes WHERE episode_id=?",
        )
        .bind(&request.episode_id)
        .fetch_one(&mut *tx)
        .await)?;
        if exists != 0 {
            return Err(AppError::InvalidParams("episode_id is unavailable; retry with its original request_id or choose another identifier".into()));
        }
        validate_episode_sources(&mut tx, &namespace, &request.content).await?;
        let mut content = request.content;
        content.source_event_refs = content
            .source_event_refs
            .iter()
            .map(|r| EventReference::parse(r).map(|v| v.canonical()))
            .collect::<Result<_, _>>()?;
        // Canonical prefixes count toward the persisted payload budget too.
        content.validate()?;
        let record = PersistedEpisode {
            episode_id: request.episode_id,
            namespace,
            recorded_at: Utc::now(),
            content,
        };
        db(sqlx::query("INSERT INTO experience_episodes (episode_id,namespace,recorded_at,payload_json) VALUES (?,?,?,?)")
            .bind(&record.episode_id).bind(record.namespace.as_str()).bind(record.recorded_at.to_rfc3339()).bind(json(&record)?).execute(&mut *tx).await)?;
        for reference in &record.content.source_event_refs {
            let event = EventReference::parse(reference)?;
            db(sqlx::query(
                "INSERT INTO experience_episode_sources (episode_id,event_id) VALUES (?,?)",
            )
            .bind(&record.episode_id)
            .bind(event.event_id())
            .execute(&mut *tx)
            .await)?;
        }
        let result = ExperienceWriteResult {
            record,
            replayed: false,
            operation_id: receipt.operation_id.clone(),
        };
        audit(&mut tx, &receipt, &result).await?;
        db(tx.commit().await)?;
        Ok(result)
    }
    async fn get_episode(
        &self,
        request: GetEpisodeRequest,
    ) -> Result<Option<PersistedEpisode>, AppError> {
        let namespace = validate_namespace(&request.namespace)?;
        identifier(&request.episode_id, "episode_id")?;
        let row = db(sqlx::query(
            "SELECT payload_json FROM experience_episodes WHERE episode_id=? AND namespace=?",
        )
        .bind(request.episode_id)
        .bind(namespace.as_str())
        .fetch_optional(&self.pool)
        .await)?;
        row.map(|row| decode(&row.get::<String, _>("payload_json")))
            .transpose()
    }
    async fn list_episodes(
        &self,
        request: ListEpisodesRequest,
    ) -> Result<Vec<PersistedEpisode>, AppError> {
        let namespace = validate_namespace(&request.namespace)?;
        validate_limit(request.limit)?;
        if let Some(id) = &request.after_id {
            identifier(id, "after_id")?;
        }
        let rows = db(sqlx::query("SELECT payload_json FROM experience_episodes WHERE namespace=? AND (? IS NULL OR episode_id>?) ORDER BY episode_id LIMIT ?")
            .bind(namespace.as_str()).bind(&request.after_id).bind(&request.after_id).bind(request.limit as i64).fetch_all(&self.pool).await)?;
        rows.iter()
            .map(|row| decode(&row.get::<String, _>("payload_json")))
            .collect()
    }
    async fn create_candidate(
        &self,
        request: CreateCandidateRequest,
    ) -> Result<ExperienceWriteResult<ExperienceCandidate>, AppError> {
        let namespace = validate_namespace(&request.namespace)?;
        identifier(&request.candidate_id, "candidate_id")?;
        request.content.validate()?;
        let receipt = WriteReceiptRequest::new(
            "create_experience_candidate",
            namespace.as_str(),
            &request.request_id,
            &request,
        )?;
        let mut tx = db(self.pool.begin_with("BEGIN IMMEDIATE").await)?;
        if let Some(result) = replay(&mut tx, &receipt).await? {
            db(tx.commit().await)?;
            return Ok(result);
        }
        let exists: i64 = db(sqlx::query_scalar(
            "SELECT count(*) FROM experience_candidates WHERE candidate_id=?",
        )
        .bind(&request.candidate_id)
        .fetch_one(&mut *tx)
        .await)?;
        if exists != 0 {
            return Err(AppError::InvalidParams("candidate_id is unavailable; retry with its original request_id or choose another identifier".into()));
        }
        validate_candidate_sources(&mut tx, &namespace, &request.content).await?;
        let record = ExperienceCandidate {
            candidate_id: request.candidate_id,
            namespace,
            version: 1,
            status: ExperienceStatus::Pending,
            recorded_at: Utc::now(),
            content: request.content,
            change_kind: ExperienceChangeKind::Created,
            previous_version: None,
            rollback_target_version: None,
        };
        db(sqlx::query("INSERT INTO experience_candidates (candidate_id,namespace,current_version) VALUES (?,?,1)").bind(&record.candidate_id).bind(record.namespace.as_str()).execute(&mut *tx).await)?;
        insert_version(&mut tx, &record).await?;
        let result = ExperienceWriteResult {
            record,
            replayed: false,
            operation_id: receipt.operation_id.clone(),
        };
        audit(&mut tx, &receipt, &result).await?;
        db(tx.commit().await)?;
        Ok(result)
    }
    async fn get_candidate(
        &self,
        request: GetCandidateRequest,
    ) -> Result<Option<ExperienceCandidate>, AppError> {
        let namespace = validate_namespace(&request.namespace)?;
        identifier(&request.candidate_id, "candidate_id")?;
        if let Some(version) = request.version {
            validate_version(version)?;
        }
        let row = db(sqlx::query("SELECT v.payload_json FROM experience_candidates c JOIN experience_candidate_versions v ON v.candidate_id=c.candidate_id AND v.version=COALESCE(?,c.current_version) WHERE c.namespace=? AND c.candidate_id=?")
            .bind(request.version).bind(namespace.as_str()).bind(&request.candidate_id).fetch_optional(&self.pool).await)?;
        row.map(|row| decode(&row.get::<String, _>("payload_json")))
            .transpose()
    }
    async fn list_candidates(
        &self,
        request: ListCandidatesRequest,
    ) -> Result<Vec<ExperienceCandidate>, AppError> {
        let namespace = validate_namespace(&request.namespace)?;
        validate_limit(request.limit)?;
        if let Some(id) = &request.after_id {
            identifier(id, "after_id")?;
        }
        let status = request.status.map(ExperienceStatus::as_str);
        let rows = db(sqlx::query("SELECT v.payload_json FROM experience_candidates c JOIN experience_candidate_versions v ON v.candidate_id=c.candidate_id AND v.version=c.current_version WHERE c.namespace=? AND (? IS NULL OR c.candidate_id>?) AND (? IS NULL OR v.status=?) ORDER BY c.candidate_id LIMIT ?")
            .bind(namespace.as_str()).bind(&request.after_id).bind(&request.after_id).bind(status).bind(status).bind(request.limit as i64).fetch_all(&self.pool).await)?;
        rows.iter()
            .map(|row| decode(&row.get::<String, _>("payload_json")))
            .collect()
    }
    async fn update_candidate_status(
        &self,
        request: UpdateCandidateStatusRequest,
    ) -> Result<ExperienceWriteResult<ExperienceCandidate>, AppError> {
        let receipt = WriteReceiptRequest::new(
            "update_experience_status",
            &request.namespace,
            &request.request_id,
            &request,
        )?;
        self.mutate_experience(
            &request.namespace,
            &request.candidate_id,
            request.expected_version,
            receipt,
            Mutation::Status(request.status),
        )
        .await
    }
    async fn revise_candidate(
        &self,
        request: ReviseCandidateRequest,
    ) -> Result<ExperienceWriteResult<ExperienceCandidate>, AppError> {
        request.content.validate()?;
        let receipt = WriteReceiptRequest::new(
            "revise_experience_candidate",
            &request.namespace,
            &request.request_id,
            &request,
        )?;
        self.mutate_experience(
            &request.namespace,
            &request.candidate_id,
            request.expected_version,
            receipt,
            Mutation::Revise(request.content),
        )
        .await
    }
    async fn rollback_candidate(
        &self,
        request: RollbackCandidateRequest,
    ) -> Result<ExperienceWriteResult<ExperienceCandidate>, AppError> {
        validate_version(request.target_version)?;
        let receipt = WriteReceiptRequest::new(
            "rollback_experience_candidate",
            &request.namespace,
            &request.request_id,
            &request,
        )?;
        self.mutate_experience(
            &request.namespace,
            &request.candidate_id,
            request.expected_version,
            receipt,
            Mutation::Rollback(request.target_version),
        )
        .await
    }
    async fn recall_candidates(
        &self,
        request: RecallCandidatesRequest,
    ) -> Result<ExperienceRecall, AppError> {
        let terms = recall_terms(&request)?;
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT v.payload_json FROM experience_candidates c JOIN experience_candidate_versions v ON v.candidate_id=c.candidate_id AND v.version=c.current_version WHERE c.namespace=",
        );
        query
            .push_bind(&request.namespace)
            .push(" AND v.status='active'");
        for term in terms {
            query
                .push(" AND instr(v.search_text,")
                .push_bind(term)
                .push(")>0");
        }
        query
            .push(" ORDER BY c.candidate_id LIMIT ")
            .push_bind(request.limit as i64 + 1);
        let rows = db(query.build().fetch_all(&self.pool).await)?;
        let mut result = ExperienceRecall {
            candidates: Vec::new(), used_bytes: 0, truncated: rows.len() > request.limit,
            selection_reason: "Current active versions; all literal terms in caller-authored text; exact namespace; identifier order. Complete JSON byte budget. Knowledge does not authorize execution.".into(),
        };
        update_recall_size(&mut result)?;
        for row in rows.into_iter().take(request.limit) {
            let candidate: ExperienceCandidate = decode(&row.get::<String, _>("payload_json"))?;
            result.candidates.push(candidate);
            update_recall_size(&mut result)?;
            if result.used_bytes > request.max_bytes {
                result.candidates.pop();
                result.truncated = true;
                update_recall_size(&mut result)?;
            }
        }
        Ok(result)
    }
}

fn update_recall_size(result: &mut ExperienceRecall) -> Result<(), AppError> {
    // The count is itself part of the response, so account for its decimal width.
    loop {
        let size = json(result)?.len();
        if size == result.used_bytes {
            return Ok(());
        }
        result.used_bytes = size;
    }
}

enum Mutation {
    Status(ExperienceStatus),
    Revise(ExperienceContent),
    Rollback(i64),
}
impl SqliteStore {
    async fn mutate_experience(
        &self,
        namespace: &str,
        id: &str,
        expected_version: i64,
        receipt: WriteReceiptRequest,
        mutation: Mutation,
    ) -> Result<ExperienceWriteResult<ExperienceCandidate>, AppError> {
        let namespace = validate_namespace(namespace)?;
        identifier(id, "candidate_id")?;
        validate_version(expected_version)?;
        let mut tx = db(self.pool.begin_with("BEGIN IMMEDIATE").await)?;
        if let Some(result) = replay(&mut tx, &receipt).await? {
            db(tx.commit().await)?;
            return Ok(result);
        }
        let mut record = current(&mut tx, &namespace, id)
            .await?
            .ok_or_else(not_found)?;
        if record.version != expected_version {
            return Err(conflict());
        }
        record.previous_version = Some(record.version);
        record.version += 1;
        validate_version(record.version)?;
        record.recorded_at = Utc::now();
        record.rollback_target_version = None;
        match mutation {
            Mutation::Status(status) => {
                record.status.validate_transition(status)?;
                record.status = status;
                record.change_kind = ExperienceChangeKind::StatusUpdated;
            }
            Mutation::Revise(content) => {
                record.content = content;
                record.status = ExperienceStatus::Pending;
                record.change_kind = ExperienceChangeKind::Revised;
            }
            Mutation::Rollback(target) => {
                if target >= expected_version {
                    return Err(AppError::InvalidParams(
                        "rollback target must be an earlier version of this candidate".into(),
                    ));
                }
                let row = db(sqlx::query("SELECT payload_json FROM experience_candidate_versions WHERE candidate_id=? AND version=?")
                    .bind(id).bind(target).fetch_optional(&mut *tx).await)?.ok_or_else(|| AppError::InvalidParams("rollback target version does not exist".into()))?;
                let historical: ExperienceCandidate =
                    decode(&row.get::<String, _>("payload_json"))?;
                record.content = historical.content;
                record.status = ExperienceStatus::Pending;
                record.change_kind = ExperienceChangeKind::RolledBack;
                record.rollback_target_version = Some(target);
            }
        }
        record.content.validate()?;
        // Recheck even for activation and rollback; references alone never prove truth.
        validate_candidate_sources(&mut tx, &namespace, &record.content).await?;
        let changed = db(sqlx::query("UPDATE experience_candidates SET current_version=? WHERE candidate_id=? AND namespace=? AND current_version=?")
            .bind(record.version).bind(id).bind(namespace.as_str()).bind(expected_version).execute(&mut *tx).await)?;
        if changed.rows_affected() != 1 {
            return Err(conflict());
        }
        insert_version(&mut tx, &record).await?;
        let result = ExperienceWriteResult {
            record,
            replayed: false,
            operation_id: receipt.operation_id.clone(),
        };
        audit(&mut tx, &receipt, &result).await?;
        db(tx.commit().await)?;
        Ok(result)
    }
}
