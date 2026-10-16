//! Private, bounded observations of action checks. Never authorization or execution evidence.
//! The caller must hold its session lock and enforce immutable session/task bindings.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::{fs, path::Path, time::Duration};

const MAX_RECORDS: i64 = 10_000;

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
                .context("action check state exists or cannot be created; no overwrite")?
        };
        drop(file);
        let pool = connect(path).await?;
        let mut tx = pool.begin().await?;
        sqlx::raw_sql(
            "CREATE TABLE action_check_clock (id INTEGER PRIMARY KEY CHECK(id=1), sequence INTEGER NOT NULL CHECK(sequence>=0), last_wall_clock INTEGER NOT NULL CHECK(last_wall_clock>=0));
             INSERT INTO action_check_clock VALUES(1,0,0);
             CREATE TABLE action_check_policies (policy_id TEXT PRIMARY KEY, epoch INTEGER NOT NULL CHECK(epoch>=0), digest TEXT NOT NULL);
             CREATE TABLE action_check_sessions (session TEXT PRIMARY KEY, task_version INTEGER NOT NULL CHECK(task_version>=0));
             CREATE TABLE action_check_receipts (
                 session TEXT NOT NULL, tool_id TEXT NOT NULL, args_digest TEXT NOT NULL,
                 policy_id TEXT NOT NULL, epoch INTEGER NOT NULL CHECK(epoch>=0), policy_digest TEXT NOT NULL,
                 task_version INTEGER NOT NULL CHECK(task_version>=0),
                 decision TEXT NOT NULL CHECK(decision IN ('pass_through','deny','needs_confirmation','unsupported')),
                 outcome TEXT NOT NULL DEFAULT 'unknown' CHECK(outcome='unknown'),
                 sequence INTEGER NOT NULL, observed_at TEXT NOT NULL,
                 PRIMARY KEY(session,tool_id));
             CREATE INDEX action_check_recent ON action_check_receipts(session,sequence DESC);
             PRAGMA user_version=1;"
        ).execute(&mut *tx).await?;
        tx.commit().await?;
        pool.close().await;
        Ok(())
    }

    pub async fn open(path: &Path) -> Result<Self> {
        let pool = connect(path).await?;
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&pool)
            .await?;
        ensure!(
            version == 1,
            "action check state version unsupported; explicit migration required"
        );
        // Reject another adapter's database even if its user_version matches.
        let _: i64 = sqlx::query_scalar("SELECT sequence FROM action_check_clock WHERE id=1")
            .fetch_one(&pool)
            .await?;
        Ok(Self { pool })
    }

    /// Commit policy observations even if task lookup or the eventual event fails.
    /// A newly observed revocation must never disappear with an event rollback.
    pub async fn observe_policy(
        &self,
        policy_id: &str,
        epoch: u64,
        policy_digest: &str,
    ) -> Result<()> {
        bounded_token(policy_id, 256)?;
        digest(policy_digest)?;
        let epoch = i64::try_from(epoch).context("policy epoch exceeds storage range")?;
        let mut tx = self.pool.begin().await?;
        let now = chrono::Utc::now();
        let changed = sqlx::query("UPDATE action_check_clock SET sequence=sequence+1,last_wall_clock=? WHERE id=1 AND sequence<9223372036854775807 AND last_wall_clock<=?")
            .bind(now.timestamp()).bind(now.timestamp()).execute(&mut *tx).await?;
        ensure!(
            changed.rows_affected() == 1,
            "action check clock rollback, sequence exhausted, or state missing"
        );
        let policy =
            sqlx::query("SELECT epoch,digest FROM action_check_policies WHERE policy_id=?")
                .bind(policy_id)
                .fetch_optional(&mut *tx)
                .await?;
        if let Some(row) = policy {
            let previous_epoch: i64 = row.try_get("epoch")?;
            let previous_digest: String = row.try_get("digest")?;
            ensure!(epoch >= previous_epoch, "policy epoch rollback");
            ensure!(
                epoch != previous_epoch || policy_digest == previous_digest,
                "policy digest changed within epoch"
            );
        } else {
            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM action_check_policies")
                .fetch_one(&mut *tx)
                .await?;
            ensure!(count < MAX_RECORDS, "action check policy capacity reached");
        }
        sqlx::query("INSERT INTO action_check_policies VALUES(?,?,?) ON CONFLICT(policy_id) DO UPDATE SET epoch=excluded.epoch,digest=excluded.digest")
            .bind(policy_id).bind(epoch).bind(policy_digest).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Keep an observed task revision even when the event cannot be recorded.
    /// The caller has already checked the immutable session/task binding.
    async fn observe_task(&self, session: &str, version: u64) -> Result<()> {
        bounded_token(session, 256)?;
        let version = i64::try_from(version).context("task version exceeds storage range")?;
        let mut tx = self.pool.begin().await?;
        let now = chrono::Utc::now().timestamp();
        let changed = sqlx::query("UPDATE action_check_clock SET sequence=sequence+1,last_wall_clock=? WHERE id=1 AND sequence<9223372036854775807 AND last_wall_clock<=?")
            .bind(now).bind(now).execute(&mut *tx).await?;
        ensure!(
            changed.rows_affected() == 1,
            "action check clock rollback, sequence exhausted, or state missing"
        );
        let minimum: Option<i64> =
            sqlx::query_scalar("SELECT task_version FROM action_check_sessions WHERE session=?")
                .bind(session)
                .fetch_optional(&mut *tx)
                .await?;
        if let Some(minimum) = minimum {
            ensure!(version >= minimum, "task version rollback");
        } else {
            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM action_check_sessions")
                .fetch_one(&mut *tx)
                .await?;
            ensure!(count < MAX_RECORDS, "action check session capacity reached");
        }
        sqlx::query("INSERT INTO action_check_sessions VALUES(?,?) ON CONFLICT(session) DO UPDATE SET task_version=excluded.task_version")
            .bind(session).bind(version).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn observe(
        &self,
        session: &str,
        tool_id: &str,
        args_digest: &str,
        policy_id: &str,
        epoch: u64,
        policy_digest: &str,
        task_version: u64,
        decision: &str,
    ) -> Result<()> {
        // Keep this commit outside the event transaction, including failures of
        // argument validation, task-version checks, or bounded receipt storage.
        self.observe_policy(policy_id, epoch, policy_digest).await?;
        self.observe_task(session, task_version).await?;
        for value in [session, tool_id, policy_id] {
            bounded_token(value, 256)?;
        }
        for value in [args_digest, policy_digest] {
            digest(value)?;
        }
        ensure!(
            matches!(
                decision,
                "pass_through" | "deny" | "needs_confirmation" | "unsupported"
            ),
            "invalid action check decision"
        );
        let epoch = i64::try_from(epoch).context("policy epoch exceeds storage range")?;
        let task_version =
            i64::try_from(task_version).context("task version exceeds storage range")?;
        let mut tx = self.pool.begin().await?;
        // Acquire SQLite's writer lock BEFORE any reads. Separate processes and
        // sessions must not race the global policy high-water marks.
        let now = chrono::Utc::now();
        let changed = sqlx::query("UPDATE action_check_clock SET sequence=sequence+1,last_wall_clock=? WHERE id=1 AND sequence<9223372036854775807 AND last_wall_clock<=?")
            .bind(now.timestamp()).bind(now.timestamp()).execute(&mut *tx).await?;
        ensure!(
            changed.rows_affected() == 1,
            "action check clock rollback, sequence exhausted, or state missing"
        );
        let prior_args: Option<String> = sqlx::query_scalar(
            "SELECT args_digest FROM action_check_receipts WHERE session=? AND tool_id=?",
        )
        .bind(session)
        .bind(tool_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(prior) = &prior_args {
            ensure!(
                prior == args_digest,
                "tool event reused with different arguments"
            );
        } else {
            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM action_check_receipts")
                .fetch_one(&mut *tx)
                .await?;
            ensure!(
                count < MAX_RECORDS,
                "action check receipt capacity reached; refusing new events"
            );
        }
        let policy =
            sqlx::query("SELECT epoch,digest FROM action_check_policies WHERE policy_id=?")
                .bind(policy_id)
                .fetch_optional(&mut *tx)
                .await?;
        if let Some(row) = policy {
            let previous_epoch: i64 = row.try_get("epoch")?;
            let previous_digest: String = row.try_get("digest")?;
            ensure!(epoch >= previous_epoch, "policy epoch rollback");
            ensure!(
                epoch != previous_epoch || policy_digest == previous_digest,
                "policy digest changed within epoch"
            );
        } else {
            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM action_check_policies")
                .fetch_one(&mut *tx)
                .await?;
            ensure!(count < MAX_RECORDS, "action check policy capacity reached");
        }
        let minimum: Option<i64> =
            sqlx::query_scalar("SELECT task_version FROM action_check_sessions WHERE session=?")
                .bind(session)
                .fetch_optional(&mut *tx)
                .await?;
        if let Some(minimum) = minimum {
            ensure!(task_version >= minimum, "task version rollback");
        } else {
            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM action_check_sessions")
                .fetch_one(&mut *tx)
                .await?;
            ensure!(count < MAX_RECORDS, "action check session capacity reached");
        }
        sqlx::query("INSERT INTO action_check_policies VALUES(?,?,?) ON CONFLICT(policy_id) DO UPDATE SET epoch=excluded.epoch,digest=excluded.digest")
            .bind(policy_id).bind(epoch).bind(policy_digest).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO action_check_sessions VALUES(?,?) ON CONFLICT(session) DO UPDATE SET task_version=excluded.task_version")
            .bind(session).bind(task_version).execute(&mut *tx).await?;
        // Every call is a fresh observation; no accessor exposes a cached allow.
        sqlx::query("INSERT INTO action_check_receipts(session,tool_id,args_digest,policy_id,epoch,policy_digest,task_version,decision,outcome,sequence,observed_at) VALUES(?,?,?,?,?,?,?,?,'unknown',(SELECT sequence FROM action_check_clock WHERE id=1),?) ON CONFLICT(session,tool_id) DO UPDATE SET policy_id=excluded.policy_id,epoch=excluded.epoch,policy_digest=excluded.policy_digest,task_version=excluded.task_version,decision=excluded.decision,outcome='unknown',sequence=excluded.sequence,observed_at=excluded.observed_at")
            .bind(session).bind(tool_id).bind(args_digest).bind(policy_id).bind(epoch)
            .bind(policy_digest).bind(task_version).bind(decision).bind(now.to_rfc3339())
            .execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn inspect(&self, session: &str) -> Result<Value> {
        bounded_token(session, 256)?;
        let rows = sqlx::query("SELECT tool_id,args_digest,policy_id,epoch,policy_digest,task_version,decision,observed_at FROM action_check_receipts WHERE session=? ORDER BY sequence DESC LIMIT 20")
            .bind(session).fetch_all(&self.pool).await?;
        let mut receipts = Vec::new();
        for row in rows {
            receipts.push(json!({
                "tool_id": row.try_get::<String,_>("tool_id")?,
                "args_digest": row.try_get::<String,_>("args_digest")?,
                "policy_id": row.try_get::<String,_>("policy_id")?,
                "epoch": row.try_get::<i64,_>("epoch")?,
                "policy_digest": row.try_get::<String,_>("policy_digest")?,
                "task_version": row.try_get::<i64,_>("task_version")?,
                "decision": row.try_get::<String,_>("decision")?,
                "observed_at": row.try_get::<String,_>("observed_at")?,
                "outcome": "unknown"
            }));
        }
        Ok(
            json!({"session":session,"receipts":receipts,"outcome":"unknown","authorization":"not_granted"}),
        )
    }
}

fn bounded_token(value: &str, max: usize) -> Result<()> {
    ensure!(
        !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control),
        "invalid bounded action check identifier or digest"
    );
    Ok(())
}

fn digest(value: &str) -> Result<()> {
    ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "action check digest must be lowercase SHA256 hex"
    );
    Ok(())
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
        "action check state requires an absolute file path"
    );
    let parent = path.parent().context("action check state parent missing")?;
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
        fs::symlink_metadata(path).context("action check state missing; initialize explicitly")?;
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
        crate::support::windows_security::private_create_directory(&temp.path().join("private"))
            .unwrap();
        let directory = if cfg!(windows) {
            temp.path().join("private")
        } else {
            temp.path().to_owned()
        };
        let path = directory.canonicalize().unwrap().join("checks.sqlite");
        State::initialize(&path).await.unwrap();
        let state = State::open(&path).await.unwrap();
        (temp, state, path)
    }
    async fn observe(
        state: &State,
        tool: &str,
        args: &str,
        epoch: u64,
        digest: &str,
        version: u64,
        decision: &str,
    ) -> Result<()> {
        state
            .observe(
                "session", tool, args, "policy", epoch, digest, version, decision,
            )
            .await
    }
    #[tokio::test]
    async fn repeated_event_is_rechecked_but_changed_arguments_fail() {
        let (_temp, state, _) = fixture().await;
        observe(
            &state,
            "event",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            1,
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            1,
            "pass_through",
        )
        .await
        .unwrap();
        observe(
            &state,
            "event",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            1,
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            1,
            "deny",
        )
        .await
        .unwrap();
        assert!(
            observe(
                &state,
                "event",
                "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                1,
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                1,
                "pass_through"
            )
            .await
            .is_err()
        );
        let report = state.inspect("session").await.unwrap();
        assert_eq!(report["receipts"].as_array().unwrap().len(), 1);
        assert_eq!(report["receipts"][0]["decision"], "deny");
        assert_eq!(report["receipts"][0]["outcome"], "unknown");
    }
    #[tokio::test]
    async fn high_water_marks_survive_reopening_and_failed_transactions() {
        let (_temp, state, path) = fixture().await;
        let args = "a".repeat(64);
        let old_digest = "b".repeat(64);
        let new_digest = "d".repeat(64);
        observe(&state, "event", &args, 2, &old_digest, 4, "deny")
            .await
            .unwrap();
        assert!(
            observe(&state, "new", &args, 3, &new_digest, 3, "deny")
                .await
                .is_err()
        );
        let reopened = State::open(&path).await.unwrap();
        assert!(
            observe(&reopened, "new", &args, 2, &old_digest, 4, "pass_through")
                .await
                .is_err()
        );
        observe(
            &reopened,
            "new",
            &args,
            3,
            &new_digest,
            4,
            "needs_confirmation",
        )
        .await
        .unwrap();
        assert!(
            observe(&reopened, "older", &args, 1, &old_digest, 4, "deny")
                .await
                .is_err()
        );
        assert!(
            observe(&reopened, "changed", &args, 3, &"e".repeat(64), 4, "deny")
                .await
                .is_err()
        );
        assert!(
            reopened
                .observe(
                    "other-session",
                    "event",
                    &args,
                    "policy",
                    2,
                    &old_digest,
                    99,
                    "deny"
                )
                .await
                .is_err()
        );
        observe(
            &reopened,
            "next",
            &args,
            4,
            &"f".repeat(64),
            5,
            "unsupported",
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn policy_observed_while_task_source_offline_cannot_be_rolled_back() {
        let (_temp, state, path) = fixture().await;
        let old_digest = "b".repeat(64);
        let revoked_digest = "c".repeat(64);
        state
            .observe_policy("policy", 1, &old_digest)
            .await
            .unwrap();
        // Task source is unavailable: no event receipt can be recorded.
        state
            .observe_policy("policy", 2, &revoked_digest)
            .await
            .unwrap();
        let reopened = State::open(&path).await.unwrap();
        assert!(
            reopened
                .observe_policy("policy", 1, &old_digest)
                .await
                .is_err()
        );
        assert!(
            reopened
                .observe_policy("policy", 2, &old_digest)
                .await
                .is_err()
        );
        reopened
            .observe_policy("policy", 2, &revoked_digest)
            .await
            .unwrap();
        assert!(
            reopened
                .observe(
                    "session",
                    "event",
                    &"a".repeat(64),
                    "policy",
                    1,
                    &old_digest,
                    1,
                    "pass_through"
                )
                .await
                .is_err()
        );
        assert!(
            reopened.inspect("session").await.unwrap()["receipts"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn inspect_is_bounded_and_never_execution_evidence() {
        let (_temp, state, _) = fixture().await;
        for i in 0..25 {
            observe(
                &state,
                &format!("event-{i}"),
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                1,
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                1,
                "deny",
            )
            .await
            .unwrap();
        }
        let report = state.inspect("session").await.unwrap();
        assert_eq!(report["receipts"].as_array().unwrap().len(), 20);
        assert_eq!(report["receipts"][0]["tool_id"], "event-24");
        assert!(
            observe(
                &state,
                "bad",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                1,
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                1,
                "executed"
            )
            .await
            .is_err()
        );
        assert!(
            observe(
                &state,
                "bad",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                u64::MAX,
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                1,
                "deny"
            )
            .await
            .is_err()
        );
        assert!(
            observe(
                &state,
                "bad",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                1,
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                u64::MAX,
                "deny"
            )
            .await
            .is_err()
        );
        assert!(
            observe(
                &state,
                &"x".repeat(257),
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                1,
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                1,
                "deny"
            )
            .await
            .is_err()
        );
        assert!(
            observe(
                &state,
                "bad",
                &"x".repeat(129),
                1,
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                1,
                "deny"
            )
            .await
            .is_err()
        );
        assert!(
            sqlx::query("UPDATE action_check_receipts SET outcome='executed'")
                .execute(&state.pool)
                .await
                .is_err()
        );
    }
    #[tokio::test]
    async fn full_store_keeps_antireplay_and_allows_existing_rechecks() {
        let (_temp, state, _) = fixture().await;
        observe(
            &state,
            "event",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            1,
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            1,
            "deny",
        )
        .await
        .unwrap();
        sqlx::query("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<9999) INSERT INTO action_check_receipts SELECT 'session','filler-'||x,'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','policy',1,'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',1,'deny','unknown',0,'time' FROM n")
            .execute(&state.pool).await.unwrap();
        assert!(
            observe(
                &state,
                "new",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                1,
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                1,
                "deny"
            )
            .await
            .is_err()
        );
        observe(
            &state,
            "event",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            1,
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            1,
            "unsupported",
        )
        .await
        .unwrap();
        assert!(
            observe(
                &state,
                "event",
                "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                1,
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                1,
                "deny"
            )
            .await
            .is_err()
        );
    }
    #[tokio::test]
    async fn task_high_water_survives_event_conflict_and_full_receipt_store() {
        let (_temp, state, path) = fixture().await;
        let args = "a".repeat(64);
        let digest = "b".repeat(64);
        observe(&state, "event", &args, 1, &digest, 1, "deny")
            .await
            .unwrap();
        assert!(
            observe(&state, "event", &"c".repeat(64), 1, &digest, 2, "deny")
                .await
                .is_err()
        );
        let reopened = State::open(&path).await.unwrap();
        assert!(
            observe(&reopened, "event", &args, 1, &digest, 1, "pass_through")
                .await
                .is_err()
        );
        sqlx::query("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<9999) INSERT INTO action_check_receipts SELECT 'session','filler-'||x,'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','policy',1,'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',1,'deny','unknown',0,'time' FROM n")
            .execute(&state.pool).await.unwrap();
        assert!(
            observe(&reopened, "new", &args, 1, &digest, 3, "deny")
                .await
                .is_err()
        );
        assert!(
            observe(&reopened, "event", &args, 1, &digest, 2, "pass_through")
                .await
                .is_err()
        );
        observe(&reopened, "event", &args, 1, &digest, 3, "deny")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn concurrent_task_observations_keep_the_highest_revision() {
        let (_temp, state, path) = fixture().await;
        let other = State::open(&path).await.unwrap();
        let (two, three) = tokio::join!(
            state.observe_task("session", 2),
            other.observe_task("session", 3)
        );
        assert!(three.is_ok());
        // Two may finish first or be rejected after three, but can never lower it.
        let _ = two;
        assert!(state.observe_task("session", 2).await.is_err());
        state.observe_task("session", 3).await.unwrap();
    }

    #[tokio::test]
    async fn initialization_never_overwrites_and_schema_is_checked() {
        let (_temp, _state, path) = fixture().await;
        assert!(State::initialize(&path).await.is_err());
        let pool = connect(&path).await.unwrap();
        sqlx::query("PRAGMA user_version=2")
            .execute(&pool)
            .await
            .unwrap();
        assert!(State::open(&path).await.is_err());
    }
    #[tokio::test]
    async fn concurrent_sessions_cannot_split_same_policy_epoch() {
        let (_temp, first, path) = fixture().await;
        let second = State::open(&path).await.unwrap();
        let args = "a".repeat(64);
        let digest_one = "b".repeat(64);
        let digest_two = "c".repeat(64);
        let (one, two) = tokio::join!(
            first.observe("one", "event", &args, "policy", 1, &digest_one, 1, "deny"),
            second.observe("two", "event", &args, "policy", 1, &digest_two, 1, "deny")
        );
        assert_ne!(one.is_ok(), two.is_ok());
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM action_check_receipts")
            .fetch_one(&first.pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }
    #[tokio::test]
    async fn detects_clock_rollback_without_claiming_trusted_time() {
        let (_temp, state, _) = fixture().await;
        sqlx::query("UPDATE action_check_clock SET last_wall_clock=9223372036854775807")
            .execute(&state.pool)
            .await
            .unwrap();
        assert!(
            observe(
                &state,
                "event",
                &"a".repeat(64),
                1,
                &"b".repeat(64),
                1,
                "deny"
            )
            .await
            .is_err()
        );
        assert_eq!(
            state.inspect("session").await.unwrap()["receipts"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn refuses_public_files_symlinks_and_hardlinks() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let (_temp, _state, path) = fixture().await;
        let alias = path.with_extension("alias");
        symlink(&path, &alias).unwrap();
        assert!(State::open(&alias).await.is_err());
        fs::remove_file(&alias).unwrap();
        fs::hard_link(&path, &alias).unwrap();
        assert!(State::open(&path).await.is_err());
        fs::remove_file(&alias).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(State::open(&path).await.is_err());
    }
}
