//! Real local MCP subprocess tests. These are test fixtures, not external-client evidence.
use super::{semantic_memory_counts, test_support};
use serde_json::{Value, json};
use sqlx::SqlitePool;

fn budget() -> Value {
    json!({
        "limits":{"retrievals":3,"reflections":1,"retries":1},
        "used":{"retrievals":0,"reflections":0,"retries":0}
    })
}

fn retrieval() -> Value {
    json!({"namespace":"project/caller-budget","query":"evidence","limit":5})
}

async fn counts_including_diagnostics(pool: &SqlitePool) -> Vec<i64> {
    let mut counts = semantic_memory_counts(pool).await;
    counts.push(
        sqlx::query_scalar("SELECT COUNT(*) FROM operation_log")
            .fetch_one(pool)
            .await
            .unwrap(),
    );
    counts
}

#[tokio::test]
async fn caller_budget_schema_is_optional_on_exactly_three_routes() {
    let mut client = test_support::spawn_stdio_client().await.unwrap();
    let tools = client.list_all_tools().await.unwrap();
    for tool in tools {
        let expected =
            ["recall_memory", "build_task_context", "run_reflection"].contains(&tool.name.as_str());
        assert_eq!(
            tool.input_schema["properties"]
                .get("caller_budget")
                .is_some(),
            expected,
            "unexpected caller-budget schema on {}",
            tool.name
        );
        if expected {
            let required = tool.input_schema["required"].as_array().unwrap();
            assert!(!required.iter().any(|field| field == "caller_budget"));
            assert!(
                serde_json::to_string(&tool.input_schema)
                    .unwrap()
                    .contains("1000")
            );
        }
    }
}

#[tokio::test]
async fn caller_budget_omission_and_null_preserve_recall_and_context_results() {
    let mut client = test_support::spawn_stdio_client().await.unwrap();
    for tool in ["recall_memory", "build_task_context"] {
        let original = client.call_tool(tool, retrieval()).await.unwrap();
        assert!(original.get("error").is_none(), "{original}");
        assert!(
            original["result"]["structuredContent"]
                .get("caller_budget")
                .is_none()
        );
        let mut params = retrieval();
        params["caller_budget"] = Value::Null;
        let explicit_null = client.call_tool(tool, params).await.unwrap();
        assert_eq!(original["result"], explicit_null["result"]);
    }
    let original = client.call_tool("run_reflection", json!({})).await.unwrap();
    let explicit_null = client
        .call_tool("run_reflection", json!({"caller_budget":null}))
        .await
        .unwrap();
    assert_eq!(original["error"], explicit_null["error"]);
}

#[tokio::test]
async fn caller_budget_real_retrieval_accounts_retry_then_stops_without_persistence() {
    let mut client = test_support::spawn_stdio_client().await.unwrap();
    let mut params = retrieval();
    params["caller_budget"] = budget();
    params["caller_budget"]["is_retry"] = json!(true);
    params["caller_budget"]["evidence"] = json!("unchanged");
    let first = client
        .call_tool("recall_memory", params.clone())
        .await
        .unwrap();
    let decision = &first["result"]["structuredContent"]["caller_budget"];
    assert_eq!(decision["allowed"], true, "{first}");
    assert_eq!(
        decision["next_used"],
        json!({"retrievals":1,"reflections":0,"retries":1})
    );
    assert_eq!(
        decision["remaining"],
        json!({"retrievals":2,"reflections":1,"retries":0})
    );
    // Repeating the identical caller report is intentionally stateless, not a durable quota.
    let same = client
        .call_tool("recall_memory", params.clone())
        .await
        .unwrap();
    assert_eq!(
        same["result"]["structuredContent"]["caller_budget"],
        *decision
    );
    params["caller_budget"]["used"] = decision["next_used"].clone();
    let stopped = client
        .call_tool("recall_memory", params.clone())
        .await
        .unwrap();
    assert_eq!(
        stopped["error"]["data"]["caller_budget"]["stop_reason"],
        "retry_budget_exhausted"
    );
    assert_eq!(
        stopped["error"]["data"]["caller_budget"]["next_used"],
        params["caller_budget"]["used"]
    );
    params["caller_budget"]["is_retry"] = json!(false);
    params["caller_budget"]["evidence"] = json!("insufficient");
    let continued = client
        .call_tool("build_task_context", params)
        .await
        .unwrap();
    assert_eq!(
        continued["result"]["structuredContent"]["caller_budget"]["next_used"]["retrievals"], 2,
        "retrieval must remain usable to find missing evidence: {continued}"
    );
}

