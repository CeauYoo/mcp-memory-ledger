use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
};

use chrono::Utc;
use serde::Serialize;
use sqlx::{
    Connection, Row, SqliteConnection,
    sqlite::{SqliteConnectOptions, SqlitePool},
};
use uuid::Uuid;

use crate::error::AppError;

use super::{
    schema::{CURRENT_SCHEMA_VERSION, SCHEMA_MIGRATIONS, init_sql},
    store::{
        SqliteStore, ensure_claims_namespace_column, ensure_events_namespace_column,
        ensure_reflection_audit_columns, seed_baseline_commitments,
    },
};

pub const CURRENT_DATABASE_SCHEMA_VERSION: i64 = CURRENT_SCHEMA_VERSION;

const REQUIRED_TABLES: [&str; 10] = [
    "events",
    "claims",
    "evidence_links",
    "episode_events",
    "reflections",
    "reflection_trigger_ledger",
    "identity_claims",
    "commitments",
    "operation_log",
    "schema_migrations",
];

const PRESERVED_DATA_TABLES: [&str; 7] = [
    "events",
    "claims",
    "evidence_links",
    "episode_events",
    "reflections",
    "reflection_trigger_ledger",
    "operation_log",
];

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DatabaseTableCount {
    pub table: String,
    pub rows: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct UnknownOwnerInventory {
    pub events: i64,
    pub claims: i64,
    pub rewrite_performed: bool,
    pub rewrite_requires_separate_approval: bool,
}

impl UnknownOwnerInventory {
    fn empty() -> Self {
        Self {
            events: 0,
            claims: 0,
            rewrite_performed: false,
            rewrite_requires_separate_approval: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DatabaseLifecycleReport {
    pub operation: String,
    pub status: String,
    pub file_backed: bool,
    pub database_exists: bool,
    pub path_writable_hint: Option<bool>,
    pub schema_version: Option<i64>,
    pub target_schema_version: i64,
    pub migration_required: bool,
    pub runtime_defaults_present: bool,
    pub migration_ledger_consistent: bool,
    pub required_tables_present: bool,
    pub schema_structure_valid: bool,
    pub schema_structure_issues: Vec<String>,
    pub foreign_key_violations: usize,
    pub table_counts: Vec<DatabaseTableCount>,
    pub unknown_owner_inventory: UnknownOwnerInventory,
    pub backup_path: Option<String>,
    pub restore_rehearsal: String,
    pub preserved_row_counts: bool,
    pub bootstrap_performed: bool,
    pub message: String,
}

impl DatabaseLifecycleReport {
    pub fn is_current(&self) -> bool {
        self.status == "current"
    }

    fn missing(path_writable_hint: Option<bool>) -> Self {
        Self {
            operation: "read_only_inspection".to_string(),
            status: "missing".to_string(),
            file_backed: true,
            database_exists: false,
            path_writable_hint,
            schema_version: None,
            target_schema_version: CURRENT_SCHEMA_VERSION,
            migration_required: false,
            runtime_defaults_present: false,
            migration_ledger_consistent: false,
            required_tables_present: false,
            schema_structure_valid: false,
            schema_structure_issues: Vec::new(),
            foreign_key_violations: 0,
            table_counts: Vec::new(),
            unknown_owner_inventory: UnknownOwnerInventory::empty(),
            backup_path: None,
            restore_rehearsal: "not_applicable".to_string(),
            preserved_row_counts: true,
            bootstrap_performed: false,
            message: "database file does not exist; run init or doctor --allow-bootstrap"
                .to_string(),
        }
    }

    fn unsupported_non_file() -> Self {
        Self {
            operation: "read_only_inspection".to_string(),
            status: "unsupported_non_file_database".to_string(),
            file_backed: false,
            database_exists: false,
            path_writable_hint: None,
            schema_version: None,
            target_schema_version: CURRENT_SCHEMA_VERSION,
            migration_required: false,
            runtime_defaults_present: false,
            migration_ledger_consistent: false,
            required_tables_present: false,
            schema_structure_valid: false,
            schema_structure_issues: Vec::new(),
            foreign_key_violations: 0,
            table_counts: Vec::new(),
            unknown_owner_inventory: UnknownOwnerInventory::empty(),
            backup_path: None,
            restore_rehearsal: "not_applicable".to_string(),
            preserved_row_counts: true,
            bootstrap_performed: false,
            message: "explicit database lifecycle requires a file-backed sqlite URL".to_string(),
        }
    }
}

pub async fn inspect_database(database_url: &str) -> Result<DatabaseLifecycleReport, AppError> {
    parse_options(database_url)?;
    let Some(path) = sqlite_file_path(database_url) else {
        return Ok(DatabaseLifecycleReport::unsupported_non_file());
    };

    if !path.exists() {
        return Ok(DatabaseLifecycleReport::missing(path_writable_hint(&path)));
    }

    let pool = connect_pool(database_url, false, true)
        .await
        .map_err(|error| {
            AppError::Message(format!(
                "read-only database inspection failed for existing sqlite file: {error}"
            ))
        })?;
    let report = inspect_pool(
        &pool,
        "read_only_inspection",
        true,
        path_writable_hint(&path),
    )
    .await;
    pool.close().await;
    report
}

pub async fn initialize_database(database_url: &str) -> Result<DatabaseLifecycleReport, AppError> {
    parse_options(database_url)?;
    let path = require_file_path(database_url)?;
    if path.exists() {
        return Err(AppError::Message(format!(
            "init refuses to overwrite existing sqlite database: {}",
            path.display()
        )));
    }

    ensure_parent_directory(&path)?;
    // Reserve the pathname atomically. Never remove a database or SQLite sidecar
    // on failure: another process may own or have replaced it by cleanup time.
    for suffix in ["-wal", "-shm", "-journal"] {
        if PathBuf::from(format!("{}{suffix}", path.display())).exists() {
            return Err(AppError::Message(
                "init refuses existing SQLite sidecar files".into(),
            ));
        }
    }
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| {
            AppError::Message(format!(
                "init could not exclusively create database: {error}"
            ))
        })?;
    migrate_in_place(database_url, false).await?;

    let mut report = inspect_database(database_url).await?;
    if !report.is_current() {
        return Err(AppError::Message(format!(
            "initialized database did not pass current-schema readback: {}",
            report.status
        )));
    }
    report.operation = "init".to_string();
    report.bootstrap_performed = true;
    report.restore_rehearsal = "not_applicable_new_database".to_string();
    report.message = "database initialized with current schema and runtime defaults".to_string();
    Ok(report)
}

pub async fn migrate_database(database_url: &str) -> Result<DatabaseLifecycleReport, AppError> {
    parse_options(database_url)?;
    let path = require_file_path(database_url)?;
    if !path.exists() {
        return Err(AppError::Message(
            "migrate requires an existing database; run init first".into(),
        ));
    }
    // A reserved SQLite write lock blocks ALL writers (including non-lifecycle
    // clients and WAL writers) throughout backup, rehearsal, and final write.
    // Readers remain possible, so VACUUM INTO can make the backup separately.
    let mut locked = begin_migration(database_url, false).await?;
    let initial = inspect_database(database_url).await?;
    if initial.status == "missing" {
        return Err(AppError::Message(
            "migrate requires an existing database; run init first".to_string(),
        ));
    }
    if initial.status == "unsupported_newer_schema" || initial.status == "schema_structure_invalid"
    {
        return Err(AppError::Message(initial.message));
    }
    if initial.is_current() {
        execute(&mut locked, "ROLLBACK").await?;
        locked.close().await.map_err(sqlite_error)?;
        let mut report = initial;
        report.operation = "migrate".to_string();
        report.restore_rehearsal = "not_needed_current".to_string();
        report.message = "database already uses the current schema".to_string();
        return Ok(report);
    }

    let backup_path = create_backup_anchor(database_url, &path, initial.schema_version).await?;
    let rehearsal_path = std::env::temp_dir().join(format!(
        "agent-llm-mm-migration-rehearsal-{}.sqlite",
        Uuid::new_v4()
    ));
    fs::copy(&backup_path, &rehearsal_path).map_err(|error| {
        AppError::Message(format!(
            "failed to create migration restore rehearsal copy {}: {error}",
            rehearsal_path.display()
        ))
    })?;
    let rehearsal_url = sqlite_url(&rehearsal_path);

    let rehearsal_result = async {
        migrate_in_place(&rehearsal_url, false).await?;
        let report = inspect_database(&rehearsal_url).await?;
        if !report.is_current() {
            return Err(AppError::Message(format!(
                "restore rehearsal did not reach current schema: {}",
                report.status
            )));
        }
        Ok::<_, AppError>(report)
    }
    .await;
    cleanup_sqlite_files(&rehearsal_path);
    let rehearsal = rehearsal_result?;

    // Validate the final state while the migration still owns its write
    // reservation. Once COMMIT releases it, ordinary writers may change rows.
    let mut report = migrate_locked(
        &mut locked,
        Some(&rehearsal.table_counts),
        path_writable_hint(&path),
    )
    .await?;
    locked.close().await.map_err(sqlite_error)?;

    report.operation = "migrate".to_string();
    report.backup_path = Some(backup_path.to_string_lossy().into_owned());
    report.restore_rehearsal = "passed_before_original_write".to_string();
    report.bootstrap_performed = true;
    report.message = "database migrated after backup and restore rehearsal; report describes the validated migration transaction snapshot at commit".to_string();
    Ok(report)
}

pub(super) async fn bootstrap_database(database_url: &str) -> Result<SqliteStore, AppError> {
    let report = inspect_database(database_url).await?;
    match report.status.as_str() {
        "missing" => {
            initialize_database(database_url).await?;
        }
        "current" => {}
        _ => {
            migrate_database(database_url).await?;
        }
    }
    open_current_database(database_url).await
}

pub async fn open_current_database(database_url: &str) -> Result<SqliteStore, AppError> {
    let report = inspect_database(database_url).await?;
    if !report.is_current() {
        return Err(AppError::Message(format!(
            "database is not ready for serve: {}; run init, migrate, or doctor --allow-bootstrap",
            report.status
        )));
    }
    let pool = connect_pool(database_url, false, false).await?;
    Ok(SqliteStore { pool })
}

pub async fn open_read_only_current_database(
    database_url: &str,
) -> Result<Option<SqliteStore>, AppError> {
    let report = inspect_database(database_url).await?;
    if !report.is_current() {
        return Ok(None);
    }
    let pool = connect_pool(database_url, false, true).await?;
    Ok(Some(SqliteStore { pool }))
}

async fn begin_migration(
    database_url: &str,
    create_if_missing: bool,
) -> Result<SqliteConnection, AppError> {
    let options = parse_options(database_url)?
        .create_if_missing(create_if_missing)
        .foreign_keys(false)
        .busy_timeout(Duration::ZERO);
    let mut connection = SqliteConnection::connect_with(&options)
        .await
        .map_err(sqlite_error)?;
    execute(&mut connection, "PRAGMA legacy_alter_table = ON").await?;
    execute(&mut connection, "BEGIN IMMEDIATE")
        .await
        .map_err(|error| {
            AppError::Message(format!(
                "exclusive lifecycle write reservation unavailable: {error}"
            ))
        })?;
    Ok(connection)
}

async fn migrate_in_place(database_url: &str, create_if_missing: bool) -> Result<(), AppError> {
    let mut connection = begin_migration(database_url, create_if_missing).await?;
    migrate_locked(&mut connection, None, None).await?;
    connection.close().await.map_err(sqlite_error)
}

async fn migrate_locked(
    connection: &mut SqliteConnection,
    rehearsal_counts: Option<&[DatabaseTableCount]>,
    path_writable_hint: Option<bool>,
) -> Result<DatabaseLifecycleReport, AppError> {
    let version = schema_version(connection).await?;
    if version > CURRENT_SCHEMA_VERSION {
        return Err(AppError::Message(format!(
            "database schema version {version} is newer than supported version {CURRENT_SCHEMA_VERSION}"
        )));
    }
    validate_existing_ledger(connection, version).await?;
    let before_counts = existing_preserved_counts(connection).await?;
    let migration_result = async {
        run_migration_steps(connection, version, &before_counts).await?;
        let report = inspect_connection(connection, "migrate", true, path_writable_hint).await?;
        if !report.is_current() {
            return Err(AppError::Message(format!(
                "migration current-schema readback failed before commit: {}",
                report.status
            )));
        }
        if rehearsal_counts.is_some_and(|counts| report.table_counts != counts) {
            return Err(AppError::Message(
                "migrated database row-count readback differs from restore rehearsal; migration rolled back; backup remains available"
                    .to_string(),
            ));
        }
        Ok::<_, AppError>(report)
    }
    .await;
    let report = match migration_result {
        Ok(report) => {
            if let Err(error) = execute(connection, "COMMIT").await {
                let _ = execute(connection, "ROLLBACK").await;
                let _ = reset_connection_pragmas(connection).await;
                return Err(error);
            }
            report
        }
        Err(error) => {
            let _ = execute(connection, "ROLLBACK").await;
            let _ = reset_connection_pragmas(connection).await;
            return Err(error);
        }
    };
    reset_connection_pragmas(connection).await?;
    Ok(report)
}

// Rebuild rather than ALTER ADD so the canonical DDL remains identical for
// fresh and migrated databases; lifecycle holds the migration write reservation.
async fn ensure_event_feedback_column(connection: &mut SqliteConnection) -> Result<(), AppError> {
    let exists = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM pragma_table_info('events') WHERE name = 'feedback_json'",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(sqlite_error)?;
    if exists == 0 {
        execute(connection, "ALTER TABLE events RENAME TO events_legacy").await?;
        execute(connection, &super::schema::events_table_sql(false)).await?;
        execute(connection, "INSERT INTO events (event_id, recorded_at, owner, namespace, kind, summary) SELECT event_id, recorded_at, owner, namespace, kind, summary FROM events_legacy").await?;
        execute(connection, "DROP TABLE events_legacy").await?;
    }
    Ok(())
}

async fn run_migration_steps(
    connection: &mut SqliteConnection,
    from_version: i64,
    before_counts: &[DatabaseTableCount],
) -> Result<(), AppError> {
    for (version, name) in SCHEMA_MIGRATIONS {
        if version <= from_version {
            continue;
        }
        match version {
            1 => execute_init_sql(connection).await?,
            2 => {
                ensure_events_namespace_column(connection).await?;
                ensure_claims_namespace_column(connection).await?;
            }
            3 => ensure_reflection_audit_columns(connection).await?,
            4 => ensure_event_feedback_column(connection).await?,
            _ => {
                return Err(AppError::Message(format!(
                    "missing migration implementation for version {version}"
                )));
            }
        }
        record_migration(connection, version, name).await?;
        execute(connection, &format!("PRAGMA user_version = {version}")).await?;
    }

    seed_baseline_commitments(&mut *connection).await?;
    seed_default_identity(connection).await?;
    validate_preserved_counts(connection, before_counts).await?;
    let foreign_key_violations = foreign_key_violation_count(connection).await?;
    if foreign_key_violations != 0 {
        return Err(AppError::Message(format!(
            "foreign_key_check found {foreign_key_violations} violation(s); migration rolled back"
        )));
    }
    validate_existing_ledger(connection, CURRENT_SCHEMA_VERSION).await?;
    let issues = schema_structure_issues(connection).await?;
    if !issues.is_empty() {
        return Err(AppError::Message(format!(
            "current-schema structural readback failed: {}",
            issues.join(", ")
        )));
    }
    Ok(())
}

async fn execute_init_sql(connection: &mut SqliteConnection) -> Result<(), AppError> {
    for statement in init_sql().split(';').filter(|part| !part.trim().is_empty()) {
        execute(connection, statement).await?;
    }
    Ok(())
}

async fn record_migration(
    connection: &mut SqliteConnection,
    version: i64,
    name: &str,
) -> Result<(), AppError> {
    sqlx::query("INSERT INTO schema_migrations (version, name, applied_at) VALUES (?, ?, ?)")
        .bind(version)
        .bind(name)
        .bind(Utc::now().to_rfc3339())
        .execute(&mut *connection)
        .await
        .map_err(sqlite_error)?;
    Ok(())
}

async fn seed_default_identity(connection: &mut SqliteConnection) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO identity_claims (position, claim) \
         SELECT 0, 'identity:self=agent_llm_mm' \
         WHERE NOT EXISTS (SELECT 1 FROM identity_claims)",
    )
    .execute(&mut *connection)
    .await
    .map_err(sqlite_error)?;
    Ok(())
}

async fn validate_existing_ledger(
    connection: &mut SqliteConnection,
    version: i64,
) -> Result<(), AppError> {
    if version == 0 {
        return Ok(());
    }
    let table_exists = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations'",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(sqlite_error)?
        > 0;
    if !table_exists {
        return Err(AppError::Message(format!(
            "schema version is {version} but schema_migrations ledger is missing"
        )));
    }

    let rows = sqlx::query("SELECT version, name FROM schema_migrations ORDER BY version")
        .fetch_all(&mut *connection)
        .await
        .map_err(sqlite_error)?;
    let expected = SCHEMA_MIGRATIONS
        .into_iter()
        .filter(|(migration_version, _)| *migration_version <= version)
        .collect::<Vec<_>>();
    if rows.len() != expected.len()
        || rows.iter().zip(expected).any(|(row, expected)| {
            row.get::<i64, _>("version") != expected.0 || row.get::<String, _>("name") != expected.1
        })
    {
        return Err(AppError::Message(format!(
            "schema_migrations ledger is inconsistent with schema version {version}"
        )));
    }
    Ok(())
}

async fn validate_preserved_counts(
    connection: &mut SqliteConnection,
    before: &[DatabaseTableCount],
) -> Result<(), AppError> {
    let after = existing_preserved_counts(connection).await?;
    for before_count in before {
        let after_count = after
            .iter()
            .find(|count| count.table == before_count.table)
            .ok_or_else(|| {
                AppError::Message(format!(
                    "preserved table disappeared during migration: {}",
                    before_count.table
                ))
            })?;
        if after_count.rows != before_count.rows {
            return Err(AppError::Message(format!(
                "row count changed during migration for {}: {} -> {}",
                before_count.table, before_count.rows, after_count.rows
            )));
        }
    }
    Ok(())
}

async fn existing_preserved_counts(
    connection: &mut SqliteConnection,
) -> Result<Vec<DatabaseTableCount>, AppError> {
    let existing = table_names_connection(connection).await?;
    let mut counts = Vec::new();
    for table in PRESERVED_DATA_TABLES {
        if existing.contains(table) {
            counts.push(DatabaseTableCount {
                table: table.to_string(),
                rows: table_count_connection(connection, table).await?,
            });
        }
    }
    Ok(counts)
}

async fn inspect_pool(
    pool: &SqlitePool,
    operation: &str,
    database_exists: bool,
    path_writable_hint: Option<bool>,
) -> Result<DatabaseLifecycleReport, AppError> {
    // All schema and data checks must describe one read snapshot.
    let mut transaction = pool.begin().await.map_err(sqlite_error)?;
    let report = inspect_connection(
        &mut transaction,
        operation,
        database_exists,
        path_writable_hint,
    )
    .await;
    transaction.rollback().await.map_err(sqlite_error)?;
    report
}

async fn inspect_connection(
    connection: &mut SqliteConnection,
    operation: &str,
    database_exists: bool,
    path_writable_hint: Option<bool>,
) -> Result<DatabaseLifecycleReport, AppError> {
    let version = sqlx::query_scalar::<_, i64>("PRAGMA user_version")
        .fetch_one(&mut *connection)
        .await
        .map_err(sqlite_error)?;
    let table_names = table_names_connection(connection).await?;
    let required_tables_present = REQUIRED_TABLES
        .iter()
        .all(|table| table_names.contains(*table));
    let schema_structure_issues = if version == CURRENT_SCHEMA_VERSION {
        schema_structure_issues(connection).await?
    } else {
        Vec::new()
    };
    let schema_structure_valid =
        version == CURRENT_SCHEMA_VERSION && schema_structure_issues.is_empty();
    if version == CURRENT_SCHEMA_VERSION && !schema_structure_valid {
        // Do not issue data queries against malformed columns or foreign keys.
        let mut report = DatabaseLifecycleReport::missing(path_writable_hint);
        report.operation = operation.into();
        report.status = "schema_structure_invalid".into();
        report.database_exists = database_exists;
        report.schema_version = Some(version);
        report.required_tables_present = required_tables_present;
        report.schema_structure_issues = schema_structure_issues;
        report.message = format!(
            "current-schema structural readback failed: {}; explicit repair is required",
            report.schema_structure_issues.join(", ")
        );
        return Ok(report);
    }
    let migration_ledger_consistent =
        ledger_is_consistent(connection, version, &table_names).await?;
    let foreign_key_violations = foreign_key_violation_count(connection).await?;
    let table_counts = required_table_counts(connection, &table_names).await?;
    let identity_present = table_count(&table_counts, "identity_claims") > 0;
    let baseline_commitment_present = if table_names.contains("commitments") {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM commitments WHERE description = 'forbid:write_identity_core_directly' AND owner = 'self'",
        )
        .fetch_one(&mut *connection)
        .await
        .map_err(sqlite_error)?
            > 0
    } else {
        false
    };
    let runtime_defaults_present = identity_present && baseline_commitment_present;
    let unknown_owner_inventory = unknown_owner_inventory(connection, &table_names).await?;

    let (status, migration_required, message) = if version > CURRENT_SCHEMA_VERSION {
        (
            "unsupported_newer_schema",
            false,
            format!(
                "database schema version {version} is newer than supported version {CURRENT_SCHEMA_VERSION}"
            ),
        )
    } else if version == CURRENT_SCHEMA_VERSION
        && required_tables_present
        && migration_ledger_consistent
        && foreign_key_violations == 0
        && runtime_defaults_present
    {
        (
            "current",
            false,
            "database schema, migration ledger, foreign keys, and runtime defaults are current"
                .to_string(),
        )
    } else if version == CURRENT_SCHEMA_VERSION
        && required_tables_present
        && migration_ledger_consistent
        && foreign_key_violations == 0
    {
        (
            "bootstrap_required",
            false,
            "schema is current but explicit runtime-default bootstrap is required".to_string(),
        )
    } else {
        (
            "migration_required",
            true,
            format!(
                "database requires explicit migration from version {version} to {CURRENT_SCHEMA_VERSION}"
            ),
        )
    };

    Ok(DatabaseLifecycleReport {
        operation: operation.to_string(),
        status: status.to_string(),
        file_backed: true,
        database_exists,
        path_writable_hint,
        schema_version: Some(version),
        target_schema_version: CURRENT_SCHEMA_VERSION,
        migration_required,
        runtime_defaults_present,
        migration_ledger_consistent,
        required_tables_present,
        schema_structure_valid,
        schema_structure_issues,
        foreign_key_violations,
        table_counts,
        unknown_owner_inventory,
        backup_path: None,
        restore_rehearsal: "not_applicable".to_string(),
        preserved_row_counts: true,
        bootstrap_performed: false,
        message,
    })
}

