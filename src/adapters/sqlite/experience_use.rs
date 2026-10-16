//! Schema 9 observations with exact relational provenance and read-time invalidation.
use super::{
    SqliteStore,
    experience::{audit, replay, validate_candidate_sources},
};
use crate::{
    domain::{
        event::EventReference,
        experience::{ExperienceCandidate, ExperienceStatus, ExperienceWriteResult},
        experience_use::*,
        task::{invalid, scope},
    },
    error::AppError,
    ports::{WriteReceiptRequest, experience_use_store::ExperienceUseStore},
};
use async_trait::async_trait;
use chrono::Utc;
use sqlx::{Row, SqliteConnection};
pub(super) const EXPERIENCE_USE_SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS experience_uses (
 namespace TEXT NOT NULL, use_id TEXT NOT NULL, task_id TEXT NOT NULL, task_version INTEGER NOT NULL,
 candidate_id TEXT NOT NULL, candidate_version INTEGER NOT NULL, related_use_id TEXT,
 payload_json TEXT NOT NULL CHECK(length(CAST(payload_json AS BLOB))<=36864),
 PRIMARY KEY(namespace,use_id),
 FOREIGN KEY(namespace,task_id,task_version) REFERENCES task_revisions(namespace,task_id,version),
 FOREIGN KEY(candidate_id,candidate_version) REFERENCES experience_candidate_versions(candidate_id,version),
 FOREIGN KEY(namespace,related_use_id) REFERENCES experience_uses(namespace,use_id)
);
CREATE TABLE IF NOT EXISTS experience_use_events (
 namespace TEXT NOT NULL,use_id TEXT NOT NULL,role TEXT NOT NULL CHECK(role IN ('action','outcome')),event_id TEXT NOT NULL,
 PRIMARY KEY(namespace,use_id,role,event_id),
 FOREIGN KEY(namespace,use_id) REFERENCES experience_uses(namespace,use_id),
 FOREIGN KEY(event_id) REFERENCES events(event_id)
);
CREATE INDEX IF NOT EXISTS idx_experience_use_task ON experience_uses(namespace,task_id,use_id);
CREATE INDEX IF NOT EXISTS idx_experience_use_candidate ON experience_uses(candidate_id,candidate_version);
"#;
pub(super) async fn install(c: &mut SqliteConnection) -> Result<(), AppError> {
    for s in EXPERIENCE_USE_SCHEMA_SQL
        .split(';')
        .filter(|s| !s.trim().is_empty())
    {
        db(sqlx::query(s).execute(&mut *c).await)?;
    }
    Ok(())
}
fn db<T>(r: Result<T, sqlx::Error>) -> Result<T, AppError> {
    r.map_err(|_| AppError::Message("experience use database operation failed".into()))
}
fn json<T: serde::Serialize>(v: &T) -> Result<String, AppError> {
    serde_json::to_string(v).map_err(|e| AppError::Message(e.to_string()))
}
fn decode(s: &str) -> Result<ExperienceUse, AppError> {
    if s.len() > 36864 {
        return invalid("persisted experience use exceeds bounds");
    }
    let r: ExperienceUse = serde_json::from_str(s)
        .map_err(|_| AppError::Message("invalid experience use payload".into()))?;
    r.observation.validate()?;
    Ok(r)
}
async fn event(c: &mut SqliteConnection, ns: &str, r: &str) -> Result<String, AppError> {
    let e = EventReference::parse(r)?;
    let count: i64 = db(sqlx::query_scalar(
        "SELECT count(*) FROM events WHERE namespace=? AND event_id=? AND owner='world'",
    )
    .bind(ns)
    .bind(e.event_id())
    .fetch_one(c)
    .await)?;
    if count != 1 {
        return invalid("use event must exist in the exact project namespace");
    }
    Ok(e.event_id().into())
}
async fn load(
    c: &mut SqliteConnection,
    ns: &str,
    id: &str,
) -> Result<Option<ExperienceUse>, AppError> {
    let row=db(sqlx::query("SELECT namespace,use_id,task_id,task_version,candidate_id,candidate_version,related_use_id,payload_json FROM experience_uses WHERE namespace=? AND use_id=?").bind(ns).bind(id).fetch_optional(&mut *c).await)?;
    let Some(row) = row else { return Ok(None) };
    let r = decode(&row.get::<String, _>("payload_json"))?;
    let o = &r.observation;
    if o.namespace != ns
        || o.use_id != id
        || o.task_id != row.get::<String, _>("task_id")
        || o.task_version != row.get::<i64, _>("task_version")
        || o.candidate_id != row.get::<String, _>("candidate_id")
        || o.candidate_version != row.get::<i64, _>("candidate_version")
        || o.related_use_id != row.get::<Option<String>, _>("related_use_id")
    {
        return invalid("experience use payload/relationship mismatch");
    }
    let rows = db(sqlx::query(
        "SELECT role,event_id FROM experience_use_events WHERE namespace=? AND use_id=?",
    )
    .bind(ns)
    .bind(id)
    .fetch_all(&mut *c)
    .await)?;
    let mut expected = std::collections::BTreeSet::new();
    for (role, refs) in [
        ("action", &o.action_event_refs),
        ("outcome", &o.outcome_event_refs),
    ] {
        for reference in refs {
            expected.insert((
                role.to_owned(),
                EventReference::parse(reference.as_str())?
                    .event_id()
                    .to_owned(),
            ));
        }
    }
    let actual: std::collections::BTreeSet<_> = rows
        .iter()
        .map(|r| (r.get::<String, _>("role"), r.get::<String, _>("event_id")))
        .collect();
    if expected != actual {
        return invalid("experience use event edges do not match payload");
    }
    Ok(Some(r))
}
async fn candidate(
    c: &mut SqliteConnection,
    ns: &str,
    id: &str,
    version: i64,
) -> Result<ExperienceCandidate, AppError> {
    let row=db(sqlx::query("SELECT v.payload_json FROM experience_candidates h JOIN experience_candidate_versions v ON h.candidate_id=v.candidate_id WHERE h.namespace=? AND h.candidate_id=? AND v.version=?").bind(ns).bind(id).bind(version).fetch_optional(&mut *c).await)?.ok_or_else(||AppError::InvalidParams("candidate version not found in scope".into()))?;
    let r: ExperienceCandidate = serde_json::from_str(&row.get::<String, _>("payload_json"))
        .map_err(|_| AppError::Message("invalid candidate payload".into()))?;
    if r.namespace.as_str() != ns || r.candidate_id != id || r.version != version {
        return invalid("candidate payload/relationship mismatch");
    }
    r.content.validate()?;
    validate_candidate_sources(c, &scope(ns)?, &r.content).await?;
    Ok(r)
}
async fn view(
    c: &mut SqliteConnection,
    record: ExperienceUse,
) -> Result<ExperienceUseView, AppError> {
    let r = &record.observation;
    let mut reasons = vec![];
    let task:Option<(i64,String)>=db(sqlx::query_as("SELECT h.current_version,v.status FROM task_heads h JOIN task_revisions v ON h.namespace=v.namespace AND h.task_id=v.task_id AND h.current_version=v.version WHERE h.namespace=? AND h.task_id=?").bind(&r.namespace).bind(&r.task_id).fetch_optional(&mut *c).await)?;
    if task
        .as_ref()
        .map(|(v, s)| *v == r.task_version && s == "active")
        != Some(true)
    {
        reasons.push("task_revision_no_longer_current_active".into());
    }
    let head: Option<i64> = db(sqlx::query_scalar(
        "SELECT current_version FROM experience_candidates WHERE namespace=? AND candidate_id=?",
    )
    .bind(&r.namespace)
    .bind(&r.candidate_id)
    .fetch_optional(&mut *c)
    .await)?;
    if head != Some(r.candidate_version) {
        reasons.push("candidate_version_no_longer_current".into());
    }
    match candidate(c, &r.namespace, &r.candidate_id, r.candidate_version).await {
        Ok(v) if v.status == ExperienceStatus::Active => {}
        Ok(_) => reasons.push("candidate_not_active".into()),
        Err(_) => reasons.push("candidate_sources_or_payload_invalid".into()),
    }
    for reference in r.action_event_refs.iter().chain(&r.outcome_event_refs) {
        if event(c, &r.namespace, reference).await.is_err() {
            reasons.push("use_event_source_invalid".into());
            break;
        }
    }
    if r.decision == ExperienceUseDecision::Rejected {
        reasons.push("use_was_rejected".into());
    }
    if r.conditions
        .iter()
        .any(|c| c.assessment != ApplicabilityAssessment::CallerReportedSatisfied)
    {
        reasons.push("applicability_not_reported_satisfied".into());
    }
    Ok(ExperienceUseView {
        currently_reusable: reasons.is_empty(),
        record,
        invalidation_reasons: reasons,
    })
}
#[async_trait]
impl ExperienceUseStore for SqliteStore {
    async fn prepare_experience_derivation(
        &self,
        r: DeriveExperienceRequest,
    ) -> Result<ExperienceWriteResult<DerivationReservation>, AppError> {
        r.validate()?;
        let receipt = WriteReceiptRequest::new(
            "derive_experience_from_use",
            &r.namespace,
            &r.request_id,
            &r,
        )?;
        let mut tx = db(self.pool.begin_with("BEGIN IMMEDIATE").await)?;
        if let Some(result) = replay(&mut tx, &receipt).await? {
            db(tx.commit().await)?;
            return Ok(result);
        }
        let source = load(&mut tx, &r.namespace, &r.use_id)
            .await?
            .ok_or_else(|| AppError::InvalidParams("source use not found in scope".into()))?;
        let allowed: std::collections::BTreeSet<String> = source
            .observation
            .action_event_refs
            .iter()
            .chain(&source.observation.outcome_event_refs)
            .map(|r| EventReference::parse(r).map(|e| e.event_id().to_owned()))
            .collect::<Result<_, _>>()?;
        for reference in &r.episode_content.source_event_refs {
            let id = event(&mut tx, &r.namespace, reference).await?;
            if !allowed.contains(&id) {
                return invalid(
                    "derived episode references must come from the selected use action/outcome events",
                );
            }
        }
        let result = ExperienceWriteResult {
            record: DerivationReservation {
                use_id: r.use_id,
                episode_id: r.episode_id,
                candidate_id: r.candidate_id,
            },
            replayed: false,
            operation_id: receipt.operation_id.clone(),
        };
        audit(&mut tx, &receipt, &result).await?;
        db(tx.commit().await)?;
        Ok(result)
    }

