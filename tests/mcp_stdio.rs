use std::{
    io::{self, BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::Arc,
};

use agent_llm_mm::support::config::{CONFIG_PATH_ENV_VAR, DATABASE_URL_ENV_VAR};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{Row, sqlite::SqlitePool};
use tempfile::TempDir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
};

#[path = "mcp_stdio/caller_budget.rs"]
mod caller_budget;

#[tokio::test]
async fn server_exposes_expected_tools_over_stdio() {
    let mut client = test_support::spawn_stdio_client().await.unwrap();
    let tools = client.list_all_tools().await.unwrap();
    let mut names = tools
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect::<Vec<_>>();
    names.sort();

    assert_eq!(
        names,
        vec![
            "build_self_snapshot".to_string(),
            "build_task_context".to_string(),
            "commit_feedback_candidate".to_string(),
            "decide_with_snapshot".to_string(),
            "export_memory".to_string(),
            "get_episode_detail".to_string(),
            "get_evidence_relation".to_string(),
            "get_experience_candidate".to_string(),
            "get_feedback_candidate".to_string(),
            "get_feedback_target_version".to_string(),
            "get_memory".to_string(),
            "get_reflection_history".to_string(),
            "get_self_model_history".to_string(),
            "ingest_interaction".to_string(),
            "inspect_retrieval_index".to_string(),
            "list_episode_details".to_string(),
            "list_experience_candidates".to_string(),
            "propose_experience_candidate".to_string(),
            "propose_feedback_candidate".to_string(),
            "rebuild_retrieval_index".to_string(),
            "recall_experience_candidates".to_string(),
            "recall_memory".to_string(),
            "record_episode".to_string(),
            "reject_feedback_candidate".to_string(),
            "revise_experience_candidate".to_string(),
            "rollback_experience_candidate".to_string(),
            "run_reflection".to_string(),
            "search_memory".to_string(),
            "set_experience_candidate_status".to_string(),
            "supersede_memory".to_string(),
            "validate_feedback_candidate".to_string(),
        ]
    );
}

#[tokio::test]
async fn server_preserves_tool_input_schemas_over_stdio() {
    let mut client = test_support::spawn_stdio_client().await.unwrap();
    let tools = client.list_all_tools().await.unwrap();
    let ingest_schema = tools
        .iter()
        .find(|tool| tool.name == "ingest_interaction")
        .map(|tool| &tool.input_schema)
        .expect("ingest_interaction tool schema");

    let required_fields = ingest_schema
        .get("required")
        .and_then(Value::as_array)
        .expect("ingest schema should expose required fields")
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();

    assert!(
        required_fields.contains(&"event"),
        "ingest schema should preserve the event parameter: {ingest_schema:?}"
    );
    assert!(
        required_fields.contains(&"claim_drafts"),
        "ingest schema should preserve the claim_drafts parameter: {ingest_schema:?}"
    );
    assert!(
        ingest_schema
            .get("properties")
            .and_then(|properties| properties.get("event"))
            .is_some(),
        "ingest schema should include event property details: {ingest_schema:?}"
    );

    let snapshot_schema = tools
        .iter()
        .find(|tool| tool.name == "build_self_snapshot")
        .map(|tool| &tool.input_schema)
        .expect("build_self_snapshot tool schema");
    let snapshot_properties = snapshot_schema
        .get("properties")
        .and_then(Value::as_object)
        .expect("snapshot schema should expose properties");
    assert!(snapshot_properties.contains_key("recorded_after"));
    assert!(snapshot_properties.contains_key("recorded_before"));
    let snapshot_required = snapshot_schema
        .get("required")
        .and_then(Value::as_array)
        .expect("snapshot schema should expose required fields");
    assert!(
        !snapshot_required
            .iter()
            .any(|field| { matches!(field.as_str(), Some("recorded_after" | "recorded_before")) })
    );

    let search_schema = tools
        .iter()
        .find(|tool| tool.name == "search_memory")
        .map(|tool| &tool.input_schema)
        .expect("search_memory tool schema");
    let search_properties = search_schema
        .get("properties")
        .and_then(Value::as_object)
        .expect("search_memory schema should expose properties");
    for field in [
        "namespace",
        "record_type",
        "event_reference",
        "kind",
        "recorded_after",
        "recorded_before",
        "claim_reference",
        "claim_status",
        "mode",
        "episode_reference",
        "reflection_reference",
        "record_types",
        "limit",
    ] {
        assert!(
            search_properties.contains_key(field),
            "search_memory schema missing {field}: {search_schema:?}"
        );
    }
    let search_required = search_schema
        .get("required")
        .and_then(Value::as_array)
        .expect("search_memory schema should expose required fields");
    assert_eq!(search_required, &[json!("namespace")]);
    assert_eq!(
        search_schema["definitions"]["SearchMemoryRecordTypeDto"]["enum"],
        json!(["Event", "Claim", "Episode", "Reflection"]),
        "search_memory record_type schema should include exactly Event, Claim, Episode, and Reflection"
    );

    let get_schema = tools
        .iter()
        .find(|tool| tool.name == "get_memory")
        .map(|tool| &tool.input_schema)
        .expect("get_memory tool schema");
    let get_required = get_schema
        .get("required")
        .and_then(Value::as_array)
        .expect("get_memory schema should expose required fields");
    assert_eq!(get_required, &[json!("namespace"), json!("id")]);
    assert!(
        get_schema["properties"].get("record_type").is_some(),
        "get_memory schema should expose optional record_type: {get_schema:?}"
    );
    assert_eq!(
        get_schema["definitions"]["MemoryRecordTypeDto"]["enum"],
        json!(["Event", "Claim", "Episode", "Reflection"]),
        "get_memory record_type schema must remain exactly Event, Claim, Episode, and Reflection"
    );

    let history_schema = tools
        .iter()
        .find(|tool| tool.name == "get_reflection_history")
        .map(|tool| &tool.input_schema)
        .expect("get_reflection_history tool schema");
    let history_required = history_schema
        .get("required")
        .and_then(Value::as_array)
        .expect("get_reflection_history schema should expose required fields");
    assert_eq!(
        history_required,
        &[json!("namespace"), json!("claim_reference")]
    );
    assert!(history_schema["properties"].get("limit").is_some());

    let self_model_schema = tools
        .iter()
        .find(|tool| tool.name == "get_self_model_history")
        .map(|tool| &tool.input_schema)
        .expect("get_self_model_history tool schema");
    let self_model_required = self_model_schema
        .get("required")
        .and_then(Value::as_array)
        .expect("get_self_model_history schema should expose required fields");
    assert_eq!(
        self_model_required,
        &[json!("namespace"), json!("history_type")]
    );
    assert_eq!(
        self_model_schema["definitions"]["SelfModelHistoryTypeDto"]["enum"],
        json!(["Identity", "Commitment"])
    );

    let supersede_schema = tools
        .iter()
        .find(|tool| tool.name == "supersede_memory")
        .map(|tool| &tool.input_schema)
        .expect("supersede_memory tool schema");
    let supersede_required = supersede_schema
        .get("required")
        .and_then(Value::as_array)
        .expect("supersede_memory schema should expose required fields");
    assert_eq!(
        supersede_required,
        &[
            json!("namespace"),
            json!("claim_reference"),
            json!("replacement_claim"),
            json!("replacement_evidence_event_ids"),
            json!("summary")
        ]
    );

    let relation_schema = tools
        .iter()
        .find(|tool| tool.name == "get_evidence_relation")
        .map(|tool| &tool.input_schema)
        .expect("get_evidence_relation tool schema");
    let relation_required = relation_schema
        .get("required")
        .and_then(Value::as_array)
        .expect("get_evidence_relation schema should expose required fields");
    assert_eq!(
        relation_required,
        &[json!("namespace"), json!("trigger_window_event_ids")]
    );
    let relation_properties = relation_schema
        .get("properties")
        .and_then(Value::as_object)
        .expect("get_evidence_relation schema should expose properties");
    for field in [
        "namespace",
        "trigger_window_event_ids",
        "selected_evidence_event_ids",
        "selection_basis",
    ] {
        assert!(
            relation_properties.contains_key(field),
            "get_evidence_relation schema missing {field}: {relation_schema:?}"
        );
    }
}

#[tokio::test]
async fn get_memory_returns_one_scoped_event_or_null_without_widening() {
    let mut client = test_support::spawn_stdio_client().await.unwrap();
    let ingest = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "World",
                    "namespace": "project/get-a",
                    "kind": "Action",
                    "summary": "lookup this exact event"
                },
                "claim_drafts": [],
                "episode_reference": "episode:get-a"
            }),
        )
        .await
        .unwrap();
    let event_id = ingest["result"]["structuredContent"]["event_id"]
        .as_str()
        .unwrap();

    let found = client
        .call_tool(
            "get_memory",
            json!({
                "namespace": "project/get-a",
                "id": format!("event:{event_id}")
            }),
        )
        .await
        .unwrap();
    let result = &found["result"]["structuredContent"];
    assert_eq!(result["owner"], "World");
    assert_eq!(result["namespace"], "project/get-a");
    assert_eq!(result["record"]["id"], format!("event:{event_id}"));
    assert_eq!(result["record"]["record_type"], "event");
    assert_eq!(result["record"]["summary"], "lookup this exact event");
    assert_eq!(
        result["record"]["provenance"]["episode_references"],
        json!(["episode:get-a"])
    );

    let raw_event_id = client
        .call_tool(
            "get_memory",
            json!({
                "namespace": "project/get-a",
                "id": event_id
            }),
        )
        .await
        .unwrap();
    assert_eq!(
        raw_event_id["result"]["structuredContent"]["record"]["id"],
        format!("event:{event_id}")
    );

    let wrong_scope = client
        .call_tool(
            "get_memory",
            json!({
                "namespace": "project/get-b",
                "id": format!("event:{event_id}")
            }),
        )
        .await
        .unwrap();
    assert!(wrong_scope["result"]["structuredContent"]["record"].is_null());

    let invalid = client
        .call_tool("get_memory", json!({"id": format!("event:{event_id}")}))
        .await
        .unwrap();
    assert_eq!(invalid["error"]["code"], -32602);

    let invalid_claim = client
        .call_tool(
            "get_memory",
            json!({
                "namespace": "project/get-a",
                "id": "claim:",
                "record_type": "Claim"
            }),
        )
        .await
        .unwrap();
    assert_eq!(invalid_claim["error"]["code"], -32602);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ingest_interaction_returns_success_even_when_best_effort_auto_reflection_fails() {
    let stub = test_support::StubServer::spawn(
        200,
        json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "not valid self revision json"
                }
            }]
        }),
    )
    .await;
    let config = format!(
        r#"
transport = "stdio"
database_url = "__DATABASE_URL__"

[model]
provider = "openai-compatible"

[model.openai_compatible]
base_url = "{}"
api_key = "example-test-key"
model = "gpt-4o-mini"
timeout_ms = 30000
"#,
        stub.base_url()
    );
    let (mut client, database_url, _database_dir) =
        test_support::spawn_stdio_client_with_config_and_database(config)
            .await
            .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    for (episode_reference, summary, trigger_hints) in [
        (
            "episode:auto-reflect-nonfatal-0",
            "first rollback after violating a hard commitment",
            json!([]),
        ),
        (
            "episode:auto-reflect-nonfatal-1",
            "rollback after violating a hard commitment",
            json!(["failure", "rollback"]),
        ),
    ] {
        let response = client
            .call_tool(
                "ingest_interaction",
                json!({
                    "event": {
                        "owner": "Self_",
                        "kind": "Action",
                        "summary": summary
                    },
                    "claim_drafts": [],
                    "episode_reference": episode_reference,
                    "trigger_hints": trigger_hints
                }),
            )
            .await
            .unwrap();

        let event_id = response
            .get("result")
            .and_then(|value| value.get("structuredContent"))
            .and_then(|value| value.get("event_id"))
            .and_then(Value::as_str);
        assert!(
            event_id.is_some(),
            "ingest should still succeed when best-effort auto-reflection fails: {response:?}"
        );
        assert!(
            response.get("error").is_none(),
            "post-ingest auto-reflection failure must not surface as MCP error: {response:?}"
        );
    }

    let pool = SqlitePool::connect(&database_url).await.unwrap();
    let event_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM events")
        .fetch_one(&pool)
        .await
        .unwrap();
    let reflection_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM reflections")
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(event_count, 2);
    assert_eq!(reflection_count, 0);
    assert_eq!(stub.request_count().await, 1);

    let trigger_row = sqlx::query(
        "SELECT operation_kind, status FROM operation_log \
         WHERE entrypoint = 'ingest_interaction' AND operation_kind = 'trigger' \
         ORDER BY occurred_at DESC, operation_id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("failed best-effort auto-reflection should append a durable trigger operation log row");

    assert_eq!(trigger_row.get::<String, _>("operation_kind"), "trigger");
    assert_eq!(trigger_row.get::<String, _>("status"), "failed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rejected_auto_reflection_trigger_log_does_not_persist_raw_model_rationale() {
    let raw_secret_rationale = "No revision because sk-secret-token-password-should-not-persist";
    let stub = test_support::StubServer::spawn(
        200,
        json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": format!(r#"{{"should_reflect":false,"rationale":"{}"}}"#, raw_secret_rationale)
                }
            }]
        }),
    )
    .await;
    let config = format!(
        r#"
transport = "stdio"
database_url = "__DATABASE_URL__"

[model]
provider = "openai-compatible"

[model.openai_compatible]
base_url = "{}"
api_key = "example-test-key"
model = "gpt-4o-mini"
timeout_ms = 30000
"#,
        stub.base_url()
    );
    let (mut client, database_url, _database_dir) =
        test_support::spawn_stdio_client_with_config_and_database(config)
            .await
            .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    for (episode_reference, summary, trigger_hints) in [
        (
            "episode:auto-reflect-rejected-diagnostic-0",
            "first rollback after violating a hard commitment",
            json!([]),
        ),
        (
            "episode:auto-reflect-rejected-diagnostic-1",
            "rollback after violating a hard commitment",
            json!(["failure", "rollback"]),
        ),
    ] {
        let response = client
            .call_tool(
                "ingest_interaction",
                json!({
                    "event": {
                        "owner": "Self_",
                        "kind": "Action",
                        "summary": summary
                    },
                    "claim_drafts": [],
                    "episode_reference": episode_reference,
                    "trigger_hints": trigger_hints
                }),
            )
            .await
            .unwrap();

        assert!(
            response.get("error").is_none(),
            "rejected auto-reflection must preserve main ingest success semantics: {response:?}"
        );
    }

    let pool = SqlitePool::connect(&database_url).await.unwrap();
    let row = sqlx::query(
        "SELECT operation_kind, status, diagnostic_summary_json FROM operation_log \
         WHERE entrypoint = 'ingest_interaction' AND operation_kind = 'trigger' \
         ORDER BY occurred_at DESC, operation_id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("rejected auto-reflection should append a trigger operation log entry");

    assert_eq!(row.get::<String, _>("operation_kind"), "trigger");
    assert_eq!(row.get::<String, _>("status"), "rejected");
    let diagnostic = row
        .get::<Option<String>, _>("diagnostic_summary_json")
        .expect("rejected trigger should include bounded diagnostic summary");
    assert!(
        !diagnostic.contains(raw_secret_rationale),
        "trigger diagnostic must not persist raw model rationale: {diagnostic}"
    );
    assert!(
        !diagnostic.contains("sk-secret-token-password-should-not-persist"),
        "trigger diagnostic must not persist secret-like rationale text: {diagnostic}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dashboard_auto_reflection_event_omits_rejected_model_rationale() {
    let raw_secret_rationale =
        "No revision because sk-dashboard-secret-token-password-should-not-persist";
    let stub = test_support::StubServer::spawn(
        200,
        json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": format!(r#"{{"should_reflect":false,"rationale":"{}"}}"#, raw_secret_rationale)
                }
            }]
        }),
    )
    .await;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve port");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);
    let config = format!(
        r#"
transport = "stdio"
database_url = "__DATABASE_URL__"

[model]
provider = "openai-compatible"

[model.openai_compatible]
base_url = "{}"
api_key = "example-test-key"
model = "gpt-4o-mini"
timeout_ms = 30000

[dashboard]
enabled = true
host = "127.0.0.1"
port = {port}
event_capacity = 50
required = true
"#,
        stub.base_url()
    );
    let (mut client, _database_url, _database_dir) =
        test_support::spawn_stdio_client_with_config_and_database(config)
            .await
            .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    for (episode_reference, summary, trigger_hints) in [
        (
            "episode:dashboard-auto-reflect-rejected-diagnostic-0",
            "first rollback after violating a hard commitment",
            json!([]),
        ),
        (
            "episode:dashboard-auto-reflect-rejected-diagnostic-1",
            "rollback after violating a hard commitment",
            json!(["failure", "rollback"]),
        ),
    ] {
        let response = client
            .call_tool(
                "ingest_interaction",
                json!({
                    "event": {
                        "owner": "Self_",
                        "kind": "Action",
                        "summary": summary
                    },
                    "claim_drafts": [],
                    "episode_reference": episode_reference,
                    "trigger_hints": trigger_hints
                }),
            )
            .await
            .unwrap();

        assert!(
            response.get("error").is_none(),
            "rejected auto-reflection must preserve main ingest success semantics: {response:?}"
        );
    }

    let events: serde_json::Value =
        reqwest::get(format!("http://127.0.0.1:{port}/api/events?limit=20"))
            .await
            .expect("dashboard events response")
            .json()
            .await
            .expect("dashboard events json");
    let reflection_event = events
        .as_array()
        .expect("events array")
        .iter()
        .find(|event| {
            event.get("operation").and_then(Value::as_str) == Some("ingest_interaction:failure")
                && event.get("status").and_then(Value::as_str) == Some("rejected")
        })
        .expect("dashboard should record the rejected auto-reflection event");

    let summary = reflection_event
        .get("summary")
        .and_then(Value::as_str)
        .expect("dashboard event should include summary");
    assert!(
        !summary.contains(raw_secret_rationale),
        "dashboard summary must not expose raw model rationale: {reflection_event:?}"
    );
    assert!(
        !summary.contains("sk-dashboard-secret-token-password-should-not-persist"),
        "dashboard summary must not expose secret-like rationale text: {reflection_event:?}"
    );

    let payload = reflection_event
        .get("payload")
        .expect("dashboard event should include payload");
    let payload_json = payload.to_string();
    assert!(
        !payload_json.contains(raw_secret_rationale),
        "dashboard payload must not expose raw model rationale: {reflection_event:?}"
    );
    assert!(
        !payload_json.contains("sk-dashboard-secret-token-password-should-not-persist"),
        "dashboard payload must not expose secret-like rationale text: {reflection_event:?}"
    );
    assert_eq!(
        payload.get("trigger_type").and_then(Value::as_str),
        Some("failure")
    );
    assert_eq!(
        payload.get("ledger_status").and_then(Value::as_str),
        Some("rejected")
    );
    assert_eq!(
        payload.get("rejection_reason").and_then(Value::as_str),
        Some("model_rationale_omitted")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn governed_auto_reflection_rejection_error_appends_rejected_trigger_operation_log() {
    let stub = test_support::StubServer::spawn(
        200,
        json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": r#"{"should_reflect":true,"rationale":"Rejected evidence should be classified as governance policy, not a runtime failure.","machine_patch":{"identity_patch":null,"commitment_patch":{"commitments":["prefer:reflect_before_repeating_rollback"]}},"proposed_evidence_event_ids":["evt-outside-window"],"confidence":"medium"}"#
                }
            }]
        }),
    )
    .await;
    let config = format!(
        r#"
transport = "stdio"
database_url = "__DATABASE_URL__"

[model]
provider = "openai-compatible"

[model.openai_compatible]
base_url = "{}"
api_key = "example-test-key"
model = "gpt-4o-mini"
timeout_ms = 30000
"#,
        stub.base_url()
    );
    let (mut client, database_url, _database_dir) =
        test_support::spawn_stdio_client_with_config_and_database(config)
            .await
            .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    for (episode_reference, summary, trigger_hints) in [
        (
            "episode:auto-reflect-governed-rejection-error-0",
            "first rollback after violating a hard commitment",
            json!([]),
        ),
        (
            "episode:auto-reflect-governed-rejection-error-1",
            "rollback after violating a hard commitment",
            json!(["failure", "rollback"]),
        ),
    ] {
        let response = client
            .call_tool(
                "ingest_interaction",
                json!({
                    "event": {
                        "owner": "Self_",
                        "kind": "Action",
                        "summary": summary
                    },
                    "claim_drafts": [],
                    "episode_reference": episode_reference,
                    "trigger_hints": trigger_hints
                }),
            )
            .await
            .unwrap();

        assert!(
            response.get("error").is_none(),
            "governed rejected auto-reflection must preserve main ingest success semantics: {response:?}"
        );
    }

    let pool = SqlitePool::connect(&database_url).await.unwrap();
    let row = sqlx::query(
        "SELECT operation_kind, status, diagnostic_summary_json FROM operation_log \
         WHERE entrypoint = 'ingest_interaction' AND operation_kind = 'trigger' \
         ORDER BY occurred_at DESC, operation_id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("governed rejection should append a trigger operation log entry");

    assert_eq!(row.get::<String, _>("operation_kind"), "trigger");
    assert_eq!(row.get::<String, _>("status"), "rejected");
    let diagnostic: serde_json::Value = serde_json::from_str(
        &row.get::<Option<String>, _>("diagnostic_summary_json")
            .expect("rejected trigger should include bounded diagnostic summary"),
    )
    .expect("diagnostic summary should be json");
    assert_eq!(
        diagnostic.get("outcome").and_then(Value::as_str),
        Some("rejected")
    );
    assert_eq!(
        diagnostic.get("rejection_category").and_then(Value::as_str),
        Some("governance_policy")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ingest_interaction_auto_reflects_once_and_does_not_recurse_inside_run_reflection() {
    let stub = test_support::StubServer::spawn(
        200,
        json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": r#"{"should_reflect":true,"rationale":"Repeated rollback should tighten commitments.","machine_patch":{"commitment_patch":{"commitments":["prefer:reflect_before_repeating_rollback"]}}}"#
                }
            }]
        }),
    )
    .await;
    let config = format!(
        r#"
transport = "stdio"
database_url = "__DATABASE_URL__"

[model]
provider = "openai-compatible"

[model.openai_compatible]
base_url = "{}"
api_key = "example-test-key"
model = "gpt-4o-mini"
timeout_ms = 30000
"#,
        stub.base_url()
    );
    let (mut client, database_url, _database_dir) =
        test_support::spawn_stdio_client_with_config_and_database(config)
            .await
            .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "Self_",
                    "kind": "Action",
                    "summary": "first rollback after violating a hard commitment"
                },
                "claim_drafts": [],
                "episode_reference": "episode:auto-reflect-0"
            }),
        )
        .await
        .unwrap();

    client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "Self_",
                    "kind": "Action",
                    "summary": "rollback after violating a hard commitment"
                },
                "claim_drafts": [],
                "episode_reference": "episode:auto-reflect-1",
                "trigger_hints": ["failure", "rollback"]
            }),
        )
        .await
        .unwrap();

    let pool = SqlitePool::connect(&database_url).await.unwrap();
    let trigger_rows = sqlx::query_as::<_, (String, String)>(
        "SELECT trigger_type, namespace FROM reflection_trigger_ledger ORDER BY rowid ASC",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let reflection_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM reflections")
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(reflection_count, 1);
    assert_eq!(
        trigger_rows,
        vec![("failure".to_string(), "self".to_string())]
    );
    assert_eq!(stub.request_count().await, 1);

    let ingest = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "User",
                    "kind": "Conversation",
                    "summary": "A direct reflection target exists for recursion checking."
                },
                "claim_drafts": [
                    {
                        "owner": "Self_",
                        "subject": "self.role",
                        "predicate": "is",
                        "object": "architect",
                        "mode": "Observed"
                    }
                ],
                "episode_reference": "episode:auto-reflect-direct-reflection-target"
            }),
        )
        .await
        .unwrap();
    let superseded_claim_id = ingest
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("event_id"))
        .and_then(Value::as_str)
        .map(|event_id| format!("{event_id}:claim:0"))
        .unwrap();

    let reflection = client
        .call_tool(
            "run_reflection",
            json!({
                "reflection": {
                    "summary": "Explicit MCP reflection should not recurse into auto-reflection."
                },
                "supersede_claim_id": superseded_claim_id,
                "replacement_claim": null
            }),
        )
        .await
        .unwrap();
    assert!(
        reflection.get("error").is_none(),
        "explicit run_reflection should still succeed: {reflection:?}"
    );

    let reflection_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM reflections")
        .fetch_one(&pool)
        .await
        .unwrap();
    let trigger_ledger_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM reflection_trigger_ledger")
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(reflection_count, 2);
    assert_eq!(trigger_ledger_count, 1);
    assert_eq!(stub.request_count().await, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ingest_interaction_auto_reflection_uses_openrouter_provider_from_config_file() {
    let stub = test_support::StubServer::spawn(
        200,
        json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": r#"{"should_reflect":true,"rationale":"OpenRouter-backed rollback evidence should tighten commitments.","machine_patch":{"commitment_patch":{"commitments":["prefer:reflect_before_repeating_openrouter_rollback"]}}}"#
                }
            }]
        }),
    )
    .await;
    let config = format!(
        r#"
transport = "stdio"
database_url = "__DATABASE_URL__"

[model]
provider = "openrouter"

[model.openrouter]
base_url = "{}"
api_key = "example-openrouter-key"
model = "openrouter/test-model"
timeout_ms = 30000
"#,
        stub.base_url()
    );
    let (mut client, database_url, _database_dir) =
        test_support::spawn_stdio_client_with_config_and_database(config)
            .await
            .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "Self_",
                    "kind": "Action",
                    "summary": "first OpenRouter rollback after violating a hard commitment"
                },
                "claim_drafts": [],
                "episode_reference": "episode:openrouter-auto-reflect-0"
            }),
        )
        .await
        .unwrap();

    client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "Self_",
                    "kind": "Action",
                    "summary": "OpenRouter rollback after violating a hard commitment"
                },
                "claim_drafts": [],
                "episode_reference": "episode:openrouter-auto-reflect-1",
                "trigger_hints": ["failure", "rollback"]
            }),
        )
        .await
        .unwrap();

    let pool = SqlitePool::connect(&database_url).await.unwrap();
    let reflection_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM reflections")
        .fetch_one(&pool)
        .await
        .unwrap();
    let commitment_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM commitments WHERE description = 'prefer:reflect_before_repeating_openrouter_rollback'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(reflection_count, 1);
    assert_eq!(commitment_count, 1);
    assert_eq!(stub.request_count().await, 1);
    assert_eq!(
        stub.last_request_path().await.as_deref(),
        Some("/chat/completions")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ingest_interaction_can_trigger_conflict_auto_reflection_when_explicit_conflict_hints_present()
 {
    let stub = test_support::StubServer::spawn(
        200,
        json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": r#"{"should_reflect":true,"rationale":"Conflict evidence suggests tighter commitment hygiene.","machine_patch":{"commitment_patch":{"commitments":["prefer:confirm_conflicting_commitment_updates_before_overwrite"]}}}"#
                }
            }]
        }),
    )
    .await;
    let config = format!(
        r#"
transport = "stdio"
database_url = "__DATABASE_URL__"

[model]
provider = "openai-compatible"

[model.openai_compatible]
base_url = "{}"
api_key = "example-test-key"
model = "gpt-4o-mini"
timeout_ms = 30000
"#,
        stub.base_url()
    );
    let (mut client, database_url, _database_dir) =
        test_support::spawn_stdio_client_with_config_and_database(config)
            .await
            .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    let response = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "Self_",
                    "kind": "Action",
                    "summary": "self attempted a commitment overwrite that may need review"
                },
                "claim_drafts": [],
                "episode_reference": "episode:ingest-conflict-auto-reflect",
                "trigger_hints": ["conflict", "commitment"]
            }),
        )
        .await
        .unwrap();

    let event_id = response
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("event_id"))
        .and_then(Value::as_str);
    assert!(
        response.get("error").is_none(),
        "ingest must still succeed when conflict auto-reflection runs: {response:?}"
    );
    assert!(
        event_id.is_some(),
        "ingest should still succeed when conflict auto-reflection runs: {response:?}"
    );

    let pool = SqlitePool::connect(&database_url).await.unwrap();
    let reflection_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM reflections")
        .fetch_one(&pool)
        .await
        .unwrap();
    let trigger_rows = sqlx::query_as::<_, (String, String)>(
        "SELECT trigger_type, status FROM reflection_trigger_ledger ORDER BY rowid ASC",
    )
    .fetch_all(&pool)
    .await
    .unwrap();

    assert_eq!(reflection_count, 1);
    assert_eq!(
        trigger_rows,
        vec![("conflict".to_string(), "handled".to_string())]
    );
    assert_eq!(stub.request_count().await, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ingest_interaction_returns_success_even_when_conflict_auto_reflection_fails() {
    let stub = test_support::StubServer::spawn(
        200,
        json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "not valid self revision json"
                }
            }]
        }),
    )
    .await;
    let config = format!(
        r#"
transport = "stdio"
database_url = "__DATABASE_URL__"

[model]
provider = "openai-compatible"

[model.openai_compatible]
base_url = "{}"
api_key = "example-test-key"
model = "gpt-4o-mini"
timeout_ms = 30000
"#,
        stub.base_url()
    );
    let (mut client, database_url, _database_dir) =
        test_support::spawn_stdio_client_with_config_and_database(config)
            .await
            .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    let response = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "Self_",
                    "kind": "Action",
                    "summary": "self attempted a commitment overwrite that may need review"
                },
                "claim_drafts": [],
                "episode_reference": "episode:ingest-conflict-auto-reflect-nonfatal",
                "trigger_hints": ["conflict", "commitment"]
            }),
        )
        .await
        .unwrap();

    assert!(
        response.get("error").is_none(),
        "ingest must still succeed when best-effort conflict auto-reflection fails: {response:?}"
    );

    let pool = SqlitePool::connect(&database_url).await.unwrap();
    let event_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM events")
        .fetch_one(&pool)
        .await
        .unwrap();
    let reflection_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM reflections")
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(event_count, 1);
    assert_eq!(reflection_count, 0);
    assert_eq!(stub.request_count().await, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ingest_interaction_does_not_auto_reflect_conflict_without_explicit_conflict_hints() {
    let stub = test_support::StubServer::spawn(
        200,
        json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": r#"{"should_reflect":true,"rationale":"Conflict evidence suggests tighter commitment hygiene.","machine_patch":{"commitment_patch":{"commitments":["prefer:confirm_conflicting_commitment_updates_before_overwrite"]}}}"#
                }
            }]
        }),
    )
    .await;
    let config = format!(
        r#"
transport = "stdio"
database_url = "__DATABASE_URL__"

[model]
provider = "openai-compatible"

[model.openai_compatible]
base_url = "{}"
api_key = "example-test-key"
model = "gpt-4o-mini"
timeout_ms = 30000
"#,
        stub.base_url()
    );
    let (mut client, database_url, _database_dir) =
        test_support::spawn_stdio_client_with_config_and_database(config)
            .await
            .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    let response = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "Self_",
                    "kind": "Action",
                    "summary": "self attempted a conflicting commitment overwrite"
                },
                "claim_drafts": [],
                "episode_reference": "episode:ingest-conflict-without-hints"
            }),
        )
        .await
        .unwrap();

    assert!(
        response.get("error").is_none(),
        "ingest must still succeed without explicit conflict hints: {response:?}"
    );

    let pool = SqlitePool::connect(&database_url).await.unwrap();
    let reflection_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM reflections")
        .fetch_one(&pool)
        .await
        .unwrap();
    let trigger_ledger_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM reflection_trigger_ledger")
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(reflection_count, 0);
    assert_eq!(trigger_ledger_count, 0);
    assert_eq!(stub.request_count().await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ingest_interaction_does_not_auto_reflect_conflict_with_non_conflict_trigger_hints() {
    let stub = test_support::StubServer::spawn(
        200,
        json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": r#"{"should_reflect":true,"rationale":"Conflict evidence suggests tighter commitment hygiene.","machine_patch":{"commitment_patch":{"commitments":["prefer:confirm_conflicting_commitment_updates_before_overwrite"]}}}"#
                }
            }]
        }),
    )
    .await;
    let config = format!(
        r#"
transport = "stdio"
database_url = "__DATABASE_URL__"

[model]
provider = "openai-compatible"

[model.openai_compatible]
base_url = "{}"
api_key = "example-test-key"
model = "gpt-4o-mini"
timeout_ms = 30000
"#,
        stub.base_url()
    );
    let (mut client, database_url, _database_dir) =
        test_support::spawn_stdio_client_with_config_and_database(config)
            .await
            .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    let response = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "Self_",
                    "kind": "Action",
                    "summary": "self attempted a conflicting commitment overwrite"
                },
                "claim_drafts": [],
                "episode_reference": "episode:ingest-conflict-with-non-conflict-hints",
                "trigger_hints": ["commitment"]
            }),
        )
        .await
        .unwrap();

    assert!(
        response.get("error").is_none(),
        "ingest must still succeed with non-conflict hints: {response:?}"
    );

    let pool = SqlitePool::connect(&database_url).await.unwrap();
    let reflection_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM reflections")
        .fetch_one(&pool)
        .await
        .unwrap();
    let trigger_ledger_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM reflection_trigger_ledger")
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(reflection_count, 0);
    assert_eq!(trigger_ledger_count, 0);
    assert_eq!(stub.request_count().await, 0);
}

