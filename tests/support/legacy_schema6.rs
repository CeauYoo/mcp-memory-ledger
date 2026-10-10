#[path = "legacy_schema7.rs"]
mod legacy_schema7;

// Test-only removal of schema-v6 additions when constructing v5 fixtures.
pub async fn remove_v6_objects(connection: &mut sqlx::SqliteConnection) {
    legacy_schema7::remove_v7_objects(connection).await;
    for table in ["events", "claims", "reflections"] {
        for suffix in ["ai", "au"] {
            sqlx::query(&format!(
                "DROP TRIGGER IF EXISTS recorded_time_{table}_{suffix}"
            ))
            .execute(&mut *connection)
            .await
            .unwrap();
        }
    }
    for index in [
        "idx_events_scope_recorded",
        "idx_claims_scope_recorded",
        "idx_reflections_recorded",
    ] {
        sqlx::query(&format!("DROP INDEX IF EXISTS {index}"))
            .execute(&mut *connection)
            .await
            .unwrap();
    }
    for table in ["reflection_evidence", "reflection_scopes"] {
        sqlx::query(&format!("DROP TABLE IF EXISTS {table}"))
            .execute(&mut *connection)
            .await
            .unwrap();
    }
    for (table, columns) in [
        (
            "events",
            vec![
                "observed_at",
                "recorded_at_seconds",
                "recorded_at_nanos",
                "recorded_at_sort_key",
            ],
        ),
        (
            "claims",
            vec![
                "recorded_at",
                "observed_at",
                "recorded_at_seconds",
                "recorded_at_nanos",
                "recorded_at_sort_key",
            ],
        ),
        (
            "reflections",
            vec![
                "recorded_at_seconds",
                "recorded_at_nanos",
                "recorded_at_sort_key",
                "scope_status",
                "evidence_normalized",
            ],
        ),
    ] {
        for column in columns {
            let exists: i64 = sqlx::query_scalar(&format!(
                "SELECT count(*) FROM pragma_table_info('{table}') WHERE name=?"
            ))
            .bind(column)
            .fetch_one(&mut *connection)
            .await
            .unwrap();
            if exists != 0 {
                sqlx::query(&format!("ALTER TABLE {table} DROP COLUMN {column}"))
                    .execute(&mut *connection)
                    .await
                    .unwrap();
            }
        }
    }
}
