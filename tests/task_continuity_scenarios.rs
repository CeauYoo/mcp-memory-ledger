//! Deterministic synthetic dialogues, explicitly mapped to API requests by fixtures.
//! This is persistence/contract testing, not natural-language or model evaluation.
use agent_llm_mm::{
    adapters::sqlite::{SqliteStore, open_current_database},
    application::{experience, experience_use, task},
    domain::{
        event::Event,
        task::{GetTaskRequest, TaskContextRequest},
        types::{EventKind, Namespace, Owner},
    },
    ports::{EventStore, StoredEvent},
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::SqlitePool;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Corpus {
    disclosure: String,
    scenarios: Vec<Scenario>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    id: String,
    category: String,
    title: String,
    turns: Vec<Turn>,
    checkpoint: Option<Value>,
    long_history: bool,
    large_history: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Turn {
    speaker: String,
    text: String,
    actions: Vec<Action>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Action {
    op: String,
    request: Value,
    expect: Value,
    error: bool,
}
fn corpus() -> Corpus {
    serde_json::from_str(include_str!("fixtures/task_continuity_scenarios.json")).unwrap()
}

// Partial object matching permits timestamps and receipts while checking every
// explicitly expected field. Arrays are ordered and complete, never a prefix.
fn assert_expected(actual: &Value, expected: &Value, path: &str) {
    if let Some(fields) = expected.as_object() {
        for (key, value) in fields {
            if key == "is_null" {
                assert_eq!(actual.is_null(), value.as_bool().unwrap(), "{path}");
            } else if key == "diagnostics_contains" {
                let diagnostics = actual["diagnostics"].as_array().unwrap();
                for diagnostic in value.as_array().unwrap() {
                    assert!(
                        diagnostics.contains(diagnostic),
                        "{path}: missing {diagnostic}"
                    );
                }
            } else {
                assert!(
                    actual.get(key).is_some(),
                    "{path}: missing field {key}: {actual}"
                );
                assert_expected(&actual[key], value, &format!("{path}.{key}"));
            }
        }
    } else if let Some(items) = expected.as_array() {
        let actual = actual.as_array().unwrap();
        assert_eq!(actual.len(), items.len(), "{path}: complete array length");
        for (i, (a, e)) in actual.iter().zip(items).enumerate() {
            assert_expected(a, e, &format!("{path}[{i}]"));
        }
    } else {
        assert_eq!(actual, expected, "{path}");
    }
}
async fn append(store: &SqliteStore, id: String, namespace: &str, text: &str) {
    store
        .append_event(StoredEvent::new(
            id,
            Utc::now(),
            Event::new_with_namespace(
                Owner::World,
                Namespace::parse(namespace).unwrap(),
                EventKind::Observation,
                text,
            )
            .unwrap(),
        ))
        .await
        .unwrap();
}

async fn run(id: &str) {
    let scenario = corpus().scenarios.into_iter().find(|s| s.id == id).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let url =
        agent_llm_mm::support::sqlite_url::sqlite_file_url(&dir.path().join("continuity.sqlite"));
    let mut store = SqliteStore::bootstrap(&url).await.unwrap();
    for (index, turn) in scenario.turns.iter().enumerate() {
        append(
            &store,
            format!("turn-{index:03}"),
            "project/continuity",
            &turn.text,
        )
        .await;
        for action in &turn.actions {
            let location = format!("{id} turn {index} {}", action.op);
            // Typed API deserialization is intentional: the fixture, not a model,
            // interprets the adjacent synthetic prose into explicit operations.
            macro_rules! call {
                ($function:path) => {
                    $function(
                        &store,
                        serde_json::from_value(action.request.clone()).unwrap(),
                    )
                    .await
                    .map(|value| serde_json::to_value(value).unwrap())
                };
            }
            let result = match action.op.as_str() {
                "create_task" => call!(task::create_task),
                "revise_task" => call!(task::revise_task),
                "record_task_checkpoint" => call!(task::record_task_checkpoint),
                "get_task" => call!(task::get_task),
                "create_episode" => call!(experience::create_episode),
                "create_candidate" => call!(experience::create_candidate),
                "candidate_status" => call!(experience::update_candidate_status),
                "revise_candidate" => call!(experience::revise_candidate),
                "get_candidate" => call!(experience::get_candidate),
                "record_experience_use" => call!(experience_use::record_experience_use),
                "list_experience_uses" => call!(experience_use::list_experience_uses),
                "context" => {
                    let request: TaskContextRequest =
                        serde_json::from_value(action.request.clone()).unwrap();
                    let budget = request.max_bytes;
                    task::build_task_resume_context(&store, request)
                        .await
                        .map(|context| {
                            let bytes = serde_json::to_vec(&context).unwrap().len();
                            assert_eq!(bytes, context.serialized_bytes, "{location}");
                            assert!(bytes <= budget, "{location}");
                            assert!(context.mandatory_snapshot_consistent, "{location}");
                            serde_json::to_value(context).unwrap()
                        })
                }
                "seed_evidence" | "seed_foreign" => {
                    let (event, namespace) = if action.op == "seed_foreign" {
                        ("foreign", "project/other")
                    } else {
                        ("evidence", "project/continuity")
                    };
                    append(&store, event.into(), namespace, &turn.text).await;
                    Ok(json!({}))
                }
                "restart" => {
                    drop(store);
                    store = open_current_database(&url).await.unwrap();
                    Ok(json!({}))
                }
                other => panic!("{location}: unknown fixture action {other}"),
            };
            if action.error {
                assert!(
                    result.is_err(),
                    "{location}: invalid action unexpectedly succeeded"
                );
            } else {
                let actual = result.unwrap_or_else(|error| panic!("{location}: {error}"));
                assert_expected(&actual, &action.expect, &location);
            }
        }
    }
    drop(store);
    let store = open_current_database(&url).await.unwrap();
    let snapshot = task::get_task_snapshot(&store, "project/continuity", "task")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.task.title, scenario.title);
    match scenario.checkpoint {
        Some(expected) => assert_expected(
            &serde_json::to_value(snapshot.checkpoint.unwrap()).unwrap(),
            &expected,
            id,
        ),
        None => assert!(
            snapshot.checkpoint.is_none(),
            "{id}: rejected checkpoints left no state"
        ),
    }
    let final_expectation = &scenario.turns.last().unwrap().actions[0].expect;
    assert_expected(
        &serde_json::to_value(snapshot.task).unwrap(),
        final_expectation,
        id,
    );
    let original = task::get_task(
        &store,
        GetTaskRequest {
            namespace: "project/continuity".into(),
            task_id: "task".into(),
            version: Some(1),
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        original.requirements.len(),
        1,
        "{id}: immutable original revision"
    );
    assert_eq!(
        original.requirements[0].state,
        agent_llm_mm::domain::task::RequirementState::Reported
    );

    // Full ordered readback, including all irrelevant long-history messages. No
    // context-window, substring, token-count, or retrieval-success proxy is used.
    let pool = SqlitePool::connect(&url).await.unwrap();
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT event_id, namespace, summary FROM events WHERE event_id LIKE 'turn-%' ORDER BY event_id"
    ).fetch_all(&pool).await.unwrap();
    assert_eq!(rows.len(), scenario.turns.len(), "{id}: no turn loss");
    for (index, ((event_id, namespace, text), turn)) in rows.iter().zip(&scenario.turns).enumerate()
    {
        assert_eq!(event_id, &format!("turn-{index:03}"));
        assert_eq!(namespace, "project/continuity");
        assert_eq!(text, &turn.text, "{id} turn {index}: full UTF-8 readback");
    }
    pool.close().await;
}

#[test]
fn fixture_integrity_and_honest_history_sizes() {
    let corpus = corpus();
    assert!(corpus.disclosure.contains("Synthetic"));
    assert!(corpus.disclosure.contains("No LLM"));
    assert_eq!(corpus.scenarios.len(), 30);
    let mut categories = BTreeMap::new();
    let mut ids = BTreeSet::new();
    let mut trajectories = BTreeSet::new();
    let mut long = 0;
    let mut large = 0;
    let mut total_turns = 0;
    for scenario in corpus.scenarios {
        assert!(ids.insert(scenario.id.clone()));
        *categories.entry(scenario.category).or_insert(0) += 1;
        total_turns += scenario.turns.len();
        if scenario.long_history {
            long += 1;
            assert_eq!(scenario.turns.len(), 64);
        }
        let words: usize = scenario
            .turns
            .iter()
            .map(|t| t.text.split_whitespace().count())
            .sum();
        if scenario.large_history {
            large += 1;
            assert!(words >= 15_000, "{}: {words}", scenario.id);
        }
        let trajectory: Vec<_> = scenario
            .turns
            .iter()
            .flat_map(|t| &t.actions)
            .map(|a| {
                format!(
                    "{}:{}:{}",
                    a.op,
                    a.request.get("status").unwrap_or(&Value::Null),
                    a.error
                )
            })
            .collect();
        assert!(
            trajectories.insert(trajectory),
            "{} duplicates an action/status/error trajectory",
            scenario.id
        );
        for turn in scenario.turns {
            assert!(matches!(turn.speaker.as_str(), "user" | "assistant"));
            assert!(!turn.text.is_empty() && turn.text.len() < 4096);
        }
    }
    assert_eq!(categories.len(), 6);
    assert!(categories.values().all(|&count| count == 5));
    assert_eq!(long, 6);
    assert_eq!(large, 2);
    assert_eq!(total_turns, 756);
}
macro_rules! scenario_tests {
    ($($name:ident),+ $(,)?) => { $(
        #[tokio::test]
        async fn $name() { run(stringify!($name)).await; }
    )+ };
}
scenario_tests!(
    requirements_1,
    requirements_2,
    requirements_3,
    requirements_4,
    requirements_5,
    pause_resume_1,
    pause_resume_2,
    pause_resume_3,
    pause_resume_4,
    pause_resume_5,
    experience_1,
    experience_2,
    experience_3,
    experience_4,
    experience_5,
    environment_1,
    environment_2,
    environment_3,
    environment_4,
    environment_5,
    scope_1,
    scope_2,
    scope_3,
    scope_4,
    scope_5,
    evidence_1,
    evidence_2,
    evidence_3,
    evidence_4,
    evidence_5,
);