#[tokio::test]
async fn ingest_interaction_rejects_ambiguous_auto_reflect_scope_before_writing() {
    let (mut client, database_url, _database_dir) =
        test_support::spawn_stdio_client_with_database()
            .await
            .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    let response = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "Self_",
                    "kind": "Action",
                    "summary": "Mixed claim scopes should not depend on draft ordering."
                },
                "claim_drafts": [
                    {
                        "owner": "World",
                        "namespace": "project/agent-llm-mm",
                        "subject": "project.memory",
                        "predicate": "needs",
                        "object": "structure",
                        "mode": "Observed"
                    },
                    {
                        "owner": "World",
                        "namespace": "world",
                        "subject": "weather",
                        "predicate": "is",
                        "object": "rainy",
                        "mode": "Observed"
                    }
                ],
                "episode_reference": "episode:auto-reflect-ambiguous-scope",
                "trigger_hints": ["failure", "rollback"]
            }),
        )
        .await
        .unwrap();

    let error = response
        .get("error")
        .expect("ambiguous mixed-scope ingest should be rejected before any write");
    assert_eq!(error.get("code").and_then(Value::as_i64), Some(-32602));

    let pool = SqlitePool::connect(&database_url).await.unwrap();
    let event_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM events")
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(event_count, 0);
}

#[tokio::test]
async fn stdio_tools_share_runtime_state_across_calls() {
    let mut client = test_support::spawn_stdio_client().await.unwrap();
    let _ = client.list_all_tools().await.unwrap();

    let ingest = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "World",
                    "namespace": "project/agent-llm-mm",
                    "kind": "Conversation",
                    "summary": "The user asked for stronger memory."
                },
                "claim_drafts": [
                    {
                        "owner": "World",
                        "namespace": "project/agent-llm-mm",
                        "subject": "project.memory",
                        "predicate": "needs",
                        "object": "structure",
                        "mode": "Observed"
                    }
                ],
                "episode_reference": "episode:task-7"
            }),
        )
        .await
        .unwrap();
    let event_id = ingest
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("event_id"))
        .and_then(Value::as_str)
        .unwrap();
    assert!(!event_id.is_empty());

    let snapshot = client
        .call_tool(
            "build_self_snapshot",
            json!({
                "budget": 4,
                "namespace": "project/agent-llm-mm",
                "evidence_manifest": [event_id, format!("event:{event_id}")],
                "recorded_after": "2000-01-01T00:00:00Z",
                "recorded_before": "2100-01-01T00:00:00Z"
            }),
        )
        .await
        .unwrap();
    let snapshot = snapshot
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("snapshot"))
        .cloned()
        .unwrap_or_else(|| panic!("scoped snapshot response missing snapshot: {snapshot:?}"));

    let claims = snapshot
        .get("claims")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert!(
        claims.contains(&"project/agent-llm-mm:project.memory needs structure"),
        "snapshot claims missing ingested claim: {claims:?}"
    );

    let evidence = snapshot
        .get("evidence")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert_eq!(evidence.len(), 1, "expected one evidence reference");
    assert!(
        evidence[0].starts_with("event:"),
        "unexpected evidence reference: {:?}",
        evidence
    );

    let episodes = snapshot
        .get("episodes")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert!(
        episodes.contains(&"episode:task-7"),
        "snapshot episodes missing ingested episode: {episodes:?}"
    );
}

