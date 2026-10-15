//! Local stdio subprocesses with mock models and isolated fixture databases.
//! These verify audit behavior, not external-client or live-model acceptance.
use super::{semantic_memory_counts, test_support};
use serde_json::{Value, json};
use sqlx::{Row, SqlitePool};
use std::time::Duration;

const NAMESPACE: &str = "project/operation-audit";
const PRIVATE_MARKER: &str = "fixture-content-must-not-enter-fallback-audit";

struct Fixture {
    client: test_support::StdioClient,
    pool: SqlitePool,
    http: reqwest::Client,
    base_url: String,
    _directory: tempfile::TempDir,
}

impl Fixture {
    async fn new() -> Self {
        let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = reservation.local_addr().unwrap().port();
        drop(reservation);
        let config = format!(
            r#"database_url = "__DATABASE_URL__"
[model]
provider = "mock"
[dashboard]
enabled = true
host = "127.0.0.1"
port = {port}
required = true
event_capacity = 100
"#
        );
        let (mut client, url, directory) =
            test_support::spawn_stdio_client_with_config_and_database(config)
                .await
                .unwrap();
        client.list_all_tools().await.unwrap();
        Self {
            client,
            pool: SqlitePool::connect(&url).await.unwrap(),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(3))
                .build()
                .unwrap(),
            base_url: format!("http://127.0.0.1:{port}"),
            _directory: directory,
        }
    }

    async fn records(&self, entrypoint: &str) -> Vec<Value> {
        sqlx::query(
            "SELECT entrypoint, namespace, status, correlation_id, request_summary_json, \
             response_summary_json, diagnostic_summary_json FROM operation_log \
             WHERE entrypoint = ? ORDER BY occurred_at, operation_id",
        )
        .bind(entrypoint)
        .fetch_all(&self.pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            json!({
                "entrypoint": row.get::<String, _>("entrypoint"),
                "namespace": row.get::<Option<String>, _>("namespace"),
                "status": row.get::<String, _>("status"),
                "correlation_id": row.get::<Option<String>, _>("correlation_id"),
                "request": row.get::<Option<String>, _>("request_summary_json"),
                "response": row.get::<Option<String>, _>("response_summary_json"),
                "diagnostic": row.get::<Option<String>, _>("diagnostic_summary_json"),
            })
        })
        .collect()
    }

    async fn events(&self) -> Vec<Value> {
        self.http
            .get(format!("{}/api/events?limit=100", self.base_url))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    async fn assert_outcome(
        &self,
        tool: &str,
        expected_count: usize,
        status: &str,
        namespace: Option<&str>,
    ) {
        let records = self.records(tool).await;
        assert_eq!(records.len(), expected_count, "{tool}: {records:?}");
        let last = records.last().unwrap();
        assert_eq!(last["status"], status);
        assert_eq!(last["namespace"], json!(namespace));
        assert_eq!(last["request"], Value::Null);
        assert_eq!(last["response"], Value::Null);
        assert!(
            last["correlation_id"]
                .as_str()
                .unwrap()
                .starts_with("mcp-tool-call-")
        );
        let events = self
            .events()
            .await
            .into_iter()
            .filter(|event| event["operation"] == tool)
            .collect::<Vec<_>>();
        assert_eq!(events.len(), expected_count, "{tool}: {events:?}");
        for record in &records {
            let linked = events
                .iter()
                .filter(|event| event["correlation_id"] == record["correlation_id"])
                .collect::<Vec<_>>();
            assert_eq!(linked.len(), 1, "one live event per durable outcome");
            assert_eq!(linked[0]["status"], record["status"]);
            assert_eq!(linked[0]["namespace"], record["namespace"]);
        }
        assert!(
            !serde_json::to_string(&records)
                .unwrap()
                .contains(PRIVATE_MARKER)
        );
        assert!(
            !serde_json::to_string(&events)
                .unwrap()
                .contains(PRIVATE_MARKER)
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recall_and_context_audit_success_and_each_failure_once_without_content() {
    let mut fixture = Fixture::new().await;
    let before = semantic_memory_counts(&fixture.pool).await;
    for tool in ["recall_memory", "build_task_context"] {
        let success = fixture
            .client
            .call_tool(tool, json!({"namespace":NAMESPACE,"query":PRIVATE_MARKER}))
            .await
            .unwrap();
        assert!(success.get("error").is_none(), "{success}");
        fixture.assert_outcome(tool, 1, "ok", Some(NAMESPACE)).await;

        // DTO decoding, application validation and invalid scope all converge
        // on one outer outcome, including map_tool_error's internal failure path.
        for (index, arguments, scope) in [
            (
                0,
                json!({"namespace":NAMESPACE,"query":17}),
                Some(NAMESPACE),
            ),
            (
                1,
                json!({"namespace":NAMESPACE,"query":""}),
                Some(NAMESPACE),
            ),
            (
                2,
                json!({"namespace":"invalid","query":PRIVATE_MARKER}),
                None,
            ),
        ] {
            let failure = fixture.client.call_tool(tool, arguments).await.unwrap();
            assert_eq!(failure["error"]["code"], -32602, "{failure}");
            fixture
                .assert_outcome(tool, index + 2, "failed", scope)
                .await;
        }
    }
    assert_eq!(semantic_memory_counts(&fixture.pool).await, before);

    let unknown = fixture
        .client
        .call_tool(
            PRIVATE_MARKER,
            json!({"namespace":NAMESPACE,"query":PRIVATE_MARKER}),
        )
        .await
        .unwrap();
    assert!(unknown.get("error").is_some());
    fixture
        .assert_outcome("unknown_tool", 1, "failed", None)
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn experience_candidate_success_replay_and_failure_each_have_one_safe_outcome() {
    let mut fixture = Fixture::new().await;
    let ingested = fixture
        .client
        .call_tool("ingest_interaction", json!({
            "event":{"owner":"World","namespace":NAMESPACE,"kind":"Observation","summary":"audit fixture source"},
            "claim_drafts":[]
        }))
        .await
        .unwrap();
    let event_id = ingested["result"]["structuredContent"]["event_id"]
        .as_str()
        .unwrap();
    let episode = fixture
        .client
        .call_tool("record_episode", json!({
            "namespace":NAMESPACE,"request_id":"audit-episode-request","episode_id":"audit-episode",
            "content":{
                "title":PRIVATE_MARKER,"objective":"verify an audit record","actions":[],
                "observations":["fixture observation"],"outcome":"fixture complete","lesson":"fixture lesson",
                "limitations":[],"source_event_refs":[format!("event:{event_id}")]
            }
        }))
        .await
        .unwrap();
    assert!(episode.get("error").is_none(), "{episode}");
    fixture
        .assert_outcome("record_episode", 1, "ok", Some(NAMESPACE))
        .await;
    let candidate = json!({
        "namespace":NAMESPACE,"request_id":"audit-candidate-request","candidate_id":"audit-candidate",
        "content":{
            "kind":"semantic","title":PRIVATE_MARKER,"statement":PRIVATE_MARKER,"steps":[],
            "limitations":[],"source_episode_ids":["audit-episode"]
        }
    });
    for count in 1..=2 {
        let response = fixture
            .client
            .call_tool("propose_experience_candidate", candidate.clone())
            .await
            .unwrap();
        assert!(response.get("error").is_none(), "{response}");
        assert_eq!(
            response["result"]["structuredContent"]["replayed"],
            count == 2
        );
        fixture
            .assert_outcome("propose_experience_candidate", count, "ok", Some(NAMESPACE))
            .await;
    }
    let mut invalid = candidate;
    invalid["namespace"] = json!("invalid");
    let response = fixture
        .client
        .call_tool("propose_experience_candidate", invalid)
        .await
        .unwrap();
    assert_eq!(response["error"]["code"], -32602);
    fixture
        .assert_outcome("propose_experience_candidate", 3, "failed", None)
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn export_success_and_failures_leave_database_and_live_audit_unchanged() {
    let mut fixture = Fixture::new().await;
    let events_before = fixture.events().await;
    // data_version on this same connection changes on any other connection's
    // commit, catching updates as well as inserts in all fixture database tables.
    let mut connection = fixture.pool.acquire().await.unwrap();
    let before: i64 = sqlx::query_scalar("PRAGMA data_version")
        .fetch_one(&mut *connection)
        .await
        .unwrap();
    for (arguments, succeeds) in [
        (json!({"namespace":NAMESPACE}), true),
        (json!({}), false),
        (json!({"namespace":"invalid"}), false),
        (json!({"namespace":NAMESPACE,"max_bytes":1}), false),
    ] {
        let result = fixture
            .client
            .call_tool("export_memory", arguments)
            .await
            .unwrap();
        assert_eq!(result.get("error").is_none(), succeeds, "{result}");
        let after: i64 = sqlx::query_scalar("PRAGMA data_version")
            .fetch_one(&mut *connection)
            .await
            .unwrap();
        assert_eq!(after, before, "export must not commit any write");
        assert_eq!(fixture.events().await, events_before);
    }
    assert!(fixture.records("export_memory").await.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fallback_preserves_budget_admission_no_write_and_no_live_event_contract() {
    let mut fixture = Fixture::new().await;
    let events_before = fixture.events().await;
    let mut connection = fixture.pool.acquire().await.unwrap();
    let before: i64 = sqlx::query_scalar("PRAGMA data_version")
        .fetch_one(&mut *connection)
        .await
        .unwrap();
    for tool in ["recall_memory", "build_task_context"] {
        for budget in [
            json!({"limits":{"retrievals":0,"reflections":0,"retries":0},"used":{"retrievals":0,"reflections":0,"retries":0}}),
            json!({"limits":{"retrievals":-1}}),
        ] {
            let result = fixture
                .client
                .call_tool(
                    tool,
                    json!({"namespace":NAMESPACE,"query":PRIVATE_MARKER,"caller_budget":budget}),
                )
                .await
                .unwrap();
            assert_eq!(result["error"]["code"], -32602, "{result}");
            let after: i64 = sqlx::query_scalar("PRAGMA data_version")
                .fetch_one(&mut *connection)
                .await
                .unwrap();
            assert_eq!(after, before);
            assert_eq!(fixture.events().await, events_before);
            assert!(fixture.records(tool).await.is_empty());
        }
    }
}
