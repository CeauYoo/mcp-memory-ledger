//! Durable Reflection scope/evidence relations. These are ledger data, not a
//! disposable retrieval index. Writes are part of the caller's transaction.
use crate::{
    domain::{
        reflection_scope::{
            ReflectionScopeMetadata, ReflectionScopeStatus, known_scope, push_unique,
        },
        types::{MemoryScope, Namespace, Owner},
    },
    error::AppError,
    ports::StoredReflection,
};
use sqlx::{Row, SqliteConnection, SqlitePool};

pub(super) const REFLECTION_RELATIONS_SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS reflection_scopes (
    reflection_id TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('origin', 'affected')),
    owner TEXT NOT NULL,
    namespace TEXT NOT NULL,
    PRIMARY KEY (reflection_id, role, owner, namespace),
    FOREIGN KEY (reflection_id) REFERENCES reflections(reflection_id),
    CHECK ((owner = 'self' AND namespace = 'self') OR
           (owner = 'user' AND namespace LIKE 'user/%') OR
           (owner = 'world' AND (namespace = 'world' OR namespace LIKE 'project/%')))
);
CREATE INDEX IF NOT EXISTS reflection_scopes_reverse ON reflection_scopes(role, owner, namespace, reflection_id);
CREATE TABLE IF NOT EXISTS reflection_evidence (
    reflection_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    position INTEGER NOT NULL CHECK (position >= 0),
    PRIMARY KEY (reflection_id, event_id),
    FOREIGN KEY (reflection_id) REFERENCES reflections(reflection_id),
    FOREIGN KEY (event_id) REFERENCES events(event_id)
);
CREATE INDEX IF NOT EXISTS reflection_evidence_reverse ON reflection_evidence(event_id, reflection_id);
"#;

fn error(e: sqlx::Error) -> AppError {
    AppError::Message(e.to_string())
}
fn owner_str(owner: Owner) -> &'static str {
    match owner {
        Owner::Self_ => "self",
        Owner::User => "user",
        Owner::World => "world",
        Owner::Unknown => "unknown",
    }
}
fn source_scope(owner: String, namespace: String) -> Option<MemoryScope> {
    let owner = match owner.as_str() {
        "self" => Owner::Self_,
        "user" => Owner::User,
        "world" => Owner::World,
        _ => return None,
    };
    let namespace = Namespace::parse(namespace).ok()?;
    known_scope(owner, &namespace)
}
async fn source(
    connection: &mut SqliteConnection,
    table: &str,
    id_column: &str,
    id: &str,
) -> Result<Option<MemoryScope>, AppError> {
    let sql = format!("SELECT owner, namespace FROM {table} WHERE {id_column} = ?");
    Ok(sqlx::query(&sql)
        .bind(id)
        .fetch_optional(&mut *connection)
        .await
        .map_err(error)?
        .and_then(|row| source_scope(row.get("owner"), row.get("namespace"))))
}

async fn write_scopes(
    connection: &mut SqliteConnection,
    id: &str,
    scope: &ReflectionScopeMetadata,
) -> Result<(), AppError> {
    for (role, scopes) in [
        ("origin", &scope.origin_scopes),
        ("affected", &scope.affected_scopes),
    ] {
        for scope in scopes {
            let (Some(owner), Some(namespace)) = (scope.owner(), scope.namespace()) else {
                return Err(AppError::InvalidParams(
                    "reflection scope must be explicit".into(),
                ));
            };
            sqlx::query("INSERT INTO reflection_scopes(reflection_id, role, owner, namespace) VALUES (?, ?, ?, ?)")
                .bind(id).bind(role).bind(owner_str(owner)).bind(namespace.as_str())
                .execute(&mut *connection).await.map_err(error)?;
        }
    }
    sqlx::query("UPDATE reflections SET scope_status = ? WHERE reflection_id = ?")
        .bind(scope.status.as_str())
        .bind(id)
        .execute(&mut *connection)
        .await
        .map_err(error)?;
    Ok(())
}