#[tokio::test]
async fn search_memory_returns_only_scoped_event_records_and_provenance_over_stdio() {
    let (mut client, database_url, _database_dir) =
        test_support::spawn_stdio_client_with_database()
            .await
            .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    let mut project_a_event_id = String::new();
    for (namespace, summary, episode_reference, with_claim) in [
        ("project/a", "project a durable memory", "episode:a", true),
        ("project/b", "project b interference", "episode:b", false),
    ] {
        let claim_drafts = if with_claim {
            json!([{
                "owner": "World",
                "namespace": namespace,
                "subject": "project.a",
                "predicate": "has",
                "object": "durable memory",
                "mode": "Observed"
            }])
        } else {
            json!([])
        };
        let response = client
            .call_tool(
                "ingest_interaction",
                json!({
                    "event": {
                        "owner": "World",
                        "namespace": namespace,
                        "kind": "Observation",
                        "summary": summary
                    },
                    "claim_drafts": claim_drafts,
                    "episode_reference": episode_reference
                }),
            )
            .await
            .unwrap();
        assert!(
            response.get("error").is_none(),
            "ingest failed: {response:?}"
        );
        if with_claim {
            project_a_event_id = response["result"]["structuredContent"]["event_id"]
                .as_str()
                .unwrap()
                .to_string();
        }
    }

    let mixed_scope = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "World",
                    "namespace": "project/a",
                    "kind": "Observation",
                    "summary": "project a event with a project b claim link"
                },
                "claim_drafts": [{
                    "owner": "World",
                    "namespace": "project/b",
                    "subject": "project.b",
                    "predicate": "must_not",
                    "object": "leak through project a event provenance",
                    "mode": "Observed"
                }],
                "episode_reference": "episode:mixed-scope"
            }),
        )
        .await
        .unwrap();
    let mixed_scope_event_id = mixed_scope["result"]["structuredContent"]["event_id"]
        .as_str()
        .unwrap();

    let pool = SqlitePool::connect(&database_url).await.unwrap();
    let semantic_counts_before = semantic_memory_counts(&pool).await;
    let response = client
        .call_tool(
            "search_memory",
            json!({
                "namespace": "project/a",
                "kind": "Observation",
                "event_reference": format!("event:{project_a_event_id}"),
                "recorded_after": "2000-01-01T00:00:00Z",
                "recorded_before": "2100-01-01T00:00:00Z",
                "limit": 10
            }),
        )
        .await
        .unwrap();
    assert!(
        response.get("error").is_none(),
        "search failed: {response:?}"
    );
    let result = &response["result"]["structuredContent"];
    assert_eq!(result["owner"], "World");
    assert_eq!(result["namespace"], "project/a");
    assert_eq!(result["limit"], 10);
    let records = result["records"].as_array().unwrap();
    assert_eq!(records.len(), 1, "unexpected scoped search: {result:?}");
    assert_eq!(records[0]["record_type"], "event");
    assert_eq!(records[0]["id"], format!("event:{project_a_event_id}"));
    assert_eq!(records[0]["owner"], "World");
    assert_eq!(records[0]["namespace"], "project/a");
    assert_eq!(records[0]["kind"], "Observation");
    assert_eq!(records[0]["summary"], "project a durable memory");
    assert!(records[0]["recorded_at"].as_str().is_some());
    assert_eq!(
        records[0]["provenance"]["evidence_event_reference"],
        format!("event:{project_a_event_id}")
    );
    assert_eq!(
        records[0]["provenance"]["episode_references"],
        json!(["episode:a"])
    );
    assert_eq!(
        records[0]["provenance"]["claim_ids"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(semantic_memory_counts(&pool).await, semantic_counts_before);

    let operation = sqlx::query(
        "SELECT namespace, status, response_summary_json FROM operation_log WHERE entrypoint = 'search_memory' ORDER BY occurred_at DESC, operation_id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(operation.get::<String, _>("namespace"), "project/a");
    assert_eq!(operation.get::<String, _>("status"), "ok");
    let summary: Value =
        serde_json::from_str(&operation.get::<String, _>("response_summary_json")).unwrap();
    assert_eq!(summary, json!({"record_type": "event", "result_count": 1}));

    let mixed_event = client
        .call_tool(
            "search_memory",
            json!({
                "namespace": "project/a",
                "event_reference": format!("event:{mixed_scope_event_id}")
            }),
        )
        .await
        .unwrap();
    assert_eq!(
        mixed_event["result"]["structuredContent"]["records"][0]["provenance"]["claim_ids"],
        json!([]),
        "event provenance must not expose a linked claim from another scope"
    );
    let mixed_claim = client
        .call_tool(
            "search_memory",
            json!({
                "namespace": "project/b",
                "record_type": "Claim",
                "claim_reference": format!("claim:{mixed_scope_event_id}:claim:0")
            }),
        )
        .await
        .unwrap();
    assert_eq!(
        mixed_claim["result"]["structuredContent"]["records"][0]["provenance"]["evidence_event_references"],
        json!([]),
        "claim provenance must not expose a linked event from another scope"
    );
}

#[tokio::test]
async fn search_memory_returns_scoped_episode_provenance_over_stdio_without_semantic_writes() {
    let (mut client, database_url, _database_dir) =
        test_support::spawn_stdio_client_with_database()
            .await
            .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    let mut ingested = Vec::new();
    for (namespace, summary) in [
        ("project/episode-a", "older scoped episode event"),
        ("project/episode-a", "newer scoped episode event"),
        ("project/episode-b", "newest cross-scope episode event"),
    ] {
        let response = client
            .call_tool(
                "ingest_interaction",
                json!({
                    "event": {
                        "owner": "World",
                        "namespace": namespace,
                        "kind": "Observation",
                        "summary": summary
                    },
                    "claim_drafts": [{
                        "owner": "World",
                        "namespace": namespace,
                        "subject": "episode.fact",
                        "predicate": "is",
                        "object": summary,
                        "mode": "Observed"
                    }],
                    "episode_reference": "episode:Shared-Exact"
                }),
            )
            .await
            .unwrap();
        assert!(
            response.get("error").is_none(),
            "episode fixture ingest failed: {response:?}"
        );
        ingested.push(
            response["result"]["structuredContent"]["event_id"]
                .as_str()
                .unwrap()
                .to_string(),
        );
    }
    let [old_a_event_id, new_a_event_id, b_event_id] = ingested.as_slice() else {
        panic!("expected three ingested episode events");
    };

    let pool = SqlitePool::connect(&database_url).await.unwrap();
    for (event_id, recorded_at) in [
        (old_a_event_id, "2026-07-15T01:00:00Z"),
        (new_a_event_id, "2026-07-15T02:00:00Z"),
        (b_event_id, "2026-07-15T03:00:00Z"),
    ] {
        sqlx::query("UPDATE events SET recorded_at = ? WHERE event_id = ?")
            .bind(recorded_at)
            .bind(event_id)
            .execute(&pool)
            .await
            .unwrap();
    }
    let semantic_counts_before = semantic_memory_counts(&pool).await;

    let response = client
        .call_tool(
            "search_memory",
            json!({
                "namespace": "project/episode-a",
                "record_type": "Episode",
                "episode_reference": "episode:Shared-Exact",
                "limit": 10
            }),
        )
        .await
        .unwrap();
    assert!(
        response.get("error").is_none(),
        "episode search failed: {response:?}"
    );
    let result = &response["result"]["structuredContent"];
    assert_eq!(result["owner"], "World");
    assert_eq!(result["namespace"], "project/episode-a");
    assert_eq!(result["limit"], 10);
    let records = result["records"].as_array().unwrap();
    assert_eq!(records.len(), 1, "unexpected Episode search: {result:?}");
    let record = &records[0];
    assert_eq!(record["record_type"], "episode");
    assert_eq!(record["id"], "episode:Shared-Exact");
    assert_eq!(record["recorded_at"], "2026-07-15T02:00:00Z");
    assert_eq!(record["owner"], "World");
    assert_eq!(record["namespace"], "project/episode-a");
    assert_eq!(
        record["provenance"]["event_references"],
        json!([
            format!("event:{new_a_event_id}"),
            format!("event:{old_a_event_id}")
        ])
    );
    let mut expected_claim_references = vec![
        format!("claim:{old_a_event_id}:claim:0"),
        format!("claim:{new_a_event_id}:claim:0"),
    ];
    expected_claim_references.sort();
    assert_eq!(
        record["provenance"]["claim_references"],
        json!(expected_claim_references)
    );
    for non_persisted_field in [
        "objective",
        "outcome",
        "lesson",
        "reflection_id",
        "source_reflection_id",
    ] {
        assert!(
            record.get(non_persisted_field).is_none(),
            "Episode search must not invent {non_persisted_field}"
        );
    }
    assert!(
        record["provenance"].get("reflection_id").is_none()
            && record["provenance"].get("source_reflection_id").is_none(),
        "Episode provenance must not include Reflection IDs"
    );

    let operation = sqlx::query(
        "SELECT namespace, status, response_summary_json FROM operation_log WHERE entrypoint = 'search_memory' ORDER BY occurred_at DESC, operation_id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(operation.get::<String, _>("namespace"), "project/episode-a");
    assert_eq!(operation.get::<String, _>("status"), "ok");
    let summary: Value =
        serde_json::from_str(&operation.get::<String, _>("response_summary_json")).unwrap();
    assert_eq!(
        summary,
        json!({"record_type": "episode", "result_count": 1})
    );

    for params in [
        json!({
            "namespace": "project/episode-a",
            "record_type": "Episode",
            "episode_reference": "episode:missing"
        }),
        json!({
            "namespace": "project/episode-a",
            "record_type": "Episode",
            "episode_reference": "episode:shared-exact"
        }),
        json!({
            "namespace": "project/empty",
            "record_type": "Episode",
            "episode_reference": "episode:Shared-Exact"
        }),
    ] {
        let empty = client.call_tool("search_memory", params).await.unwrap();
        assert_eq!(
            empty["result"]["structuredContent"]["records"],
            json!([]),
            "missing or cross-scope Episode references must not widen the query"
        );
    }

    let found = client
        .call_tool(
            "get_memory",
            json!({
                "namespace": "project/episode-a",
                "id": "episode:Shared-Exact",
                "record_type": "Episode"
            }),
        )
        .await
        .unwrap();
    assert!(
        found.get("error").is_none(),
        "episode lookup failed: {found:?}"
    );
    let lookup = &found["result"]["structuredContent"];
    assert_eq!(lookup["record"], *record);
    assert_eq!(lookup["namespace"], "project/episode-a");

    for params in [
        json!({
            "namespace": "project/episode-a",
            "id": "episode:missing",
            "record_type": "Episode"
        }),
        json!({
            "namespace": "project/episode-a",
            "id": "episode:shared-exact",
            "record_type": "Episode"
        }),
        json!({
            "namespace": "project/empty",
            "id": "episode:Shared-Exact",
            "record_type": "Episode"
        }),
    ] {
        let missing = client.call_tool("get_memory", params).await.unwrap();
        assert_eq!(
            missing["result"]["structuredContent"]["record"],
            json!(null),
            "missing or cross-scope Episode lookup must return null without widening"
        );
    }

    let omitted_type = client
        .call_tool(
            "get_memory",
            json!({
                "namespace": "project/episode-a",
                "id": "episode:Shared-Exact"
            }),
        )
        .await
        .unwrap();
    assert_eq!(
        omitted_type["result"]["structuredContent"]["record"],
        json!(null),
        "omitted record_type must keep Event lookup semantics"
    );
    assert_eq!(semantic_memory_counts(&pool).await, semantic_counts_before);
}

#[tokio::test]
async fn search_memory_returns_scoped_claims_with_revision_provenance_over_stdio() {
    let (mut client, database_url, _database_dir) =
        test_support::spawn_stdio_client_with_database()
            .await
            .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    let project_a = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "World",
                    "namespace": "project/claim-a",
                    "kind": "Observation",
                    "summary": "old scoped claim evidence"
                },
                "claim_drafts": [{
                    "owner": "World",
                    "namespace": "project/claim-a",
                    "subject": "project.claim",
                    "predicate": "is",
                    "object": "old",
                    "mode": "Observed"
                }],
                "episode_reference": "episode:claim-a-old"
            }),
        )
        .await
        .unwrap();
    let project_a_event_id = project_a["result"]["structuredContent"]["event_id"]
        .as_str()
        .unwrap();
    let old_claim_id = format!("{project_a_event_id}:claim:0");

    let project_b = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "World",
                    "namespace": "project/claim-b",
                    "kind": "Observation",
                    "summary": "cross-scope interference"
                },
                "claim_drafts": [{
                    "owner": "World",
                    "namespace": "project/claim-b",
                    "subject": "project.claim",
                    "predicate": "is",
                    "object": "interference",
                    "mode": "Observed"
                }],
                "episode_reference": "episode:claim-b"
            }),
        )
        .await
        .unwrap();
    let project_b_event_id = project_b["result"]["structuredContent"]["event_id"]
        .as_str()
        .unwrap();
    let project_b_claim_id = format!("{project_b_event_id}:claim:0");

    let reflection = client
        .call_tool(
            "run_reflection",
            json!({
                "reflection": {"summary": "replace the old scoped claim"},
                "supersede_claim_id": old_claim_id,
                "replacement_claim": {
                    "owner": "World",
                    "namespace": "project/claim-a",
                    "subject": "project.claim",
                    "predicate": "is",
                    "object": "new",
                    "mode": "Observed"
                },
                "replacement_evidence_event_ids": [format!("event:{project_a_event_id}")]
            }),
        )
        .await
        .unwrap();
    assert!(
        reflection.get("error").is_none(),
        "reflection failed: {reflection:?}"
    );
    let reflection_id = reflection["result"]["structuredContent"]["reflection_id"]
        .as_str()
        .unwrap();
    let replacement_claim_id = reflection["result"]["structuredContent"]["replacement_claim_id"]
        .as_str()
        .unwrap();

    let pool = SqlitePool::connect(&database_url).await.unwrap();
    let semantic_counts_before = semantic_memory_counts(&pool).await;

    for claim_reference in [
        format!("claim:{old_claim_id}"),
        replacement_claim_id.to_string(),
    ] {
        let history = client
            .call_tool(
                "get_reflection_history",
                json!({
                    "namespace": "project/claim-a",
                    "claim_reference": claim_reference,
                    "limit": 10
                }),
            )
            .await
            .unwrap();
        assert!(
            history.get("error").is_none(),
            "reflection history failed: {history:?}"
        );
        let structured = &history["result"]["structuredContent"];
        assert_eq!(structured["owner"], "World");
        assert_eq!(structured["namespace"], "project/claim-a");
        assert_eq!(structured["limit"], 10);
        assert_eq!(structured["has_more"], false);
        let history_records = structured["reflections"].as_array().unwrap();
        assert_eq!(history_records.len(), 1);
        assert_eq!(history_records[0]["reflection_id"], reflection_id);
        assert_eq!(
            history_records[0]["summary"],
            "replace the old scoped claim"
        );
        assert_eq!(
            history_records[0]["superseded_claim_reference"],
            format!("claim:{old_claim_id}")
        );
        assert_eq!(
            history_records[0]["replacement_claim_reference"],
            format!("claim:{replacement_claim_id}")
        );
        assert_eq!(
            history_records[0]["supporting_evidence_event_references"],
            json!([format!("event:{project_a_event_id}")])
        );
    }

    for claim_reference in [
        format!("claim:{project_b_claim_id}"),
        "claim:missing-history-claim".to_string(),
    ] {
        let history = client
            .call_tool(
                "get_reflection_history",
                json!({
                    "namespace": "project/claim-a",
                    "claim_reference": claim_reference
                }),
            )
            .await
            .unwrap();
        assert_eq!(
            history["result"]["structuredContent"]["reflections"],
            json!([]),
            "missing and cross-scope claims must not widen history reads"
        );
    }
    let history_operation = sqlx::query(
        "SELECT response_summary_json FROM operation_log WHERE entrypoint = 'get_reflection_history' ORDER BY occurred_at DESC, operation_id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let history_summary: Value =
        serde_json::from_str(&history_operation.get::<String, _>("response_summary_json")).unwrap();
    assert_eq!(
        history_summary,
        json!({"history_type": "claim", "result_count": 0, "has_more": false})
    );

    let active = client
        .call_tool(
            "search_memory",
            json!({
                "namespace": "project/claim-a",
                "record_type": "Claim",
                "mode": "Observed",
                "limit": 10
            }),
        )
        .await
        .unwrap();
    assert!(
        active.get("error").is_none(),
        "claim search failed: {active:?}"
    );
    let records = active["result"]["structuredContent"]["records"]
        .as_array()
        .unwrap();
    assert_eq!(
        records.len(),
        1,
        "default claim search should return active claims only"
    );
    assert_eq!(records[0]["record_type"], "claim");
    assert_eq!(records[0]["id"], format!("claim:{replacement_claim_id}"));
    assert_eq!(records[0]["owner"], "World");
    assert_eq!(records[0]["namespace"], "project/claim-a");
    assert_eq!(records[0]["subject"], "project.claim");
    assert_eq!(records[0]["predicate"], "is");
    assert_eq!(records[0]["object"], "new");
    assert_eq!(records[0]["mode"], "Observed");
    assert_eq!(records[0]["status"], "Active");
    let replacement_time = chrono::DateTime::parse_from_rfc3339(
        records[0]["recorded_at"]
            .as_str()
            .expect("new replacement has application recording time"),
    )
    .unwrap();
    let audit_time: String =
        sqlx::query_scalar("SELECT recorded_at FROM reflections WHERE reflection_id = ?")
            .bind(reflection_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        replacement_time,
        chrono::DateTime::parse_from_rfc3339(&audit_time).unwrap()
    );
    assert!(records[0]["observed_at"].is_null());
    assert_eq!(
        records[0]["provenance"]["evidence_event_references"],
        json!([format!("event:{project_a_event_id}")])
    );
    assert_eq!(
        records[0]["provenance"]["episode_references"],
        json!(["episode:claim-a-old"])
    );
    assert_eq!(
        records[0]["provenance"]["source_reflection_id"],
        reflection_id
    );
    assert_eq!(
        records[0]["provenance"]["supersedes_claim_reference"],
        format!("claim:{old_claim_id}")
    );

    let old = client
        .call_tool(
            "search_memory",
            json!({
                "namespace": "project/claim-a",
                "record_type": "Claim",
                "claim_reference": format!("claim:{old_claim_id}"),
                "claim_status": "Superseded"
            }),
        )
        .await
        .unwrap();
    let old_record = &old["result"]["structuredContent"]["records"][0];
    assert_eq!(old_record["status"], "Superseded");
    assert_eq!(
        old_record["provenance"]["superseded_by_reflection_id"],
        reflection_id
    );
    assert_eq!(
        old_record["provenance"]["replacement_claim_reference"],
        format!("claim:{replacement_claim_id}")
    );

    let old_lookup = client
        .call_tool(
            "get_memory",
            json!({
                "namespace": "project/claim-a",
                "id": format!("claim:{old_claim_id}"),
                "record_type": "Claim"
            }),
        )
        .await
        .unwrap();
    let old_lookup_record = &old_lookup["result"]["structuredContent"]["record"];
    assert_eq!(old_lookup_record["id"], format!("claim:{old_claim_id}"));
    assert_eq!(old_lookup_record["status"], "Superseded");
    assert_eq!(
        old_lookup_record["provenance"]["replacement_claim_reference"],
        format!("claim:{replacement_claim_id}")
    );

    let active_lookup = client
        .call_tool(
            "get_memory",
            json!({
                "namespace": "project/claim-a",
                "id": replacement_claim_id,
                "record_type": "Claim"
            }),
        )
        .await
        .unwrap();
    assert_eq!(
        active_lookup["result"]["structuredContent"]["record"]["status"],
        "Active"
    );

    let cross_scope_lookup = client
        .call_tool(
            "get_memory",
            json!({
                "namespace": "project/claim-a",
                "id": format!("claim:{project_b_claim_id}"),
                "record_type": "Claim"
            }),
        )
        .await
        .unwrap();
    assert!(
        cross_scope_lookup["result"]["structuredContent"]["record"].is_null(),
        "claim lookup must not widen across namespaces"
    );

    let missing_lookup = client
        .call_tool(
            "get_memory",
            json!({
                "namespace": "project/claim-a",
                "id": "claim:missing-claim",
                "record_type": "Claim"
            }),
        )
        .await
        .unwrap();
    assert!(missing_lookup["result"]["structuredContent"]["record"].is_null());

    let get_operation = sqlx::query(
        "SELECT response_summary_json FROM operation_log WHERE entrypoint = 'get_memory' ORDER BY occurred_at DESC, operation_id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let get_summary: Value =
        serde_json::from_str(&get_operation.get::<String, _>("response_summary_json")).unwrap();
    assert_eq!(get_summary, json!({"record_type": "claim", "found": false}));

    let cross_scope = client
        .call_tool(
            "search_memory",
            json!({
                "namespace": "project/claim-a",
                "record_type": "Claim",
                "claim_reference": format!("claim:{project_b_claim_id}")
            }),
        )
        .await
        .unwrap();
    assert_eq!(
        cross_scope["result"]["structuredContent"]["records"],
        json!([])
    );
    assert_eq!(semantic_memory_counts(&pool).await, semantic_counts_before);

    let operation = sqlx::query(
        "SELECT response_summary_json FROM operation_log WHERE entrypoint = 'search_memory' ORDER BY occurred_at DESC, operation_id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let summary: Value =
        serde_json::from_str(&operation.get::<String, _>("response_summary_json")).unwrap();
    assert_eq!(summary, json!({"record_type": "claim", "result_count": 0}));
}

#[tokio::test]
async fn search_and_get_memory_hide_mixed_scope_claim_revision_edges_over_stdio() {
    let (mut client, database_url, _database_dir) =
        test_support::spawn_stdio_client_with_database()
            .await
            .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    let project_a = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "World",
                    "namespace": "project/revision-a",
                    "kind": "Observation",
                    "summary": "old scoped claim"
                },
                "claim_drafts": [{
                    "owner": "World",
                    "namespace": "project/revision-a",
                    "subject": "project.claim",
                    "predicate": "is",
                    "object": "old",
                    "mode": "Observed"
                }]
            }),
        )
        .await
        .unwrap();
    let project_b = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "World",
                    "namespace": "project/revision-b",
                    "kind": "Observation",
                    "summary": "foreign replacement claim"
                },
                "claim_drafts": [{
                    "owner": "World",
                    "namespace": "project/revision-b",
                    "subject": "project.claim",
                    "predicate": "is",
                    "object": "new",
                    "mode": "Observed"
                }]
            }),
        )
        .await
        .unwrap();
    let old_claim_id = format!(
        "{}:claim:0",
        project_a["result"]["structuredContent"]["event_id"]
            .as_str()
            .unwrap()
    );
    let new_claim_id = format!(
        "{}:claim:0",
        project_b["result"]["structuredContent"]["event_id"]
            .as_str()
            .unwrap()
    );

    let pool = SqlitePool::connect(&database_url).await.unwrap();
    sqlx::query(
        r#"
        INSERT INTO reflections (
            reflection_id, recorded_at, summary, superseded_claim_id,
            replacement_claim_id, supporting_evidence_event_ids
        ) VALUES (?, ?, ?, ?, ?, '[]')
        "#,
    )
    .bind("reflection-mixed-scope")
    .bind("2026-08-13T00:00:00.000000000Z")
    .bind("persisted mixed-scope revision edge")
    .bind(&old_claim_id)
    .bind(&new_claim_id)
    .execute(&pool)
    .await
    .unwrap();

    for (namespace, claim_id, tool, record_path) in [
        (
            "project/revision-a",
            old_claim_id.as_str(),
            "search_memory",
            "records",
        ),
        (
            "project/revision-b",
            new_claim_id.as_str(),
            "search_memory",
            "records",
        ),
        (
            "project/revision-a",
            old_claim_id.as_str(),
            "get_memory",
            "record",
        ),
        (
            "project/revision-b",
            new_claim_id.as_str(),
            "get_memory",
            "record",
        ),
    ] {
        let response = if tool == "search_memory" {
            client
                .call_tool(
                    tool,
                    json!({
                        "namespace": namespace,
                        "record_type": "Claim",
                        "claim_reference": format!("claim:{claim_id}"),
                        "claim_status": "Active"
                    }),
                )
                .await
                .unwrap()
        } else {
            client
                .call_tool(
                    tool,
                    json!({
                        "namespace": namespace,
                        "id": format!("claim:{claim_id}"),
                        "record_type": "Claim"
                    }),
                )
                .await
                .unwrap()
        };
        let record = if record_path == "records" {
            &response["result"]["structuredContent"]["records"][0]
        } else {
            &response["result"]["structuredContent"]["record"]
        };
        assert_eq!(record["id"], format!("claim:{claim_id}"));
        let provenance = &record["provenance"];
        assert!(
            provenance.get("source_reflection_id").is_none()
                || provenance["source_reflection_id"].is_null(),
            "{tool} {namespace} leaked source_reflection_id: {provenance}"
        );
        assert!(
            provenance.get("superseded_by_reflection_id").is_none()
                || provenance["superseded_by_reflection_id"].is_null(),
            "{tool} {namespace} leaked superseded_by_reflection_id: {provenance}"
        );
        assert!(
            provenance.get("supersedes_claim_reference").is_none()
                || provenance["supersedes_claim_reference"].is_null(),
            "{tool} {namespace} leaked supersedes_claim_reference: {provenance}"
        );
        assert!(
            provenance.get("replacement_claim_reference").is_none()
                || provenance["replacement_claim_reference"].is_null(),
            "{tool} {namespace} leaked replacement_claim_reference: {provenance}"
        );
    }
}

#[tokio::test]
async fn search_memory_returns_scoped_reflection_provenance_and_hides_record_only_over_stdio() {
    let (mut client, database_url, _database_dir) =
        test_support::spawn_stdio_client_with_database()
            .await
            .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    let ingest = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "World",
                    "namespace": "project/reflection-search-a",
                    "kind": "Observation",
                    "summary": "old scoped fact"
                },
                "claim_drafts": [{
                    "owner": "World",
                    "namespace": "project/reflection-search-a",
                    "subject": "project.claim",
                    "predicate": "is",
                    "object": "old",
                    "mode": "Observed"
                }]
            }),
        )
        .await
        .unwrap();
    let event_id = ingest["result"]["structuredContent"]["event_id"]
        .as_str()
        .unwrap();
    let old_claim_id = format!("{event_id}:claim:0");
    let reflection = client
        .call_tool(
            "run_reflection",
            json!({
                "reflection": {"summary": "replace the old scoped fact"},
                "supersede_claim_id": old_claim_id,
                "replacement_claim": {
                    "owner": "World",
                    "namespace": "project/reflection-search-a",
                    "subject": "project.claim",
                    "predicate": "is",
                    "object": "new",
                    "mode": "Observed"
                },
                "replacement_evidence_event_ids": [format!("event:{event_id}")]
            }),
        )
        .await
        .unwrap();
    assert!(
        reflection.get("error").is_none(),
        "reflection failed: {reflection:?}"
    );
    let reflection_id = reflection["result"]["structuredContent"]["reflection_id"]
        .as_str()
        .unwrap();
    let replacement_claim_id = reflection["result"]["structuredContent"]["replacement_claim_id"]
        .as_str()
        .unwrap();

    let pool = SqlitePool::connect(&database_url).await.unwrap();
    sqlx::query(
        r#"
        INSERT INTO reflections (
            reflection_id, recorded_at, summary, superseded_claim_id,
            replacement_claim_id, supporting_evidence_event_ids
        ) VALUES (?, ?, ?, NULL, NULL, '[]')
        "#,
    )
    .bind("reflection-record-only")
    .bind("2026-08-13T00:00:00.000000000Z")
    .bind("record-only must not inherit a namespace")
    .execute(&pool)
    .await
    .unwrap();

    let search = client
        .call_tool(
            "search_memory",
            json!({
                "namespace": "project/reflection-search-a",
                "record_type": "Reflection"
            }),
        )
        .await
        .unwrap();
    let records = search["result"]["structuredContent"]["records"]
        .as_array()
        .cloned()
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["record_type"], "reflection");
    assert_eq!(records[0]["id"], reflection_id);
    assert_eq!(records[0]["namespace"], "project/reflection-search-a");
    assert_eq!(
        records[0]["provenance"]["superseded_claim_reference"],
        format!("claim:{old_claim_id}")
    );
    assert_eq!(
        records[0]["provenance"]["replacement_claim_reference"],
        format!("claim:{replacement_claim_id}")
    );
    assert_eq!(
        records[0]["provenance"]["supporting_evidence_event_references"],
        json!([format!("event:{event_id}")])
    );

    let hidden = client
        .call_tool(
            "search_memory",
            json!({
                "namespace": "project/reflection-search-a",
                "record_type": "Reflection",
                "reflection_reference": "reflection-record-only"
            }),
        )
        .await
        .unwrap();
    assert_eq!(hidden["result"]["structuredContent"]["records"], json!([]));

    let found = client
        .call_tool(
            "get_memory",
            json!({
                "namespace": "project/reflection-search-a",
                "id": reflection_id,
                "record_type": "Reflection"
            }),
        )
        .await
        .unwrap();
    assert!(
        found.get("error").is_none(),
        "reflection lookup failed: {found:?}"
    );
    assert_eq!(found["result"]["structuredContent"]["record"], records[0]);

    for params in [
        json!({
            "namespace": "project/reflection-search-a",
            "id": "reflection-record-only",
            "record_type": "Reflection"
        }),
        json!({
            "namespace": "project/reflection-search-a",
            "id": "reflection-missing",
            "record_type": "Reflection"
        }),
        json!({
            "namespace": "project/empty",
            "id": reflection_id,
            "record_type": "Reflection"
        }),
    ] {
        let missing = client.call_tool("get_memory", params).await.unwrap();
        assert_eq!(
            missing["result"]["structuredContent"]["record"],
            json!(null),
            "missing, record-only, or cross-scope Reflection lookup must return null"
        );
    }
}

