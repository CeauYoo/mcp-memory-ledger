#[cfg(windows)]
#[path = "support/windows_private.rs"]
mod windows_private;

use agent_llm_mm::adapters::codex_context::protocol::{self, Binding, HookInput, ReceiptStage};
use serde_json::{Value, json};
use std::{path::PathBuf, process::Command};

fn binding() -> Binding {
    Binding {
        host_session_id: "session-1".into(),
        workspace_id: "workspace-1".into(),
        namespace: "project/legacy".into(),
        task_id: "task-1".into(),
        workspace_root: PathBuf::from(if cfg!(windows) {
            "C:\\workspace"
        } else {
            "/workspace"
        }),
    }
}
fn input(source: &str) -> HookInput {
    HookInput::parse(&serde_json::to_vec(&json!({"session_id":"session-1","cwd":binding().workspace_root,"hook_event_name":"SessionStart","source":source,"transcript_path":"/must/not/read","prompt":"NOT A TRUSTED TASK REQUIREMENT"})).unwrap()).unwrap()
}
fn context() -> Value {
    json!({"namespace":"project/legacy","task_id":"task-1","task_version":2,"status":"active","context_id":"context-1","complete":true,"can_resume":true,"snapshot_consistent":false,"mandatory_snapshot_consistent":true,"mandatory_requirements":[{"id":"r1","text":"Keep original user constraint"}],"checkpoint":null,"diagnostics":[]})
}
#[test]
fn lifecycle_contract_uses_public_fields_only() {
    for source in ["startup", "resume", "clear", "compact", "fork"] {
        let event = input(source);
        let output = protocol::render(&event, &binding(), &context(), 8192, 1).unwrap();
        assert_eq!(
            output["hookSpecificOutput"]["hookEventName"],
            "SessionStart"
        );
        let text = output["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert!(text.contains("Keep original user constraint"));
        assert!(!text.contains("NOT A TRUSTED TASK REQUIREMENT"));
        assert!(!text.contains("/must/not/read"));
        assert!(output.get("permissionDecision").is_none());
    }
}
#[test]
fn malformed_unknown_oversized_events_are_rejected() {
    assert!(HookInput::parse(b"{}").is_err());
    assert!(HookInput::parse(&vec![b' '; protocol::MAX_INPUT_BYTES + 1]).is_err());
    let mut event = json!({"session_id":"s","cwd":binding().workspace_root,"hook_event_name":"PreToolUse","source":"startup"});
    assert!(HookInput::parse(&serde_json::to_vec(&event).unwrap()).is_err());
    event["hook_event_name"] = json!("SessionStart");
    event["source"] = json!("unknown");
    assert!(HookInput::parse(&serde_json::to_vec(&event).unwrap()).is_err());
}
#[test]
fn stale_incomplete_paused_and_cross_scope_do_not_emit_context() {
    for (key, value) in [
        ("complete", json!(false)),
        ("can_resume", json!(false)),
        ("status", json!("paused")),
        ("status", json!("cancelled")),
        ("namespace", json!("project/other")),
        ("task_id", json!("other")),
        ("mandatory_snapshot_consistent", json!(false)),
        ("diagnostics", json!(["stale_checkpoint"])),
    ] {
        let mut data = context();
        data[key] = value;
        assert!(
            protocol::render(&input("resume"), &binding(), &data, 8192, 1).is_err(),
            "{key}"
        );
    }
    assert!(protocol::render(&input("resume"), &binding(), &context(), 8192, 3).is_err());
}
#[test]
fn escaped_wire_envelope_is_bounded_without_truncation() {
    let mut data = context();
    data["mandatory_requirements"][0]["text"] = json!("\\\"".repeat(2000));
    assert!(protocol::render(&input("compact"), &binding(), &data, 8192, 1).is_err());
    let output = protocol::render(&input("compact"), &binding(), &context(), 8192, 1).unwrap();
    assert!(serde_json::to_vec(&output).unwrap().len() < 8192);
    assert!(protocol::budget(0).is_err());
    assert!(protocol::budget(32769).is_err());
}
#[test]
fn receipt_levels_do_not_equate_output_with_delivery() {
    assert_ne!(ReceiptStage::OutputWritten, ReceiptStage::HostDelivered);
    assert_ne!(ReceiptStage::ModelClaimed, ReceiptStage::ResultVerified);
    assert_eq!(
        serde_json::to_value(ReceiptStage::OutputWritten).unwrap(),
        "output_written"
    );
}
#[test]
fn preview_is_bounded_and_does_not_create_state_or_config() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("adapter.sqlite");
    let result = Command::new(env!("CARGO_BIN_EXE_agent_llm_mm"))
        .args(["codex-context", "preview", "--state"])
        .arg(&state)
        .arg("--registry")
        .arg(root.path().join("registry.json"))
        .arg("--socket")
        .arg(root.path().join("core.socket"))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(output["preview_only"], true);
    assert_eq!(output["max_context_bytes"], 8192);
    assert_eq!(
        output["hooks_json"]["hooks"]["SessionStart"][0]["hooks"][0]["additionalContextLimit"],
        0
    );
    assert!(!state.exists());
    assert_eq!(root.path().read_dir().unwrap().count(), 0);
}
#[test]
fn state_initialization_is_explicit_and_not_overwritten() {
    let root = tempfile::tempdir().unwrap();
    #[cfg(windows)]
    windows_private::private_directory(root.path());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let state = root.path().canonicalize().unwrap().join("adapter.sqlite");
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_agent_llm_mm"))
            .args(["codex-context", "init", "--state"])
            .arg(&state)
            .output()
            .unwrap()
    };
    let first = run();
    assert!(
        first.status.success(),
        "init stderr: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(!run().status.success());
}