async fn ledger_is_consistent(
    connection: &mut SqliteConnection,
    version: i64,
    tables: &BTreeSet<String>,
) -> Result<bool, AppError> {
    if version == 0 {
        return Ok(!tables.contains("schema_migrations"));
    }
    if !tables.contains("schema_migrations") {
        return Ok(false);
    }
    let rows = sqlx::query("SELECT version, name FROM schema_migrations ORDER BY version")
        .fetch_all(&mut *connection)
        .await
        .map_err(sqlite_error)?;
    let expected = SCHEMA_MIGRATIONS
        .into_iter()
        .filter(|(migration_version, _)| *migration_version <= version)
        .collect::<Vec<_>>();
    Ok(rows.len() == expected.len()
        && rows.iter().zip(expected).all(|(row, expected)| {
            row.get::<i64, _>("version") == expected.0 && row.get::<String, _>("name") == expected.1
        }))
}

async fn required_table_counts(
    connection: &mut SqliteConnection,
    tables: &BTreeSet<String>,
) -> Result<Vec<DatabaseTableCount>, AppError> {
    let mut counts = Vec::new();
    for table in REQUIRED_TABLES {
        if tables.contains(table) {
            counts.push(DatabaseTableCount {
                table: table.to_string(),
                rows: table_count_connection(connection, table).await?,
            });
        }
    }
    Ok(counts)
}