#[tokio::test]
async fn get_self_model_history_returns_scoped_identity_and_commitment_audits_over_stdio() {
    let (mut client, database_url, _database_dir) =
        test_support::spawn_stdio_client_with_database()
            .await
            .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    let ingest = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "World",
                    "namespace": "project/self-model-stdio-a",
                    "kind": "Observation",
                    "summary": "scoped evidence for self-model history"
                },
                "claim_drafts": [{
                    "owner": "World",
                    "namespace": "project/self-model-stdio-a",
                    "subject": "project.role",
                    "predicate": "is",
                    "object": "old",
                    "mode": "Observed"
                }]
            }),
        )
        .await
        .unwrap();
    let event_id = ingest["result"]["structuredContent"]["event_id"]
        .as_str()
        .unwrap();
    let old_claim_id = format!("{event_id}:claim:0");
    let reflection = client
        .call_tool(
            "run_reflection",
            json!({
                "reflection": {"summary": "update identity and commitments in this scope"},
                "supersede_claim_id": old_claim_id,
                "replacement_claim": {
                    "owner": "World",
                    "namespace": "project/self-model-stdio-a",
                    "subject": "project.role",
                    "predicate": "is",
                    "object": "new",
                    "mode": "Observed"
                },
                "replacement_evidence_event_ids": [format!("event:{event_id}")],
                "identity_update": {
                    "canonical_claims": ["identity:self=scoped_history"]
                },
                "commitment_updates": [{
                    "owner": "Self_",
                    "description": "prefer:scoped_self_model_history"
                }]
            }),
        )
        .await
        .unwrap();
    assert!(
        reflection.get("error").is_none(),
        "self-model reflection failed: {reflection:?}"
    );
    let reflection_id = reflection["result"]["structuredContent"]["reflection_id"]
        .as_str()
        .unwrap();

    let identity = client
        .call_tool(
            "get_self_model_history",
            json!({
                "namespace": "project/self-model-stdio-a",
                "history_type": "Identity"
            }),
        )
        .await
        .unwrap();
    assert!(
        identity.get("error").is_none(),
        "identity history failed: {identity:?}"
    );
    let identity_result = &identity["result"]["structuredContent"];
    assert_eq!(identity_result["history_type"], "identity");
    assert_eq!(identity_result["has_more"], false);
    assert_eq!(
        identity_result["records"][0]["reflection_id"],
        reflection_id
    );
    assert_eq!(
        identity_result["records"][0]["identity_update"]["canonical_claims"],
        json!(["identity:self=scoped_history"])
    );
    assert!(
        identity_result["records"][0]
            .get("commitment_updates")
            .is_none()
    );

    let commitments = client
        .call_tool(
            "get_self_model_history",
            json!({
                "namespace": "project/self-model-stdio-a",
                "history_type": "Commitment",
                "limit": 10
            }),
        )
        .await
        .unwrap();
    let commitment_records = &commitments["result"]["structuredContent"]["records"];
    assert_eq!(commitment_records[0]["reflection_id"], reflection_id);
    assert_eq!(
        commitment_records[0]["commitment_updates"][0]["description"],
        "prefer:scoped_self_model_history"
    );
    assert!(commitment_records[0].get("identity_update").is_none());

    for invalid in [
        json!({}),
        json!({"namespace": "invalid", "history_type": "Identity"}),
        json!({"namespace": "project/self-model-stdio-a"}),
        json!({
            "namespace": "project/self-model-stdio-a",
            "history_type": "Identity",
            "limit": 0
        }),
    ] {
        let response = client
            .call_tool("get_self_model_history", invalid)
            .await
            .unwrap();
        assert_eq!(
            response["error"]["code"], -32602,
            "invalid self-model history must fail closed: {response:?}"
        );
    }

    let empty = client
        .call_tool(
            "get_self_model_history",
            json!({
                "namespace": "project/self-model-stdio-empty",
                "history_type": "Identity"
            }),
        )
        .await
        .unwrap();
    assert_eq!(empty["result"]["structuredContent"]["records"], json!([]));

    let pool = SqlitePool::connect(&database_url).await.unwrap();
    let operation = sqlx::query(
        "SELECT response_summary_json FROM operation_log WHERE entrypoint = 'get_self_model_history' ORDER BY occurred_at DESC, operation_id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let summary: Value =
        serde_json::from_str(&operation.get::<String, _>("response_summary_json")).unwrap();
    assert_eq!(
        summary,
        json!({"history_type": "identity", "result_count": 0, "has_more": false})
    );
}

#[tokio::test]
async fn supersede_memory_replaces_scoped_claim_without_hard_delete_over_stdio() {
    let (mut client, database_url, _database_dir) =
        test_support::spawn_stdio_client_with_database()
            .await
            .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    let ingest_a = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "World",
                    "namespace": "project/supersede-stdio-a",
                    "kind": "Observation",
                    "summary": "scoped evidence for audited supersede"
                },
                "claim_drafts": [{
                    "owner": "World",
                    "namespace": "project/supersede-stdio-a",
                    "subject": "project.role",
                    "predicate": "is",
                    "object": "old",
                    "mode": "Observed"
                }]
            }),
        )
        .await
        .unwrap();
    let event_a = ingest_a["result"]["structuredContent"]["event_id"]
        .as_str()
        .unwrap()
        .to_string();
    let old_claim_id = format!("{event_a}:claim:0");

    let ingest_b = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "World",
                    "namespace": "project/supersede-stdio-b",
                    "kind": "Observation",
                    "summary": "cross-scope interference"
                },
                "claim_drafts": [{
                    "owner": "World",
                    "namespace": "project/supersede-stdio-b",
                    "subject": "project.role",
                    "predicate": "is",
                    "object": "other",
                    "mode": "Observed"
                }]
            }),
        )
        .await
        .unwrap();
    let event_b = ingest_b["result"]["structuredContent"]["event_id"]
        .as_str()
        .unwrap()
        .to_string();
    let other_claim_id = format!("{event_b}:claim:0");

    let superseded = client
        .call_tool(
            "supersede_memory",
            json!({
                "namespace": "project/supersede-stdio-a",
                "claim_reference": format!("claim:{old_claim_id}"),
                "replacement_claim": {
                    "owner": "World",
                    "namespace": "project/supersede-stdio-a",
                    "subject": "project.role",
                    "predicate": "is",
                    "object": "new",
                    "mode": "Observed"
                },
                "replacement_evidence_event_ids": [format!("event:{event_a}")],
                "summary": "replace the incorrect scoped claim"
            }),
        )
        .await
        .unwrap();
    assert!(
        superseded.get("error").is_none(),
        "scoped supersede failed: {superseded:?}"
    );
    let superseded_result = &superseded["result"]["structuredContent"];
    assert_eq!(superseded_result["durable_write_path"], "run_reflection");
    assert_eq!(
        superseded_result["superseded_claim_reference"],
        format!("claim:{old_claim_id}")
    );
    let replacement_claim_id = superseded_result["replacement_claim_id"]
        .as_str()
        .unwrap()
        .to_string();
    let reflection_id = superseded_result["reflection_id"].as_str().unwrap();

    let pool = SqlitePool::connect(&database_url).await.unwrap();
    let operation = sqlx::query(
        "SELECT response_summary_json FROM operation_log WHERE entrypoint = 'supersede_memory' ORDER BY occurred_at DESC, operation_id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let summary: Value =
        serde_json::from_str(&operation.get::<String, _>("response_summary_json")).unwrap();
    assert_eq!(
        summary,
        json!({"correction_type": "supersede", "durable_write_path": "run_reflection"})
    );

    let active = client
        .call_tool(
            "search_memory",
            json!({
                "namespace": "project/supersede-stdio-a",
                "record_type": "Claim"
            }),
        )
        .await
        .unwrap();
    let active_records = &active["result"]["structuredContent"]["records"];
    assert_eq!(active_records.as_array().unwrap().len(), 1);
    assert_eq!(
        active_records[0]["id"],
        format!("claim:{replacement_claim_id}")
    );
    assert_eq!(active_records[0]["status"], "Active");

    let old_lookup = client
        .call_tool(
            "get_memory",
            json!({
                "namespace": "project/supersede-stdio-a",
                "id": old_claim_id,
                "record_type": "Claim"
            }),
        )
        .await
        .unwrap();
    assert_eq!(
        old_lookup["result"]["structuredContent"]["record"]["status"],
        "Superseded"
    );

    let history = client
        .call_tool(
            "get_reflection_history",
            json!({
                "namespace": "project/supersede-stdio-a",
                "claim_reference": format!("claim:{old_claim_id}")
            }),
        )
        .await
        .unwrap();
    assert_eq!(
        history["result"]["structuredContent"]["reflections"][0]["reflection_id"],
        reflection_id
    );

    for invalid in [
        json!({
            "namespace": "project/supersede-stdio-a",
            "claim_reference": format!("claim:{other_claim_id}"),
            "replacement_claim": {
                "owner": "World",
                "namespace": "project/supersede-stdio-a",
                "subject": "project.role",
                "predicate": "is",
                "object": "leaked",
                "mode": "Observed"
            },
            "replacement_evidence_event_ids": [format!("event:{event_a}")],
            "summary": "cross-scope target"
        }),
        json!({
            "namespace": "project/supersede-stdio-a",
            "claim_reference": format!("claim:{old_claim_id}"),
            "replacement_claim": {
                "owner": "World",
                "namespace": "project/supersede-stdio-a",
                "subject": "project.role",
                "predicate": "is",
                "object": "leaked",
                "mode": "Observed"
            },
            "replacement_evidence_event_ids": [format!("event:{event_b}")],
            "summary": "cross-scope evidence"
        }),
        json!({
            "namespace": "project/supersede-stdio-a",
            "claim_reference": "claim:missing",
            "replacement_claim": {
                "owner": "World",
                "namespace": "project/supersede-stdio-a",
                "subject": "project.role",
                "predicate": "is",
                "object": "missing",
                "mode": "Observed"
            },
            "replacement_evidence_event_ids": [format!("event:{event_a}")],
            "summary": "missing target"
        }),
        json!({
            "namespace": "invalid",
            "claim_reference": format!("claim:{old_claim_id}"),
            "replacement_claim": {
                "owner": "World",
                "namespace": "project/supersede-stdio-a",
                "subject": "project.role",
                "predicate": "is",
                "object": "bad",
                "mode": "Observed"
            },
            "replacement_evidence_event_ids": [format!("event:{event_a}")],
            "summary": "invalid namespace"
        }),
    ] {
        let response = client.call_tool("supersede_memory", invalid).await.unwrap();
        assert_eq!(
            response["error"]["code"], -32602,
            "invalid scoped supersede must fail closed: {response:?}"
        );
    }

    let remaining_old = sqlx::query(
        "SELECT status FROM claims WHERE claim_id = ? AND namespace = 'project/supersede-stdio-a'",
    )
    .bind(&old_claim_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(remaining_old.get::<String, _>("status"), "superseded");
}

#[tokio::test]
async fn get_evidence_relation_returns_scoped_window_and_hides_cross_scope_ids_over_stdio() {
    let (mut client, database_url, _database_dir) =
        test_support::spawn_stdio_client_with_database()
            .await
            .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    let event_a1 = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "World",
                    "namespace": "project/relation-stdio-a",
                    "kind": "Observation",
                    "summary": "first scoped evidence"
                },
                "claim_drafts": []
            }),
        )
        .await
        .unwrap();
    let event_a1_id = event_a1["result"]["structuredContent"]["event_id"]
        .as_str()
        .unwrap()
        .to_string();
    let event_a2 = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "World",
                    "namespace": "project/relation-stdio-a",
                    "kind": "Observation",
                    "summary": "second scoped evidence"
                },
                "claim_drafts": []
            }),
        )
        .await
        .unwrap();
    let event_a2_id = event_a2["result"]["structuredContent"]["event_id"]
        .as_str()
        .unwrap()
        .to_string();
    let event_b = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "World",
                    "namespace": "project/relation-stdio-b",
                    "kind": "Observation",
                    "summary": "cross-scope interferer"
                },
                "claim_drafts": []
            }),
        )
        .await
        .unwrap();
    let event_b_id = event_b["result"]["structuredContent"]["event_id"]
        .as_str()
        .unwrap()
        .to_string();

    let report = client
        .call_tool(
            "get_evidence_relation",
            json!({
                "namespace": "project/relation-stdio-a",
                "trigger_window_event_ids": [
                    format!("event:{event_a2_id}"),
                    event_b_id,
                    "relation-missing",
                    event_a1_id,
                    event_a2_id
                ],
                "selected_evidence_event_ids": [format!("event:{event_a1_id}")],
                "selection_basis": "explicit_model_ids"
            }),
        )
        .await
        .unwrap();
    assert!(
        report.get("error").is_none(),
        "scoped evidence relation failed: {report:?}"
    );
    let structured = &report["result"]["structuredContent"];
    assert_eq!(structured["owner"], "World");
    assert_eq!(structured["namespace"], "project/relation-stdio-a");
    assert_eq!(structured["trigger_window_size"], 2);
    assert_eq!(structured["selected_count"], 1);
    assert_eq!(structured["rejected_count"], 1);
    assert_eq!(
        structured["no_widening_policy"],
        "selected_subset_of_trigger_window"
    );
    assert_eq!(
        structured["relations"],
        json!([
            {
                "event_reference": format!("event:{event_a2_id}"),
                "window_rank": 1,
                "selected": false,
                "relation_status": "available_not_selected",
                "selection_weight": 0,
                "selection_basis": null,
                "rejection_reason": "not_selected_by_current_policy"
            },
            {
                "event_reference": format!("event:{event_a1_id}"),
                "window_rank": 2,
                "selected": true,
                "relation_status": "selected",
                "selection_weight": 100,
                "selection_basis": "explicit_model_ids",
                "rejection_reason": null
            }
        ])
    );

    let rejected = client
        .call_tool(
            "get_evidence_relation",
            json!({
                "namespace": "project/relation-stdio-a",
                "trigger_window_event_ids": [event_a1_id, event_b_id],
                "selected_evidence_event_ids": [event_b_id]
            }),
        )
        .await
        .unwrap();
    assert_eq!(rejected["error"]["code"], -32602);

    for invalid in [
        json!({}),
        json!({"namespace": "invalid", "trigger_window_event_ids": ["evt-1"]}),
        json!({"namespace": "project/relation-stdio-a"}),
        json!({
            "namespace": "project/relation-stdio-a",
            "trigger_window_event_ids": [""]
        }),
        json!({
            "namespace": "project/relation-stdio-a",
            "trigger_window_event_ids": ["evt-1"],
            "selection_basis": "   "
        }),
    ] {
        let response = client
            .call_tool("get_evidence_relation", invalid)
            .await
            .unwrap();
        assert_eq!(
            response["error"]["code"], -32602,
            "invalid evidence relation must fail closed: {response:?}"
        );
    }

    let empty = client
        .call_tool(
            "get_evidence_relation",
            json!({
                "namespace": "project/relation-stdio-a",
                "trigger_window_event_ids": []
            }),
        )
        .await
        .unwrap();
    assert_eq!(empty["result"]["structuredContent"]["relations"], json!([]));

    let pool = SqlitePool::connect(&database_url).await.unwrap();
    let operation = sqlx::query(
        "SELECT response_summary_json FROM operation_log WHERE entrypoint = 'get_evidence_relation' ORDER BY occurred_at DESC, operation_id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let summary: Value =
        serde_json::from_str(&operation.get::<String, _>("response_summary_json")).unwrap();
    assert_eq!(
        summary,
        json!({
            "report_type": "evidence_relation",
            "trigger_window_size": 0,
            "selected_count": 0,
            "result_count": 0
        })
    );
}

#[tokio::test]
async fn search_memory_union_returns_scoped_mixed_records_and_preserves_event_default_over_stdio() {
    let (mut client, database_url, _database_dir) =
        test_support::spawn_stdio_client_with_database()
            .await
            .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    let ingest = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "World",
                    "namespace": "project/union-stdio-a",
                    "kind": "Observation",
                    "summary": "scoped union event"
                },
                "claim_drafts": [{
                    "owner": "World",
                    "namespace": "project/union-stdio-a",
                    "subject": "union.fact",
                    "predicate": "is",
                    "object": "old",
                    "mode": "Observed"
                }],
                "episode_reference": "episode:union-stdio-a"
            }),
        )
        .await
        .unwrap();
    let event_id = ingest["result"]["structuredContent"]["event_id"]
        .as_str()
        .unwrap()
        .to_string();
    let old_claim_id = format!("{event_id}:claim:0");
    let reflection = client
        .call_tool(
            "run_reflection",
            json!({
                "reflection": {"summary": "replace the scoped union claim"},
                "supersede_claim_id": old_claim_id,
                "replacement_claim": {
                    "owner": "World",
                    "namespace": "project/union-stdio-a",
                    "subject": "union.fact",
                    "predicate": "is",
                    "object": "new",
                    "mode": "Observed"
                },
                "replacement_evidence_event_ids": [format!("event:{event_id}")]
            }),
        )
        .await
        .unwrap();
    assert!(
        reflection.get("error").is_none(),
        "reflection failed: {reflection:?}"
    );
    let reflection_id = reflection["result"]["structuredContent"]["reflection_id"]
        .as_str()
        .unwrap();
    let replacement_claim_id = reflection["result"]["structuredContent"]["replacement_claim_id"]
        .as_str()
        .unwrap();
    let _ = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "World",
                    "namespace": "project/union-stdio-b",
                    "kind": "Observation",
                    "summary": "cross-scope interferer"
                },
                "claim_drafts": [{
                    "owner": "World",
                    "namespace": "project/union-stdio-b",
                    "subject": "union.fact",
                    "predicate": "is",
                    "object": "other",
                    "mode": "Observed"
                }],
                "episode_reference": "episode:union-stdio-b"
            }),
        )
        .await
        .unwrap();

    let union = client
        .call_tool(
            "search_memory",
            json!({
                "namespace": "project/union-stdio-a",
                "record_types": ["Event", "Claim", "Episode", "Reflection"],
                "limit": 10
            }),
        )
        .await
        .unwrap();
    assert!(
        union.get("error").is_none(),
        "union search failed: {union:?}"
    );
    let structured = &union["result"]["structuredContent"];
    assert_eq!(
        structured["record_types"],
        json!(["event", "claim", "episode", "reflection"])
    );
    let records = structured["records"].as_array().cloned().unwrap();
    let types = records
        .iter()
        .map(|record| record["record_type"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(types, vec!["reflection", "claim", "event", "episode"]);
    assert_eq!(records[0]["id"], reflection_id);
    assert_eq!(records[1]["id"], format!("claim:{replacement_claim_id}"));
    assert_eq!(records[1]["recorded_at"], records[0]["recorded_at"]);
    assert_eq!(records[2]["id"], format!("event:{event_id}"));
    assert_eq!(records[3]["id"], "episode:union-stdio-a");
    assert!(
        records
            .iter()
            .all(|record| record["namespace"] == "project/union-stdio-a")
    );

    let default_event = client
        .call_tool(
            "search_memory",
            json!({"namespace": "project/union-stdio-a"}),
        )
        .await
        .unwrap();
    let default_records = default_event["result"]["structuredContent"]["records"]
        .as_array()
        .unwrap();
    assert_eq!(default_records.len(), 1);
    assert_eq!(default_records[0]["record_type"], "event");

    for invalid in [
        json!({
            "namespace": "project/union-stdio-a",
            "record_type": "Event",
            "record_types": ["Claim"]
        }),
        json!({
            "namespace": "project/union-stdio-a",
            "record_types": []
        }),
        json!({
            "namespace": "project/union-stdio-a",
            "record_types": ["Event", "Claim"],
            "kind": "Observation"
        }),
    ] {
        let response = client.call_tool("search_memory", invalid).await.unwrap();
        assert_eq!(
            response["error"]["code"], -32602,
            "invalid union search must fail closed: {response:?}"
        );
    }

    let empty = client
        .call_tool(
            "search_memory",
            json!({"namespace": "project/union-stdio-empty"}),
        )
        .await
        .unwrap();
    assert_eq!(empty["result"]["structuredContent"]["records"], json!([]));

    let pool = SqlitePool::connect(&database_url).await.unwrap();
    let operation = sqlx::query(
        "SELECT response_summary_json FROM operation_log WHERE entrypoint = 'search_memory' ORDER BY occurred_at DESC, operation_id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let summary: Value =
        serde_json::from_str(&operation.get::<String, _>("response_summary_json")).unwrap();
    assert_eq!(summary, json!({"record_type": "event", "result_count": 0}));
}

#[tokio::test]
async fn search_memory_invalid_or_empty_scope_fails_closed_over_stdio() {
    let mut client = test_support::spawn_stdio_client().await.unwrap();
    let _ = client.list_all_tools().await.unwrap();

    for invalid in [
        json!({}),
        json!({"namespace": "invalid"}),
        json!({"namespace": "project/a", "limit": 0}),
        json!({"namespace": "project/a", "limit": 101}),
        json!({
            "namespace": "project/a",
            "recorded_after": "2026-07-15T02:00:00Z",
            "recorded_before": "2026-07-15T01:00:00Z"
        }),
        json!({
            "namespace": "project/a",
            "record_type": "Claim",
            "claim_reference": "claim:"
        }),
        json!({
            "namespace": "project/a",
            "record_type": "Claim",
            "recorded_after": "2026-07-15T02:00:00Z"
        }),
        json!({
            "namespace": "project/a",
            "mode": "Observed"
        }),
        json!({
            "namespace": "project/a",
            "record_type": "Episode",
            "episode_reference": "   "
        }),
        json!({
            "namespace": "project/a",
            "record_type": "Episode",
            "episode_reference": " episode:boundary-whitespace"
        }),
        json!({
            "namespace": "project/a",
            "record_type": "Episode",
            "event_reference": "event:not-an-episode-filter"
        }),
        json!({
            "namespace": "project/a",
            "episode_reference": "episode:requires-explicit-episode-type"
        }),
    ] {
        let response = client.call_tool("search_memory", invalid).await.unwrap();
        assert_eq!(
            response["error"]["code"], -32602,
            "invalid search must fail closed: {response:?}"
        );
    }

    let empty = client
        .call_tool("search_memory", json!({"namespace": "project/empty"}))
        .await
        .unwrap();
    assert_eq!(empty["result"]["structuredContent"]["records"], json!([]));

    for invalid in [
        json!({}),
        json!({"namespace": "invalid", "claim_reference": "claim:a"}),
        json!({"namespace": "project/a", "claim_reference": "claim:"}),
        json!({"namespace": "project/a", "claim_reference": "claim:a", "limit": 0}),
        json!({"namespace": "project/a", "claim_reference": "claim:a", "limit": 101}),
    ] {
        let response = client
            .call_tool("get_reflection_history", invalid)
            .await
            .unwrap();
        assert_eq!(
            response["error"]["code"], -32602,
            "invalid history read must fail closed: {response:?}"
        );
    }
}

#[tokio::test]
async fn search_memory_survives_stdio_reconnect_with_offline_provider() {
    let (mut writer, database_url, database_dir) = test_support::spawn_stdio_client_with_database()
        .await
        .unwrap();
    let ingest = writer
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "World",
                    "namespace": "project/reconnect",
                    "kind": "Conversation",
                    "summary": "persist across an MCP reconnect"
                },
                "claim_drafts": [],
                "episode_reference": "episode:reconnect"
            }),
        )
        .await
        .unwrap();
    assert!(ingest.get("error").is_none(), "ingest failed: {ingest:?}");
    drop(writer);

    let config = r#"