/// Called immediately after inserting the Reflection row, on that same writer
/// connection. FK or scope failures roll back the row, mutations and audit too.
pub(super) async fn persist_relations(
    connection: &mut SqliteConnection,
    reflection: &StoredReflection,
) -> Result<(), AppError> {
    let mut evidence = Vec::new();
    for id in &reflection.supporting_evidence_event_ids {
        if !evidence.contains(id) {
            evidence.push(id.clone());
        }
    }
    // Recheck verified attribution against durable sources rather than trusting a
    // supplied label. Unknown legacy writes keep their prior behavior.
    if reflection.scope.status != ReflectionScopeStatus::Unknown {
        let mut sources = Vec::new();
        for id in [
            &reflection.superseded_claim_id,
            &reflection.replacement_claim_id,
        ]
        .into_iter()
        .flatten()
        {
            sources.push(source(connection, "claims", "claim_id", id).await?);
        }
        for id in &evidence {
            sources.push(source(connection, "events", "event_id", id).await?);
        }
        if sources.is_empty()
            || sources.iter().any(Option::is_none)
            || reflection.scope.origin_scopes.len() != 1
            || sources
                .iter()
                .flatten()
                .any(|scope| Some(scope) != reflection.scope.origin_scopes.first())
        {
            return Err(AppError::InvalidParams(
                "reflection scope metadata does not match durable sources".into(),
            ));
        }
        let mut effects = Vec::new();
        if let Some(id) = &reflection.superseded_claim_id
            && let Some(scope) = source(connection, "claims", "claim_id", id).await?
        {
            push_unique(&mut effects, scope);
        }
        if reflection.requested_identity_update.is_some()
            || reflection.requested_commitment_updates.is_some()
        {
            push_unique(&mut effects, MemoryScope::self_());
        }
        if effects.len() != reflection.scope.affected_scopes.len()
            || effects
                .iter()
                .any(|scope| !reflection.scope.affected_scopes.contains(scope))
        {
            return Err(AppError::InvalidParams(
                "reflection affected scopes do not match its mutations".into(),
            ));
        }
    }
    for (position, id) in evidence.into_iter().enumerate() {
        sqlx::query(
            "INSERT INTO reflection_evidence(reflection_id, event_id, position) VALUES (?, ?, ?)",
        )
        .bind(&reflection.reflection_id)
        .bind(id)
        .bind(position as i64)
        .execute(&mut *connection)
        .await
        .map_err(error)?;
    }
    write_scopes(connection, &reflection.reflection_id, &reflection.scope).await?;
    sqlx::query("UPDATE reflections SET evidence_normalized = 1 WHERE reflection_id = ?")
        .bind(&reflection.reflection_id)
        .execute(&mut *connection)
        .await
        .map_err(error)?;
    Ok(())
}