#[tokio::test]
async fn caller_budget_stops_before_unrelated_decode_and_any_ledger_or_diagnostic_write() {
    let (mut client, url, _dir) = test_support::spawn_stdio_client_with_database()
        .await
        .unwrap();
    client.list_all_tools().await.unwrap();
    let pool = SqlitePool::connect(&url).await.unwrap();
    let before = counts_including_diagnostics(&pool).await;
    for (tool, reason, evidence) in [
        ("run_reflection", "no_new_evidence", "unchanged"),
        ("run_reflection", "insufficient_evidence", "insufficient"),
        ("run_reflection", "reflection_budget_exhausted", "unknown"),
        ("recall_memory", "retrieval_budget_exhausted", "unknown"),
        (
            "build_task_context",
            "retrieval_budget_exhausted",
            "unknown",
        ),
    ] {
        let mut input = budget();
        input["evidence"] = json!(evidence);
        input["limits"]["retrievals"] = json!(0);
        input["limits"]["reflections"] = json!(0);
        // Missing reflection/query and mistyped limit would otherwise fail typed decoding.
        let mut params = json!({"caller_budget":input,"origin_namespace":"invalid","limit":"bad"});
        for with_reflection in [false, true] {
            if with_reflection {
                params["reflection"] = json!({"summary":"must never be recorded"});
            }
            let stopped = client.call_tool(tool, params.clone()).await.unwrap();
            let receipt = &stopped["error"]["data"]["caller_budget"];
            assert_eq!(stopped["error"]["code"], -32602, "{stopped}");
            assert_eq!(receipt["stop_reason"], reason, "{stopped}");
            assert_eq!(receipt["allowed"], false);
            assert_eq!(receipt["next_used"], input["used"]);
            assert_eq!(counts_including_diagnostics(&pool).await, before);
        }
    }
    for invalid in [json!(1001), json!(-1), json!(1.2), json!("1")] {
        let mut input = budget();
        input["used"]["retrievals"] = invalid;
        let result = client
            .call_tool("run_reflection", json!({"caller_budget": input}))
            .await
            .unwrap();
        assert_eq!(result["error"]["code"], -32602);
        assert!(result["error"]["data"].get("caller_budget").is_none());
        assert_eq!(counts_including_diagnostics(&pool).await, before);
    }
}

