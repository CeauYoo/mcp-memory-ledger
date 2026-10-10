//! Internal, append-only global self-model ledger. Full snapshots never leave
//! this adapter through scoped history/export/context responses.
use chrono::{DateTime, Utc};
use sqlx::{Row, SqliteConnection, sqlite::SqliteRow};

use crate::{
    domain::{
        commitment::Commitment,
        identity_core::IdentityCore,
        reflection::Reflection,
        reflection_scope::{ReflectionScopeMetadata, ReflectionScopeStatus, known_scope},
        self_model_version::{SelfModelVersion, SelfModelVersionKind},
        types::{Namespace, Owner},
    },
    error::AppError,
    ports::StoredReflection,
};

const TABLE_SQL: &str = r#"CREATE TABLE IF NOT EXISTS self_model_versions (
    version INTEGER NOT NULL PRIMARY KEY CHECK (version >= 0),
    previous_version INTEGER,
    kind TEXT NOT NULL CHECK (kind IN ('initialization_baseline', 'migration_baseline', 'update', 'rollback')),
    reflection_id TEXT UNIQUE,
    recorded_at TEXT NOT NULL,
    effective_at TEXT,
    identity_json TEXT NOT NULL CHECK (json_valid(identity_json)),
    commitments_json TEXT NOT NULL CHECK (json_valid(commitments_json)),
    identity_written INTEGER NOT NULL CHECK (identity_written IN (0, 1)),
    commitments_written INTEGER NOT NULL CHECK (commitments_written IN (0, 1)),
    identity_source_version INTEGER NOT NULL CHECK (identity_source_version >= 0 AND identity_source_version <= version),
    commitment_source_version INTEGER NOT NULL CHECK (commitment_source_version >= 0 AND commitment_source_version <= version),
    rollback_target_version INTEGER,
    FOREIGN KEY (previous_version) REFERENCES self_model_versions(version),
    FOREIGN KEY (identity_source_version) REFERENCES self_model_versions(version),
    FOREIGN KEY (commitment_source_version) REFERENCES self_model_versions(version),
    FOREIGN KEY (rollback_target_version) REFERENCES self_model_versions(version),
    FOREIGN KEY (reflection_id) REFERENCES reflections(reflection_id),
    CHECK ((version = 0 AND previous_version IS NULL AND reflection_id IS NULL
        AND kind IN ('initialization_baseline', 'migration_baseline')
        AND identity_written = 0 AND commitments_written = 0
        AND identity_source_version = 0 AND commitment_source_version = 0
        AND rollback_target_version IS NULL)
        OR (version > 0 AND previous_version IS NOT NULL AND previous_version = version - 1
        AND reflection_id IS NOT NULL AND kind IN ('update', 'rollback')
        AND effective_at IS NOT NULL AND (identity_written = 1 OR commitments_written = 1))),
    CHECK ((kind = 'migration_baseline' AND effective_at IS NULL)
        OR (kind <> 'migration_baseline' AND effective_at IS NOT NULL)),
    CHECK ((kind = 'rollback' AND rollback_target_version IS NOT NULL AND rollback_target_version < version)
        OR (kind <> 'rollback' AND rollback_target_version IS NULL))
)"#;

pub(super) fn trigger_ddl() -> [(&'static str, &'static str); 3] {
    [
        (
            "self_model_versions_append",
            "CREATE TRIGGER self_model_versions_append BEFORE INSERT ON self_model_versions BEGIN SELECT CASE WHEN NEW.version <> COALESCE((SELECT MAX(version) + 1 FROM self_model_versions), 0) THEN RAISE(ABORT, 'self-model versions must append contiguously') END; SELECT CASE WHEN NEW.reflection_id IS NOT NULL AND EXISTS (SELECT 1 FROM self_model_versions WHERE reflection_id = NEW.reflection_id) THEN RAISE(ABORT, 'self-model reflection provenance is immutable') END; END",
        ),
        (
            "self_model_versions_no_update",
            "CREATE TRIGGER self_model_versions_no_update BEFORE UPDATE ON self_model_versions BEGIN SELECT RAISE(ABORT, 'self-model versions are immutable'); END",
        ),
        (
            "self_model_versions_no_delete",
            "CREATE TRIGGER self_model_versions_no_delete BEFORE DELETE ON self_model_versions BEGIN SELECT RAISE(ABORT, 'self-model versions are immutable'); END",
        ),
    ]
}

/// Structural installation only. Canonical schema references and index repair
/// must never seed history or manufacture an effective time.
pub(super) async fn install(connection: &mut SqliteConnection) -> Result<(), AppError> {
    sqlx::query(TABLE_SQL)
        .execute(&mut *connection)
        .await
        .map_err(error)?;
    for (_, ddl) in trigger_ddl() {
        sqlx::query(ddl)
            .execute(&mut *connection)
            .await
            .map_err(error)?;
    }
    Ok(())
}