/// Migration only: do not run on every open and never use this as index recovery.
/// Missing/malformed evidence, unknown owners and mixed origins remain unknown.
pub(super) async fn backfill_legacy(connection: &mut SqliteConnection) -> Result<(), AppError> {
    let rows = sqlx::query("SELECT reflection_id, superseded_claim_id, replacement_claim_id, supporting_evidence_event_ids, requested_identity_update, requested_commitment_updates FROM reflections WHERE scope_status = 'unknown' AND evidence_normalized = 0")
        .fetch_all(&mut *connection).await.map_err(error)?;
    for row in rows {
        let id: String = row.get("reflection_id");
        let old: Option<String> = row.get("superseded_claim_id");
        let new: Option<String> = row.get("replacement_claim_id");
        let evidence = serde_json::from_str::<Vec<String>>(
            &row.get::<String, _>("supporting_evidence_event_ids"),
        )
        .ok();
        let mut origins = Vec::new();
        let mut effects = Vec::new();
        let mut unknown = evidence.is_none();
        for endpoint in [&old, &new].into_iter().flatten() {
            match source(connection, "claims", "claim_id", endpoint).await? {
                Some(scope) => {
                    push_unique(&mut origins, scope.clone());
                    push_unique(&mut effects, scope);
                }
                None => unknown = true,
            }
        }
        let mut evidence_complete = evidence.is_some();
        let mut unique_evidence = Vec::new();
        for event_id in evidence.unwrap_or_default() {
            // Orphans cannot enter an FK relation, but historical JSON is retained.
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM events WHERE event_id = ?)")
                    .bind(&event_id)
                    .fetch_one(&mut *connection)
                    .await
                    .map_err(error)?;
            if !exists {
                evidence_complete = false;
            }
            match source(connection, "events", "event_id", &event_id).await? {
                Some(scope) => push_unique(&mut origins, scope),
                None => unknown = true,
            }
            if !unique_evidence.contains(&event_id) {
                unique_evidence.push(event_id);
            }
        }
        if row
            .get::<Option<String>, _>("requested_identity_update")
            .is_some()
            || row
                .get::<Option<String>, _>("requested_commitment_updates")
                .is_some()
        {
            push_unique(&mut effects, MemoryScope::self_());
        }
        let verified = !unknown && origins.len() == 1;
        let metadata = ReflectionScopeMetadata {
            status: if verified {
                ReflectionScopeStatus::LegacyUnambiguous
            } else {
                ReflectionScopeStatus::Unknown
            },
            origin_scopes: if verified { origins } else { Vec::new() },
            affected_scopes: effects,
        };
        write_scopes(connection, &id, &metadata).await?;
        if evidence_complete {
            for (position, event_id) in unique_evidence.into_iter().enumerate() {
                sqlx::query("INSERT INTO reflection_evidence(reflection_id, event_id, position) VALUES (?, ?, ?)").bind(&id).bind(event_id).bind(position as i64).execute(&mut *connection).await.map_err(error)?;
            }
            sqlx::query("UPDATE reflections SET evidence_normalized = 1 WHERE reflection_id = ?")
                .bind(&id)
                .execute(&mut *connection)
                .await
                .map_err(error)?;
        }
    }
    Ok(())
}

