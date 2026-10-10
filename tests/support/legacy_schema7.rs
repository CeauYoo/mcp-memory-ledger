// Test-only removal of v7 objects while constructing an actual older-schema fixture.
pub async fn remove_v7_objects(connection: &mut sqlx::SqliteConnection) {
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
