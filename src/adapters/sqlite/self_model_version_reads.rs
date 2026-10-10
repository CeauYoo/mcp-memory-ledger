//! All provenance, projection and page reads share one SQLite snapshot.
use async_trait::async_trait;
use sqlx::{Row, SqliteConnection};

use super::{
    SqliteStore,
    self_model_versions::{load_current, load_reflection, load_version, load_versions_before},
};
use crate::{
    domain::{
        reflection::ReflectionIdentityUpdate,
        reflection_scope::ReflectionScopeStatus,
        self_model_version::{SelfModelVersion, SelfModelVersionKind},
        types::{MemoryScope, Owner},
    },
    error::AppError,
    ports::{
        StoredReflection,
        self_model_version_store::{
            MAX_SELF_MODEL_VERSION_LIMIT, ScopedCommitmentVersionPatch, ScopedIdentityVersionPatch,
            ScopedSelfModelVersionRecord, SelfModelVersionPage, SelfModelVersionQuery,
            SelfModelVersionStore,
        },
    },
};

const REDACTED: &str = "Previous values are hidden because their component source lacks verified same-namespace durable evidence.";

fn unavailable() -> AppError {
    AppError::Message("self-model version read unavailable: stored state failed validation".into())
}

#[async_trait]
impl SelfModelVersionStore for SqliteStore {
    async fn query_self_model_versions(
        &self,
        query: SelfModelVersionQuery,
    ) -> Result<SelfModelVersionPage, AppError> {
        if !query.scope.is_explicitly_scoped()
            || !(1..=MAX_SELF_MODEL_VERSION_LIMIT).contains(&query.limit)
            || query
                .before_version
                .is_some_and(|version| version > i64::MAX as u64)
        {
            return Err(AppError::InvalidParams(
                "invalid self-model version query".into(),
            ));
        }
        // The first read establishes the snapshot. Do not use a pool helper here.
        let mut transaction = self.pool.begin().await.map_err(|_| unavailable())?;
        let result = read_page(&mut transaction, query)
            .await
            .map_err(|_| unavailable())?;
        transaction.commit().await.map_err(|_| unavailable())?;
        Ok(result)
    }
}

async fn read_page(
    connection: &mut SqliteConnection,
    query: SelfModelVersionQuery,
) -> Result<SelfModelVersionPage, AppError> {
    let head = load_current(connection).await?;
    let mut cursor = query.before_version;
    let mut records = Vec::with_capacity(query.limit + 1);
    loop {
        // Bounded batches avoid materializing all internal aggregate snapshots.
        let versions = load_versions_before(connection, cursor, 100).await?;
        if versions.is_empty() {
            break;
        }
        let batch_len = versions.len();
        for version in versions {
            cursor = Some(version.version);
            if matches!(
                version.kind,
                SelfModelVersionKind::InitializationBaseline
                    | SelfModelVersionKind::MigrationBaseline
            ) {
                continue;
            }
            if let Some(record) = scoped_record(connection, &version, &query.scope).await? {
                records.push(record);
                if records.len() > query.limit {
                    records.truncate(query.limit);
                    return Ok(SelfModelVersionPage {
                        current_version: head.version,
                        has_more: true,
                        records,
                    });
                }
            }
        }
        if batch_len < 100 {
            break;
        }
    }
    Ok(SelfModelVersionPage {
        current_version: head.version,
        has_more: false,
        records,
    })
}

async fn scoped_record(
    connection: &mut SqliteConnection,
    version: &SelfModelVersion,
    scope: &MemoryScope,
) -> Result<Option<ScopedSelfModelVersionRecord>, AppError> {
    let identity = if version.identity_written {
        identity_source(connection, version, scope).await?
    } else {
        None
    };
    let commitments = if version.commitments_written {
        commitment_source(connection, version, scope).await?
    } else {
        None
    };
    if identity.is_none() && commitments.is_none() {
        return Ok(None);
    }
    let previous = match version.previous_version {
        Some(id) => Some(
            load_version(connection, id)
                .await?
                .ok_or_else(unavailable)?,
        ),
        None => None,
    };
    let identity_update = if let Some((source_version, source_reflection_id, patch)) = identity {
        let before = if let Some(previous) = &previous {
            identity_source(connection, previous, scope)
                .await?
                .map(|(_, _, patch)| patch)
        } else {
            None
        };
        let previous_claims = previous
            .as_ref()
            .map(|v| v.identity.canonical_claims())
            .unwrap_or(&[]);
        Some(ScopedIdentityVersionPatch {
            source_version,
            source_reflection_id,
            changed: previous_claims != patch.canonical_claims.as_slice(),
            previous_count: previous_claims.len(),
            current_count: patch.canonical_claims.len(),
            previous_values_redacted: before.is_none().then(|| REDACTED.to_owned()),
            previous_patch: before,
            patch,
        })
    } else {
        None
    };
    let commitment_updates =
        if let Some((source_version, source_reflection_id, patch)) = commitments {
            let before = if let Some(previous) = &previous {
                commitment_source(connection, previous, scope)
                    .await?
                    .map(|(_, _, patch)| patch)
            } else {
                None
            };
            let previous_commitments = previous
                .as_ref()
                .map(|v| v.commitments.as_slice())
                .unwrap_or(&[]);
            Some(ScopedCommitmentVersionPatch {
                source_version,
                source_reflection_id,
                changed: previous_commitments != patch.as_slice(),
                previous_count: previous_commitments.len(),
                current_count: patch.len(),
                previous_values_redacted: before.is_none().then(|| REDACTED.to_owned()),
                previous_patch: before,
                patch,
            })
        } else {
            None
        };
    Ok(Some(ScopedSelfModelVersionRecord {
        version: version.version,
        previous_version: version.previous_version,
        kind: version.kind,
        recorded_at: version.recorded_at,
        effective_at: version.effective_at,
        rollback_target_version: version.rollback_target_version,
        identity_update,
        commitment_updates,
    }))
}