#[tokio::test]
async fn caller_budget_admitted_decode_conversion_and_execution_failures_keep_receipts() {
    let (mut client, url, _dir) = test_support::spawn_stdio_client_with_database()
        .await
        .unwrap();
    client.list_all_tools().await.unwrap();
    let pool = SqlitePool::connect(&url).await.unwrap();
    let before = semantic_memory_counts(&pool).await;
    for (tool, mut params, operation, counter) in [
        (
            "recall_memory",
            json!({"limit":"wrong"}),
            "retrieval",
            "retrievals",
        ),
        (
            "recall_memory",
            json!({"namespace":"invalid","query":"evidence"}),
            "retrieval",
            "retrievals",
        ),
        (
            "recall_memory",
            json!({"namespace":"project/caller-budget","query":""}),
            "retrieval",
            "retrievals",
        ),
        (
            "build_task_context",
            json!({"namespace":"project/caller-budget","query":"evidence","max_bytes":1}),
            "retrieval",
            "retrievals",
        ),
        ("run_reflection", json!({}), "reflection", "reflections"),
        (
            "run_reflection",
            json!({"reflection":{"summary":"Invalid origin"},"origin_namespace":"invalid"}),
            "reflection",
            "reflections",
        ),
        (
            "run_reflection",
            json!({"reflection":{"summary":"Missing actual evidence"},"origin_namespace":"project/caller-budget","replacement_evidence_event_ids":["event:missing"]}),
            "reflection",
            "reflections",
        ),
    ] {
        params["caller_budget"] = budget();
        params["caller_budget"]["evidence"] = json!("new");
        let response = client.call_tool(tool, params).await.unwrap();
        assert!(
            response.get("error").is_some(),
            "new signal must not bypass validation: {response}"
        );
        let receipt = &response["error"]["data"]["caller_budget"];
        assert_eq!(receipt["allowed"], true, "{response}");
        assert_eq!(receipt["operation"], operation);
        assert_eq!(receipt["next_used"][counter], 1);
        assert!(receipt["stop_reason"].is_null());
        assert_eq!(semantic_memory_counts(&pool).await, before);
    }
}

#[tokio::test]
async fn caller_budget_reflection_success_and_context_receipt_are_real_and_byte_bounded() {
    let (mut client, url, _dir) = test_support::spawn_stdio_client_with_database()
        .await
        .unwrap();
    let ingest = client
        .call_tool(
            "ingest_interaction",
            json!({
                "event":{"owner":"World","namespace":"project/caller-budget","kind":"Observation",
                    "summary":"Bounded evidence with 漢字 and provenance"},
                "claim_drafts":[]
            }),
        )
        .await
        .unwrap();
    let event_id = ingest["result"]["structuredContent"]["event_id"]
        .as_str()
        .unwrap();
    let mut params = json!({
        "reflection":{"summary":"Caller-approved evidence-backed reflection"},
        "origin_namespace":"project/caller-budget",
        "replacement_evidence_event_ids":[format!("event:{event_id}")],
        "caller_budget":budget()
    });
    let reflected = client
        .call_tool("run_reflection", params.clone())
        .await
        .unwrap();
    let data = &reflected["result"]["structuredContent"];
    assert!(data["reflection_id"].is_string(), "{reflected}");
    assert_eq!(data["caller_budget"]["next_used"]["reflections"], 1);
    params["caller_budget"]["used"] = data["caller_budget"]["next_used"].clone();
    let pool = SqlitePool::connect(&url).await.unwrap();
    let before = counts_including_diagnostics(&pool).await;
    let stopped = client.call_tool("run_reflection", params).await.unwrap();
    assert_eq!(
        stopped["error"]["data"]["caller_budget"]["stop_reason"],
        "reflection_budget_exhausted"
    );
    assert_eq!(counts_including_diagnostics(&pool).await, before);

    for cap in [1000, 1800, 2500, 65536] {
        let mut params = retrieval();
        params["caller_budget"] = budget();
        params["max_bytes"] = json!(cap);
        let response = client
            .call_tool("build_task_context", params)
            .await
            .unwrap();
        let data = &response["result"]["structuredContent"];
        if response.get("error").is_some() {
            assert_eq!(
                cap, 1000,
                "only the smallest cap may not fit metadata: {response}"
            );
            assert_eq!(response["error"]["data"]["caller_budget"]["allowed"], true);
            continue;
        }
        assert_eq!(data["caller_budget"]["next_used"]["retrievals"], 1);
        let actual = serde_json::to_vec(data).unwrap().len();
        assert_eq!(data["serialized_bytes"], actual, "{response}");
        assert!(
            actual <= cap as usize,
            "receipt must be inside full UTF-8 cap: {actual} > {cap}"
        );
        if cap == 65536 {
            assert!(!data["records"].as_array().unwrap().is_empty());
        }
    }
}