transport = "stdio"
database_url = "__DATABASE_URL__"

[model]
provider = "openai-compatible"

[model.openai_compatible]
base_url = "http://127.0.0.1:9/v1"
api_key = "offline-provider-test-key"
model = "gpt-4o-mini"
timeout_ms = 100
"#;
    let mut reader = test_support::spawn_stdio_client_for_existing_database_with_config(
        config,
        &database_url,
        database_dir.path(),
    )
    .unwrap();
    let response = reader
        .call_tool("search_memory", json!({"namespace": "project/reconnect"}))
        .await
        .unwrap();
    assert!(
        response.get("error").is_none(),
        "search failed: {response:?}"
    );
    assert_eq!(
        response["result"]["structuredContent"]["records"][0]["summary"],
        "persist across an MCP reconnect"
    );
    let episode = reader
        .call_tool(
            "search_memory",
            json!({
                "namespace": "project/reconnect",
                "record_type": "Episode",
                "episode_reference": "episode:reconnect"
            }),
        )
        .await
        .unwrap();
    assert!(
        episode.get("error").is_none(),
        "provider-free Episode search failed after reconnect: {episode:?}"
    );
    assert_eq!(
        episode["result"]["structuredContent"]["records"][0]["id"],
        "episode:reconnect"
    );
    let history = reader
        .call_tool(
            "get_reflection_history",
            json!({
                "namespace": "project/reconnect",
                "claim_reference": "claim:missing-after-reconnect"
            }),
        )
        .await
        .unwrap();
    assert!(
        history.get("error").is_none(),
        "provider-free reflection history read failed: {history:?}"
    );
    assert_eq!(
        history["result"]["structuredContent"]["reflections"],
        json!([])
    );
}

async fn semantic_memory_counts(pool: &SqlitePool) -> Vec<i64> {
    let mut counts = Vec::new();
    for table in [
        "events",
        "claims",
        "evidence_links",
        "episode_events",
        "reflections",
        "identity_claims",
        "commitments",
    ] {
        counts.push(
            sqlx::query_scalar::<_, i64>(&format!("SELECT COUNT(*) FROM {table}"))
                .fetch_one(pool)
                .await
                .unwrap(),
        );
    }
    counts
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn build_self_snapshot_can_trigger_periodic_auto_reflection_once_for_explicit_namespace() {
    let stub_response = r#"{"should_reflect":true,"rationale":"Periodic review should tighten commitments after repeated project evidence.","machine_patch":{"identity_patch":null,"commitment_patch":{"commitments":["prefer:review_project_commitments_before_repeating_snapshot_builds"]}}}"#;
    let stub = test_support::StubServer::spawn(
        200,
        json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": stub_response
                }
            }]
        }),
    )
    .await;
    let config = format!(
        r#"
transport = "stdio"
database_url = "__DATABASE_URL__"

[model]
provider = "openai-compatible"

[model.openai_compatible]
base_url = "{}"
api_key = "example-test-key"
model = "gpt-4o-mini"
timeout_ms = 30000
"#,
        stub.base_url()
    );
    let (mut client, database_url, _database_dir) =
        test_support::spawn_stdio_client_with_config_and_database(config)
            .await
            .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    for (summary, episode_reference) in [
        (
            "Observed one project-memory maintenance gap for periodic review.",
            "episode:build-self-snapshot-periodic-0",
        ),
        (
            "Observed another project-memory maintenance gap for periodic review.",
            "episode:build-self-snapshot-periodic-1",
        ),
    ] {
        let ingest = client
            .call_tool(
                "ingest_interaction",
                json!({
                    "event": {
                        "owner": "World",
                        "namespace": "project/agent-llm-mm",
                        "kind": "Observation",
                        "summary": summary
                    },
                    "claim_drafts": [
                        {
                            "owner": "World",
                            "namespace": "project/agent-llm-mm",
                            "subject": "project.memory",
                            "predicate": "needs",
                            "object": "periodic-review",
                            "mode": "Observed"
                        }
                    ],
                    "episode_reference": episode_reference
                }),
            )
            .await
            .unwrap();
        assert!(
            ingest.get("error").is_none(),
            "seed ingest should succeed without depending on auto-reflection: {ingest:?}"
        );
    }

    let first = client
        .call_tool(
            "build_self_snapshot",
            json!({
                "budget": 4,
                "auto_reflect_namespace": "project/agent-llm-mm"
            }),
        )
        .await
        .unwrap();
    let second = client
        .call_tool(
            "build_self_snapshot",
            json!({
                "budget": 4,
                "auto_reflect_namespace": "project/agent-llm-mm"
            }),
        )
        .await
        .unwrap();

    for response in [&first, &second] {
        assert!(
            response.get("error").is_none(),
            "build_self_snapshot should still return a snapshot object: {response:?}"
        );
        let snapshot = response
            .get("result")
            .and_then(|value| value.get("structuredContent"))
            .and_then(|value| value.get("snapshot"));
        assert!(
            snapshot.is_some_and(Value::is_object),
            "build_self_snapshot must preserve the snapshot payload shape: {response:?}"
        );
    }

    let pool = SqlitePool::connect(&database_url).await.unwrap();
    let reflection_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM reflections")
        .fetch_one(&pool)
        .await
        .unwrap();
    let trigger_rows = sqlx::query(
        r#"
        SELECT trigger_type, namespace, trigger_key, status
        FROM reflection_trigger_ledger
        ORDER BY rowid ASC
        "#,
    )
    .fetch_all(&pool)
    .await
    .unwrap()
    .into_iter()
    .map(|row| {
        (
            row.get::<String, _>("trigger_type"),
            row.get::<String, _>("namespace"),
            row.get::<String, _>("trigger_key"),
            row.get::<String, _>("status"),
        )
    })
    .collect::<Vec<_>>();

    assert_eq!(reflection_count, 1);
    assert_eq!(
        trigger_rows,
        vec![
            (
                "periodic".to_string(),
                "project/agent-llm-mm".to_string(),
                "project/agent-llm-mm:periodic".to_string(),
                "handled".to_string(),
            ),
            (
                "periodic".to_string(),
                "project/agent-llm-mm".to_string(),
                "project/agent-llm-mm:periodic".to_string(),
                "suppressed".to_string(),
            ),
        ],
        "explicit build_self_snapshot wiring should record one handled periodic trigger and one suppressed retry"
    );
    assert_eq!(stub.request_count().await, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn build_self_snapshot_returns_snapshot_when_best_effort_periodic_auto_reflection_fails() {
    let stub = test_support::StubServer::spawn(
        200,
        json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "not valid self revision json"
                }
            }]
        }),
    )
    .await;
    let config = format!(
        r#"
transport = "stdio"
database_url = "__DATABASE_URL__"

[model]
provider = "openai-compatible"

[model.openai_compatible]
base_url = "{}"
api_key = "example-test-key"
model = "gpt-4o-mini"
timeout_ms = 30000
"#,
        stub.base_url()
    );
    let (mut client, database_url, _database_dir) =
        test_support::spawn_stdio_client_with_config_and_database(config)
            .await
            .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    for (summary, episode_reference) in [
        (
            "Observed one project-memory maintenance gap before periodic snapshot reflection.",
            "episode:build-self-snapshot-periodic-nonfatal-0",
        ),
        (
            "Observed another project-memory maintenance gap before periodic snapshot reflection.",
            "episode:build-self-snapshot-periodic-nonfatal-1",
        ),
    ] {
        let ingest = client
            .call_tool(
                "ingest_interaction",
                json!({
                    "event": {
                        "owner": "World",
                        "namespace": "project/agent-llm-mm",
                        "kind": "Observation",
                        "summary": summary
                    },
                    "claim_drafts": [
                        {
                            "owner": "World",
                            "namespace": "project/agent-llm-mm",
                            "subject": "project.memory",
                            "predicate": "needs",
                            "object": "periodic-review",
                            "mode": "Observed"
                        }
                    ],
                    "episode_reference": episode_reference
                }),
            )
            .await
            .unwrap();
        assert!(
            ingest.get("error").is_none(),
            "seed ingest should succeed before best-effort periodic auto-reflection is attempted: {ingest:?}"
        );
    }

    let response = client
        .call_tool(
            "build_self_snapshot",
            json!({
                "budget": 4,
                "auto_reflect_namespace": "project/agent-llm-mm"
            }),
        )
        .await
        .unwrap();

    assert!(
        response.get("error").is_none(),
        "build_self_snapshot should not surface periodic auto-reflection failures as MCP errors: {response:?}"
    );
    let snapshot = response
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("snapshot"));
    assert!(
        snapshot.is_some_and(Value::is_object),
        "build_self_snapshot should still return a snapshot object after best-effort periodic auto-reflection fails: {response:?}"
    );

    let pool = SqlitePool::connect(&database_url).await.unwrap();
    let reflection_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM reflections")
        .fetch_one(&pool)
        .await
        .unwrap();
    let trigger_ledger_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM reflection_trigger_ledger")
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(reflection_count, 0);
    assert_eq!(trigger_ledger_count, 0);
    assert_eq!(stub.request_count().await, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn build_self_snapshot_does_not_auto_reflect_without_explicit_namespace() {
    let stub = test_support::StubServer::spawn(
        200,
        json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": r#"{"should_reflect":true,"rationale":"Periodic review is warranted.","machine_patch":{"commitment_patch":{"commitments":["prefer:periodic_project_review"]}}}"#
                }
            }]
        }),
    )
    .await;
    let config = format!(
        r#"
transport = "stdio"
database_url = "__DATABASE_URL__"

[model]
provider = "openai-compatible"

[model.openai_compatible]
base_url = "{}"
api_key = "example-test-key"
model = "gpt-4o-mini"
timeout_ms = 30000
"#,
        stub.base_url()
    );
    let (mut client, database_url, _database_dir) =
        test_support::spawn_stdio_client_with_config_and_database(config)
            .await
            .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    let ingest = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "User",
                    "kind": "Conversation",
                    "summary": "Seed an episode before a snapshot without auto-reflect namespace."
                },
                "claim_drafts": [
                    {
                        "owner": "World",
                        "namespace": "project/agent-llm-mm",
                        "subject": "project.memory",
                        "predicate": "needs",
                        "object": "structure",
                        "mode": "Observed"
                    }
                ],
                "episode_reference": "episode:build-self-snapshot-without-namespace"
            }),
        )
        .await
        .unwrap();
    assert!(
        ingest.get("error").is_none(),
        "seed ingest should succeed before a namespace-free snapshot: {ingest:?}"
    );

    let response = client
        .call_tool("build_self_snapshot", json!({ "budget": 4 }))
        .await
        .unwrap();

    assert!(
        response.get("error").is_none(),
        "build_self_snapshot must still succeed without explicit auto_reflect_namespace: {response:?}"
    );

    let pool = SqlitePool::connect(&database_url).await.unwrap();
    let reflection_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM reflections")
        .fetch_one(&pool)
        .await
        .unwrap();
    let trigger_ledger_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM reflection_trigger_ledger")
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(reflection_count, 0);
    assert_eq!(trigger_ledger_count, 0);
    assert_eq!(stub.request_count().await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_snapshot_filters_are_rejected_before_auto_reflection_side_effects() {
    let stub = test_support::StubServer::spawn(
        200,
        json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": r#"{"should_reflect":false,"rationale":"No revision needed.","machine_patch":{}}"#
                }
            }]
        }),
    )
    .await;
    let config = format!(
        r#"
transport = "stdio"
database_url = "__DATABASE_URL__"

[model]
provider = "openai-compatible"

[model.openai_compatible]
base_url = "{}"
api_key = "example-test-key"
model = "gpt-4o-mini"
timeout_ms = 30000
"#,
        stub.base_url()
    );
    let (mut client, database_url, _database_dir) =
        test_support::spawn_stdio_client_with_config_and_database(config)
            .await
            .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    let ingest = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "World",
                    "namespace": "project/agent-llm-mm",
                    "kind": "Observation",
                    "summary": "Seed one periodic candidate before validating snapshot inputs."
                },
                "claim_drafts": [],
                "episode_reference": "episode:invalid-snapshot-must-not-auto-reflect"
            }),
        )
        .await
        .unwrap();
    assert!(ingest.get("error").is_none());

    for invalid_params in [
        json!({
                "budget": 4,
                "evidence_manifest": [],
                "auto_reflect_namespace": "project/agent-llm-mm"
        }),
        json!({
            "budget": 4,
            "namespace": "project/agent-llm-mm",
            "recorded_after": "2026-07-11T04:00:00Z",
            "recorded_before": "2026-07-11T03:00:00Z",
            "auto_reflect_namespace": "project/agent-llm-mm"
        }),
    ] {
        let response = client
            .call_tool("build_self_snapshot", invalid_params)
            .await
            .unwrap();

        let error = response
            .get("error")
            .expect("invalid snapshot filter should return invalid params");
        assert_eq!(error.get("code").and_then(Value::as_i64), Some(-32602));
    }

    let pool = SqlitePool::connect(&database_url).await.unwrap();
    let reflection_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM reflections")
        .fetch_one(&pool)
        .await
        .unwrap();
    let trigger_ledger_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM reflection_trigger_ledger")
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(reflection_count, 0);
    assert_eq!(trigger_ledger_count, 0);
    assert_eq!(stub.request_count().await, 0);
}

#[tokio::test]
async fn conflicting_reflection_over_stdio_removes_claim_from_active_snapshot() {
    let mut client = test_support::spawn_stdio_client().await.unwrap();
    let _ = client.list_all_tools().await.unwrap();

    let ingest = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "User",
                    "kind": "Conversation",
                    "summary": "The user described a role conflict."
                },
                "claim_drafts": [
                    {
                        "owner": "Self_",
                        "subject": "self.role",
                        "predicate": "is",
                        "object": "architect",
                        "mode": "Observed"
                    }
                ],
                "episode_reference": "episode:task-8"
            }),
        )
        .await
        .unwrap();
    let event_id = ingest
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("event_id"))
        .and_then(Value::as_str)
        .unwrap()
        .to_string();

    let reflection = client
        .call_tool(
            "run_reflection",
            json!({
                "reflection": {
                    "summary": "This reflection conflicts with the previous claim."
                },
                "supersede_claim_id": format!("{event_id}:claim:0"),
                "replacement_claim": null
            }),
        )
        .await
        .unwrap();
    let replacement_claim_id = reflection
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("replacement_claim_id"));
    assert!(
        replacement_claim_id.is_some_and(Value::is_null),
        "conflicting reflection should not create a replacement claim: {reflection:?}"
    );

    let snapshot = client
        .call_tool("build_self_snapshot", json!({ "budget": 4 }))
        .await
        .unwrap();
    let claims = snapshot
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("snapshot"))
        .and_then(|value| value.get("claims"))
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();

    assert!(
        !claims.contains(&"self:self.role is architect"),
        "conflicting reflection should remove disputed claims from active snapshot: {claims:?}"
    );
}