async fn verified_source(
    connection: &mut SqliteConnection,
    source: &SelfModelVersion,
    scope: &MemoryScope,
) -> Result<Option<StoredReflection>, AppError> {
    if matches!(
        source.kind,
        SelfModelVersionKind::InitializationBaseline | SelfModelVersionKind::MigrationBaseline
    ) {
        return Ok(None);
    }
    let Some(id) = &source.reflection_id else {
        return Ok(None);
    };
    let Some(reflection) = load_reflection(connection, id).await? else {
        return Ok(None);
    };
    if reflection.scope.status != ReflectionScopeStatus::Verified
        || reflection.scope.origin_scopes.as_slice() != std::slice::from_ref(scope)
        || reflection.supporting_evidence_event_ids.is_empty()
    {
        return Ok(None);
    }
    let owner = match scope.owner() {
        Some(Owner::Self_) => "self",
        Some(Owner::User) => "user",
        Some(Owner::World) => "world",
        _ => return Ok(None),
    };
    let Some(namespace) = scope.namespace() else {
        return Ok(None);
    };
    for id in &reflection.supporting_evidence_event_ids {
        let row = sqlx::query("SELECT owner, namespace FROM events WHERE event_id = ?")
            .bind(id)
            .fetch_optional(&mut *connection)
            .await
            .map_err(|_| unavailable())?;
        let Some(row) = row else { return Ok(None) };
        if row
            .try_get::<String, _>("owner")
            .map_err(|_| unavailable())?
            != owner
            || row
                .try_get::<String, _>("namespace")
                .map_err(|_| unavailable())?
                != namespace.as_str()
        {
            return Ok(None);
        }
    }
    Ok(Some(reflection))
}

async fn identity_source(
    connection: &mut SqliteConnection,
    version: &SelfModelVersion,
    scope: &MemoryScope,
) -> Result<Option<(u64, String, ReflectionIdentityUpdate)>, AppError> {
    let Some(source) = load_version(connection, version.identity_source_version).await? else {
        return Ok(None);
    };
    if !source.identity_written
        || source.identity_source_version != source.version
        || (version.identity_written && version.identity_source_version != version.version)
        || source.identity != version.identity
    {
        return Ok(None);
    }
    let Some(reflection) = verified_source(connection, &source, scope).await? else {
        return Ok(None);
    };
    let Some(patch) = reflection.requested_identity_update else {
        return Ok(None);
    };
    // Preserve ordered duplicate semantics; never sort, deduplicate or hash values.
    if patch.canonical_claims.as_slice() != version.identity.canonical_claims() {
        return Ok(None);
    }
    Ok(Some((source.version, reflection.reflection_id, patch)))
}

async fn commitment_source(
    connection: &mut SqliteConnection,
    version: &SelfModelVersion,
    scope: &MemoryScope,
) -> Result<Option<(u64, String, Vec<crate::domain::commitment::Commitment>)>, AppError> {
    let Some(source) = load_version(connection, version.commitment_source_version).await? else {
        return Ok(None);
    };
    if !source.commitments_written
        || source.commitment_source_version != source.version
        || (version.commitments_written && version.commitment_source_version != version.version)
        || source.commitments != version.commitments
    {
        return Ok(None);
    }
    let Some(reflection) = verified_source(connection, &source, scope).await? else {
        return Ok(None);
    };
    let Some(patch) = reflection.requested_commitment_updates else {
        return Ok(None);
    };
    if patch != version.commitments {
        return Ok(None);
    }
    Ok(Some((source.version, reflection.reflection_id, patch)))
}
