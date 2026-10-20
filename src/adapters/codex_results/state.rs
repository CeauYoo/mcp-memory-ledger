//! Separate private metadata/outbox. Never modifies action-check receipts.
use super::protocol::{MAX_RECORD, token};
use crate::adapters::codex_context::protocol::{Binding, digest};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::{fs, path::Path, time::Duration};
pub const CAPACITY: i64 = 10_000;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Link {
    pub binding: Binding,
    pub turn_id: String,
    pub tool_id: String,
    pub tool_name: String,
    pub input_digest: String,
    pub use_id: String,
    pub task_version: i64,
    pub candidate_id: String,
    pub candidate_version: i64,
    pub decision: crate::domain::experience_use::ExperienceUseDecision,
}
impl Link {
    pub fn key(&self) -> String {
        digest(
            &serde_json::to_vec(&(&self.binding.host_session_id, &self.tool_id))
                .expect("strings serialize"),
        )
    }
    pub fn validate(&self) -> Result<()> {
        for s in [
            &self.binding.host_session_id,
            &self.turn_id,
            &self.tool_id,
            &self.tool_name,
            &self.use_id,
            &self.candidate_id,
            &self.binding.task_id,
            &self.binding.workspace_id,
        ] {
            token(s)?;
        }
        ensure!(
            self.task_version > 0
                && self.candidate_version > 0
                && self.binding.workspace_root.is_absolute(),
            "invalid linkage"
        );
        crate::domain::types::Namespace::parse(self.binding.namespace.clone())
            .map_err(|_| anyhow::anyhow!("invalid namespace"))?;
        fingerprint(&self.input_digest)
    }
}
fn fingerprint(s: &str) -> Result<()> {
    ensure!(
        s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid evidence digest"
    );
    Ok(())
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub response_digest: String,
    pub evidence_digest: String,
    pub response_bytes: usize,
    pub observed_at: String,
}
impl Observation {
    pub fn digest_for(link: &Link, response_digest: &str, response_bytes: usize) -> Result<String> {
        Ok(digest(&serde_json::to_vec(
            &json!({"contract":"codex-0.159.2","session":link.binding.host_session_id,"turn":link.turn_id,"tool":link.tool_name,"tool_id":link.tool_id,"input_digest":link.input_digest,"response_digest":response_digest,"response_bytes":response_bytes}),
        )?))
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub link: Link,
    pub observation: Option<Observation>,
    pub event_id: Option<String>,
    pub event_request: Option<Value>,
    pub use_request: Option<Value>,
    pub use_attempt: u32,
    pub minimum_task_head: i64,
    pub minimum_candidate_head: i64,
    pub event_rejected: bool,
    pub rejected: bool,
    pub completed: bool,
}
impl Record {
    pub fn validate(&self) -> Result<()> {
        self.link.validate()?;
        if let Some(o) = &self.observation {
            fingerprint(&o.response_digest)?;
            fingerprint(&o.evidence_digest)?;
            ensure!(
                o.evidence_digest
                    == Observation::digest_for(&self.link, &o.response_digest, o.response_bytes)?,
                "evidence metadata digest mismatch"
            );
            ensure!(
                o.response_bytes <= super::protocol::MAX_INPUT,
                "invalid observation size"
            );
            chrono::DateTime::parse_from_rfc3339(&o.observed_at)?;
        }
        if let Some(request) = &self.event_request {
            ensure!(
                self.observation.is_some() && *request == super::runtime::event_request(self)?,
                "persisted event request mismatch"
            );
        }
        ensure!(
            (self.event_id.is_none() && !self.event_rejected) || self.event_request.is_some(),
            "event stage missing durable request"
        );
        if let Some(id) = &self.event_id {
            token(id)?;
        }
        ensure!(
            self.observation.is_some()
                || (self.event_id.is_none()
                    && self.use_request.is_none()
                    && !self.event_rejected
                    && !self.rejected
                    && !self.completed),
            "invalid capture stage"
        );
        ensure!(
            !self.event_rejected
                || (self.event_id.is_none() && self.use_request.is_none() && !self.completed),
            "invalid event rejection stage"
        );
        ensure!(
            !self.rejected || self.use_request.is_some(),
            "invalid use rejection stage"
        );
        ensure!(
            !self.completed
                || (self.use_request.is_some() && !self.rejected && !self.event_rejected),
            "invalid completion stage"
        );
        if let Some(request) = &self.use_request {
            ensure!(self.event_id.is_some(), "use request before event");
            let tv = request["expected_task_version"]
                .as_i64()
                .context("invalid task expectation")?;
            let cv = request["expected_candidate_version"]
                .as_i64()
                .context("invalid candidate expectation")?;
            ensure!(
                *request == super::runtime::build_use_request(self, tv, cv)?,
                "persisted request differs from deterministic minimized report"
            );
        }
        ensure!(
            self.minimum_task_head >= self.link.task_version
                && self.minimum_candidate_head >= self.link.candidate_version,
            "head floor rollback"
        );
        ensure!(self.use_attempt <= 32, "retry limit");
        ensure!(
            serde_json::to_vec(self)?.len() <= MAX_RECORD,
            "record exceeds bounds"
        );
        Ok(())
    }
}
pub struct State {
    pool: SqlitePool,
}
impl State {
    pub async fn initialize(path: &Path) -> Result<()> {
        validate_parent(path)?;
        #[cfg(windows)]
        let f = crate::support::windows_security::private_create(path)?;
        #[cfg(not(windows))]
        let f = {
            let mut o = fs::OpenOptions::new();
            o.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                o.mode(0o600);
            }
            o.open(path)?
        };
        drop(f);
        let pool = connect(path).await?;
        sqlx::raw_sql("CREATE TABLE host_result_outbox (id TEXT PRIMARY KEY, payload TEXT NOT NULL CHECK(length(CAST(payload AS BLOB))<=16384)); PRAGMA user_version=1;").execute(&pool).await?;
        pool.close().await;
        Ok(())
    }
    pub async fn open(path: &Path) -> Result<Self> {
        let pool = connect(path).await?;
        let v: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&pool)
            .await?;
        ensure!(v == 1, "unsupported result state schema");
        let _: i64 = sqlx::query_scalar("SELECT count(*) FROM host_result_outbox")
            .fetch_one(&pool)
            .await?;
        Ok(Self { pool })
    }
    pub async fn load(&self, id: &str) -> Result<Record> {
        fingerprint(id)?;
        let s: String = sqlx::query_scalar("SELECT payload FROM host_result_outbox WHERE id=?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .context("action-use linkage missing")?;
        ensure!(s.len() <= MAX_RECORD, "oversized state");
        let r: Record = serde_json::from_str(&s)?;
        r.validate()?;
        ensure!(r.link.key() == id, "state identity mismatch");
        Ok(r)
    }
    pub async fn link(&self, link: Link) -> Result<Record> {
        link.validate()?;
        let id = link.key();
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let old: Option<String> =
            sqlx::query_scalar("SELECT payload FROM host_result_outbox WHERE id=?")
                .bind(&id)
                .fetch_optional(&mut *tx)
                .await?;
        if let Some(old) = old {
            let r: Record = serde_json::from_str(&old)?;
            r.validate()?;
            ensure!(r.link == link, "immutable linkage conflict");
            tx.commit().await?;
            return Ok(r);
        }
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM host_result_outbox")
            .fetch_one(&mut *tx)
            .await?;
        ensure!(n < CAPACITY, "result state capacity reached");
        let r = Record {
            minimum_task_head: link.task_version,
            minimum_candidate_head: link.candidate_version,
            link,
            observation: None,
            event_id: None,
            event_request: None,
            use_request: None,
            use_attempt: 0,
            event_rejected: false,
            rejected: false,
            completed: false,
        };
        r.validate()?;
        sqlx::query("INSERT INTO host_result_outbox(id,payload) VALUES(?,?)")
            .bind(id)
            .bind(serde_json::to_string(&r)?)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(r)
    }
    /// Caller holds per-session lock. CAS protects against stale writers nevertheless.
    pub async fn save(&self, old: &Record, new: &Record) -> Result<()> {
        new.validate()?;
        ensure!(old.link == new.link, "link cannot change");
        ensure!(
            old.event_request.is_none() || old.event_request == new.event_request,
            "event request immutable"
        );
        ensure!(
            new.minimum_task_head >= old.minimum_task_head
                && new.minimum_candidate_head >= old.minimum_candidate_head,
            "head floor cannot decrease"
        );
        ensure!(
            old.observation.is_none() || old.observation == new.observation,
            "captured evidence immutable"
        );
        ensure!(
            old.event_id.is_none() || old.event_id == new.event_id,
            "event identity immutable"
        );
        ensure!(!old.completed || new.completed, "completion cannot regress");
        if new.use_attempt != old.use_attempt {
            ensure!(
                old.rejected
                    && new.use_attempt == old.use_attempt + 1
                    && new.use_request.is_none()
                    && !new.rejected,
                "invalid retry transition"
            );
        } else if old.use_request.is_some() {
            ensure!(
                old.use_request == new.use_request,
                "uncertain request cannot change"
            );
        }

        let n = sqlx::query("UPDATE host_result_outbox SET payload=? WHERE id=? AND payload=?")
            .bind(serde_json::to_string(new)?)
            .bind(old.link.key())
            .bind(serde_json::to_string(old)?)
            .execute(&self.pool)
            .await?
            .rows_affected();
        ensure!(n == 1, "outbox concurrent change");
        Ok(())
    }
}
async fn connect(path: &Path) -> Result<SqlitePool> {
    validate_file(path)?;
    Ok(SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(path)
                .create_if_missing(false)
                .busy_timeout(Duration::from_secs(2)),
        )
        .await?)
}
fn validate_parent(path: &Path) -> Result<()> {
    ensure!(
        path.is_absolute() && path.file_name().is_some(),
        "absolute state path required"
    );
    let parent = path.parent().context("parent missing")?;
    for p in parent.ancestors() {
        let m = fs::symlink_metadata(p)?;
        ensure!(
            m.is_dir() && !m.file_type().is_symlink(),
            "invalid state ancestor"
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let m = fs::metadata(parent)?;
        ensure!(
            m.permissions().mode() & 0o077 == 0 && m.uid() == unsafe { libc::geteuid() },
            "private current-owner directory required"
        );
    }
    #[cfg(windows)]
    crate::support::windows_security::validate_private_parent(path)?;
    Ok(())
}
fn validate_file(path: &Path) -> Result<()> {
    validate_parent(path)?;
    let m = fs::symlink_metadata(path)?;
    ensure!(
        m.is_file() && !m.file_type().is_symlink(),
        "regular state file required"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        ensure!(
            m.nlink() == 1
                && m.permissions().mode() & 0o077 == 0
                && m.uid() == unsafe { libc::geteuid() },
            "private current-owner file required"
        );
    }
    #[cfg(windows)]
    crate::support::windows_security::validate_private_file(path)?;
    Ok(())
}