fn error(error: impl std::fmt::Display) -> AppError {
    AppError::Message(error.to_string())
}
fn invalid(message: &str) -> AppError {
    AppError::Message(format!("self-model ledger: {message}"))
}
fn signed(version: u64) -> Result<i64, AppError> {
    i64::try_from(version).map_err(|_| invalid("version exceeds SQLite integer range"))
}
fn unsigned(version: i64) -> Result<u64, AppError> {
    u64::try_from(version).map_err(|_| invalid("negative stored version"))
}
fn timestamp(value: &str) -> Result<DateTime<Utc>, AppError> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(error)
}
fn owner(value: &str) -> Result<Owner, AppError> {
    match value {
        "self" => Ok(Owner::Self_),
        "user" => Ok(Owner::User),
        "world" => Ok(Owner::World),
        "unknown" => Ok(Owner::Unknown),
        _ => Err(invalid("unknown stored owner")),
    }
}
fn kind(value: &str) -> Result<SelfModelVersionKind, AppError> {
    match value {
        "initialization_baseline" => Ok(SelfModelVersionKind::InitializationBaseline),
        "migration_baseline" => Ok(SelfModelVersionKind::MigrationBaseline),
        "update" => Ok(SelfModelVersionKind::Update),
        "rollback" => Ok(SelfModelVersionKind::Rollback),
        _ => Err(invalid("unknown version kind")),
    }
}
fn kind_str(value: SelfModelVersionKind) -> &'static str {
    match value {
        SelfModelVersionKind::InitializationBaseline => "initialization_baseline",
        SelfModelVersionKind::MigrationBaseline => "migration_baseline",
        SelfModelVersionKind::Update => "update",
        SelfModelVersionKind::Rollback => "rollback",
    }
}
fn from_row(row: SqliteRow) -> Result<SelfModelVersion, AppError> {
    Ok(SelfModelVersion {
        version: unsigned(row.try_get("version").map_err(error)?)?,
        previous_version: row
            .try_get::<Option<i64>, _>("previous_version")
            .map_err(error)?
            .map(unsigned)
            .transpose()?,
        kind: kind(&row.try_get::<String, _>("kind").map_err(error)?)?,
        reflection_id: row.try_get("reflection_id").map_err(error)?,
        recorded_at: timestamp(&row.try_get::<String, _>("recorded_at").map_err(error)?)?,
        effective_at: row
            .try_get::<Option<String>, _>("effective_at")
            .map_err(error)?
            .as_deref()
            .map(timestamp)
            .transpose()?,
        identity: serde_json::from_str(&row.try_get::<String, _>("identity_json").map_err(error)?)
            .map_err(|_| invalid("malformed stored identity snapshot"))?,
        commitments: serde_json::from_str(
            &row.try_get::<String, _>("commitments_json")
                .map_err(error)?,
        )
        .map_err(|_| invalid("malformed stored commitments snapshot"))?,
        identity_written: row.try_get("identity_written").map_err(error)?,
        commitments_written: row.try_get("commitments_written").map_err(error)?,
        identity_source_version: unsigned(row.try_get("identity_source_version").map_err(error)?)?,
        commitment_source_version: unsigned(
            row.try_get("commitment_source_version").map_err(error)?,
        )?,
        rollback_target_version: row
            .try_get::<Option<i64>, _>("rollback_target_version")
            .map_err(error)?
            .map(unsigned)
            .transpose()?,
    })
}

pub(super) async fn load_version(
    connection: &mut SqliteConnection,
    version: u64,
) -> Result<Option<SelfModelVersion>, AppError> {
    sqlx::query("SELECT * FROM self_model_versions WHERE version = ?")
        .bind(signed(version)?)
        .fetch_optional(connection)
        .await
        .map_err(error)?
        .map(from_row)
        .transpose()
}

pub(super) async fn load_versions_before(
    connection: &mut SqliteConnection,
    before_version: Option<u64>,
    limit: usize,
) -> Result<Vec<SelfModelVersion>, AppError> {
    let before = before_version.map(signed).transpose()?;
    let limit = i64::try_from(limit).map_err(error)?;
    sqlx::query("SELECT * FROM self_model_versions WHERE (? IS NULL OR version < ?) ORDER BY version DESC LIMIT ?")
        .bind(before).bind(before).bind(limit).fetch_all(connection).await.map_err(error)?
        .into_iter().map(from_row).collect()
}