async fn unknown_owner_inventory(
    connection: &mut SqliteConnection,
    tables: &BTreeSet<String>,
) -> Result<UnknownOwnerInventory, AppError> {
    let events = if tables.contains("events") {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM events WHERE owner = 'unknown'")
            .fetch_one(&mut *connection)
            .await
            .map_err(sqlite_error)?
    } else {
        0
    };
    let claims = if tables.contains("claims") {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM claims WHERE owner = 'unknown'")
            .fetch_one(&mut *connection)
            .await
            .map_err(sqlite_error)?
    } else {
        0
    };
    Ok(UnknownOwnerInventory {
        events,
        claims,
        rewrite_performed: false,
        rewrite_requires_separate_approval: true,
    })
}

async fn create_backup_anchor(
    database_url: &str,
    path: &Path,
    from_version: Option<i64>,
) -> Result<PathBuf, AppError> {
    let timestamp = Utc::now().format("%Y%m%dT%H%M%S%fZ");
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| AppError::Message("sqlite path has no valid file name".to_string()))?;
    let backup_path = path.with_file_name(format!(
        "{file_name}.pre-migrate-v{}-to-v{}-{timestamp}.bak",
        from_version.unwrap_or(0),
        CURRENT_SCHEMA_VERSION
    ));
    if backup_path.exists() {
        return Err(AppError::Message(format!(
            "backup anchor already exists: {}",
            backup_path.display()
        )));
    }

    let mut connection = SqliteConnection::connect_with(
        &parse_options(database_url)?
            .create_if_missing(false)
            .foreign_keys(true),
    )
    .await
    .map_err(sqlite_error)?;
    let escaped = backup_path.to_string_lossy().replace('\'', "''");
    execute(&mut connection, &format!("VACUUM INTO '{escaped}'")).await?;
    connection.close().await.map_err(sqlite_error)?;
    if !backup_path.is_file() {
        return Err(AppError::Message(format!(
            "backup anchor was not created: {}",
            backup_path.display()
        )));
    }
    Ok(backup_path)
}