#[tokio::test]
async fn fresh_stdio_runtime_blocks_forbidden_action_with_seeded_commitment() {
    let mut client = test_support::spawn_stdio_client().await.unwrap();
    let _ = client.list_all_tools().await.unwrap();

    client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "User",
                    "kind": "Observation",
                    "summary": "Bootstrap one evidence event so the snapshot can be built."
                },
                "claim_drafts": [],
                "episode_reference": "episode:task-8-gate"
            }),
        )
        .await
        .unwrap();

    let snapshot = client
        .call_tool("build_self_snapshot", json!({ "budget": 4 }))
        .await
        .unwrap();
    let mut snapshot = snapshot
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("snapshot"))
        .cloned()
        .unwrap_or_else(|| panic!("scoped snapshot response missing snapshot: {snapshot:?}"));

    let commitments = snapshot
        .get("commitments")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert!(
        commitments.contains(&"forbid:write_identity_core_directly"),
        "fresh stdio runtime should seed the baseline commitment: {commitments:?}"
    );
    snapshot["commitments"] = json!([]);

    let decision = client
        .call_tool(
            "decide_with_snapshot",
            json!({
                "task": "attempt a forbidden direct identity write",
                "action": "write_identity_core_directly",
                "snapshot": snapshot,
            }),
        )
        .await
        .unwrap();

    let blocked = decision
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("blocked"))
        .and_then(Value::as_bool)
        .unwrap();
    let protocol_version = decision
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("protocol_version"))
        .and_then(Value::as_u64);
    let requested_action = decision
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("requested_action"))
        .and_then(Value::as_str);
    let selected_action = decision
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("selected_action"));
    let gate_blocked = decision
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("gate"))
        .and_then(|value| value.get("blocked"))
        .and_then(Value::as_bool);
    let model_decision = decision
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("decision"));

    assert!(blocked, "baseline commitment should block forbidden action");
    assert_eq!(protocol_version, Some(2));
    assert_eq!(requested_action, Some("write_identity_core_directly"));
    assert!(selected_action.is_some_and(Value::is_null));
    assert_eq!(gate_blocked, Some(true));
    assert!(
        model_decision.is_some_and(Value::is_null),
        "blocked decisions should not call the model: {decision:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_selected_forbidden_action_is_blocked_over_stdio() {
    let stub = test_support::StubServer::spawn(
        200,
        json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "write_identity_core_directly"
                }
            }]
        }),
    )
    .await;
    let config = format!(
        r#"
transport = "stdio"
database_url = "__DATABASE_URL__"

[model]
provider = "openai-compatible"

[model.openai_compatible]
base_url = "{}"
api_key = "example-test-key"
model = "gpt-4o-mini"
timeout_ms = 30000
"#,
        stub.base_url()
    );
    let mut client = test_support::spawn_stdio_client_with_config(config)
        .await
        .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    let response = client
        .call_tool(
            "decide_with_snapshot",
            json!({
                "task": "summarize current memory",
                "action": "read_identity_core",
                "snapshot": {
                    "identity": ["identity:self=architect"],
                    "commitments": [],
                    "claims": ["self.role is architect"],
                    "evidence": ["event:evt-1"],
                    "episodes": ["episode:task-6"]
                }
            }),
        )
        .await
        .unwrap();
    let structured = response
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .expect("structured decision response");

    assert_eq!(structured["blocked"], true, "{response:?}");
    assert_eq!(structured["decision"], Value::Null);
    assert_eq!(structured["requested_action"], "read_identity_core");
    assert_eq!(
        structured["selected_action"],
        "write_identity_core_directly"
    );
    assert_eq!(
        structured["reason"],
        "commitment_gate_blocked_selected_action"
    );
    assert_eq!(structured["gate"]["blocked"], true);
    assert_eq!(
        structured["provider_diagnostics_class"],
        "bounded-local-policy-rejected"
    );
    assert_eq!(stub.request_count().await, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn blocked_decide_with_snapshot_does_not_auto_reflect_conflict_hints() {
    let stub_response = r#"{"should_reflect":true,"rationale":"Conflict suggests tighter commitment hygiene.","machine_patch":{"identity_patch":null,"commitment_patch":{"commitments":["prefer:confirm_conflicting_commitment_updates_before_overwrite"]}}}"#;
    let stub = test_support::StubServer::spawn(
        200,
        json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": stub_response
                }
            }]
        }),
    )
    .await;
    let config = format!(
        r#"
transport = "stdio"
database_url = "__DATABASE_URL__"

[model]
provider = "openai-compatible"

[model.openai_compatible]
base_url = "{}"
api_key = "example-test-key"
model = "gpt-4o-mini"
timeout_ms = 30000
"#,
        stub.base_url()
    );
    let (mut client, database_url, _database_dir) =
        test_support::spawn_stdio_client_with_config_and_database(config)
            .await
            .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "User",
                    "kind": "Observation",
                    "summary": "Bootstrap one evidence event so the blocked decision can still build a snapshot."
                },
                "claim_drafts": [],
                "episode_reference": "episode:blocked-decide-with-snapshot-conflict-auto-reflect"
            }),
        )
        .await
        .unwrap();

    let snapshot = client
        .call_tool("build_self_snapshot", json!({ "budget": 4 }))
        .await
        .unwrap();
    let snapshot = snapshot
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("snapshot"))
        .cloned()
        .unwrap();

    let decision = client
        .call_tool(
            "decide_with_snapshot",
            json!({
                "task": "attempt a forbidden direct identity write with conflict hints",
                "action": "write_identity_core_directly",
                "snapshot": snapshot,
                "auto_reflect_namespace": "self",
                "trigger_hints": ["conflict", "commitment"]
            }),
        )
        .await
        .unwrap();

    let blocked = decision
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("blocked"))
        .and_then(Value::as_bool)
        .unwrap();
    let model_decision = decision
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("decision"));

    assert!(
        decision.get("error").is_none(),
        "blocked decide_with_snapshot must not surface MCP errors: {decision:?}"
    );
    assert!(
        blocked,
        "commitment gate should still block this action: {decision:?}"
    );
    assert!(
        model_decision.is_some_and(Value::is_null),
        "blocked decisions must preserve the original null decision payload: {decision:?}"
    );

    let pool = SqlitePool::connect(&database_url).await.unwrap();
    let reflection_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM reflections")
        .fetch_one(&pool)
        .await
        .unwrap();
    let trigger_ledger_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM reflection_trigger_ledger")
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(reflection_count, 0);
    assert_eq!(trigger_ledger_count, 0);
    assert_eq!(stub.request_count().await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unhinted_decide_with_snapshot_does_not_auto_reflect_conflict_from_existing_evidence() {
    let stub_response = r#"{"should_reflect":true,"rationale":"Conflict suggests tighter commitment hygiene.","machine_patch":{"identity_patch":null,"commitment_patch":{"commitments":["prefer:confirm_conflicting_commitment_updates_before_overwrite"]}}}"#;
    let stub = test_support::StubServer::spawn(
        200,
        json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": stub_response
                }
            }]
        }),
    )
    .await;
    let config = format!(
        r#"
transport = "stdio"
database_url = "__DATABASE_URL__"

[model]
provider = "openai-compatible"

[model.openai_compatible]
base_url = "{}"
api_key = "example-test-key"
model = "gpt-4o-mini"
timeout_ms = 30000
"#,
        stub.base_url()
    );
    let (mut client, database_url, _database_dir) =
        test_support::spawn_stdio_client_with_config_and_database(config)
            .await
            .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "User",
                    "kind": "Conversation",
                    "summary": "Seed one evidence event before an unhinted decision."
                },
                "claim_drafts": [],
                "episode_reference": "episode:decide-with-snapshot-unhinted-conflict-auto-reflect"
            }),
        )
        .await
        .unwrap();

    let snapshot = client
        .call_tool("build_self_snapshot", json!({ "budget": 4 }))
        .await
        .unwrap();
    let snapshot = snapshot
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("snapshot"))
        .cloned()
        .unwrap();

    let decision = client
        .call_tool(
            "decide_with_snapshot",
            json!({
                "task": "resolve a routine commitment update without explicit conflict hints",
                "action": "overwrite_commitment",
                "snapshot": snapshot,
                "auto_reflect_namespace": "self"
            }),
        )
        .await
        .unwrap();

    let blocked = decision
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("blocked"))
        .and_then(Value::as_bool)
        .unwrap();
    let model_decision = decision
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("decision"))
        .cloned();

    assert!(
        decision.get("error").is_none(),
        "unhinted decide_with_snapshot must not surface MCP errors: {decision:?}"
    );
    assert!(
        !blocked,
        "unhinted decide_with_snapshot should preserve the non-blocked decision flow: {decision:?}"
    );
    assert_eq!(
        model_decision,
        Some(json!({ "action": stub_response })),
        "decision payload must remain the original decision response when conflict auto-reflection is not hinted: {decision:?}"
    );

    let pool = SqlitePool::connect(&database_url).await.unwrap();
    let reflection_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM reflections")
        .fetch_one(&pool)
        .await
        .unwrap();
    let trigger_ledger_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM reflection_trigger_ledger")
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(reflection_count, 0);
    assert_eq!(trigger_ledger_count, 0);
    assert_eq!(stub.request_count().await, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn decide_with_snapshot_does_not_auto_reflect_conflict_without_explicit_namespace() {
    let stub_response = r#"{"should_reflect":true,"rationale":"Conflict suggests tighter commitment hygiene.","machine_patch":{"identity_patch":null,"commitment_patch":{"commitments":["prefer:confirm_conflicting_commitment_updates_before_overwrite"]}}}"#;
    let stub = test_support::StubServer::spawn(
        200,
        json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": stub_response
                }
            }]
        }),
    )
    .await;
    let config = format!(
        r#"
transport = "stdio"
database_url = "__DATABASE_URL__"

[model]
provider = "openai-compatible"

[model.openai_compatible]
base_url = "{}"
api_key = "example-test-key"
model = "gpt-4o-mini"
timeout_ms = 30000
"#,
        stub.base_url()
    );
    let (mut client, database_url, _database_dir) =
        test_support::spawn_stdio_client_with_config_and_database(config)
            .await
            .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "User",
                    "kind": "Conversation",
                    "summary": "Seed one evidence event before a namespaceless conflict-hinted decision."
                },
                "claim_drafts": [],
                "episode_reference": "episode:decide-with-snapshot-without-namespace"
            }),
        )
        .await
        .unwrap();

    let snapshot = client
        .call_tool("build_self_snapshot", json!({ "budget": 4 }))
        .await
        .unwrap();
    let snapshot = snapshot
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("snapshot"))
        .cloned()
        .unwrap();

    let decision = client
        .call_tool(
            "decide_with_snapshot",
            json!({
                "task": "resolve a commitment overwrite with explicit conflict hints but no namespace opt-in",
                "action": "overwrite_commitment",
                "snapshot": snapshot,
                "trigger_hints": ["conflict", "commitment"]
            }),
        )
        .await
        .unwrap();

    assert!(
        decision.get("error").is_none(),
        "decide_with_snapshot must still succeed without explicit auto_reflect_namespace: {decision:?}"
    );

    let pool = SqlitePool::connect(&database_url).await.unwrap();
    let reflection_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM reflections")
        .fetch_one(&pool)
        .await
        .unwrap();
    let trigger_ledger_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM reflection_trigger_ledger")
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(reflection_count, 0);
    assert_eq!(trigger_ledger_count, 0);
    assert_eq!(stub.request_count().await, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn decide_with_snapshot_can_trigger_conflict_auto_reflection_without_breaking_decision_flow()
{
    let stub_response = r#"{"should_reflect":true,"rationale":"Conflict suggests tighter commitment hygiene.","machine_patch":{"identity_patch":null,"commitment_patch":{"commitments":["prefer:confirm_conflicting_commitment_updates_before_overwrite"]}}}"#;
    let stub = test_support::StubServer::spawn(
        200,
        json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": stub_response
                }
            }]
        }),
    )
    .await;
    let config = format!(
        r#"
transport = "stdio"
database_url = "__DATABASE_URL__"

[model]
provider = "openai-compatible"

[model.openai_compatible]
base_url = "{}"
api_key = "example-test-key"
model = "gpt-4o-mini"
timeout_ms = 30000
"#,
        stub.base_url()
    );
    let (mut client, database_url, _database_dir) =
        test_support::spawn_stdio_client_with_config_and_database(config)
            .await
            .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "World",
                    "namespace": "world",
                    "kind": "Conversation",
                    "summary": "Seed one evidence event before resolving a conflicting commitment update."
                },
                "claim_drafts": [],
                "episode_reference": "episode:decide-with-snapshot-conflict-auto-reflect"
            }),
        )
        .await
        .unwrap();

    let snapshot = client
        .call_tool(
            "build_self_snapshot",
            json!({ "budget": 4, "namespace": "world" }),
        )
        .await
        .unwrap();
    let snapshot = snapshot
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("snapshot"))
        .cloned()
        .unwrap();

    let decision = client
        .call_tool(
            "decide_with_snapshot",
            json!({
                "task": "resolve a conflicting commitment update",
                "action": "overwrite_commitment",
                "snapshot": snapshot,
                "auto_reflect_namespace": "world",
                "trigger_hints": ["conflict", "commitment"]
            }),
        )
        .await
        .unwrap();

    let blocked = decision
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("blocked"))
        .and_then(Value::as_bool)
        .unwrap();
    let model_decision = decision
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("decision"))
        .cloned();

    assert!(
        decision.get("error").is_none(),
        "conflict auto-reflection must not surface as an MCP error: {decision:?}"
    );
    assert!(
        !blocked,
        "conflict auto-reflection should not block a successful decision flow: {decision:?}"
    );
    assert_eq!(
        model_decision,
        Some(json!({ "action": stub_response })),
        "decision payload must remain in the original shape after conflict auto-reflection: {decision:?}"
    );

    let pool = SqlitePool::connect(&database_url).await.unwrap();
    let reflection_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM reflections")
        .fetch_one(&pool)
        .await
        .unwrap();
    let trigger_ledger_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM reflection_trigger_ledger")
            .fetch_one(&pool)
            .await
            .unwrap();
    let trigger_namespaces = sqlx::query_scalar::<_, String>(
        "SELECT namespace FROM reflection_trigger_ledger ORDER BY rowid ASC",
    )
    .fetch_all(&pool)
    .await
    .unwrap();

    assert_eq!(reflection_count, 1);
    assert_eq!(trigger_ledger_count, 1);
    assert_eq!(trigger_namespaces, vec!["world".to_string()]);
    assert_eq!(stub.request_count().await, 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn decide_with_snapshot_over_stdio_uses_openai_compatible_provider_from_config_file() {
    let stub = test_support::StubServer::spawn(
        200,
        json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "provider_selected_action"
                }
            }]
        }),
    )
    .await;
    let config = format!(
        r#"
transport = "stdio"
database_url = "__DATABASE_URL__"

[model]
provider = "openai-compatible"

[model.openai_compatible]
base_url = "{}"
api_key = "example-test-key"
model = "gpt-4o-mini"
timeout_ms = 30000
"#,
        stub.base_url()
    );
    let mut client = test_support::spawn_stdio_client_with_config(config)
        .await
        .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    let response = client
        .call_tool(
            "decide_with_snapshot",
            json!({
                "task": "summarize current memory",
                "action": "read_identity_core",
                "snapshot": {
                    "identity": ["identity:self=architect"],
                    "commitments": [],
                    "claims": ["self.role is architect"],
                    "evidence": ["event:evt-1"],
                    "episodes": ["episode:task-6"]
                }
            }),
        )
        .await
        .unwrap();

    let action = response
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("decision"))
        .and_then(|value| value.get("action"))
        .and_then(Value::as_str);
    let structured = response
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .expect("structured decision response");

    assert_eq!(
        action,
        Some("provider_selected_action"),
        "unexpected stdio response: {response:?}"
    );
    assert_eq!(structured["protocol_version"], 2);
    assert_eq!(structured["requested_action"], "read_identity_core");
    assert_eq!(structured["selected_action"], "provider_selected_action");
    assert_eq!(structured["confidence"], "bounded-local-metadata");
    assert_eq!(
        structured["decision_authority"],
        "experimental_non_authoritative"
    );
    assert_eq!(structured["policy_scope"], "server_commitment_gate_only");
    assert_eq!(structured["gate"]["name"], "commitment_gate");
    assert_eq!(structured["gate"]["blocked"], false);
    assert!(
        structured["non_claims"]
            .as_array()
            .expect("non_claims array")
            .iter()
            .any(|claim| claim == "not provider-native structured decision JSON")
    );
    assert!(
        structured["non_claims"]
            .as_array()
            .expect("non_claims array")
            .iter()
            .any(|claim| claim == "not an authoritative policy decision")
    );
    assert!(
        !response.to_string().contains("example-test-key"),
        "decision response must not expose provider secrets: {response:?}"
    );
    assert_eq!(
        stub.last_request_path().await.as_deref(),
        Some("/chat/completions")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn decide_with_snapshot_over_stdio_uses_openrouter_provider_from_config_file() {
    let stub = test_support::StubServer::spawn(
        200,
        json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "openrouter_selected_action"
                }
            }]
        }),
    )
    .await;
    let config = format!(
        r#"
transport = "stdio"
database_url = "__DATABASE_URL__"

[model]
provider = "openrouter"

[model.openrouter]
base_url = "{}"
api_key = "example-openrouter-key"
model = "openrouter/test-model"
timeout_ms = 30000
"#,
        stub.base_url()
    );
    let mut client = test_support::spawn_stdio_client_with_config(config)
        .await
        .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    let response = client
        .call_tool(
            "decide_with_snapshot",
            json!({
                "task": "summarize current memory",
                "action": "read_identity_core",
                "snapshot": {
                    "identity": ["identity:self=architect"],
                    "commitments": [],
                    "claims": ["self.role is architect"],
                    "evidence": ["event:evt-1"],
                    "episodes": ["episode:task-6"]
                }
            }),
        )
        .await
        .unwrap();

    let action = response
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("decision"))
        .and_then(|value| value.get("action"))
        .and_then(Value::as_str);

    assert_eq!(
        action,
        Some("openrouter_selected_action"),
        "unexpected stdio response: {response:?}"
    );
    assert_eq!(
        stub.last_request_path().await.as_deref(),
        Some("/chat/completions")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_tool_failure_does_not_persist_provider_error_payload_in_operation_log() {
    let provider_payload_secret = "sk-provider-payload-should-not-persist";
    let stub = test_support::StubServer::spawn(
        500,
        json!({
            "error": {
                "message": provider_payload_secret
            }
        }),
    )
    .await;
    let config = format!(
        r#"
transport = "stdio"
database_url = "__DATABASE_URL__"

[model]
provider = "openai-compatible"

[model.openai_compatible]
base_url = "{}"
api_key = "example-test-key"
model = "gpt-4o-mini"
timeout_ms = 30000
"#,
        stub.base_url()
    );
    let (mut client, database_url, _database_dir) =
        test_support::spawn_stdio_client_with_config_and_database(config)
            .await
            .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    let response = client
        .call_tool(
            "decide_with_snapshot",
            json!({
                "task": "summarize current memory",
                "action": "read_identity_core",
                "snapshot": {
                    "identity": ["identity:self=architect"],
                    "commitments": [],
                    "claims": ["self.role is architect"],
                    "evidence": ["event:evt-1"],
                    "episodes": ["episode:task-6"]
                }
            }),
        )
        .await
        .unwrap();

    let error = response
        .get("error")
        .expect("provider error should still be returned as MCP error");
    assert_eq!(error.get("code").and_then(Value::as_i64), Some(-32603));

    let pool = SqlitePool::connect(&database_url).await.unwrap();
    let row = sqlx::query(
        "SELECT status, diagnostic_summary_json \
         FROM operation_log WHERE entrypoint = 'decide_with_snapshot' \
         ORDER BY occurred_at DESC, operation_id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("provider-backed failed MCP tool call should write operation log entry");

    assert_eq!(row.get::<String, _>("status"), "failed");
    let diagnostic_summary = row
        .get::<Option<String>, _>("diagnostic_summary_json")
        .expect("failed provider operation should include diagnostic summary");
    assert!(
        !diagnostic_summary.contains(provider_payload_secret),
        "operation log must not persist raw provider error payload: {diagnostic_summary}"
    );
    assert!(
        diagnostic_summary.contains("internal error"),
        "diagnostic summary should preserve only the MCP error class: {diagnostic_summary}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dashboard_failed_tool_event_does_not_expose_provider_error_payload() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve port");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);
    let provider_payload_secret = "sk-provider-dashboard-payload-should-not-persist";
    let stub = test_support::StubServer::spawn(
        500,
        json!({
            "error": {
                "message": provider_payload_secret
            }
        }),
    )
    .await;
    let config = format!(
        r#"
transport = "stdio"
database_url = "__DATABASE_URL__"

[model]
provider = "openai-compatible"

[model.openai_compatible]
base_url = "{}"
api_key = "example-test-key"
model = "gpt-4o-mini"
timeout_ms = 30000

[dashboard]
enabled = true
host = "127.0.0.1"
port = {port}
event_capacity = 50
required = true
"#,
        stub.base_url()
    );
    let mut client = test_support::spawn_stdio_client_with_config(config)
        .await
        .expect("client");
    let _ = client.list_all_tools().await.expect("list tools");

    let response = client
        .call_tool(
            "decide_with_snapshot",
            json!({
                "task": "summarize current memory",
                "action": "read_identity_core",
                "snapshot": {
                    "identity": ["identity:self=architect"],
                    "commitments": [],
                    "claims": ["self.role is architect"],
                    "evidence": ["event:evt-1"],
                    "episodes": ["episode:task-6"]
                }
            }),
        )
        .await
        .unwrap();

    let error = response
        .get("error")
        .expect("provider error should still be returned as MCP error");
    assert_eq!(error.get("code").and_then(Value::as_i64), Some(-32603));

    let events: serde_json::Value =
        reqwest::get(format!("http://127.0.0.1:{port}/api/events?limit=10"))
            .await
            .expect("dashboard events response")
            .json()
            .await
            .expect("dashboard events json");
    let failed_event = events
        .as_array()
        .expect("events array")
        .iter()
        .find(|event| {
            event.get("operation").and_then(Value::as_str) == Some("decide_with_snapshot")
                && event.get("status").and_then(Value::as_str) == Some("failed")
        })
        .expect("dashboard should record the failed MCP tool operation");
    let failed_event_json = failed_event.to_string();
    assert!(
        !failed_event_json.contains(provider_payload_secret),
        "dashboard failed event must not expose raw provider payload: {failed_event:?}"
    );
    assert!(
        failed_event_json.contains("internal error"),
        "dashboard failed event should keep a bounded error class: {failed_event:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dashboard_enabled_does_not_corrupt_mcp_stdout_and_records_tool_event() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve port");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);
    let config = r#"
transport = "stdio"
database_url = "__DATABASE_URL__"

[dashboard]
enabled = true
host = "127.0.0.1"
port = __DASHBOARD_PORT__
event_capacity = 50
required = true
"#
    .replace("__DASHBOARD_PORT__", &port.to_string());
    let mut client = test_support::spawn_stdio_client_with_config(config)
        .await
        .expect("client");

    let tools = client.list_all_tools().await.expect("list tools");
    assert_eq!(tools.len(), 31);

    let health: serde_json::Value = reqwest::get(format!("http://127.0.0.1:{port}/api/health"))
        .await
        .expect("dashboard health response")
        .json()
        .await
        .expect("dashboard health json");
    assert_eq!(health["status"], "ok");

    let response = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "User",
                    "kind": "Conversation",
                    "summary": "Dashboard should record this MCP operation."
                },
                "claim_drafts": [],
                "episode_reference": "episode:dashboard-stdio-smoke"
            }),
        )
        .await
        .expect("ingest response");

    assert!(
        response.get("result").is_some(),
        "dashboard logs must not corrupt MCP stdout: {response:?}"
    );

    let events: serde_json::Value =
        reqwest::get(format!("http://127.0.0.1:{port}/api/events?limit=10"))
            .await
            .expect("dashboard events response")
            .json()
            .await
            .expect("dashboard events json");
    let event_operations = events
        .as_array()
        .expect("events array")
        .iter()
        .filter_map(|event| event.get("operation").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert!(
        event_operations.contains(&"ingest_interaction"),
        "dashboard should record the MCP tool operation: {events:?}"
    );
    let ingest_correlation_id = events
        .as_array()
        .expect("events array")
        .iter()
        .find(|event| event.get("operation").and_then(Value::as_str) == Some("ingest_interaction"))
        .and_then(|event| event.get("correlation_id"))
        .and_then(Value::as_str);
    assert!(
        ingest_correlation_id
            .is_some_and(|correlation_id| correlation_id.starts_with("mcp-tool-call-")),
        "dashboard tool event should expose generated MCP correlation id: {events:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serve_starts_observe_only_daemon_when_enabled_without_semantic_writes() {
    let config = r#"
transport = "stdio"
database_url = "__DATABASE_URL__"

[daemon]
enabled = true
poll_interval_ms = 10
max_concurrent_tasks = 1
"#
    .to_string();
    let (mut client, database_url, _database_dir) =
        test_support::spawn_stdio_client_with_config_and_database(config)
            .await
            .unwrap();
    let tools = client.list_all_tools().await.unwrap();
    assert_eq!(tools.len(), 31);

    client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "User",
                    "kind": "Conversation",
                    "summary": "Seed one event before observing daemon-enabled stdio serve."
                },
                "claim_drafts": [],
                "episode_reference": "episode:observe-only-daemon-stdio"
            }),
        )
        .await
        .expect("ingest response");

    let response = client
        .call_tool("build_self_snapshot", json!({ "budget": 4 }))
        .await
        .expect("snapshot response");
    assert!(
        response.get("result").is_some(),
        "observe-only daemon must not corrupt MCP stdio: {response:?}"
    );

    let pool = SqlitePool::connect(&database_url).await.unwrap();
    let reflection_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM reflections")
        .fetch_one(&pool)
        .await
        .unwrap();
    let trigger_ledger_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM reflection_trigger_ledger")
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(reflection_count, 0);
    assert_eq!(trigger_ledger_count, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dashboard_exposes_durable_operation_log_history_for_mcp_calls() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve port");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);
    let config = r#"
transport = "stdio"
database_url = "__DATABASE_URL__"

[dashboard]
enabled = true
host = "127.0.0.1"
port = __DASHBOARD_PORT__
event_capacity = 50
required = true
"#
    .replace("__DASHBOARD_PORT__", &port.to_string());
    let mut client = test_support::spawn_stdio_client_with_config(config)
        .await
        .expect("client");

    let _ = client.list_all_tools().await.expect("list tools");
    let response = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "User",
                    "kind": "Conversation",
                    "summary": "Dashboard should expose durable history for this MCP call."
                },
                "claim_drafts": [],
                "episode_reference": "episode:dashboard-operation-log-history"
            }),
        )
        .await
        .expect("ingest response");
    assert!(
        response.get("result").is_some(),
        "tool call should succeed before operation-log history assertion: {response:?}"
    );

    let events: serde_json::Value =
        reqwest::get(format!("http://127.0.0.1:{port}/api/events?limit=10"))
            .await
            .expect("dashboard events response")
            .json()
            .await
            .expect("dashboard events json");
    let correlation_id = events
        .as_array()
        .expect("events array")
        .iter()
        .find(|event| event.get("operation").and_then(Value::as_str) == Some("ingest_interaction"))
        .and_then(|event| event.get("correlation_id"))
        .and_then(Value::as_str)
        .expect("dashboard event should include correlation id")
        .to_string();

    let history: serde_json::Value = reqwest::get(format!(
        "http://127.0.0.1:{port}/api/operation-log?correlation_id={correlation_id}&limit=5"
    ))
    .await
    .expect("operation-log history response")
    .json()
    .await
    .expect("operation-log history json");
    let entries = history.as_array().expect("history array");
    assert_eq!(
        entries.len(),
        1,
        "dashboard operation-log history should expose the matching durable entry: {history:?}"
    );
    assert_eq!(entries[0]["operation"], "ingest_interaction");
    assert_eq!(entries[0]["kind"], "tool");
    assert_eq!(entries[0]["status"], "ok");
    assert_eq!(entries[0]["correlation_id"], correlation_id);
    assert_eq!(entries[0]["read_only"], true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dashboard_records_distinct_correlation_ids_for_distinct_mcp_calls() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve port");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);
    let config = r#"
transport = "stdio"
database_url = "__DATABASE_URL__"

[dashboard]
enabled = true
host = "127.0.0.1"
port = __DASHBOARD_PORT__
event_capacity = 50
required = true
"#
    .replace("__DASHBOARD_PORT__", &port.to_string());
    let mut client = test_support::spawn_stdio_client_with_config(config)
        .await
        .expect("client");

    let _ = client.list_all_tools().await.expect("list tools");
    client
        .call_tool("build_self_snapshot", json!({ "budget": 4 }))
        .await
        .expect("first snapshot response");
    client
        .call_tool("build_self_snapshot", json!({ "budget": 4 }))
        .await
        .expect("second snapshot response");

    let events: serde_json::Value =
        reqwest::get(format!("http://127.0.0.1:{port}/api/events?limit=10"))
            .await
            .expect("dashboard events response")
            .json()
            .await
            .expect("dashboard events json");
    let correlation_ids = events
        .as_array()
        .expect("events array")
        .iter()
        .filter(|event| {
            event.get("operation").and_then(Value::as_str) == Some("build_self_snapshot")
        })
        .filter_map(|event| event.get("correlation_id").and_then(Value::as_str))
        .collect::<Vec<_>>();

    assert_eq!(
        correlation_ids.len(),
        2,
        "each dashboard tool event should carry a correlation id: {events:?}"
    );
    assert_ne!(
        correlation_ids[0], correlation_ids[1],
        "distinct MCP calls should have distinct correlation ids"
    );
}

