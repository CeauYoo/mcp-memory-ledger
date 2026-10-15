use super::test_support;
use serde_json::{Value, json};
use std::time::Duration;

fn config(port: u16, required: bool) -> String {
    format!(
        r#"database_url = "__DATABASE_URL__"
[model]
provider = "mock"
[dashboard]
enabled = true
host = "127.0.0.1"
port = {port}
base_path = "/takeover"
required = {required}
"#
    )
}

fn reserve_port() -> std::net::TcpListener {
    std::net::TcpListener::bind("127.0.0.1:0").expect("reserve isolated port")
}

async fn status(client: &reqwest::Client, port: u16) -> Option<Value> {
    client
        .get(format!("http://127.0.0.1:{port}/takeover/api/status"))
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn optional_dashboard_survivor_takes_over_after_owner_stdin_eof() {
    let reservation = reserve_port();
    let port = reservation.local_addr().unwrap().port();
    drop(reservation);
    let template = config(port, false);
    let (mut owner, database_url, directory) =
        test_support::spawn_stdio_client_with_config_and_database(template.clone())
            .await
            .unwrap();
    owner.list_all_tools().await.unwrap();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(1))
        .build()
        .unwrap();
    let original = status(&client, port)
        .await
        .expect("owner dashboard available");
    assert_eq!(original["process_id"], owner.process_id());

    let mut survivor = test_support::spawn_stdio_client_for_existing_database_with_config(
        &template,
        &database_url,
        directory.path(),
    )
    .unwrap();
    survivor.list_all_tools().await.unwrap();
    let reply = survivor
        .call_tool(
            "build_self_snapshot",
            json!({
                "namespace": "project/dashboard-takeover", "budget": 4, "evidence_manifest": []
            }),
        )
        .await
        .unwrap();
    assert!(reply.get("error").is_none());
    assert_eq!(
        status(&client, port).await.unwrap()["process_id"],
        owner.process_id()
    );
    assert!(
        owner.close_transport().await.unwrap().success(),
        "normal client EOF must exit cleanly"
    );

    let successor = tokio::time::timeout(Duration::from_secs(6), async {
        loop {
            if let Some(value) = status(&client, port).await
                && value["process_id"] == survivor.process_id()
            {
                break value;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("surviving MCP must take over the released port");
    assert_ne!(original["instance_id"], successor["instance_id"]);
    assert_eq!(successor["read_only"], true);
    let events: Vec<Value> = client
        .get(format!("http://127.0.0.1:{port}/takeover/api/events"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        events
            .iter()
            .any(|event| event["operation"] == "build_self_snapshot"),
        "waiting MCP must retain its own bounded process events"
    );
    survivor.list_all_tools().await.unwrap();
    assert!(survivor.close_transport().await.unwrap().success());
    assert!(
        status(&client, port).await.is_none(),
        "final EOF must release the dashboard"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn optional_dashboard_waiter_stops_with_its_mcp_session() {
    let reservation = reserve_port();
    let port = reservation.local_addr().unwrap().port();
    let mut client = test_support::spawn_stdio_client_with_config(config(port, false))
        .await
        .unwrap();
    client.list_all_tools().await.unwrap();
    assert!(client.close_transport().await.unwrap().success());
    drop(reservation);
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let released = std::net::TcpListener::bind(("127.0.0.1", port));
    assert!(
        released.is_ok(),
        "closed MCP must not leave a retry task or listener"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn required_dashboard_still_fails_when_port_is_occupied() {
    let reservation = reserve_port();
    let port = reservation.local_addr().unwrap().port();
    let mut client = test_support::spawn_stdio_client_with_config(config(port, true))
        .await
        .unwrap();
    assert!(client.list_all_tools().await.is_err());
    assert!(!client.close_transport().await.unwrap().success());
}