async fn connect_pool(
    database_url: &str,
    create_if_missing: bool,
    read_only: bool,
) -> Result<SqlitePool, AppError> {
    SqlitePool::connect_with(
        parse_options(database_url)?
            .create_if_missing(create_if_missing)
            .read_only(read_only)
            .foreign_keys(true),
    )
    .await
    .map_err(sqlite_error)
}

fn parse_options(database_url: &str) -> Result<SqliteConnectOptions, AppError> {
    SqliteConnectOptions::from_str(database_url)
        .map_err(|error| AppError::Message(error.to_string()))
}

fn require_file_path(database_url: &str) -> Result<PathBuf, AppError> {
    sqlite_file_path(database_url).ok_or_else(|| {
        AppError::Message(
            "explicit database lifecycle requires a file-backed sqlite URL".to_string(),
        )
    })
}

fn sqlite_file_path(database_url: &str) -> Option<PathBuf> {
    let path = database_url.strip_prefix("sqlite://")?;
    let path = path.split_once('?').map_or(path, |(path, _)| path);
    if path.is_empty() || path == ":memory:" {
        return None;
    }

    #[cfg(windows)]
    let path = normalize_windows_sqlite_path(path);

    #[cfg(not(windows))]
    let path = path.to_string();

    Some(PathBuf::from(path))
}