#[tokio::test]
async fn mcp_tool_calls_append_operation_logs_with_correlation_id_and_snapshot_scope() {
    let (mut client, database_url, _database_dir) =
        test_support::spawn_stdio_client_with_database()
            .await
            .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    let response = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "User",
                    "kind": "Conversation",
                    "summary": "Operation log should record this MCP call with a correlation id."
                },
                "claim_drafts": [],
                "episode_reference": "episode:operation-log-correlation"
            }),
        )
        .await
        .expect("ingest response");
    assert!(
        response.get("result").is_some(),
        "tool call should succeed before operation log assertion: {response:?}"
    );

    let pool = SqlitePool::connect(&database_url).await.unwrap();
    let row = sqlx::query(
        "SELECT entrypoint, operation_kind, status, correlation_id FROM operation_log WHERE entrypoint = 'ingest_interaction' ORDER BY occurred_at DESC, operation_id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("operation log entry should be written for MCP tool call");

    assert_eq!(row.get::<String, _>("entrypoint"), "ingest_interaction");
    assert_eq!(row.get::<String, _>("operation_kind"), "tool");
    assert_eq!(row.get::<String, _>("status"), "ok");
    let correlation_id = row.get::<Option<String>, _>("correlation_id");
    assert!(
        correlation_id
            .as_deref()
            .is_some_and(|correlation_id| correlation_id.starts_with("mcp-tool-call-")),
        "operation log entry should include generated MCP correlation id"
    );

    let snapshot = client
        .call_tool(
            "build_self_snapshot",
            json!({
                "budget": 4,
                "namespace": "user/default"
            }),
        )
        .await
        .expect("scoped snapshot response");
    assert!(
        snapshot.get("error").is_none(),
        "scoped snapshot should succeed before operation log assertion: {snapshot:?}"
    );

    let snapshot_row = sqlx::query(
        "SELECT namespace, correlation_id FROM operation_log WHERE entrypoint = 'build_self_snapshot' AND status = 'ok' ORDER BY occurred_at DESC, operation_id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("scoped snapshot operation log entry should be written");
    assert_eq!(
        snapshot_row
            .get::<Option<String>, _>("namespace")
            .as_deref(),
        Some("user/default")
    );
    assert!(
        snapshot_row
            .get::<Option<String>, _>("correlation_id")
            .as_deref()
            .is_some_and(|correlation_id| correlation_id.starts_with("mcp-tool-call-"))
    );
}

/// 驱动一次 server 端「积累 → handled → cooldown 抑制」的失败 auto-reflection 流：
/// 三次 ingest 后第三次会因 cooldown_active 被抑制。返回数据库 URL（与 doctor 共用），
/// 以及保持数据库目录存活的 guard。供 B1 的两条断言（operation log 写入 / doctor 计数）复用。
async fn drive_suppressed_auto_reflection_over_stdio() -> (String, TempDir) {
    let (mut client, database_url, database_dir) = test_support::spawn_stdio_client_with_database()
        .await
        .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    // 先累积两条 Self/Action 事件（无 failure 提示 → 不触发，故不调用模型）。
    // 失败触发的证据窗口阈值为 2，凑满后第三次带提示的 ingest 才会进入抑制评估。
    for index in 0..2 {
        client
            .call_tool(
                "ingest_interaction",
                json!({
                    "event": {
                        "owner": "Self_",
                        "kind": "Action",
                        "summary": format!("rollback #{index} after violating a hard commitment")
                    },
                    "claim_drafts": [],
                    "episode_reference": format!("episode:suppressed-auto-reflect-{index}")
                }),
            )
            .await
            .unwrap();
    }

    // 种入一条 Handled 触发账本记录并设置仍在窗口内的 cooldown，作为「先前已处理过反思」的前置条件。
    // 注意：这里种的是触发账本（cooldown 来源），而非 doctor 计数的 operation_log；
    // operation_log 的 Trigger/Suppressed 条目仍由下一步真实 ingest 在运行时写入，故仍验证了修复点。
    let pool = SqlitePool::connect(&database_url).await.unwrap();
    let cooldown_until = (chrono::Utc::now() + chrono::Duration::hours(24)).to_rfc3339();
    let handled_at = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO reflection_trigger_ledger \
         (ledger_id, trigger_type, namespace, trigger_key, status, evidence_window, handled_at, cooldown_until, episode_watermark, reflection_id) \
         VALUES (?, 'failure', 'self', 'self:failure', 'handled', '[\"seed-evidence\"]', ?, ?, NULL, ?)",
    )
    .bind("ledger-seed-handled")
    .bind(&handled_at)
    .bind(&cooldown_until)
    .bind("reflection-seed-handled")
    .execute(&pool)
    .await
    .expect("seed a handled trigger ledger entry to establish the cooldown precondition");

    // 第三次 ingest 带 failure 提示：should_consider 为真（≥2 条证据 + 提示），
    // 但 cooldown 仍在窗口内 → 在调用模型之前即被 cooldown_active 抑制（不产生模型请求）。
    client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "Self_",
                    "kind": "Action",
                    "summary": "rollback #2 after violating a hard commitment"
                },
                "claim_drafts": [],
                "episode_reference": "episode:suppressed-auto-reflect-2",
                "trigger_hints": ["failure", "rollback"]
            }),
        )
        .await
        .unwrap();

    (database_url, database_dir)
}

/// B1：被抑制的 auto-reflection 必须在 durable operation log 留下一条 Trigger/Suppressed 条目，
/// 且诊断里带可解释的 cooldown_active。此前无任何代码写 Trigger 条目，移除写入则此断言失败。
#[tokio::test]
async fn suppressed_auto_reflection_appends_trigger_operation_log_entry() {
    let (database_url, _database_dir) = drive_suppressed_auto_reflection_over_stdio().await;

    let pool = SqlitePool::connect(&database_url).await.unwrap();
    let row = sqlx::query(
        "SELECT operation_kind, status, diagnostic_summary_json FROM operation_log \
         WHERE operation_kind = 'trigger' AND status = 'suppressed' \
         ORDER BY occurred_at DESC, operation_id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("suppressed auto-reflection must append a trigger operation log entry");

    assert_eq!(row.get::<String, _>("operation_kind"), "trigger");
    assert_eq!(row.get::<String, _>("status"), "suppressed");
    let diagnostic = row
        .get::<Option<String>, _>("diagnostic_summary_json")
        .expect("trigger operation log entry should carry diagnostic summary");
    assert!(
        diagnostic.contains("cooldown_active"),
        "trigger diagnostic should explain the suppression reason: {diagnostic}"
    );
}

/// B1：doctor 的 trigger_candidates_suppressed 此前因无生产写入而运行时恒为 0。
/// 经真实 server 抑制流后（不手动 seed operation log），doctor 必须读到非零计数。
#[tokio::test]
async fn doctor_counts_runtime_suppressed_auto_reflection_without_manual_seed() {
    use agent_llm_mm::support::config::{AppConfig, DaemonConfig};

    let (database_url, _database_dir) = drive_suppressed_auto_reflection_over_stdio().await;

    let report = agent_llm_mm::run_doctor(AppConfig {
        database_url,
        daemon: DaemonConfig {
            enabled: true,
            poll_interval_ms: 250,
            max_concurrent_tasks: 1,
        },
        ..Default::default()
    })
    .await
    .expect("doctor should read runtime operation-log diagnostics");

    assert!(
        report.daemon_observe_only.trigger_candidates_suppressed >= 1,
        "doctor must count runtime-suppressed auto-reflection candidates, got {}",
        report.daemon_observe_only.trigger_candidates_suppressed
    );
    assert_eq!(report.daemon_observe_only.read_errors, Vec::<String>::new());
}

#[tokio::test]
async fn mcp_tool_failure_appends_failed_operation_log_without_changing_error_semantics() {
    let (mut client, database_url, _database_dir) =
        test_support::spawn_stdio_client_with_database()
            .await
            .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    let response = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "User",
                    "kind": "Conversation",
                    "summary": "This should fail because the namespace is incompatible."
                },
                "claim_drafts": [
                    {
                        "owner": "Self_",
                        "namespace": "user/default",
                        "subject": "self.role",
                        "predicate": "is",
                        "object": "architect",
                        "mode": "Observed"
                    }
                ],
                "episode_reference": null
            }),
        )
        .await
        .unwrap();

    let error = response
        .get("error")
        .expect("invalid params should still return an MCP error");
    assert_eq!(error.get("code").and_then(Value::as_i64), Some(-32602));

    let pool = SqlitePool::connect(&database_url).await.unwrap();
    let row = sqlx::query(
        "SELECT entrypoint, operation_kind, status, correlation_id, diagnostic_summary_json \
         FROM operation_log WHERE entrypoint = 'ingest_interaction' \
         ORDER BY occurred_at DESC, operation_id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("failed MCP tool call should write an operation log entry");

    assert_eq!(row.get::<String, _>("entrypoint"), "ingest_interaction");
    assert_eq!(row.get::<String, _>("operation_kind"), "tool");
    assert_eq!(row.get::<String, _>("status"), "failed");
    let correlation_id = row.get::<Option<String>, _>("correlation_id");
    assert!(
        correlation_id
            .as_deref()
            .is_some_and(|correlation_id| correlation_id.starts_with("mcp-tool-call-")),
        "failed operation log entry should include generated MCP correlation id"
    );
    let diagnostic_summary = row
        .get::<Option<String>, _>("diagnostic_summary_json")
        .expect("failed operation should include diagnostic summary");
    assert!(
        diagnostic_summary.contains("invalid params"),
        "diagnostic summary should preserve the error class: {diagnostic_summary}"
    );
}

#[tokio::test]
async fn handler_reached_missing_fields_append_failed_operation_log_without_changing_error_semantics()
 {
    let (mut client, database_url, _database_dir) =
        test_support::spawn_stdio_client_with_database()
            .await
            .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    let response = client
        .call_tool("ingest_interaction", json!({}))
        .await
        .unwrap();

    let error = response
        .get("error")
        .expect("handler-reached missing fields should still return an MCP error");
    assert_eq!(error.get("code").and_then(Value::as_i64), Some(-32602));

    let pool = SqlitePool::connect(&database_url).await.unwrap();
    let row = sqlx::query(
        "SELECT entrypoint, operation_kind, status, correlation_id, diagnostic_summary_json \
         FROM operation_log WHERE entrypoint = 'ingest_interaction' \
         ORDER BY occurred_at DESC, operation_id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("handler-reached missing fields should write a failed operation log entry");

    assert_eq!(row.get::<String, _>("entrypoint"), "ingest_interaction");
    assert_eq!(row.get::<String, _>("operation_kind"), "tool");
    assert_eq!(row.get::<String, _>("status"), "failed");
    let correlation_id = row.get::<Option<String>, _>("correlation_id");
    assert!(
        correlation_id
            .as_deref()
            .is_some_and(|correlation_id| correlation_id.starts_with("mcp-tool-call-")),
        "failed operation log entry should include generated MCP correlation id"
    );
    let diagnostic_summary = row
        .get::<Option<String>, _>("diagnostic_summary_json")
        .expect("failed operation should include diagnostic summary");
    assert!(
        diagnostic_summary.contains("invalid params"),
        "diagnostic summary should preserve the MCP error class: {diagnostic_summary}"
    );
    assert!(
        diagnostic_summary.contains("missing field"),
        "diagnostic summary should preserve bounded parameter-shape detail: {diagnostic_summary}"
    );
}

#[tokio::test]
async fn non_object_mcp_tool_arguments_do_not_reach_handler_operation_log() {
    let (mut client, database_url, _database_dir) =
        test_support::spawn_stdio_client_with_database()
            .await
            .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    let response = client
        .call_tool("ingest_interaction", json!("not-an-object"))
        .await;

    if let Ok(response) = response {
        assert!(
            response.get("error").is_some(),
            "non-object arguments should not produce a successful tool result: {response:?}"
        );
    }

    let pool = SqlitePool::connect(&database_url).await.unwrap();
    let entry_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM operation_log WHERE entrypoint = 'ingest_interaction'",
    )
    .fetch_one(&pool)
    .await
    .expect("operation log count should be queryable");

    assert_eq!(
        entry_count, 0,
        "framework-level non-object arguments should not be reported as handler operation logs"
    );
}

#[tokio::test]
async fn invalid_namespace_is_reported_as_invalid_params_over_stdio() {
    let mut client = test_support::spawn_stdio_client().await.unwrap();
    let _ = client.list_all_tools().await.unwrap();

    let response = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "User",
                    "kind": "Conversation",
                    "summary": "This should fail because the namespace is incompatible."
                },
                "claim_drafts": [
                    {
                        "owner": "Self_",
                        "namespace": "user/default",
                        "subject": "self.role",
                        "predicate": "is",
                        "object": "architect",
                        "mode": "Observed"
                    }
                ],
                "episode_reference": null
            }),
        )
        .await
        .unwrap();

    let error = response
        .get("error")
        .expect("invalid params should return error");
    assert_eq!(error.get("code").and_then(Value::as_i64), Some(-32602));
}

#[tokio::test]
async fn inferred_replacement_reflection_with_evidence_is_accepted_over_stdio() {
    let (mut client, database_url, _database_dir) =
        test_support::spawn_stdio_client_with_database()
            .await
            .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    let mut evidence_event_ids = Vec::new();
    for summary in [
        "The first external observation supports the inferred replacement.",
        "The second external observation independently supports the inferred replacement.",
    ] {
        let response = client
            .call_tool(
                "ingest_interaction",
                json!({
                    "event": {
                        "owner": "World",
                        "namespace": "world",
                        "kind": "Observation",
                        "summary": summary
                    },
                    "claim_drafts": [],
                    "episode_reference": "episode:reflection-evidence-source"
                }),
            )
            .await
            .unwrap();
        let event_id = response
            .get("result")
            .and_then(|value| value.get("structuredContent"))
            .and_then(|value| value.get("event_id"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap();
        evidence_event_ids.push(event_id);
    }

    let ingest = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "User",
                    "kind": "Conversation",
                    "summary": "The user suggested the role may have evolved."
                },
                "claim_drafts": [
                    {
                        "owner": "Self_",
                        "subject": "self.role",
                        "predicate": "is",
                        "object": "architect",
                        "mode": "Observed"
                    }
                ],
                "episode_reference": "episode:reflection-evidence"
            }),
        )
        .await
        .unwrap();
    let superseded_claim_id = ingest
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("event_id"))
        .and_then(Value::as_str)
        .map(|event_id| format!("{event_id}:claim:0"))
        .unwrap();

    let expected_evidence_event_ids = evidence_event_ids.clone();
    let mixed_evidence_event_ids = vec![
        format!("event:{}", evidence_event_ids[0]),
        evidence_event_ids[0].clone(),
        format!("event:{}", evidence_event_ids[1]),
    ];
    let reflection = client
        .call_tool(
            "run_reflection",
            json!({
                "reflection": {
                    "summary": "Two external observations support promoting the inferred replacement."
                },
                "supersede_claim_id": superseded_claim_id,
                "replacement_claim": {
                    "owner": "Self_",
                    "subject": "self.role",
                    "predicate": "is",
                    "object": "principal_architect",
                    "mode": "Inferred"
                },
                "replacement_evidence_event_ids": mixed_evidence_event_ids
            }),
        )
        .await
        .unwrap();

    let replacement_claim_id = reflection
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("replacement_claim_id"))
        .and_then(Value::as_str);
    assert!(
        replacement_claim_id.is_some_and(|claim_id| claim_id.ends_with(":replacement")),
        "replacement claim id should be present and use the reflection replacement suffix: {reflection:?}"
    );
    let reflection_id = reflection
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("reflection_id"))
        .and_then(Value::as_str)
        .expect("reflection result should expose its audit id");
    let pool = SqlitePool::connect(&database_url).await.unwrap();
    let stored_event_ids = sqlx::query_scalar::<_, String>(
        "SELECT supporting_evidence_event_ids FROM reflections WHERE reflection_id = ?",
    )
    .bind(reflection_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        serde_json::from_str::<Vec<String>>(&stored_event_ids).unwrap(),
        expected_evidence_event_ids,
        "MCP raw and canonical references must persist as one ordered raw-id audit list"
    );
}

#[tokio::test]
async fn missing_replacement_evidence_event_ids_are_invalid_params_over_stdio() {
    let mut client = test_support::spawn_stdio_client().await.unwrap();
    let _ = client.list_all_tools().await.unwrap();

    let ingest = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "User",
                    "kind": "Conversation",
                    "summary": "The user suggested the role may have evolved."
                },
                "claim_drafts": [
                    {
                        "owner": "Self_",
                        "subject": "self.role",
                        "predicate": "is",
                        "object": "architect",
                        "mode": "Observed"
                    }
                ],
                "episode_reference": "episode:reflection-missing-evidence"
            }),
        )
        .await
        .unwrap();
    let superseded_claim_id = ingest
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("event_id"))
        .and_then(Value::as_str)
        .map(|event_id| format!("{event_id}:claim:0"))
        .unwrap();

    let reflection = client
        .call_tool(
            "run_reflection",
            json!({
                "reflection": {
                    "summary": "Unknown evidence ids should be rejected before persistence."
                },
                "supersede_claim_id": superseded_claim_id,
                "replacement_claim": {
                    "owner": "Self_",
                    "subject": "self.role",
                    "predicate": "is",
                    "object": "principal_architect",
                    "mode": "Inferred"
                },
                "replacement_evidence_event_ids": ["evt-missing"]
            }),
        )
        .await
        .unwrap();

    let error = reflection
        .get("error")
        .expect("unknown evidence ids should be reported as invalid params");
    assert_eq!(error.get("code").and_then(Value::as_i64), Some(-32602));
}

#[tokio::test]
async fn reflected_claim_replacement_query_is_accepted_over_stdio() {
    let mut client = test_support::spawn_stdio_client().await.unwrap();
    let _ = client.list_all_tools().await.unwrap();

    for summary in ["World observed update A.", "World observed update B."] {
        client
            .call_tool(
                "ingest_interaction",
                json!({
                    "event": {
                        "owner": "World",
                        "namespace": "world",
                        "kind": "Observation",
                        "summary": summary
                    },
                    "claim_drafts": [],
                    "episode_reference": "episode:reflection-query-source"
                }),
            )
            .await
            .unwrap();
    }

    let ingest = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "User",
                    "kind": "Conversation",
                    "summary": "The role may still evolve."
                },
                "claim_drafts": [
                    {
                        "owner": "Self_",
                        "subject": "self.role",
                        "predicate": "is",
                        "object": "architect",
                        "mode": "Observed"
                    }
                ],
                "episode_reference": "episode:reflection-query-target"
            }),
        )
        .await
        .unwrap();
    let superseded_claim_id = ingest
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("event_id"))
        .and_then(Value::as_str)
        .map(|event_id| format!("{event_id}:claim:0"))
        .unwrap();

    let reflection = client
        .call_tool(
            "run_reflection",
            json!({
                "reflection": {
                    "summary": "Query-based evidence lookup should still allow replacement."
                },
                "supersede_claim_id": superseded_claim_id,
                "replacement_claim": {
                    "owner": "Self_",
                    "subject": "self.role",
                    "predicate": "is",
                    "object": "principal_architect",
                    "mode": "Inferred"
                },
                "replacement_evidence_query": {
                    "namespace": "world",
                    "owner": "World",
                    "kind": "Observation",
                    "limit": 2
                }
            }),
        )
        .await
        .unwrap();

    let replacement_claim_id = reflection
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("replacement_claim_id"))
        .and_then(Value::as_str);

    assert!(
        replacement_claim_id.is_some_and(|claim_id| claim_id.ends_with(":replacement")),
        "query-based replacement should resolve a replacement claim id: {reflection:?}"
    );
}