#[test]
fn artifact_checks_are_local_bounded_and_do_not_promote_checkpoint_claims() {
    use agent_llm_mm::adapters::codex_context::artifacts::observe;
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("result.txt"), b"hello").unwrap();
    let hash = format!("sha256:{}", protocol::digest(b"hello"));
    let data = json!({"checkpoint":{"artifacts":[
        {"uri":"result.txt","digest":hash},
        {"uri":"missing.txt","digest":hash},
        {"uri":"https://example.invalid/private","digest":hash},
        {"uri":"../outside","digest":hash},
        {"uri":"result.txt","digest":"sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"},
        {"uri":"result.txt"}
    ]},"artifact_verification":"not_performed"});
    // Production bindings canonicalize the workspace root before artifact observation.
    // macOS temp paths may use /var aliases; Windows canonical paths use verbatim prefixes.
    let canonical_root = root.path().canonicalize().unwrap();
    let observed = observe(&canonical_root, &data);
    assert_eq!(observed[0]["observation"]["result"], "passed");
    assert_eq!(observed[1]["observation"]["reason"], "artifact_missing");
    assert_eq!(observed[2]["observation"]["result"], "not_performed");
    assert_eq!(observed[3]["observation"]["reason"], "outside_workspace");
    assert_eq!(observed[4]["observation"]["digest_matches"], false);
    assert_eq!(observed[5]["observation"]["result"], "not_performed");
    assert_eq!(data["artifact_verification"], "not_performed");
}
#[cfg(unix)]
#[test]
fn artifact_symlink_escape_is_not_read() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret"), b"outside").unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("link")).unwrap();
    let data = json!({"checkpoint":{"artifacts":[{"uri":"link/secret","digest":format!("sha256:{}",protocol::digest(b"outside"))}]}});
    let result = agent_llm_mm::adapters::codex_context::artifacts::observe(root.path(), &data);
    assert_eq!(
        result[0]["observation"]["reason"],
        "symlink_reference_unsupported"
    );
}

#[cfg(unix)]
#[test]
fn artifact_observation_uses_canonical_binding_with_aliased_parent() {
    use agent_llm_mm::adapters::codex_context::artifacts::observe;
    let parent = tempfile::tempdir().unwrap();
    let real_root = parent.path().join("workspace");
    std::fs::create_dir(&real_root).unwrap();
    std::fs::write(real_root.join("result.txt"), b"hello").unwrap();
    let alias = parent.path().join("workspace-alias");
    std::os::unix::fs::symlink(&real_root, &alias).unwrap();
    let data = json!({"checkpoint":{"artifacts":[{"uri":"result.txt",
        "digest":format!("sha256:{}", protocol::digest(b"hello"))}]}});
    // An unnormalized root must not weaken the observer's canonical boundary check.
    assert_eq!(
        observe(&alias, &data)[0]["observation"]["reason"],
        "outside_workspace"
    );
    let bound_root = alias.canonicalize().unwrap();
    assert_eq!(
        observe(&bound_root, &data)[0]["observation"]["result"],
        "passed"
    );
}