#[cfg(windows)]
fn normalize_windows_sqlite_path(path: &str) -> String {
    let bytes = path.as_bytes();
    if bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b':' {
        return path[1..].to_string();
    }
    path.to_string()
}

fn ensure_parent_directory(path: &Path) -> Result<(), AppError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() {
        return Ok(());
    }
    fs::create_dir_all(parent).map_err(|error| {
        AppError::Message(format!(
            "failed to create sqlite parent directory {}: {error}",
            parent.display()
        ))
    })
}

fn path_writable_hint(path: &Path) -> Option<bool> {
    let target = if path.exists() {
        path.to_path_buf()
    } else {
        path.parent()?.to_path_buf()
    };
    fs::metadata(target)
        .ok()
        .map(|metadata| !metadata.permissions().readonly())
}

fn cleanup_sqlite_files(path: &Path) {
    for candidate in [
        path.to_path_buf(),
        PathBuf::from(format!("{}-wal", path.display())),
        PathBuf::from(format!("{}-shm", path.display())),
    ] {
        let _ = fs::remove_file(candidate);
    }
}

fn sqlite_url(path: &Path) -> String {
    format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"))
}

async fn reset_connection_pragmas(connection: &mut SqliteConnection) -> Result<(), AppError> {
    execute(connection, "PRAGMA legacy_alter_table = OFF").await?;
    execute(connection, "PRAGMA foreign_keys = ON").await
}

