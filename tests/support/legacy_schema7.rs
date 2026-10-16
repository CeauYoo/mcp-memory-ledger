// Test-only removal of v7 objects while constructing an actual older-schema fixture.
pub async fn remove_v7_objects(connection: &mut sqlx::SqliteConnection) {
    // A fixture downgraded from current schema must remove all later additions too.
    for table in [
        "task_checkpoint_sources",
        "task_checkpoints",
        "task_requirements",
        "task_revisions",
        "task_heads",
    ] {
        sqlx::query(&format!("DROP TABLE IF EXISTS {table}"))
            .execute(&mut *connection)
            .await
            .unwrap();
    }
    for trigger in [
        "self_model_versions_append",
        "self_model_versions_no_update",
        "self_model_versions_no_delete",
    ] {
        sqlx::query(&format!("DROP TRIGGER IF EXISTS {trigger}"))
            .execute(&mut *connection)
            .await
            .unwrap();
    }
    sqlx::query("DROP TABLE IF EXISTS self_model_versions")
        .execute(&mut *connection)
        .await
        .unwrap();
}