async fn load_head(connection: &mut SqliteConnection) -> Result<SelfModelVersion, AppError> {
    let row = sqlx::query("SELECT * FROM self_model_versions ORDER BY version DESC LIMIT 1")
        .fetch_optional(&mut *connection)
        .await
        .map_err(error)?
        .ok_or_else(|| invalid("missing baseline; explicit repair is required"))?;
    let head = from_row(row)?;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM self_model_versions")
        .fetch_one(connection)
        .await
        .map_err(error)?;
    if unsigned(count)?
        != head
            .version
            .checked_add(1)
            .ok_or_else(|| invalid("version overflow"))?
    {
        return Err(invalid("version chain is not contiguous"));
    }
    Ok(head)
}

async fn projections(
    connection: &mut SqliteConnection,
) -> Result<(IdentityCore, Vec<Commitment>), AppError> {
    let claims: Vec<String> =
        sqlx::query_scalar("SELECT claim FROM identity_claims ORDER BY position")
            .fetch_all(&mut *connection)
            .await
            .map_err(error)?;
    let commitments = sqlx::query("SELECT owner, description FROM commitments ORDER BY rowid")
        .fetch_all(connection)
        .await
        .map_err(error)?
        .into_iter()
        .map(|row| {
            Ok(Commitment::new(
                owner(&row.get::<String, _>("owner"))?,
                row.get::<String, _>("description"),
            ))
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    Ok((IdentityCore::new(claims), commitments))
}

async fn verify_projections(
    connection: &mut SqliteConnection,
    version: &SelfModelVersion,
) -> Result<(), AppError> {
    let (identity, commitments) = projections(connection).await?;
    if identity != version.identity || commitments != version.commitments {
        return Err(invalid(
            "current projection drift; explicit repair is required",
        ));
    }
    Ok(())
}

pub(super) async fn load_current(
    connection: &mut SqliteConnection,
) -> Result<SelfModelVersion, AppError> {
    let head = load_head(connection).await?;
    verify_projections(connection, &head).await?;
    Ok(head)
}

/// Only explicit initialization/migration calls this after runtime defaults.
pub(super) async fn seed_baseline(
    connection: &mut SqliteConnection,
    initialization: bool,
) -> Result<(), AppError> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM self_model_versions")
        .fetch_one(&mut *connection)
        .await
        .map_err(error)?;
    if count != 0 {
        return Err(invalid("refusing to rebaseline existing history"));
    }
    let (identity, commitments) = projections(connection).await?;
    let recorded_at = Utc::now();
    insert(
        connection,
        &SelfModelVersion {
            version: 0,
            previous_version: None,
            kind: if initialization {
                SelfModelVersionKind::InitializationBaseline
            } else {
                SelfModelVersionKind::MigrationBaseline
            },
            reflection_id: None,
            recorded_at,
            effective_at: initialization.then_some(recorded_at),
            identity,
            commitments,
            identity_written: false,
            commitments_written: false,
            identity_source_version: 0,
            commitment_source_version: 0,
            rollback_target_version: None,
        },
    )
    .await
}

async fn insert(
    connection: &mut SqliteConnection,
    value: &SelfModelVersion,
) -> Result<(), AppError> {
    sqlx::query("INSERT INTO self_model_versions (version, previous_version, kind, reflection_id, recorded_at, effective_at, identity_json, commitments_json, identity_written, commitments_written, identity_source_version, commitment_source_version, rollback_target_version) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(signed(value.version)?).bind(value.previous_version.map(signed).transpose()?)
        .bind(kind_str(value.kind)).bind(&value.reflection_id).bind(value.recorded_at.to_rfc3339())
        .bind(value.effective_at.map(|value| value.to_rfc3339()))
        .bind(serde_json::to_string(&value.identity).map_err(error)?)
        .bind(serde_json::to_string(&value.commitments).map_err(error)?)
        .bind(value.identity_written).bind(value.commitments_written)
        .bind(signed(value.identity_source_version)?).bind(signed(value.commitment_source_version)?)
        .bind(value.rollback_target_version.map(signed).transpose()?)
        .execute(connection).await.map_err(error)?;
    Ok(())
}

pub(super) async fn append(
    connection: &mut SqliteConnection,
    expected_version: u64,
    value: &SelfModelVersion,
) -> Result<(), AppError> {
    let head = load_head(connection).await?;
    if head.version != expected_version
        || value.previous_version != Some(expected_version)
        || Some(value.version) != expected_version.checked_add(1)
    {
        return Err(invalid("stale expected version"));
    }
    if !matches!(
        value.kind,
        SelfModelVersionKind::Update | SelfModelVersionKind::Rollback
    ) || !(value.identity_written || value.commitments_written)
    {
        return Err(invalid("append requires a written component"));
    }
    if (!value.identity_written
        && (value.identity != head.identity
            || value.identity_source_version != head.identity_source_version))
        || (!value.commitments_written
            && (value.commitments != head.commitments
                || value.commitment_source_version != head.commitment_source_version))
    {
        return Err(invalid("unwritten component changed"));
    }
    if (value.identity_written && value.identity_source_version != value.version)
        || (value.commitments_written && value.commitment_source_version != value.version)
    {
        return Err(invalid(
            "written component must name its new source version",
        ));
    }
    if let Some(target_version) = value.rollback_target_version {
        if value.kind != SelfModelVersionKind::Rollback || target_version >= value.version {
            return Err(invalid("invalid rollback target"));
        }
        let target = load_version(connection, target_version)
            .await?
            .ok_or_else(|| invalid("missing rollback target"))?;
        if (value.identity_written && value.identity != target.identity)
            || (value.commitments_written && value.commitments != target.commitments)
        {
            return Err(invalid(
                "rollback patches differ from their target snapshot",
            ));
        }
    } else if value.kind == SelfModelVersionKind::Rollback {
        return Err(invalid("rollback requires a target"));
    }
    verify_projections(connection, value).await?;
    let reflection_id = value
        .reflection_id
        .as_deref()
        .ok_or_else(|| invalid("append requires reflection provenance"))?;
    let reflection = load_reflection(connection, reflection_id)
        .await?
        .ok_or_else(|| invalid("missing reflection provenance"))?;
    if reflection.recorded_at != value.recorded_at
        || value.effective_at != Some(value.recorded_at)
        || reflection.requested_identity_update.is_some() != value.identity_written
        || reflection.requested_commitment_updates.is_some() != value.commitments_written
        || reflection
            .requested_identity_update
            .as_ref()
            .is_some_and(|patch| patch.canonical_claims != value.identity.canonical_claims())
        || reflection
            .requested_commitment_updates
            .as_ref()
            .is_some_and(|patch| patch != &value.commitments)
    {
        return Err(invalid("version and reflection patches differ"));
    }
    insert(connection, value).await
}

/// Internal provenance reconstruction uses durable scope/evidence relations,
/// never historical JSON fallbacks after normalization.
pub(super) async fn load_reflection(
    connection: &mut SqliteConnection,
    id: &str,
) -> Result<Option<StoredReflection>, AppError> {
    let Some(row) = sqlx::query("SELECT * FROM reflections WHERE reflection_id = ?")
        .bind(id)
        .fetch_optional(&mut *connection)
        .await
        .map_err(error)?
    else {
        return Ok(None);
    };
    let mut scope = ReflectionScopeMetadata {
        status: match row.get::<String, _>("scope_status").as_str() {
            "verified" => ReflectionScopeStatus::Verified,
            "legacy_unambiguous" => ReflectionScopeStatus::LegacyUnambiguous,
            _ => ReflectionScopeStatus::Unknown,
        },
        ..Default::default()
    };
    let normalized: bool = row.get("evidence_normalized");
    if !normalized {
        scope.status = ReflectionScopeStatus::Unknown;
    }
    for row in sqlx::query("SELECT role, owner, namespace FROM reflection_scopes WHERE reflection_id = ? ORDER BY role, namespace, owner")
        .bind(id).fetch_all(&mut *connection).await.map_err(error)? {
        let namespace = Namespace::parse(row.get::<String, _>("namespace")).map_err(AppError::from)?;
        let source = known_scope(owner(&row.get::<String, _>("owner"))?, &namespace).ok_or_else(|| invalid("invalid stored reflection scope"))?;
        match row.get::<String, _>("role").as_str() {
            "origin" => scope.origin_scopes.push(source),
            "affected" => scope.affected_scopes.push(source),
            _ => return Err(invalid("invalid stored reflection scope role")),
        }
    }
    let supporting_evidence_event_ids = if normalized {
        sqlx::query_scalar("SELECT event_id FROM reflection_evidence WHERE reflection_id = ? ORDER BY position, event_id")
            .bind(id).fetch_all(connection).await.map_err(error)?
    } else {
        Vec::new()
    };
    Ok(Some(StoredReflection {
        scope,
        reflection_id: id.to_string(),
        recorded_at: timestamp(&row.get::<String, _>("recorded_at"))?,
        reflection: Reflection::new(row.get::<String, _>("summary")),
        superseded_claim_id: row.get("superseded_claim_id"),
        replacement_claim_id: row.get("replacement_claim_id"),
        supporting_evidence_event_ids,
        requested_identity_update: row
            .get::<Option<String>, _>("requested_identity_update")
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|_| invalid("malformed stored reflection patch"))?,
        requested_commitment_updates: row
            .get::<Option<String>, _>("requested_commitment_updates")
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|_| invalid("malformed stored reflection patch"))?,
    }))
}