async fn schema_version(connection: &mut SqliteConnection) -> Result<i64, AppError> {
    sqlx::query_scalar::<_, i64>("PRAGMA user_version")
        .fetch_one(connection)
        .await
        .map_err(sqlite_error)
}

async fn foreign_key_violation_count(connection: &mut SqliteConnection) -> Result<usize, AppError> {
    sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(connection)
        .await
        .map(|rows| rows.len())
        .map_err(sqlite_error)
}

async fn table_names_connection(
    connection: &mut SqliteConnection,
) -> Result<BTreeSet<String>, AppError> {
    sqlx::query(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )
    .fetch_all(connection)
    .await
    .map_err(sqlite_error)
    .map(|rows| {
        rows.into_iter()
            .map(|row| row.get::<String, _>("name"))
            .collect()
    })
}

async fn table_count_connection(
    connection: &mut SqliteConnection,
    table: &str,
) -> Result<i64, AppError> {
    sqlx::query_scalar::<_, i64>(&format!("SELECT COUNT(*) FROM \"{table}\""))
        .fetch_one(connection)
        .await
        .map_err(sqlite_error)
}

fn table_count(counts: &[DatabaseTableCount], table: &str) -> i64 {
    counts
        .iter()
        .find(|count| count.table == table)
        .map_or(0, |count| count.rows)
}

