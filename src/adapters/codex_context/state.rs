//! Adapter-only state. It is not part of the ledger backup/export contract.
use super::protocol::{Binding, ReceiptStage, identifier};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::{fs, path::Path, time::Duration};

pub const SCHEMA_VERSION: i64 = 2;
const OUTPUT_SCHEMA: &str = "CREATE TABLE context_outputs (receipt_id TEXT PRIMARY KEY REFERENCES context_receipts(receipt_id), output_digest TEXT NOT NULL CHECK(length(output_digest)=64), context_fingerprint TEXT NOT NULL CHECK(length(context_fingerprint)=64), output_kind TEXT NOT NULL CHECK(output_kind IN ('hook','refresh')), output_bytes INTEGER NOT NULL CHECK(output_bytes > 0 AND output_bytes <= 32768)); PRAGMA user_version = 2;";

pub struct State {
    pool: SqlitePool,
}
impl State {
    pub async fn initialize(path: &Path) -> Result<()> {
        validate_parent(path)?;
        #[cfg(windows)]
        let file = crate::support::windows_security::private_create(path)?;
        #[cfg(not(windows))]
        let file = {
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            options
                .open(path)
                .context("state already exists or cannot be created; no overwrite")?
        };
        drop(file);
        let pool = connect(path).await?;
        let mut tx = pool.begin().await?;
        sqlx::raw_sql("CREATE TABLE adapter_bindings (host_session_id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL, namespace TEXT NOT NULL, task_id TEXT NOT NULL, workspace_root TEXT NOT NULL, minimum_version INTEGER NOT NULL DEFAULT 0); CREATE TABLE adapter_defaults (workspace_root TEXT PRIMARY KEY, workspace_id TEXT NOT NULL, namespace TEXT NOT NULL, task_id TEXT NOT NULL); CREATE TABLE context_receipts (receipt_id TEXT PRIMARY KEY, host_session_id TEXT NOT NULL, workspace_id TEXT NOT NULL, task_id TEXT NOT NULL, context_id TEXT NOT NULL, task_version INTEGER NOT NULL, context_digest TEXT NOT NULL, hook_event TEXT NOT NULL, source TEXT, stage TEXT NOT NULL CHECK(stage IN ('core_built','output_written')), created_at TEXT NOT NULL); PRAGMA user_version = 1;").execute(&mut *tx).await?;
        sqlx::raw_sql(OUTPUT_SCHEMA).execute(&mut *tx).await?;
        tx.commit().await?;
        pool.close().await;
        Ok(())
    }
    pub async fn open(path: &Path) -> Result<Self> {
        validate_file(path)?;
        let pool = connect(path).await?;
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&pool)
            .await?;
        ensure!(
            version == SCHEMA_VERSION,
            "adapter state version unsupported; explicit migration required"
        );
        Ok(Self { pool })
    }
    /// Operator-only, explicit adapter migration. Hook/refresh/open never migrate.
    pub async fn migrate(path: &Path) -> Result<()> {
        validate_file(path)?;
        let pool = connect(path).await?;
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&mut *tx)
            .await?;
        ensure!(
            version == 1 || version == SCHEMA_VERSION,
            "adapter state version unsupported"
        );
        if version == 1 {
            // Version alone is not proof that this is an intact adapter database.
            for sql in [
                "SELECT host_session_id,workspace_id,namespace,task_id,workspace_root,minimum_version FROM adapter_bindings LIMIT 0",
                "SELECT workspace_root,workspace_id,namespace,task_id FROM adapter_defaults LIMIT 0",
                "SELECT receipt_id,host_session_id,workspace_id,task_id,context_id,task_version,context_digest,hook_event,source,stage,created_at FROM context_receipts LIMIT 0",
            ] {
                sqlx::query(sql).execute(&mut *tx).await?;
            }
            sqlx::raw_sql(OUTPUT_SCHEMA).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        pool.close().await;
        Ok(())
    }
    pub async fn bind(&self, binding: &Binding) -> Result<()> {
        for value in [
            &binding.host_session_id,
            &binding.workspace_id,
            &binding.task_id,
        ] {
            identifier(value)?;
        }
        let root = binding
            .workspace_root
            .to_str()
            .context("workspace root is not UTF-8")?;
        sqlx::query("INSERT INTO adapter_bindings(host_session_id,workspace_id,namespace,task_id,workspace_root) VALUES(?,?,?,?,?) ON CONFLICT(host_session_id) DO NOTHING")
            .bind(&binding.host_session_id).bind(&binding.workspace_id).bind(&binding.namespace).bind(&binding.task_id).bind(root).execute(&self.pool).await?;
        let (existing, _) = self.binding(&binding.host_session_id).await?;
        ensure!(
            existing == *binding,
            "session already bound differently; use a new session, never guess or overwrite"
        );
        Ok(())
    }
    pub async fn has_binding(&self, session: &str) -> Result<bool> {
        Ok(sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM adapter_bindings WHERE host_session_id=?",
        )
        .bind(session)
        .fetch_one(&self.pool)
        .await?
            > 0)
    }
    pub async fn select_default(&self, binding: &Binding) -> Result<()> {
        sqlx::query("INSERT INTO adapter_defaults VALUES(?,?,?,?) ON CONFLICT(workspace_root) DO UPDATE SET workspace_id=excluded.workspace_id,namespace=excluded.namespace,task_id=excluded.task_id")
            .bind(binding.workspace_root.to_str().context("root is not UTF-8")?).bind(&binding.workspace_id).bind(&binding.namespace).bind(&binding.task_id).execute(&self.pool).await?;
        Ok(())
    }
    pub async fn default_for(&self, session: &str, cwd: &Path) -> Result<Binding> {
        let rows = sqlx::query("SELECT * FROM adapter_defaults LIMIT 1025")
            .fetch_all(&self.pool)
            .await?;
        ensure!(
            rows.len() <= 1024,
            "too many defaults; explicit session binding required"
        );
        let mut candidates = Vec::new();
        for row in rows {
            let root = std::path::PathBuf::from(row.try_get::<String, _>("workspace_root")?);
            if cwd.starts_with(&root) {
                candidates.push(Binding {
                    host_session_id: session.to_owned(),
                    workspace_id: row.try_get("workspace_id")?,
                    namespace: row.try_get("namespace")?,
                    task_id: row.try_get("task_id")?,
                    workspace_root: root,
                });
            }
        }
        ensure!(
            candidates.len() == 1,
            "no unique explicitly selected default; select the task"
        );
        Ok(candidates.remove(0))
    }
    pub async fn binding(&self, session: &str) -> Result<(Binding, u64)> {
        identifier(session)?;
        let row = sqlx::query("SELECT * FROM adapter_bindings WHERE host_session_id=?").bind(session).fetch_optional(&self.pool).await?.context("session unbound; explicitly select workspace and task (forks need their own binding)")?;
        Ok((
            Binding {
                host_session_id: row.try_get("host_session_id")?,
                workspace_id: row.try_get("workspace_id")?,
                namespace: row.try_get("namespace")?,
                task_id: row.try_get("task_id")?,
                workspace_root: std::path::PathBuf::from(
                    row.try_get::<String, _>("workspace_root")?,
                ),
            },
            row.try_get::<i64, _>("minimum_version")? as u64,
        ))
    }
    pub async fn built(
        &self,
        binding: &Binding,
        context_id: &str,
        version: u64,
        digest: &str,
        event: &str,
        source: Option<&str>,
    ) -> Result<String> {
        let receipt = uuid::Uuid::new_v4().to_string();
        let mut tx = self.pool.begin().await?;
        let minimum: i64 = sqlx::query_scalar(
            "SELECT minimum_version FROM adapter_bindings WHERE host_session_id=?",
        )
        .bind(&binding.host_session_id)
        .fetch_one(&mut *tx)
        .await?;
        ensure!(
            version >= minimum as u64,
            "stale task version; no receipt published"
        );
        sqlx::query("UPDATE adapter_bindings SET minimum_version=MAX(minimum_version,?) WHERE host_session_id=?").bind(version as i64).bind(&binding.host_session_id).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO context_receipts VALUES(?,?,?,?,?,?,?,?,?,?,?)")
            .bind(&receipt)
            .bind(&binding.host_session_id)
            .bind(&binding.workspace_id)
            .bind(&binding.task_id)
            .bind(context_id)
            .bind(version as i64)
            .bind(digest)
            .bind(event)
            .bind(source)
            .bind("core_built")
            .bind(chrono::Utc::now().to_rfc3339())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(receipt)
    }
    /// Persist exact serialized output identity BEFORE writing stdout. A prepared
    /// output is not evidence that any bytes were written or accepted by a host.
    pub async fn prepare_output(
        &self,
        receipt: &str,
        output: &Value,
        fingerprint: &str,
        kind: &str,
    ) -> Result<()> {
        identifier(receipt)?;
        super::protocol::fingerprint(fingerprint)?;
        ensure!(
            matches!(kind, "hook" | "refresh"),
            "unsupported output kind"
        );
        let mut bytes = serde_json::to_vec(output)?;
        bytes.push(b'\n');
        ensure!(
            bytes.len() <= super::protocol::MAX_CONTEXT_BYTES,
            "output exceeds limit"
        );
        let hash = super::protocol::digest(&bytes);
        let mut tx = self.pool.begin().await?;
        let stage: Option<String> =
            sqlx::query_scalar("SELECT stage FROM context_receipts WHERE receipt_id=?")
                .bind(receipt)
                .fetch_optional(&mut *tx)
                .await?;
        ensure!(
            stage.as_deref() == Some("core_built"),
            "receipt missing or already written"
        );
        sqlx::query(
            "INSERT INTO context_outputs VALUES(?,?,?,?,?) ON CONFLICT(receipt_id) DO NOTHING",
        )
        .bind(receipt)
        .bind(&hash)
        .bind(fingerprint)
        .bind(kind)
        .bind(bytes.len() as i64)
        .execute(&mut *tx)
        .await?;
        let row = sqlx::query("SELECT output_digest,context_fingerprint,output_kind,output_bytes FROM context_outputs WHERE receipt_id=?").bind(receipt).fetch_one(&mut *tx).await?;
        ensure!(
            row.try_get::<String, _>("output_digest")? == hash
                && row.try_get::<String, _>("context_fingerprint")? == fingerprint
                && row.try_get::<String, _>("output_kind")? == kind
                && row.try_get::<i64, _>("output_bytes")? == bytes.len() as i64,
            "conflicting prepared output; no overwrite"
        );
        tx.commit().await?;
        Ok(())
    }
    pub async fn output_written(&self, receipt: &str) -> Result<()> {
        // No generic stage setter: untrusted claims cannot mark delivery.
        let changed = sqlx::query("UPDATE context_receipts SET stage='output_written' WHERE receipt_id=? AND stage='core_built' AND EXISTS(SELECT 1 FROM context_outputs o WHERE o.receipt_id=context_receipts.receipt_id)").bind(receipt).execute(&self.pool).await?;
        ensure!(
            changed.rows_affected() == 1,
            "receipt missing or stage changed"
        );
        Ok(())
    }
    pub async fn inspect(&self, session: &str) -> Result<Value> {
        let (binding, minimum) = self.binding(session).await?;
        let rows = sqlx::query("SELECT r.receipt_id,context_id,task_version,context_digest,hook_event,source,stage,created_at,output_digest,context_fingerprint,output_kind,output_bytes FROM context_receipts r LEFT JOIN context_outputs o ON o.receipt_id=r.receipt_id WHERE host_session_id=? ORDER BY created_at DESC,r.receipt_id DESC LIMIT 20").bind(session).fetch_all(&self.pool).await?;
        let mut receipts = Vec::new();
        for row in rows {
            receipts.push(json!({"receipt_id":row.try_get::<String,_>("receipt_id")?,"context_id":row.try_get::<String,_>("context_id")?,"task_version":row.try_get::<i64,_>("task_version")?,"context_digest":row.try_get::<String,_>("context_digest")?,"hook_event":row.try_get::<String,_>("hook_event")?,"source":row.try_get::<Option<String>,_>("source")?,"stage":row.try_get::<String,_>("stage")?,"created_at":row.try_get::<String,_>("created_at")?,"output_digest":row.try_get::<Option<String>,_>("output_digest")?,"context_fingerprint":row.try_get::<Option<String>,_>("context_fingerprint")?,"output_kind":row.try_get::<Option<String>,_>("output_kind")?,"output_bytes":row.try_get::<Option<i64>,_>("output_bytes")?}));
        }
        Ok(
            json!({"binding":binding,"minimum_task_version_seen":minimum,"receipts":receipts,"delivery":"unverified","model_adoption":"unknown","result_effect":"unknown","reserved_evidence_levels":[ReceiptStage::HostDelivered,ReceiptStage::ModelClaimed,ReceiptStage::ResultVerified]}),
        )
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
        "state requires an absolute file path"
    );
    let parent = path.parent().context("state parent missing")?;
    for ancestor in parent.ancestors() {
        let metadata = fs::symlink_metadata(ancestor)?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "state ancestors must be real directories"
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            fs::metadata(parent)?.permissions().mode() & 0o077 == 0,
            "state parent must be private (0700)"
        );
    }
    #[cfg(windows)]
    crate::support::windows_security::validate_private_parent(path)?;
    Ok(())
}
fn validate_file(path: &Path) -> Result<()> {
    validate_parent(path)?;
    let metadata =
        fs::symlink_metadata(path).context("adapter state missing; initialize explicitly")?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "state must be a regular non-symlink file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        ensure!(
            metadata.nlink() == 1
                && metadata.permissions().mode() & 0o077 == 0
                && metadata.uid() == fs::metadata(path.parent().unwrap())?.uid(),
            "state must be privately owned without hardlink aliases"
        );
    }
    #[cfg(windows)]
    crate::support::windows_security::validate_private_file(path)?;
    Ok(())
}

