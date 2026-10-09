//! Legacy migration helpers retained for the explicit database lifecycle.

use sqlx::{Row, Sqlite};

use crate::error::AppError;

use super::rows::map_sqlite;

use crate::adapters::sqlite::schema::{
    OWNER_NAMESPACE_SCOPE_CONSTRAINT_NAME, claims_table_sql, events_table_sql,
    legacy_namespace_backfill_expression,
};

pub(in crate::adapters::sqlite) async fn seed_baseline_commitments<'e, E>(
    executor: E,
) -> Result<(), AppError>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    map_sqlite(
        sqlx::query(
            r#"
            INSERT OR IGNORE INTO commitments (description, owner)
            VALUES (?, ?)
            "#,
        )
        .bind("forbid:write_identity_core_directly")
        .bind("self")
        .execute(executor)
        .await,
    )?;

    Ok(())
}

pub(in crate::adapters::sqlite) async fn ensure_claims_namespace_column(
    connection: &mut sqlx::SqliteConnection,
) -> Result<(), AppError> {
    let namespace_column_exists = map_sqlite(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM pragma_table_info('claims') WHERE name = 'namespace'",
        )
        .fetch_one(&mut *connection)
        .await,
    )? > 0;
    let namespace_is_not_null = if namespace_column_exists {
        map_sqlite(
            sqlx::query_scalar::<_, i64>(
                r#"SELECT "notnull" FROM pragma_table_info('claims') WHERE name = 'namespace'"#,
            )
            .fetch_one(&mut *connection)
            .await,
        )? == 1
    } else {
        false
    };
    let claims_has_scope_check = if namespace_column_exists {
        map_sqlite(
            sqlx::query_scalar::<_, String>(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'claims'",
            )
            .fetch_one(&mut *connection)
            .await,
        )?
        .contains(OWNER_NAMESPACE_SCOPE_CONSTRAINT_NAME)
    } else {
        false
    };

    if !namespace_column_exists || !namespace_is_not_null || !claims_has_scope_check {
        rebuild_claims_table_with_namespace(connection, namespace_column_exists).await?;
    }

    Ok(())
}

pub(in crate::adapters::sqlite) async fn ensure_events_namespace_column(
    connection: &mut sqlx::SqliteConnection,
) -> Result<(), AppError> {
    let namespace_column_exists = map_sqlite(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM pragma_table_info('events') WHERE name = 'namespace'",
        )
        .fetch_one(&mut *connection)
        .await,
    )? > 0;
    let namespace_is_not_null = if namespace_column_exists {
        map_sqlite(
            sqlx::query_scalar::<_, i64>(
                r#"SELECT "notnull" FROM pragma_table_info('events') WHERE name = 'namespace'"#,
            )
            .fetch_one(&mut *connection)
            .await,
        )? == 1
    } else {
        false
    };
    let events_has_scope_check = if namespace_column_exists {
        map_sqlite(
            sqlx::query_scalar::<_, String>(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'events'",
            )
            .fetch_one(&mut *connection)
            .await,
        )?
        .contains(OWNER_NAMESPACE_SCOPE_CONSTRAINT_NAME)
    } else {
        false
    };

    if !namespace_column_exists || !namespace_is_not_null || !events_has_scope_check {
        rebuild_events_table_with_namespace(connection, namespace_column_exists).await?;
    }

    Ok(())
}

pub(in crate::adapters::sqlite) async fn ensure_reflection_audit_columns(
    connection: &mut sqlx::SqliteConnection,
) -> Result<(), AppError> {
    let columns = map_sqlite(
        sqlx::query("SELECT name FROM pragma_table_info('reflections')")
            .fetch_all(&mut *connection)
            .await,
    )?
    .into_iter()
    .map(|row| row.get::<String, _>("name"))
    .collect::<Vec<_>>();

    if !columns.contains(&"supporting_evidence_event_ids".to_string()) {
        map_sqlite(
            sqlx::query(
                "ALTER TABLE reflections ADD COLUMN supporting_evidence_event_ids TEXT NOT NULL DEFAULT '[]'",
            )
            .execute(&mut *connection)
            .await,
        )?;
    }

    if !columns.contains(&"requested_identity_update".to_string()) {
        map_sqlite(
            sqlx::query("ALTER TABLE reflections ADD COLUMN requested_identity_update TEXT")
                .execute(&mut *connection)
                .await,
        )?;
    }

    if !columns.contains(&"requested_commitment_updates".to_string()) {
        map_sqlite(
            sqlx::query("ALTER TABLE reflections ADD COLUMN requested_commitment_updates TEXT")
                .execute(&mut *connection)
                .await,
        )?;
    }

    Ok(())
}

async fn rebuild_claims_table_with_namespace(
    connection: &mut sqlx::SqliteConnection,
    legacy_table_has_namespace: bool,
) -> Result<(), AppError> {
    let create_claims_table_sql = claims_table_sql(false);
    let namespace_expression = legacy_namespace_backfill_expression(legacy_table_has_namespace);
    let copy_sql = format!(
        r#"
        INSERT INTO claims (claim_id, owner, namespace, subject, predicate, object, mode, status)
        SELECT claim_id, owner, {namespace_expression}, subject, predicate, object, mode, status
        FROM claims_legacy
        "#
    );

    map_sqlite(
        sqlx::query("ALTER TABLE claims RENAME TO claims_legacy")
            .execute(&mut *connection)
            .await,
    )?;
    map_sqlite(
        sqlx::query(&create_claims_table_sql)
            .execute(&mut *connection)
            .await,
    )?;
    map_sqlite(sqlx::query(&copy_sql).execute(&mut *connection).await)?;
    map_sqlite(
        sqlx::query("DROP TABLE claims_legacy")
            .execute(&mut *connection)
            .await,
    )?;
    Ok(())
}

async fn rebuild_events_table_with_namespace(
    connection: &mut sqlx::SqliteConnection,
    legacy_table_has_namespace: bool,
) -> Result<(), AppError> {
    let create_events_table_sql = events_table_sql(false);
    let namespace_expression = legacy_namespace_backfill_expression(legacy_table_has_namespace);
    let copy_sql = format!(
        r#"
        INSERT INTO events (event_id, recorded_at, owner, namespace, kind, summary)
        SELECT event_id, recorded_at, owner, {namespace_expression}, kind, summary
        FROM events_legacy
        "#
    );

    map_sqlite(
        sqlx::query("ALTER TABLE events RENAME TO events_legacy")
            .execute(&mut *connection)
            .await,
    )?;
    map_sqlite(
        sqlx::query(&create_events_table_sql)
            .execute(&mut *connection)
            .await,
    )?;
    map_sqlite(sqlx::query(&copy_sql).execute(&mut *connection).await)?;
    map_sqlite(
        sqlx::query("DROP TABLE events_legacy")
            .execute(&mut *connection)
            .await,
    )?;
    Ok(())
}