async fn execute(connection: &mut SqliteConnection, sql: &str) -> Result<(), AppError> {
    sqlx::query(sql)
        .execute(connection)
        .await
        .map(|_| ())
        .map_err(sqlite_error)
}

fn sqlite_error(error: sqlx::Error) -> AppError {
    AppError::Message(error.to_string())
}

// Compare against the actual SQLite interpretation of our canonical schema, not
// hand-maintained fragments or the mere presence of a CHECK constraint name.
async fn schema_structure_issues(
    connection: &mut SqliteConnection,
) -> Result<Vec<String>, AppError> {
    let mut reference = SqliteConnection::connect("sqlite::memory:")
        .await
        .map_err(sqlite_error)?;
    execute_init_sql(&mut reference).await?;
    let mut issues = Vec::new();
    for table in REQUIRED_TABLES {
        let expected = table_structure(&mut reference, table).await?;
        let actual = table_structure(connection, table).await?;
        if actual != expected {
            issues.push(format!(
                "{table}: columns, constraints, foreign keys, or indexes differ"
            ));
        }
    }
    reference.close().await.map_err(sqlite_error)?;
    Ok(issues)
}

async fn table_structure(
    connection: &mut SqliteConnection,
    table: &str,
) -> Result<Vec<String>, AppError> {
    let mut result = Vec::new();
    let ddl = sqlx::query_scalar::<_, String>(
        "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?",
    )
    .bind(table)
    .fetch_optional(&mut *connection)
    .await
    .map_err(sqlite_error)?;
    let Some(ddl) = ddl else {
        return Ok(result);
    };
    // SQLite rewrites CREATE TABLE names when renaming and drops IF NOT EXISTS.
    // Compare the body, preserving quoted string contents exactly.
    result.push(normalize_schema_ddl(
        ddl.split_once('(').map_or(ddl.as_str(), |(_, body)| body),
    ));
    for row in sqlx::query(&format!("PRAGMA table_xinfo('{table}')"))
        .fetch_all(&mut *connection)
        .await
        .map_err(sqlite_error)?
    {
        result.push(format!(
            "column:{:?}",
            (
                row.get::<i64, _>("cid"),
                row.get::<String, _>("name"),
                row.get::<String, _>("type"),
                row.get::<i64, _>("notnull"),
                row.get::<Option<String>, _>("dflt_value"),
                row.get::<i64, _>("pk"),
                row.get::<i64, _>("hidden")
            )
        ));
    }
    let mut foreign_keys = Vec::new();
    for row in sqlx::query(&format!("PRAGMA foreign_key_list('{table}')"))
        .fetch_all(&mut *connection)
        .await
        .map_err(sqlite_error)?
    {
        foreign_keys.push(format!(
            "fk:{:?}",
            (
                row.get::<i64, _>("id"),
                row.get::<i64, _>("seq"),
                row.get::<String, _>("table"),
                row.get::<String, _>("from"),
                row.get::<Option<String>, _>("to"),
                row.get::<String, _>("on_update"),
                row.get::<String, _>("on_delete"),
                row.get::<String, _>("match")
            )
        ));
    }
    foreign_keys.sort();
    result.extend(foreign_keys);
    let mut indexes = Vec::new();
    for row in sqlx::query(&format!("PRAGMA index_list('{table}')"))
        .fetch_all(&mut *connection)
        .await
        .map_err(sqlite_error)?
    {
        let name = row.get::<String, _>("name");
        let mut index = format!(
            "index:{:?}",
            (
                &name,
                row.get::<i64, _>("unique"),
                row.get::<String, _>("origin"),
                row.get::<i64, _>("partial")
            )
        );
        for column in sqlx::query(
            "SELECT seqno, cid, name, desc, coll, key FROM pragma_index_xinfo(?) ORDER BY seqno",
        )
        .bind(&name)
        .fetch_all(&mut *connection)
        .await
        .map_err(sqlite_error)?
        {
            index.push_str(&format!(
                "{:?}",
                (
                    column.get::<i64, _>("seqno"),
                    column.get::<i64, _>("cid"),
                    column.get::<Option<String>, _>("name"),
                    column.get::<i64, _>("desc"),
                    column.get::<Option<String>, _>("coll"),
                    column.get::<i64, _>("key")
                )
            ));
        }
        let definition = sqlx::query_scalar::<_, Option<String>>(
            "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = ?",
        )
        .bind(&name)
        .fetch_one(&mut *connection)
        .await
        .map_err(sqlite_error)?;
        index.push_str(&format!(
            "ddl:{:?}",
            definition.map(|ddl| normalize_schema_ddl(&ddl))
        ));
        indexes.push(index);
    }
    indexes.sort();
    result.extend(indexes);
    Ok(result)
}

fn normalize_schema_ddl(ddl: &str) -> String {
    let mut literal = false;
    ddl.chars()
        .filter_map(|ch| {
            if ch == '\'' {
                literal = !literal;
            }
            if literal || ch == '\'' {
                Some(ch)
            } else if ch.is_whitespace() || ch == '"' {
                None
            } else {
                Some(ch.to_ascii_lowercase())
            }
        })
        .collect()
}