    async fn record_experience_use(
        &self,
        mut r: RecordExperienceUseRequest,
    ) -> Result<ExperienceWriteResult<ExperienceUse>, AppError> {
        r.validate()?;
        let receipt =
            WriteReceiptRequest::new("record_experience_use", &r.namespace, &r.request_id, &r)?;
        let mut tx = db(self.pool.begin_with("BEGIN IMMEDIATE").await)?;
        if let Some(result) = replay(&mut tx, &receipt).await? {
            db(tx.commit().await)?;
            return Ok(result);
        }
        let current_task: Option<i64> = db(sqlx::query_scalar(
            "SELECT current_version FROM task_heads WHERE namespace=? AND task_id=?",
        )
        .bind(&r.namespace)
        .bind(&r.task_id)
        .fetch_optional(&mut *tx)
        .await)?;
        let current_candidate:Option<i64>=db(sqlx::query_scalar("SELECT current_version FROM experience_candidates WHERE namespace=? AND candidate_id=?").bind(&r.namespace).bind(&r.candidate_id).fetch_optional(&mut *tx).await)?;
        if current_task != Some(r.expected_task_version)
            || current_candidate != Some(r.expected_candidate_version)
        {
            return invalid(
                "experience use CAS conflict: expected task and candidate heads must match",
            );
        }
        let task_exists: i64 = db(sqlx::query_scalar(
            "SELECT count(*) FROM task_revisions WHERE namespace=? AND task_id=? AND version=?",
        )
        .bind(&r.namespace)
        .bind(&r.task_id)
        .bind(r.task_version)
        .fetch_one(&mut *tx)
        .await)?;
        if task_exists != 1 {
            return invalid("exact task revision not found in scope");
        }
        let selected =
            candidate(&mut tx, &r.namespace, &r.candidate_id, r.candidate_version).await?;
        if let Some(id) = &r.related_use_id {
            let prior = load(&mut tx, &r.namespace, id)
                .await?
                .ok_or_else(|| AppError::InvalidParams("related use not found in scope".into()))?;
            let p = prior.observation;
            if p.task_id != r.task_id
                || p.task_version != r.task_version
                || p.candidate_id != r.candidate_id
                || p.candidate_version != r.candidate_version
                || p.decision != r.decision
            {
                return invalid(
                    "related result must preserve exact task/candidate versions and historical decision",
                );
            }
        } else if r.decision != ExperienceUseDecision::Rejected
            && (r.task_version != r.expected_task_version
                || r.candidate_version != r.expected_candidate_version
                || selected.status != ExperienceStatus::Active)
        {
            return invalid(
                "new proposals/adoptions require current active candidate and current task revision",
            );
        }
        if r.related_use_id.is_none() && r.decision != ExperienceUseDecision::Rejected {
            let status: String = db(sqlx::query_scalar(
                "SELECT status FROM task_revisions WHERE namespace=? AND task_id=? AND version=?",
            )
            .bind(&r.namespace)
            .bind(&r.task_id)
            .bind(r.task_version)
            .fetch_one(&mut *tx)
            .await)?;
            if status != "active" {
                return invalid("new proposals/adoptions require active task");
            }
        }
        for refs in [&mut r.action_event_refs, &mut r.outcome_event_refs] {
            for reference in refs.iter_mut() {
                event(&mut tx, &r.namespace, reference).await?;
                *reference = EventReference::parse(reference.as_str())?.canonical();
            }
        }
        r.validate()?;
        let record = ExperienceUse {
            observation: r.into(),
            recorded_at: Utc::now(),
            verification_basis: "caller_reported_not_independently_verified_no_causal_attribution"
                .into(),
        };
        let r = &record.observation;
        db(sqlx::query("INSERT INTO experience_uses(namespace,use_id,task_id,task_version,candidate_id,candidate_version,related_use_id,payload_json) VALUES(?,?,?,?,?,?,?,?)").bind(&r.namespace).bind(&r.use_id).bind(&r.task_id).bind(r.task_version).bind(&r.candidate_id).bind(r.candidate_version).bind(&r.related_use_id).bind(json(&record)?).execute(&mut *tx).await)?;
        for (role, refs) in [
            ("action", &r.action_event_refs),
            ("outcome", &r.outcome_event_refs),
        ] {
            for reference in refs {
                db(sqlx::query("INSERT INTO experience_use_events(namespace,use_id,role,event_id) VALUES(?,?,?,?)").bind(&r.namespace).bind(&r.use_id).bind(role).bind(EventReference::parse(reference.as_str())?.event_id()).execute(&mut *tx).await)?;
            }
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
    async fn list_experience_uses(
        &self,
        r: ListExperienceUsesRequest,
    ) -> Result<Vec<ExperienceUseView>, AppError> {
        r.validate()?;
        let mut tx = db(self.pool.begin().await)?;
        let ids:Vec<String>=db(sqlx::query_scalar("SELECT use_id FROM experience_uses WHERE namespace=? AND task_id=? AND (? IS NULL OR use_id>?) ORDER BY use_id LIMIT ?").bind(&r.namespace).bind(&r.task_id).bind(&r.after_id).bind(&r.after_id).bind(r.limit as i64).fetch_all(&mut *tx).await)?;
        let mut out = vec![];
        for id in ids {
            let record = load(&mut tx, &r.namespace, &id)
                .await?
                .ok_or_else(|| AppError::Message("use disappeared inside snapshot".into()))?;
            out.push(view(&mut tx, record).await?);
        }
        db(tx.commit().await)?;
        Ok(out)
    }
}