pub(super) async fn load_metadata(
    pool: &SqlitePool,
    id: &str,
) -> Result<ReflectionScopeMetadata, AppError> {
    let status: Option<String> =
        sqlx::query_scalar("SELECT scope_status FROM reflections WHERE reflection_id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
            .map_err(error)?;
    let mut metadata = ReflectionScopeMetadata {
        status: match status.as_deref() {
            Some("verified") => ReflectionScopeStatus::Verified,
            Some("legacy_unambiguous") => ReflectionScopeStatus::LegacyUnambiguous,
            _ => ReflectionScopeStatus::Unknown,
        },
        ..Default::default()
    };
    for row in sqlx::query("SELECT role, owner, namespace FROM reflection_scopes WHERE reflection_id = ? ORDER BY role, namespace, owner").bind(id).fetch_all(pool).await.map_err(error)? {
        if let Some(scope) = source_scope(row.get("owner"), row.get("namespace")) {
            if row.get::<String, _>("role") == "origin" { metadata.origin_scopes.push(scope); } else { metadata.affected_scopes.push(scope); }
        }
    }
    Ok(metadata)
}

/// Use in SELECT instead of the JSON column. Once normalized, the relation is
/// authoritative even when empty; old JSON must never revive deleted relations.
pub(super) fn evidence_ids_sql(alias: &str) -> String {
    format!(
        "CASE WHEN {alias}.evidence_normalized = 1 THEN (SELECT json_group_array(event_id) FROM (SELECT event_id FROM reflection_evidence WHERE reflection_id = {alias}.reflection_id ORDER BY position, event_id)) ELSE {alias}.supporting_evidence_event_ids END"
    )
}

/// Only the targetless branch uses this predicate. Existing anchored branches
/// MUST retain both endpoint scope checks independently of attribution metadata.
pub(super) fn targetless_visibility_sql(alias: &str) -> String {
    format!(
        "({alias}.superseded_claim_id IS NULL AND {alias}.replacement_claim_id IS NULL AND {alias}.scope_status IN ('verified', 'legacy_unambiguous') AND {alias}.evidence_normalized = 1 AND EXISTS (SELECT 1 FROM reflection_scopes origin WHERE origin.reflection_id = {alias}.reflection_id AND origin.role = 'origin' AND origin.owner = ? AND origin.namespace = ?) AND (SELECT count(*) FROM reflection_scopes origin WHERE origin.reflection_id = {alias}.reflection_id AND origin.role = 'origin') = 1 AND NOT EXISTS (SELECT 1 FROM reflection_scopes effect WHERE effect.reflection_id = {alias}.reflection_id AND effect.role = 'affected' AND (effect.owner <> ? OR effect.namespace <> ?)) AND EXISTS (SELECT 1 FROM reflection_evidence re WHERE re.reflection_id = {alias}.reflection_id) AND NOT EXISTS (SELECT 1 FROM reflection_evidence re LEFT JOIN events e ON e.event_id = re.event_id WHERE re.reflection_id = {alias}.reflection_id AND (e.event_id IS NULL OR e.owner <> ? OR e.namespace <> ?)))"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::Connection;

    #[tokio::test]
    async fn legacy_backfill_does_not_guess_mixed_unknown_or_empty_origins() {
        let mut connection = SqliteConnection::connect("sqlite::memory:").await.unwrap();
        sqlx::raw_sql("PRAGMA foreign_keys = ON;
            CREATE TABLE events(event_id TEXT PRIMARY KEY, owner TEXT, namespace TEXT);
            CREATE TABLE claims(claim_id TEXT PRIMARY KEY, owner TEXT, namespace TEXT);
            CREATE TABLE reflections(reflection_id TEXT PRIMARY KEY, superseded_claim_id TEXT, replacement_claim_id TEXT, supporting_evidence_event_ids TEXT NOT NULL, requested_identity_update TEXT, requested_commitment_updates TEXT, scope_status TEXT NOT NULL DEFAULT 'unknown', evidence_normalized INTEGER NOT NULL DEFAULT 0);
            INSERT INTO events VALUES ('a','world','project/a'),('b','world','project/b'),('unknown','unknown','project/a');
            INSERT INTO reflections(reflection_id,supporting_evidence_event_ids) VALUES
                ('safe','[\"a\",\"a\"]'),('mixed','[\"a\",\"b\"]'),('empty','[]'),('unknown','[\"unknown\"]'),('orphan','[\"missing\"]'),('malformed','broken-json');
            INSERT INTO reflections(reflection_id,supporting_evidence_event_ids,requested_identity_update) VALUES ('project-global','[\"a\"]','{\"canonical_claims\":[\"global\"]}');")
            .execute(&mut connection).await.unwrap();
        sqlx::raw_sql(REFLECTION_RELATIONS_SCHEMA_SQL)
            .execute(&mut connection)
            .await
            .unwrap();
        backfill_legacy(&mut connection).await.unwrap();
        let statuses: Vec<(String,String,i64)> = sqlx::query_as("SELECT reflection_id,scope_status,evidence_normalized FROM reflections ORDER BY reflection_id").fetch_all(&mut connection).await.unwrap();
        for (id, status, normalized) in statuses {
            assert_eq!(
                status,
                if ["safe", "project-global"].contains(&id.as_str()) {
                    "legacy_unambiguous"
                } else {
                    "unknown"
                },
                "{id}"
            );
            assert_eq!(
                normalized,
                if ["orphan", "malformed"].contains(&id.as_str()) {
                    0
                } else {
                    1
                },
                "{id}"
            );
        }
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM reflection_evidence WHERE reflection_id = 'safe'"
            )
            .fetch_one(&mut connection)
            .await
            .unwrap(),
            1
        );
        let predicate = targetless_visibility_sql("r");
        for (owner, namespace, expected) in [
            ("world", "project/a", vec!["safe"]),
            ("world", "project/b", vec![]),
            ("self", "self", vec![]),
        ] {
            let ids: Vec<String> = sqlx::query_scalar(&format!(
                "SELECT reflection_id FROM reflections r WHERE {predicate} ORDER BY reflection_id"
            ))
            .bind(owner)
            .bind(namespace)
            .bind(owner)
            .bind(namespace)
            .bind(owner)
            .bind(namespace)
            .fetch_all(&mut connection)
            .await
            .unwrap();
            assert_eq!(ids, expected);
        }
        // Relations constrain their source as durable ledger data.
        assert!(
            sqlx::query("DELETE FROM events WHERE event_id = 'a'")
                .execute(&mut connection)
                .await
                .is_err()
        );
    }
}
