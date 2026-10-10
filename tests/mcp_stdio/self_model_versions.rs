//! Local subprocess coverage, not external-client validation.
use super::test_support;
use serde_json::json;
use sqlx::SqlitePool;

#[tokio::test]
async fn version_reader_requires_metadata_opt_in_and_exposes_only_scoped_written_patches() {
    let (mut client, database_url, directory) = test_support::spawn_stdio_client_with_database()
        .await
        .unwrap();
    let tools = client.list_all_tools().await.unwrap();
    let schema = &tools
        .iter()
        .find(|tool| tool.name == "get_self_model_versions")
        .unwrap()
        .input_schema;
    assert!(
        schema["properties"]
            .get("allow_global_version_metadata")
            .is_some()
    );
    assert!(schema["properties"].get("before_version").is_some());
    let reflection_schema = &tools
        .iter()
        .find(|tool| tool.name == "run_reflection")
        .unwrap()
        .input_schema;
    for field in [
        "expected_self_model_version",
        "self_model_rollback",
        "request_id",
    ] {
        assert!(
            reflection_schema["properties"].get(field).is_some(),
            "{reflection_schema:?}"
        );
        assert!(
            !reflection_schema["required"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == field)
        );
    }
    let baseline = client
        .call_tool(
            "get_self_model_versions",
            json!({
                "namespace":"project/version-stdio", "allow_global_version_metadata":true
            }),
        )
        .await
        .unwrap();
    assert_eq!(
        baseline["result"]["structuredContent"]["current_version"],
        0
    );
    assert_eq!(
        baseline["result"]["structuredContent"]["records"],
        json!([])
    );
    for arguments in [
        json!({"namespace":"project/version-stdio"}),
        json!({"namespace":"project/version-stdio", "allow_global_version_metadata":false}),
        json!({"namespace":"project/version-stdio", "allow_global_version_metadata":true, "limit":101}),
    ] {
        let denied = client
            .call_tool("get_self_model_versions", arguments)
            .await
            .unwrap();
        assert_eq!(denied["error"]["code"], -32602, "{denied}");
    }
    let ingest = client.call_tool("ingest_interaction", json!({
        "event":{"owner":"World", "namespace":"project/version-stdio", "kind":"Observation", "summary":"version evidence"},
        "claim_drafts":[{"owner":"World", "namespace":"project/version-stdio", "subject":"role", "predicate":"is", "object":"prior", "mode":"Observed"}]
    })).await.unwrap();
    assert!(ingest.get("error").is_none(), "{ingest}");
    let event = ingest["result"]["structuredContent"]["event_id"]
        .as_str()
        .unwrap();
    let patch = json!({
        "request_id":"stdio-version-1", "expected_self_model_version":0,
        "reflection":{"summary":"update the explicit scoped self-model source"},
        "origin_namespace":"project/version-stdio", "supersede_claim_id":format!("{event}:claim:0"),
        "replacement_evidence_event_ids":[event], "identity_update":{"canonical_claims":["visible","visible","ordered"]}
    });
    let write = client
        .call_tool("run_reflection", patch.clone())
        .await
        .unwrap();
    assert!(write.get("error").is_none(), "{write}");
    assert_eq!(
        write["result"]["structuredContent"]["self_model_version"],
        1
    );
    let replay = client
        .call_tool("run_reflection", patch.clone())
        .await
        .unwrap();
    assert_eq!(write["result"], replay["result"]);
    let read = client
        .call_tool(
            "get_self_model_versions",
            json!({
                "namespace":"project/version-stdio", "allow_global_version_metadata":true, "limit":1
            }),
        )
        .await
        .unwrap();
    assert!(read.get("error").is_none(), "{read}");
    let result = &read["result"]["structuredContent"];
    assert_eq!(result["current_version"], 1);
    assert_eq!(result["records"].as_array().unwrap().len(), 1);
    assert_eq!(
        result["records"][0]["identity_update"]["patch"]["canonical_claims"],
        json!(["visible", "visible", "ordered"])
    );
    assert!(
        result["records"][0]["identity_update"]
            .get("previous_patch")
            .is_none()
    );
    assert!(
        result["records"][0]["identity_update"]
            .get("previous_values_redacted")
            .is_some()
    );
    assert!(result["records"][0].get("commitment_updates").is_none());
    assert!(result["records"][0].get("identity").is_none());
    let other = client
        .call_tool(
            "get_self_model_versions",
            json!({
                "namespace":"project/other", "allow_global_version_metadata":true
            }),
        )
        .await
        .unwrap();
    assert_eq!(other["result"]["structuredContent"]["current_version"], 1);
    assert_eq!(other["result"]["structuredContent"]["records"], json!([]));
    let foreign_ingest = client.call_tool("ingest_interaction", json!({
        "event":{"owner":"World", "namespace":"project/version-foreign", "kind":"Observation", "summary":"foreign version evidence"},
        "claim_drafts":[{"owner":"World", "namespace":"project/version-foreign", "subject":"role", "predicate":"is", "object":"prior", "mode":"Observed"}]
    })).await.unwrap();
    let foreign_event = foreign_ingest["result"]["structuredContent"]["event_id"]
        .as_str()
        .unwrap();
    let foreign_write = client.call_tool("run_reflection", json!({
        "request_id":"stdio-foreign", "expected_self_model_version":1,
        "reflection":{"summary":"foreign source"}, "origin_namespace":"project/version-foreign",
        "supersede_claim_id":format!("{foreign_event}:claim:0"), "replacement_evidence_event_ids":[foreign_event],
        "identity_update":{"canonical_claims":["foreign-private-value"]}
    })).await.unwrap();
    assert_eq!(
        foreign_write["result"]["structuredContent"]["self_model_version"], 2,
        "{foreign_write}"
    );
    let mut next_patch = patch.clone();
    next_patch["request_id"] = "stdio-version-3".into();
    next_patch["expected_self_model_version"] = 2.into();
    next_patch["identity_update"]["canonical_claims"] = json!(["own-current"]);
    let third = client
        .call_tool("run_reflection", next_patch.clone())
        .await
        .unwrap();
    assert_eq!(
        third["result"]["structuredContent"]["self_model_version"], 3,
        "{third}"
    );
    let redacted = client
        .call_tool(
            "get_self_model_versions",
            json!({
                "namespace":"project/version-stdio", "allow_global_version_metadata":true, "limit":1
            }),
        )
        .await
        .unwrap();
    let third_record = &redacted["result"]["structuredContent"]["records"][0];
    assert_eq!(third_record["version"], 3);
    assert!(
        third_record["identity_update"]
            .get("previous_patch")
            .is_none()
    );
    assert!(
        third_record["identity_update"]
            .get("previous_values_redacted")
            .is_some()
    );
    assert!(!redacted.to_string().contains("foreign-private-value"));
    assert_eq!(
        redacted["result"]["structuredContent"]["next_before_version"],
        3
    );
    let rollback = json!({
        "request_id":"stdio-rollback", "expected_self_model_version":3,
        "reflection":{"summary":"restore first scoped identity"}, "origin_namespace":"project/version-stdio",
        "supersede_claim_id":format!("{event}:claim:0"), "replacement_evidence_event_ids":[event],
        "self_model_rollback":{"target_version":1,"components":["identity"],"confirm":true}
    });
    let restored = client
        .call_tool("run_reflection", rollback.clone())
        .await
        .unwrap();
    assert_eq!(
        restored["result"]["structuredContent"]["self_model_version"], 4,
        "{restored}"
    );
    let again = client
        .call_tool("run_reflection", rollback.clone())
        .await
        .unwrap();
    assert_eq!(restored["result"], again["result"]);
    let mut stale = next_patch;
    stale["request_id"] = "stdio-stale".into();
    stale["expected_self_model_version"] = 3.into();
    let denied = client.call_tool("run_reflection", stale).await.unwrap();
    assert_eq!(denied["error"]["code"], -32602, "{denied}");
    let mut targetless = rollback.clone();
    targetless
        .as_object_mut()
        .unwrap()
        .remove("supersede_claim_id");
    targetless["request_id"] = "stdio-targetless".into();
    let denied = client
        .call_tool("run_reflection", targetless)
        .await
        .unwrap();
    assert_eq!(denied["error"]["code"], -32602, "{denied}");
    drop(client);
    let config = r#"transport = "stdio"
database_url = "__DATABASE_URL__"
[model]
provider = "mock"
"#;
    let mut client = test_support::spawn_stdio_client_for_existing_database_with_config(
        config,
        &database_url,
        directory.path(),
    )
    .unwrap();
    let after_restart = client
        .call_tool(
            "get_self_model_versions",
            json!({
                "namespace":"project/version-stdio", "allow_global_version_metadata":true, "limit":1
            }),
        )
        .await
        .unwrap();
    assert_eq!(
        after_restart["result"]["structuredContent"]["current_version"], 4,
        "{after_restart}"
    );
    assert_eq!(
        after_restart["result"]["structuredContent"]["records"][0]["kind"],
        "rollback"
    );
    assert_eq!(
        after_restart["result"]["structuredContent"]["records"][0]["identity_update"]["patch"]["canonical_claims"],
        json!(["visible", "visible", "ordered"])
    );
    let replay_after_restart = client.call_tool("run_reflection", rollback).await.unwrap();
    assert_eq!(restored["result"], replay_after_restart["result"]);
    let pool = SqlitePool::connect(&database_url).await.unwrap();
    sqlx::query("UPDATE identity_claims SET claim = 'do-not-leak-drift'")
        .execute(&pool)
        .await
        .unwrap();
    let drift = client
        .call_tool(
            "get_self_model_versions",
            json!({
                "namespace":"project/version-stdio", "allow_global_version_metadata":true
            }),
        )
        .await
        .unwrap();
    assert_eq!(drift["error"]["code"], -32603, "{drift}");
    assert!(!drift.to_string().contains("do-not-leak-drift"));
}
