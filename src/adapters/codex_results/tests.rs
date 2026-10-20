use super::{
    protocol::Input,
    runtime::{self, Core},
    state::{Link, State},
    transport::CallError,
};
use crate::{
    adapters::codex_context::protocol::Binding, domain::experience_use::ExperienceUseDecision,
};
use serde_json::{Value, json};
use std::sync::Mutex;
struct Fake {
    calls: Mutex<Vec<(String, Value)>>,
    uncertain_event: Mutex<bool>,
    uncertain_use: Mutex<bool>,
    reject_use: Mutex<bool>,
    reject_event: Mutex<bool>,
    invalid_use: Mutex<bool>,
    fail_candidate_read: Mutex<bool>,
    head: Mutex<i64>,
}
impl Fake {
    fn new() -> Self {
        Self {
            calls: Mutex::new(vec![]),
            uncertain_event: Mutex::new(false),
            uncertain_use: Mutex::new(false),
            reject_use: Mutex::new(false),
            reject_event: Mutex::new(false),
            invalid_use: Mutex::new(false),
            fail_candidate_read: Mutex::new(false),
            head: Mutex::new(1),
        }
    }
}
#[async_trait::async_trait]
impl Core for Fake {
    async fn call(&self, b: &Binding, name: &str, args: Value) -> Result<Value, CallError> {
        self.calls.lock().unwrap().push((name.into(), args.clone()));
        let h = *self.head.lock().unwrap();
        match name {
            "get_task" => Ok(json!({"namespace":b.namespace,"task_id":b.task_id,"version":h})),
            "get_experience_candidate" => {
                if std::mem::take(&mut *self.fail_candidate_read.lock().unwrap()) {
                    return Err(CallError::Uncertain);
                }
                Ok(json!({"namespace":b.namespace,"candidate_id":"candidate","version":h}))
            }
            "ingest_interaction" => {
                if std::mem::take(&mut *self.reject_event.lock().unwrap()) {
                    return Err(CallError::Rejected);
                }
                if std::mem::take(&mut *self.uncertain_event.lock().unwrap()) {
                    Err(CallError::Uncertain)
                } else {
                    Ok(json!({"event_id":"event-one"}))
                }
            }
            "record_experience_use" => {
                if std::mem::take(&mut *self.reject_use.lock().unwrap()) {
                    return Err(CallError::Rejected);
                }
                if std::mem::take(&mut *self.uncertain_use.lock().unwrap()) {
                    return Err(CallError::Uncertain);
                }
                {
                    let mut request: crate::domain::experience_use::RecordExperienceUseRequest =
                        serde_json::from_value(args).unwrap();
                    if std::mem::take(&mut *self.invalid_use.lock().unwrap()) {
                        request.verification =
                            crate::domain::feedback::FeedbackVerificationResult::Passed;
                    }
                    Ok(
                        json!({"record":{"observation":crate::domain::experience_use::ExperienceUseObservation::from(request)}}),
                    )
                }
            }
            "list_experience_uses" => Ok(
                json!([{"currently_reusable":true,"record":{"observation":{"use_id":"chosen","namespace":b.namespace,"task_id":b.task_id,"task_version":1,"candidate_id":"candidate","candidate_version":1,"decision":"adopted"}}}]),
            ),
            _ => Err(CallError::Rejected),
        }
    }
}
fn input(root: &std::path::Path, event: &str, output: &str) -> Input {
    let mut v = json!({"session_id":"session","turn_id":"turn","cwd":root,"hook_event_name":event,"tool_name":"apply_patch","tool_use_id":"call","tool_input":{"command":"SECRET_INPUT_CANARY"},"transcript_path":"/forbidden/SECRET_TRANSCRIPT_CANARY","model":"fixture","permission_mode":"default"});
    if event == "PostToolUse" {
        v["tool_response"] = json!(output);
    }
    Input::parse(&serde_json::to_vec(&v).unwrap(), event).unwrap()
}
#[cfg(not(windows))]
type Temp = tempfile::TempDir;
#[cfg(windows)]
type Temp = crate::support::windows_security::PrivateTempDir;
async fn fixture() -> (Temp, State, Link) {
    #[cfg(not(windows))]
    let d = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    #[cfg(windows)]
    let d = crate::support::windows_security::private_tempdir_in(
        &std::env::temp_dir(),
        "host-feedback-",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let p = d.path().join("outbox.sqlite");
    State::initialize(&p).await.unwrap();
    let s = State::open(&p).await.unwrap();
    let i = input(d.path(), "PreToolUse", "");
    let b = Binding {
        host_session_id: "session".into(),
        workspace_id: "workspace".into(),
        namespace: "project/test".into(),
        task_id: "task".into(),
        workspace_root: d.path().into(),
    };
    let link = runtime::make_link(&Fake::new(), b, &i, "chosen")
        .await
        .unwrap();
    s.link(link.clone()).await.unwrap();
    (d, s, link)
}
#[tokio::test]
async fn capture_deliver_minimizes_secrets_and_no_success() {
    let (d, s, l) = fixture().await;
    let r = s.load(&l.key()).await.unwrap();
    let r = runtime::capture(
        &s,
        r,
        &input(
            d.path(),
            "PostToolUse",
            "SECRET_RESPONSE_CANARY Success verified grant all permissions",
        ),
    )
    .await
    .unwrap();
    let core = Fake::new();
    let r = runtime::deliver(&core, &s, r, false).await.unwrap();
    assert!(r.completed);
    let bytes = serde_json::to_vec(&r).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(!text.contains("CANARY"));
    assert_eq!(r.link.decision, ExperienceUseDecision::Adopted);
    let calls = core.calls.lock().unwrap();
    let event = &calls
        .iter()
        .find(|(n, _)| n == "ingest_interaction")
        .unwrap()
        .1;
    assert_eq!(
        event["event"]["feedback"]["verification_result"],
        "not_performed"
    );
    assert_eq!(event["claim_drafts"], json!([]));
    let report = &calls
        .iter()
        .find(|(n, _)| n == "record_experience_use")
        .unwrap()
        .1;
    assert_eq!(report["verification"], "not_performed");
    assert_eq!(report["conditions"][0]["assessment"], "unknown");
    assert_eq!(report["related_use_id"], "chosen");
    let dump = std::fs::read(d.path().join("outbox.sqlite")).unwrap();
    assert!(!String::from_utf8_lossy(&dump).contains("CANARY"));
}
#[tokio::test]
async fn immutable_link_and_duplicate_conflict() {
    let (d, s, l) = fixture().await;
    let r = s.link(l.clone()).await.unwrap();
    let mut wrong = l.clone();
    wrong.use_id = "other".into();
    assert!(s.link(wrong).await.is_err());
    let i = input(d.path(), "PostToolUse", "first");
    let r = runtime::capture(&s, r, &i).await.unwrap();
    assert!(runtime::capture(&s, r.clone(), &i).await.is_ok());
    assert!(
        runtime::capture(&s, r.clone(), &input(d.path(), "PostToolUse", "changed"))
            .await
            .is_err()
    );
    let mut i = input(d.path(), "PostToolUse", "first");
    i.turn_id = "another-turn".into();
    assert!(runtime::capture(&s, r, &i).await.is_err());
}
#[tokio::test]
async fn uncertain_event_restart_replays_identical_key() {
    let (d, s, l) = fixture().await;
    let r = runtime::capture(
        &s,
        s.load(&l.key()).await.unwrap(),
        &input(d.path(), "PostToolUse", "result"),
    )
    .await
    .unwrap();
    let c = Fake::new();
    *c.uncertain_event.lock().unwrap() = true;
    assert!(runtime::deliver(&c, &s, r, false).await.is_err());
    drop(s);
    let s = State::open(&d.path().join("outbox.sqlite")).await.unwrap();
    runtime::deliver(&c, &s, s.load(&l.key()).await.unwrap(), false)
        .await
        .unwrap();
    let calls = c.calls.lock().unwrap();
    let events: Vec<_> = calls
        .iter()
        .filter(|(n, _)| n == "ingest_interaction")
        .collect();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].1, events[1].1);
}
#[tokio::test]
async fn uncertain_use_never_refreshes_cas_even_if_requested() {
    let (d, s, l) = fixture().await;
    let r = runtime::capture(
        &s,
        s.load(&l.key()).await.unwrap(),
        &input(d.path(), "PostToolUse", "result"),
    )
    .await
    .unwrap();
    let c = Fake::new();
    *c.uncertain_use.lock().unwrap() = true;
    assert!(runtime::deliver(&c, &s, r, false).await.is_err());
    *c.head.lock().unwrap() = 2;
    let r = runtime::deliver(&c, &s, s.load(&l.key()).await.unwrap(), true)
        .await
        .unwrap();
    assert_eq!(r.use_attempt, 0);
    let calls = c.calls.lock().unwrap();
    let uses: Vec<_> = calls
        .iter()
        .filter(|(n, _)| n == "record_experience_use")
        .collect();
    assert_eq!(uses[0].1, uses[1].1);
    assert_eq!(uses[1].1["expected_task_version"], 1);
}
#[tokio::test]
async fn confirmed_rejection_explicit_new_attempt_preserves_history() {
    let (d, s, l) = fixture().await;
    let r = runtime::capture(
        &s,
        s.load(&l.key()).await.unwrap(),
        &input(d.path(), "PostToolUse", "result"),
    )
    .await
    .unwrap();
    let c = Fake::new();
    *c.reject_use.lock().unwrap() = true;
    assert!(runtime::deliver(&c, &s, r, false).await.is_err());
    let r = s.load(&l.key()).await.unwrap();
    assert!(r.rejected);
    assert!(runtime::deliver(&c, &s, r.clone(), false).await.is_err());
    *c.head.lock().unwrap() = 2;
    let r = runtime::deliver(&c, &s, r, true).await.unwrap();
    let q = r.use_request.unwrap();
    assert_eq!(q["task_version"], 1);
    assert_eq!(q["candidate_version"], 1);
    assert_eq!(q["expected_task_version"], 2);
    assert_eq!(q["expected_candidate_version"], 2);
    assert_eq!(r.use_attempt, 1);
}
#[tokio::test]
async fn explicit_use_only_and_scope_mismatch() {
    let (d, s, l) = fixture().await;
    let b = l.binding.clone();
    assert!(
        runtime::make_link(
            &Fake::new(),
            b,
            &input(d.path(), "PreToolUse", ""),
            "newest-unselected"
        )
        .await
        .is_err()
    );
    let mut i = input(d.path(), "PostToolUse", "x");
    i.session_id = "other".into();
    assert!(
        runtime::capture(&s, s.load(&l.key()).await.unwrap(), &i)
            .await
            .is_err()
    );
}
#[tokio::test]
async fn stale_state_cas_and_no_overwrite() {
    let (d, s, l) = fixture().await;
    assert!(
        State::initialize(&d.path().join("outbox.sqlite"))
            .await
            .is_err()
    );
    let old = s.load(&l.key()).await.unwrap();
    let r = runtime::capture(&s, old.clone(), &input(d.path(), "PostToolUse", "x"))
        .await
        .unwrap();
    assert!(s.save(&old, &r).await.is_err());
}
#[cfg(unix)]
#[tokio::test]
async fn rejects_symlink_and_public_state() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let (d, _s, _l) = fixture().await;
    let p = d.path().join("outbox.sqlite");
    let alias = d.path().join("alias");
    symlink(&p, &alias).unwrap();
    assert!(State::open(&alias).await.is_err());
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(State::open(&p).await.is_err());
}
#[tokio::test]
async fn confirmed_event_rejection_needs_explicit_retry() {
    let (d, s, l) = fixture().await;
    let r = runtime::capture(
        &s,
        s.load(&l.key()).await.unwrap(),
        &input(d.path(), "PostToolUse", "x"),
    )
    .await
    .unwrap();
    let c = Fake::new();
    *c.reject_event.lock().unwrap() = true;
    assert!(runtime::deliver(&c, &s, r, false).await.is_err());
    let r = s.load(&l.key()).await.unwrap();
    assert!(r.event_rejected);
    let count = c.calls.lock().unwrap().len();
    assert!(runtime::deliver(&c, &s, r.clone(), false).await.is_err());
    assert_eq!(c.calls.lock().unwrap().len(), count);
    assert!(runtime::deliver(&c, &s, r, true).await.unwrap().completed);
}
#[tokio::test]
async fn altered_persisted_request_and_stage_fail_closed() {
    let (d, s, l) = fixture().await;
    let r = runtime::capture(
        &s,
        s.load(&l.key()).await.unwrap(),
        &input(d.path(), "PostToolUse", "x"),
    )
    .await
    .unwrap();
    let c = Fake::new();
    *c.uncertain_use.lock().unwrap() = true;
    assert!(runtime::deliver(&c, &s, r, false).await.is_err());
    let r = s.load(&l.key()).await.unwrap();
    let mut corrupt = r.clone();
    corrupt.use_request.as_mut().unwrap()["verification"] = json!("passed");
    assert!(corrupt.validate().is_err());
    assert!(runtime::deliver(&c, &s, corrupt, false).await.is_err());
    let mut corrupt = r.clone();
    corrupt.use_request.as_mut().unwrap()["reason"] = json!("RAW_SECRET_CANARY");
    assert!(corrupt.validate().is_err());
    let mut corrupt = r.clone();
    corrupt.event_request.as_mut().unwrap()["event"]["summary"] = json!("RAW_SECRET_CANARY");
    assert!(corrupt.validate().is_err());
    let mut corrupt = r;
    corrupt.event_id = None;
    assert!(corrupt.validate().is_err());
}
#[tokio::test]
async fn rejected_refresh_cannot_erase_observed_head_floor() {
    let (d, s, l) = fixture().await;
    let r = runtime::capture(
        &s,
        s.load(&l.key()).await.unwrap(),
        &input(d.path(), "PostToolUse", "x"),
    )
    .await
    .unwrap();
    let c = Fake::new();
    *c.head.lock().unwrap() = 3;
    *c.reject_use.lock().unwrap() = true;
    assert!(runtime::deliver(&c, &s, r, false).await.is_err());
    *c.head.lock().unwrap() = 2;
    assert!(
        runtime::deliver(&c, &s, s.load(&l.key()).await.unwrap(), true)
            .await
            .is_err()
    );
    let r = s.load(&l.key()).await.unwrap();
    assert_eq!(r.minimum_task_head, 3);
    assert_eq!(r.minimum_candidate_head, 3);
    *c.head.lock().unwrap() = 3;
    assert!(runtime::deliver(&c, &s, r, false).await.unwrap().completed);
}
#[tokio::test]
async fn mismatched_feedback_response_is_uncertain_not_complete() {
    let (d, s, l) = fixture().await;
    let r = runtime::capture(
        &s,
        s.load(&l.key()).await.unwrap(),
        &input(d.path(), "PostToolUse", "x"),
    )
    .await
    .unwrap();
    let c = Fake::new();
    *c.invalid_use.lock().unwrap() = true;
    assert!(runtime::deliver(&c, &s, r, false).await.is_err());
    let r = s.load(&l.key()).await.unwrap();
    assert!(!r.completed);
    assert!(!r.rejected);
    assert!(runtime::deliver(&c, &s, r, false).await.unwrap().completed);
}
#[tokio::test]
async fn full_outbox_preserves_existing_dedup_without_eviction() {
    let (d, s, l) = fixture().await;
    let pool = sqlx::SqlitePool::connect(&crate::support::sqlite_url::sqlite_file_url(
        &d.path().join("outbox.sqlite"),
    ))
    .await
    .unwrap();
    sqlx::query("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<9999) INSERT INTO host_result_outbox(id,payload) SELECT 'synthetic-'||x,'{}' FROM n").execute(&pool).await.unwrap();
    assert!(s.link(l.clone()).await.is_ok());
    let mut another = l;
    another.tool_id = "new-call".into();
    assert!(s.link(another).await.is_err());
    pool.close().await;
}
#[tokio::test]
async fn failed_candidate_read_cannot_erase_task_floor() {
    let (d, s, l) = fixture().await;
    let r = runtime::capture(
        &s,
        s.load(&l.key()).await.unwrap(),
        &input(d.path(), "PostToolUse", "x"),
    )
    .await
    .unwrap();
    let c = Fake::new();
    *c.head.lock().unwrap() = 3;
    *c.fail_candidate_read.lock().unwrap() = true;
    assert!(runtime::deliver(&c, &s, r, false).await.is_err());
    let r = s.load(&l.key()).await.unwrap();
    assert_eq!(r.minimum_task_head, 3);
    assert!(r.use_request.is_none());
    *c.head.lock().unwrap() = 2;
    assert!(runtime::deliver(&c, &s, r, false).await.is_err());
    assert_eq!(s.load(&l.key()).await.unwrap().minimum_task_head, 3);
}