#[cfg(unix)]
pub(crate) type SessionLock = crate::support::local_file_lock::PrivateLock;
#[cfg(windows)]
pub(crate) type SessionLock = crate::support::windows_security::PrivateLock;
#[cfg(not(any(unix, windows)))]
pub(crate) type SessionLock = std::fs::File;

pub(crate) fn lock_session(state_path: &Path, session: &str) -> Result<SessionLock> {
    identifier(session)?;
    validate_file(state_path)?;
    let path = state_path.with_extension(format!(
        "session-{}.lock",
        super::protocol::digest(session.as_bytes())
    ));
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        crate::support::local_file_lock::PrivateLock::acquire(
            &path,
            fs::metadata(state_path)?.uid(),
        )
    }
    #[cfg(windows)]
    {
        crate::support::windows_security::PrivateLock::acquire(&path)
    }
    #[cfg(not(any(unix, windows)))]
    {
        anyhow::bail!("session locks unsupported on this platform")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn fixture() -> (tempfile::TempDir, State, std::path::PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700)).unwrap();
        }
        #[cfg(windows)]
        {
            // Use the production helper to create a private child rather than
            // relaxing checks for inherited test-directory ACLs.
            crate::support::windows_security::private_create_directory(
                &temp.path().join("private"),
            )
            .unwrap();
        }
        let directory = if cfg!(windows) {
            temp.path().join("private")
        } else {
            temp.path().to_owned()
        };
        let path = directory.canonicalize().unwrap().join("state.sqlite");
        State::initialize(&path).await.unwrap();
        let state = State::open(&path).await.unwrap();
        (temp, state, path)
    }
    fn binding(root: &Path, session: &str, task: &str) -> Binding {
        Binding {
            host_session_id: session.into(),
            workspace_id: "workspace".into(),
            namespace: "project/legacy".into(),
            task_id: task.into(),
            workspace_root: root.to_owned(),
        }
    }
    #[tokio::test]
    async fn sessions_are_explicit_and_cannot_be_rebound() {
        let (temp, state, _) = fixture().await;
        let b = binding(temp.path(), "session", "task-one");
        state.bind(&b).await.unwrap();
        state.bind(&b).await.unwrap();
        assert!(
            state
                .bind(&binding(temp.path(), "session", "task-two"))
                .await
                .is_err()
        );
        assert!(!state.has_binding("other-session").await.unwrap());
    }
    #[tokio::test]
    async fn explicit_defaults_do_not_change_bound_sessions_and_ambiguous_roots_fail() {
        let (temp, state, _) = fixture().await;
        let b = binding(temp.path(), "old-session", "one");
        state.select_default(&b).await.unwrap();
        let selected = state.default_for("new-session", temp.path()).await.unwrap();
        assert_eq!(selected.task_id, "one");
        state.bind(&selected).await.unwrap();
        state
            .select_default(&binding(temp.path(), "unused", "two"))
            .await
            .unwrap();
        assert_eq!(state.binding("new-session").await.unwrap().0.task_id, "one");
        let child = temp.path().join("child");
        state
            .select_default(&binding(&child, "unused", "three"))
            .await
            .unwrap();
        assert!(state.default_for("other", &child).await.is_err());
    }
    #[tokio::test]
    async fn receipts_reject_regression_and_no_database_stage_promotes_delivery() {
        let (temp, state, _) = fixture().await;
        let b = binding(temp.path(), "session", "one");
        state.bind(&b).await.unwrap();
        let receipt = state
            .built(&b, "context", 2, "digest", "SessionStart", Some("resume"))
            .await
            .unwrap();
        assert!(
            state
                .built(&b, "older", 1, "digest", "SessionStart", Some("resume"))
                .await
                .is_err()
        );
        assert_eq!(
            state.inspect("session").await.unwrap()["receipts"][0]["stage"],
            "core_built"
        );
        state
            .prepare_output(
                &receipt,
                &json!({"synthetic":true}),
                &super::super::protocol::digest(b"context"),
                "hook",
            )
            .await
            .unwrap();
        state.output_written(&receipt).await.unwrap();
        assert_eq!(
            state.inspect("session").await.unwrap()["delivery"],
            "unverified"
        );
        assert!(
            sqlx::query("UPDATE context_receipts SET stage='host_delivered'")
                .execute(&state.pool)
                .await
                .is_err()
        );
    }
    #[tokio::test]
    async fn paused_or_incomplete_new_revision_prevents_old_active_rollback() {
        use super::super::protocol::{self, HookInput};
        let (temp, state, _) = fixture().await;
        let binding = binding(temp.path(), "session", "one");
        state.bind(&binding).await.unwrap();
        let input=HookInput::parse(&serde_json::to_vec(&json!({"session_id":"session","cwd":temp.path(),"hook_event_name":"SessionStart","source":"resume"})).unwrap()).unwrap();
        for (version, status, complete) in [(2, "paused", true), (3, "active", false)] {
            let context = json!({"namespace":binding.namespace,"task_id":binding.task_id,"task_version":version,"context_id":format!("context-{version}"),"status":status,"complete":complete,"can_resume":false,"mandatory_snapshot_consistent":true,"mandatory_requirements":[],"diagnostics":[]});
            let version = protocol::context_version(&context, &binding).unwrap();
            state
                .built(
                    &binding,
                    context["context_id"].as_str().unwrap(),
                    version,
                    "digest",
                    "SessionStart",
                    Some("resume"),
                )
                .await
                .unwrap();
            assert!(protocol::render(&input, &binding, &context, 8192, version).is_err());
            assert_eq!(state.binding("session").await.unwrap().1, version);
        }
        assert!(
            state
                .built(
                    &binding,
                    "old-active",
                    1,
                    "digest",
                    "SessionStart",
                    Some("resume")
                )
                .await
                .is_err()
        );
        let report = state.inspect("session").await.unwrap();
        assert!(
            report["receipts"]
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r["stage"] == "core_built")
        );
    }
    #[tokio::test]
    async fn exact_outputs_are_durable_immutable_and_do_not_prove_delivery() {
        let (temp, state, path) = fixture().await;
        let b = binding(temp.path(), "session", "task");
        state.bind(&b).await.unwrap();
        let receipt = state
            .built(&b, "context", 1, "core-digest", "ControllerRefresh", None)
            .await
            .unwrap();
        assert!(state.output_written(&receipt).await.is_err());
        let output = json!({"changed":true,"context":{"requirement":"中文 / English / 日本語"}});
        let fingerprint = super::super::protocol::digest(b"fingerprint");
        state
            .prepare_output(&receipt, &output, &fingerprint, "refresh")
            .await
            .unwrap();
        state
            .prepare_output(&receipt, &output, &fingerprint, "refresh")
            .await
            .unwrap();
        assert!(
            state
                .prepare_output(&receipt, &json!({"changed":false}), &fingerprint, "refresh")
                .await
                .is_err()
        );
        assert!(
            state
                .prepare_output(&receipt, &output, &fingerprint, "hook")
                .await
                .is_err()
        );
        let before = state.inspect("session").await.unwrap();
        assert_eq!(before["receipts"][0]["stage"], "core_built");
        assert_eq!(before["delivery"], "unverified");
        state.output_written(&receipt).await.unwrap();
        drop(state);
        let reopened = State::open(&path).await.unwrap();
        let report = reopened.inspect("session").await.unwrap();
        let mut bytes = serde_json::to_vec(&output).unwrap();
        bytes.push(b'\n');
        assert_eq!(
            report["receipts"][0]["output_digest"],
            super::super::protocol::digest(&bytes)
        );
        assert_eq!(report["receipts"][0]["output_bytes"], bytes.len());
        assert_eq!(report["receipts"][0]["stage"], "output_written");
        assert_eq!(report["delivery"], "unverified");
        assert!(
            reopened
                .prepare_output(&receipt, &output, &fingerprint, "refresh")
                .await
                .is_err()
        );
    }
    #[tokio::test]
    async fn schema_one_requires_explicit_migration_and_preserves_legacy_receipts() {
        let (temp, state, path) = fixture().await;
        let b = binding(temp.path(), "session", "task");
        state.bind(&b).await.unwrap();
        state
            .built(
                &b,
                "legacy",
                4,
                "legacy-digest",
                "SessionStart",
                Some("resume"),
            )
            .await
            .unwrap();
        sqlx::raw_sql("DROP TABLE context_outputs; UPDATE context_receipts SET stage='output_written'; PRAGMA user_version=1;").execute(&state.pool).await.unwrap();
        drop(state);
        assert!(State::open(&path).await.is_err());
        State::migrate(&path).await.unwrap();
        State::migrate(&path).await.unwrap();
        let reopened = State::open(&path).await.unwrap();
        let report = reopened.inspect("session").await.unwrap();
        assert_eq!(report["minimum_task_version_seen"], 4);
        assert_eq!(report["receipts"][0]["context_id"], "legacy");
        assert_eq!(report["receipts"][0]["stage"], "output_written");
        assert!(report["receipts"][0]["output_digest"].is_null());
        assert_eq!(report["delivery"], "unverified");
        sqlx::raw_sql("PRAGMA user_version=99")
            .execute(&reopened.pool)
            .await
            .unwrap();
        drop(reopened);
        assert!(State::migrate(&path).await.is_err());
    }
    #[tokio::test]
    async fn conflicting_migration_rolls_back_without_replacing_state() {
        let (_temp, state, path) = fixture().await;
        sqlx::raw_sql("PRAGMA user_version=1")
            .execute(&state.pool)
            .await
            .unwrap();
        drop(state);
        // An unexpected preexisting table must not be dropped or silently reused.
        assert!(State::migrate(&path).await.is_err());
        let pool = connect(&path).await.unwrap();
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(version, 1);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM context_outputs")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
    #[tokio::test]
    async fn damaged_schema_one_is_not_promoted_by_migration() {
        let (_temp, state, path) = fixture().await;
        sqlx::raw_sql(
            "DROP TABLE context_outputs; DROP TABLE adapter_defaults; PRAGMA user_version=1",
        )
        .execute(&state.pool)
        .await
        .unwrap();
        drop(state);
        assert!(State::migrate(&path).await.is_err());
        let pool = connect(&path).await.unwrap();
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(version, 1);
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE name='context_outputs'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count, 0);
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn broken_pipe_keeps_exact_prepared_output_unconfirmed() {
        use std::io::Write;
        use std::os::fd::FromRawFd;
        let (temp, state, path) = fixture().await;
        let b = binding(temp.path(), "session", "task");
        state.bind(&b).await.unwrap();
        let receipt = state
            .built(&b, "context", 1, "digest", "ControllerRefresh", None)
            .await
            .unwrap();
        let mut descriptors = [-1; 2];
        // Anonymous pipe only, no Unix socket, host process or network access.
        assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
        let reader = unsafe { std::fs::File::from_raw_fd(descriptors[0]) };
        let mut writer = unsafe { std::fs::File::from_raw_fd(descriptors[1]) };
        drop(reader);
        let output = json!({"context":"synthetic"});
        let result = super::super::cli::persist_and_write_output(
            &state,
            &receipt,
            &output,
            &super::super::protocol::digest(b"fingerprint"),
            "refresh",
            |value| {
                let mut bytes = serde_json::to_vec(value)?;
                bytes.push(b'\n');
                let error = writer.write_all(&bytes).unwrap_err();
                assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
                Err(error.into())
            },
        )
        .await;
        assert!(result.is_err());
        drop(state);
        let report = State::open(&path)
            .await
            .unwrap()
            .inspect("session")
            .await
            .unwrap();
        assert_eq!(report["receipts"][0]["stage"], "core_built");
        assert!(report["receipts"][0]["output_digest"].is_string());
        assert_eq!(report["delivery"], "unverified");
    }
    #[tokio::test]
    async fn same_session_lock_is_exclusive() {
        let (_temp, _state, path) = fixture().await;
        let lock = lock_session(&path, "session").unwrap();
        assert!(lock_session(&path, "session").is_err());
        assert!(lock_session(&path, "other").is_ok());
        drop(lock);
        assert!(lock_session(&path, "session").is_ok());
    }
}