#[tokio::test]
async fn reflected_claim_replacement_query_without_matches_is_invalid_params_over_stdio() {
    let mut client = test_support::spawn_stdio_client().await.unwrap();
    let _ = client.list_all_tools().await.unwrap();

    let ingest = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "User",
                    "kind": "Conversation",
                    "summary": "No world observations are stored for this test."
                },
                "claim_drafts": [
                    {
                        "owner": "Self_",
                        "subject": "self.role",
                        "predicate": "is",
                        "object": "architect",
                        "mode": "Observed"
                    }
                ],
                "episode_reference": "episode:reflection-query-missing"
            }),
        )
        .await
        .unwrap();
    let superseded_claim_id = ingest
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("event_id"))
        .and_then(Value::as_str)
        .map(|event_id| format!("{event_id}:claim:0"))
        .unwrap();

    let reflection = client
        .call_tool(
            "run_reflection",
            json!({
                "reflection": {
                    "summary": "Query returns nothing, so this should be rejected."
                },
                "supersede_claim_id": superseded_claim_id,
                "replacement_claim": {
                    "owner": "Self_",
                    "subject": "self.role",
                    "predicate": "is",
                    "object": "principal_architect",
                    "mode": "Inferred"
                },
                "replacement_evidence_query": {
                    "namespace": "project/missing",
                    "owner": "World",
                    "kind": "Conversation",
                    "limit": 3
                }
            }),
        )
        .await
        .unwrap();

    let error = reflection
        .get("error")
        .expect("query without matching evidence should return invalid params");
    assert_eq!(error.get("code").and_then(Value::as_i64), Some(-32602));
}

#[tokio::test]
async fn reflection_identity_and_commitment_updates_are_applied_and_audited_over_stdio() {
    let (mut client, database_url, _database_dir) =
        test_support::spawn_stdio_client_with_database()
            .await
            .unwrap();
    let _ = client.list_all_tools().await.unwrap();

    for summary in [
        "World observed stronger evidence for the updated role.",
        "World observed the preference for evidence-backed identity changes.",
    ] {
        client
            .call_tool(
                "ingest_interaction",
                json!({
                    "event": {
                        "owner": "World",
                        "kind": "Observation",
                        "summary": summary
                    },
                    "claim_drafts": [],
                    "episode_reference": "episode:reflection-deeper-updates-source"
                }),
            )
            .await
            .unwrap();
    }

    let ingest = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "User",
                    "kind": "Conversation",
                    "summary": "The user clarified that the role has shifted."
                },
                "claim_drafts": [
                    {
                        "owner": "Self_",
                        "subject": "self.role",
                        "predicate": "is",
                        "object": "architect",
                        "mode": "Observed"
                    }
                ],
                "episode_reference": "episode:reflection-deeper-updates-target"
            }),
        )
        .await
        .unwrap();
    let superseded_claim_id = ingest
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("event_id"))
        .and_then(Value::as_str)
        .map(|event_id| format!("{event_id}:claim:0"))
        .unwrap();

    let reflection = client
        .call_tool(
            "run_reflection",
            json!({
                "reflection": {
                    "summary": "Shared evidence should update the replacement claim, identity, and commitments."
                },
                "supersede_claim_id": superseded_claim_id,
                "replacement_claim": {
                    "owner": "Self_",
                    "subject": "self.role",
                    "predicate": "is",
                    "object": "staff_architect",
                    "mode": "Observed"
                },
                "replacement_evidence_query": {
                    "owner": "World",
                    "kind": "Observation",
                    "limit": 2
                },
                "identity_update": {
                    "canonical_claims": [
                        "identity:self=staff_architect",
                        "identity:style=evidence_first"
                    ]
                },
                "commitment_updates": [
                    {
                        "owner": "Self_",
                        "description": "prefer:evidence_backed_identity_updates"
                    },
                    {
                        "owner": "Self_",
                        "description": "forbid:write_identity_core_directly"
                    }
                ]
            }),
        )
        .await
        .unwrap();

    let reflection_id = reflection
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("reflection_id"))
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    let replacement_claim_id = reflection
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("replacement_claim_id"))
        .and_then(Value::as_str);
    assert!(
        replacement_claim_id.is_some_and(|claim_id| claim_id.ends_with(":replacement")),
        "deeper reflection should still create a replacement claim: {reflection:?}"
    );

    let snapshot = client
        .call_tool("build_self_snapshot", json!({ "budget": 8 }))
        .await
        .unwrap();
    let snapshot = snapshot
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("snapshot"))
        .cloned()
        .unwrap();

    let identity = snapshot
        .get("identity")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    let commitments = snapshot
        .get("commitments")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    let claims = snapshot
        .get("claims")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();

    assert_eq!(
        identity,
        vec![
            "identity:self=staff_architect",
            "identity:style=evidence_first",
        ]
    );
    assert_eq!(
        commitments,
        vec![
            "prefer:evidence_backed_identity_updates",
            "forbid:write_identity_core_directly",
        ]
    );
    assert!(
        claims.contains(&"self:self.role is staff_architect"),
        "replacement claim should be visible in the snapshot: {claims:?}"
    );

    let pool = SqlitePool::connect(&database_url).await.unwrap();
    let audit_row = sqlx::query(
        r#"
        SELECT
            supporting_evidence_event_ids,
            requested_identity_update,
            requested_commitment_updates
        FROM reflections
        WHERE reflection_id = ?
        "#,
    )
    .bind(&reflection_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let supporting_evidence_event_ids = serde_json::from_str::<Vec<String>>(
        &audit_row.get::<String, _>("supporting_evidence_event_ids"),
    )
    .unwrap();
    let requested_identity_update = serde_json::from_str::<serde_json::Value>(
        &audit_row.get::<String, _>("requested_identity_update"),
    )
    .unwrap();
    let requested_commitment_updates = serde_json::from_str::<Vec<serde_json::Value>>(
        &audit_row.get::<String, _>("requested_commitment_updates"),
    )
    .unwrap();

    assert_eq!(supporting_evidence_event_ids.len(), 2);
    assert_eq!(
        requested_identity_update,
        json!({
            "canonical_claims": [
                "identity:self=staff_architect",
                "identity:style=evidence_first"
            ]
        })
    );
    assert_eq!(
        requested_commitment_updates,
        vec![
            json!({
                "owner": "Self_",
                "description": "prefer:evidence_backed_identity_updates"
            }),
            json!({
                "owner": "Self_",
                "description": "forbid:write_identity_core_directly"
            }),
        ]
    );
}

#[tokio::test]
async fn reflection_identity_or_commitment_updates_require_evidence_over_stdio() {
    let mut client = test_support::spawn_stdio_client().await.unwrap();
    let _ = client.list_all_tools().await.unwrap();

    let ingest = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "User",
                    "kind": "Conversation",
                    "summary": "A claim exists, but no supporting evidence event is provided."
                },
                "claim_drafts": [
                    {
                        "owner": "Self_",
                        "subject": "self.role",
                        "predicate": "is",
                        "object": "architect",
                        "mode": "Observed"
                    }
                ],
                "episode_reference": "episode:reflection-deeper-updates-missing-evidence"
            }),
        )
        .await
        .unwrap();
    let superseded_claim_id = ingest
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("event_id"))
        .and_then(Value::as_str)
        .map(|event_id| format!("{event_id}:claim:0"))
        .unwrap();

    let reflection = client
        .call_tool(
            "run_reflection",
            json!({
                "reflection": {
                    "summary": "Identity-only updates still need resolved evidence."
                },
                "supersede_claim_id": superseded_claim_id.clone(),
                "replacement_claim": null,
                "identity_update": {
                    "canonical_claims": ["identity:self=principal_architect"]
                }
            }),
        )
        .await
        .unwrap();

    let error = reflection
        .get("error")
        .expect("identity update without evidence should return invalid params");
    assert_eq!(error.get("code").and_then(Value::as_i64), Some(-32602));

    let reflection = client
        .call_tool(
            "run_reflection",
            json!({
                "reflection": {
                    "summary": "Commitment-only updates still need resolved evidence."
                },
                "supersede_claim_id": superseded_claim_id,
                "replacement_claim": null,
                "commitment_updates": [
                    {
                        "owner": "Self_",
                        "description": "prefer:reflect_before_identity_changes"
                    }
                ]
            }),
        )
        .await
        .unwrap();

    let error = reflection
        .get("error")
        .expect("missing evidence for deeper reflection updates should return invalid params");
    assert_eq!(error.get("code").and_then(Value::as_i64), Some(-32602));
}

#[tokio::test]
async fn replacement_evidence_query_limit_overflow_is_invalid_params_over_stdio() {
    let mut client = test_support::spawn_stdio_client().await.unwrap();
    let _ = client.list_all_tools().await.unwrap();

    client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "World",
                    "kind": "Observation",
                    "summary": "A matching observation exists, so overflow must not be masked as an empty-query error."
                },
                "claim_drafts": [],
                "episode_reference": "episode:reflection-limit-overflow-source"
            }),
        )
        .await
        .unwrap();

    let ingest = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "User",
                    "kind": "Conversation",
                    "summary": "Overflowing query limits should be rejected."
                },
                "claim_drafts": [
                    {
                        "owner": "Self_",
                        "subject": "self.role",
                        "predicate": "is",
                        "object": "architect",
                        "mode": "Observed"
                    }
                ],
                "episode_reference": "episode:reflection-limit-overflow"
            }),
        )
        .await
        .unwrap();
    let superseded_claim_id = ingest
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("event_id"))
        .and_then(Value::as_str)
        .map(|event_id| format!("{event_id}:claim:0"))
        .unwrap();

    let reflection = client
        .call_tool(
            "run_reflection",
            json!({
                "reflection": {
                    "summary": "Oversized evidence query limits should fail before SQLite treats them as unbounded."
                },
                "supersede_claim_id": superseded_claim_id,
                "replacement_claim": {
                    "owner": "Self_",
                    "subject": "self.role",
                    "predicate": "is",
                    "object": "principal_architect",
                    "mode": "Observed"
                },
                "replacement_evidence_query": {
                    "owner": "World",
                    "kind": "Observation",
                    "limit": 9223372036854775808u64
                }
            }),
        )
        .await
        .unwrap();

    let error = reflection
        .get("error")
        .expect("overflowing query limit should be reported as invalid params");
    assert_eq!(error.get("code").and_then(Value::as_i64), Some(-32602));
}

#[tokio::test]
async fn replacement_evidence_query_zero_limit_is_invalid_params_over_stdio() {
    let mut client = test_support::spawn_stdio_client().await.unwrap();
    let _ = client.list_all_tools().await.unwrap();

    client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "World",
                    "kind": "Observation",
                    "summary": "A matching observation exists, so a zero limit must surface as invalid params, not an empty-query error."
                },
                "claim_drafts": [],
                "episode_reference": "episode:reflection-zero-limit-source"
            }),
        )
        .await
        .unwrap();

    let ingest = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event": {
                    "owner": "User",
                    "kind": "Conversation",
                    "summary": "Zero query limits should be rejected."
                },
                "claim_drafts": [
                    {
                        "owner": "Self_",
                        "subject": "self.role",
                        "predicate": "is",
                        "object": "architect",
                        "mode": "Observed"
                    }
                ],
                "episode_reference": "episode:reflection-zero-limit"
            }),
        )
        .await
        .unwrap();
    let superseded_claim_id = ingest
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("event_id"))
        .and_then(Value::as_str)
        .map(|event_id| format!("{event_id}:claim:0"))
        .unwrap();

    let reflection = client
        .call_tool(
            "run_reflection",
            json!({
                "reflection": {
                    "summary": "A zero evidence query limit should fail before SQLite treats it as an empty match."
                },
                "supersede_claim_id": superseded_claim_id,
                "replacement_claim": {
                    "owner": "Self_",
                    "subject": "self.role",
                    "predicate": "is",
                    "object": "principal_architect",
                    "mode": "Observed"
                },
                "replacement_evidence_query": {
                    "owner": "World",
                    "kind": "Observation",
                    "limit": 0
                }
            }),
        )
        .await
        .unwrap();

    let error = reflection
        .get("error")
        .expect("zero query limit should be reported as invalid params");
    assert_eq!(error.get("code").and_then(Value::as_i64), Some(-32602));
    // 仅断言 -32602 不够鉴别：「无匹配空结果」路径同样返回 -32602。必须断言 message
    // 指明是 limit 校验拒绝，否则移除 C1 guard、零 limit 被当成空匹配掩盖时本测试仍为绿。
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .expect("error must carry a message");
    assert!(
        message.contains("limit must be at least 1"),
        "zero limit must be rejected by the limit guard, not masked as an empty match: {message}"
    );
}

mod test_support {
    use super::*;

    pub async fn spawn_stdio_client() -> io::Result<StdioClient> {
        let database = database_override().await?;
        StdioClient::spawn(&database.url, Some(database.temp_dir))
    }

    pub async fn spawn_stdio_client_with_config(
        config_template: String,
    ) -> io::Result<StdioClient> {
        let database = database_override().await?;
        let config_path = database.temp_dir.path().join("agent-llm-mm.local.toml");
        let config = config_template.replace("__DATABASE_URL__", &database.url);
        std::fs::write(&config_path, config)?;

        StdioClient::spawn_with_env(
            Some(database.temp_dir),
            &[(
                CONFIG_PATH_ENV_VAR,
                config_path.to_string_lossy().into_owned(),
            )],
        )
    }

    pub async fn spawn_stdio_client_with_config_and_database(
        config_template: String,
    ) -> io::Result<(StdioClient, String, TempDir)> {
        let database = database_override().await?;
        let url = database.url.clone();
        let config_path = database.temp_dir.path().join("agent-llm-mm.local.toml");
        let config = config_template.replace("__DATABASE_URL__", &url);
        std::fs::write(&config_path, config)?;
        let temp_dir = database.temp_dir;
        let client = StdioClient::spawn_with_env(
            None,
            &[(
                CONFIG_PATH_ENV_VAR,
                config_path.to_string_lossy().into_owned(),
            )],
        )?;

        Ok((client, url, temp_dir))
    }

    pub async fn spawn_stdio_client_with_database() -> io::Result<(StdioClient, String, TempDir)> {
        let database = database_override().await?;
        let url = database.url.clone();
        let temp_dir = database.temp_dir;
        let client = StdioClient::spawn(&url, None)?;
        Ok((client, url, temp_dir))
    }

    pub fn spawn_stdio_client_for_existing_database_with_config(
        config_template: &str,
        database_url: &str,
        config_dir: &Path,
    ) -> io::Result<StdioClient> {
        let config_path = config_dir.join("agent-llm-mm.reconnect.toml");
        let config = config_template.replace("__DATABASE_URL__", database_url);
        std::fs::write(&config_path, config)?;
        StdioClient::spawn_with_env(
            None,
            &[(
                CONFIG_PATH_ENV_VAR,
                config_path.to_string_lossy().into_owned(),
            )],
        )
    }

    struct DatabaseOverride {
        temp_dir: TempDir,
        url: String,
    }

    pub struct StdioClient {
        _database_dir: Option<TempDir>,
        child: Child,
        initialized: bool,
        stdin: ChildStdin,
        stdout: BufReader<ChildStdout>,
    }

    pub struct StubServer {
        base_url: String,
        last_request_path: Arc<tokio::sync::Mutex<Option<String>>>,
        request_count: Arc<tokio::sync::Mutex<usize>>,
        shutdown: Option<oneshot::Sender<()>>,
    }

    #[derive(Debug, Deserialize)]
    pub struct Tool {
        pub name: String,
        #[serde(rename = "inputSchema")]
        pub input_schema: Value,
    }

    impl StdioClient {
        fn spawn(database_url: &str, database_dir: Option<TempDir>) -> io::Result<Self> {
            Self::spawn_with_env(
                database_dir,
                &[(DATABASE_URL_ENV_VAR, database_url.to_string())],
            )
        }

        fn spawn_with_env(
            database_dir: Option<TempDir>,
            envs: &[(&str, String)],
        ) -> io::Result<Self> {
            let mut command = Command::new(env!("CARGO_BIN_EXE_agent_llm_mm"));
            for (key, value) in envs {
                command.env(key, value);
            }
            let mut child = command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?;

            let stdin = child
                .stdin
                .take()
                .ok_or_else(|| io::Error::other("missing child stdin"))?;
            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| io::Error::other("missing child stdout"))?;

            // 必须排空子进程 stderr：它是 piped 但从不读取，一旦 tracing 日志写满 ~64KB
            // 管道缓冲区，子进程下一次写 stderr 就会阻塞（表现为模型调用「超时」）。
            // 多次 ingest + 多轮模型往返的用例（如抑制流）正好会触顶。丢弃即可，无测试断言 stderr。
            if let Some(stderr) = child.stderr.take() {
                std::thread::spawn(move || {
                    let mut stderr = stderr;
                    let _ = io::copy(&mut stderr, &mut io::sink());
                });
            }

            Ok(Self {
                _database_dir: database_dir,
                child,
                initialized: false,
                stdin,
                stdout: BufReader::new(stdout),
            })
        }

        pub async fn list_all_tools(&mut self) -> io::Result<Vec<Tool>> {
            self.initialize()?;
            self.list_tools()
        }

        pub async fn call_tool(&mut self, name: &str, arguments: Value) -> io::Result<Value> {
            self.initialize()?;
            self.send(json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": name,
                    "arguments": arguments
                }
            }))?;
            self.read_message()
        }

        fn initialize(&mut self) -> io::Result<()> {
            if self.initialized {
                return Ok(());
            }

            self.send(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-03-26",
                    "capabilities": {},
                    "clientInfo": {
                        "name": "mcp-stdio-test",
                        "version": "0.1.0"
                    }
                }
            }))?;
            let _ = self.read_message()?;

            self.send(json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            }))?;
            self.initialized = true;
            Ok(())
        }

        fn list_tools(&mut self) -> io::Result<Vec<Tool>> {
            self.send(json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/list",
                "params": {}
            }))?;

            let message = self.read_message()?;
            let tools = message
                .get("result")
                .and_then(|result| result.get("tools"))
                .cloned()
                .ok_or_else(|| io::Error::other("missing tools in response"))?;

            serde_json::from_value(tools)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
        }

        fn send(&mut self, payload: Value) -> io::Result<()> {
            let mut body = serde_json::to_vec(&payload)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            body.push(b'\n');
            self.stdin.write_all(&body)?;
            self.stdin.flush()
        }

        fn read_message(&mut self) -> io::Result<Value> {
            loop {
                let mut line = String::new();
                let bytes_read = self.stdout.read_line(&mut line)?;
                if bytes_read == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "child process closed stdout before sending an MCP message",
                    ));
                }

                let trimmed = line.trim();
                if !trimmed.starts_with('{') {
                    continue;
                }

                return serde_json::from_str(trimmed)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error));
            }
        }
    }

    impl Drop for StdioClient {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    impl StubServer {
        pub async fn spawn(status: u16, body: Value) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
            let address = listener.local_addr().expect("local addr");
            let base_url = format!("http://{address}");
            let last_request_path = Arc::new(tokio::sync::Mutex::new(None));
            let request_path = Arc::clone(&last_request_path);
            let request_count = Arc::new(tokio::sync::Mutex::new(0));
            let request_counter = Arc::clone(&request_count);
            let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
            let response_body = body.to_string();

            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        _ = &mut shutdown_rx => break,
                        accept = listener.accept() => {
                            if let Ok((mut stream, _)) = accept {
                                let mut buffer = vec![0_u8; 16 * 1024];
                                let bytes_read = stream.read(&mut buffer).await.expect("read");
                                let request = String::from_utf8_lossy(&buffer[..bytes_read]);
                                let path = request
                                    .lines()
                                    .next()
                                    .and_then(|line| line.split_whitespace().nth(1))
                                    .map(str::to_string);
                                *request_path.lock().await = path;
                                *request_counter.lock().await += 1;

                                let status_text = match status {
                                    200 => "OK",
                                    503 => "Service Unavailable",
                                    _ => "Test Status",
                                };
                                let response = format!(
                                    "HTTP/1.1 {status} {status_text}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                                    response_body.len(),
                                    response_body
                                );
                                stream
                                    .write_all(response.as_bytes())
                                    .await
                                    .expect("write");
                            }
                        }
                    }
                }
            });

            Self {
                base_url,
                last_request_path,
                request_count,
                shutdown: Some(shutdown_tx),
            }
        }

        pub fn base_url(&self) -> String {
            self.base_url.clone()
        }

        pub async fn last_request_path(&self) -> Option<String> {
            self.last_request_path.lock().await.clone()
        }

        pub async fn request_count(&self) -> usize {
            *self.request_count.lock().await
        }
    }

    impl Drop for StubServer {
        fn drop(&mut self) {
            if let Some(shutdown) = self.shutdown.take() {
                let _ = shutdown.send(());
            }
        }
    }

    async fn database_override() -> io::Result<DatabaseOverride> {
        let temp_dir = tempfile::tempdir()?;
        let database_path = temp_dir.path().join("agent-llm-mm.sqlite");
        let url = sqlite_url(&database_path);
        agent_llm_mm::adapters::sqlite::initialize_database(&url)
            .await
            .map_err(|error| io::Error::other(error.to_string()))?;
        Ok(DatabaseOverride { url, temp_dir })
    }

    fn sqlite_url(path: &Path) -> String {
        format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"))
    }
}
