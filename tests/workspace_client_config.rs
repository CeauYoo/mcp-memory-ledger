#[cfg(windows)]
#[path = "support/windows_private.rs"]
mod windows_private;

use agent_llm_mm::support::{
    workspace_client_config::{ClientMode, inspect_registry, render_client_config},
    workspace_registry::{
        register_workspace, set_personal_grant, set_workspace_active, unbind_workspace_root,
    },
};
use std::{fs, path::PathBuf};

fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf, String) {
    let temp = tempfile::tempdir().unwrap();
    #[cfg(windows)]
    windows_private::private_directory(temp.path());
    let dir = temp.path().canonicalize().unwrap();
    #[cfg(not(windows))]
    let root = dir.join("项目 with \"quotes\"");
    #[cfg(windows)]
    let root = dir.join("项目 with spaces");
    fs::create_dir(&root).unwrap();
    let registry = dir.join("registry.json");
    #[cfg(not(windows))]
    let binary = dir.join("binary with \"引号\"");
    #[cfg(windows)]
    let binary = dir.join("binary with 空格");
    fs::write(&binary, "fixture").unwrap();
    let registration = register_workspace(&registry, &root).unwrap();
    (temp, registry, root, binary, registration.id)
}

#[test]
fn inspection_is_read_only_and_reports_effective_grants_separately() {
    let (_temp, registry, root, _, id) = fixture();
    set_personal_grant(&registry, &id, "user/alice", true, true).unwrap();
    let before = fs::read(&registry).unwrap();
    let report = inspect_registry(&registry, None, Some(&root)).unwrap();
    assert_eq!(report.total, 1);
    assert_eq!(
        report.workspaces[0].effective_personal_grants.len(),
        usize::from(cfg!(any(unix, windows)))
    );
    assert!(report.core_runtime.starts_with("not checked"));
    assert_eq!(before, fs::read(&registry).unwrap());
    set_workspace_active(&registry, &id, false).unwrap();
    let report = inspect_registry(&registry, None, None).unwrap();
    assert!(!report.workspaces[0].configuration_available);
    assert_eq!(report.workspaces[0].configured_personal_grants.len(), 1);
    assert!(report.workspaces[0].effective_personal_grants.is_empty());
    unbind_workspace_root(&registry, &id, &root).unwrap();
    let report = inspect_registry(&registry, Some(&id), None).unwrap();
    assert!(!report.workspaces[0].bound);
}

#[cfg(any(unix, windows))]
#[test]
fn toml_roundtrip_escapes_paths_and_uses_exact_cwd_without_writes() {
    let (temp, registry, root, binary, id) = fixture();
    #[cfg(not(windows))]
    let socket = temp.path().join("ipc/quoted \"socket\"");
    #[cfg(windows)]
    let socket = temp.path().join("ipc/管道 with spaces");
    let before = fs::read(&registry).unwrap();
    let text = render_client_config(
        &registry,
        &socket,
        &binary,
        ClientMode::Fixed,
        None,
        None,
        &root,
    )
    .unwrap();
    let parsed: toml::Value = toml::from_str(&text).unwrap();
    let server = &parsed["mcp_servers"]["memory-ledger"];
    assert_eq!(
        server["command"].as_str().unwrap(),
        binary.to_str().unwrap()
    );
    let args = server["args"].as_array().unwrap();
    assert_eq!(args[2].as_str().unwrap(), socket.to_str().unwrap());
    assert_eq!(args[4].as_str().unwrap(), id);
    assert_eq!(args[6].as_str().unwrap(), root.to_str().unwrap());
    let text = render_client_config(
        &registry,
        &socket,
        &binary,
        ClientMode::Global,
        None,
        None,
        &root,
    )
    .unwrap();
    assert!(text.contains("--registry"));
    assert!(!text.contains("--workspace-id"));
    let nested = root.join("nested");
    fs::create_dir(&nested).unwrap();
    assert!(
        render_client_config(
            &registry,
            &socket,
            &binary,
            ClientMode::Global,
            None,
            None,
            &nested
        )
        .is_err()
    );
    assert!(
        render_client_config(
            &registry,
            &socket,
            &binary,
            ClientMode::Global,
            Some(&id),
            None,
            &root
        )
        .is_err()
    );
    assert_eq!(before, fs::read(&registry).unwrap());
    assert!(!socket.exists());
    set_workspace_active(&registry, &id, false).unwrap();
    assert!(
        render_client_config(
            &registry,
            &socket,
            &binary,
            ClientMode::Fixed,
            None,
            None,
            &root
        )
        .is_err()
    );
}

#[test]
fn missing_root_is_visible_and_unbound_cannot_render() {
    let (temp, registry, root, binary, id) = fixture();
    fs::remove_dir(&root).unwrap();
    let report = inspect_registry(&registry, Some(&id), None).unwrap();
    assert!(!report.workspaces[0].roots[0].valid);
    assert!(!report.workspaces[0].configuration_available);
    assert!(
        render_client_config(
            &registry,
            &temp.path().join("core.sock"),
            &binary,
            ClientMode::Fixed,
            Some(&id),
            None,
            temp.path()
        )
        .is_err()
    );
    unbind_workspace_root(&registry, &id, &root).unwrap();
    let report = inspect_registry(&registry, Some(&id), None).unwrap();
    assert!(report.workspaces[0].active);
    assert!(!report.workspaces[0].bound);
}

#[test]
fn cli_inspect_status_config_and_help_parse_fail_closed() {
    use agent_llm_mm::interfaces::local_core::cli::{LocalCommand, WorkspaceAction, parse_args};
    let parse = |tail: &[&str]| {
        parse_args(
            std::iter::once("app")
                .chain(tail.iter().copied())
                .map(str::to_owned),
        )
    };
    assert!(matches!(
        parse(&["workspace", "--help"]).unwrap(),
        Some(LocalCommand::WorkspaceHelp)
    ));
    #[cfg(any(unix, windows))]
    {
        let registry = if cfg!(windows) { r"C:\r" } else { "/r" };
        let socket = if cfg!(windows) { r"C:\s" } else { "/s" };
        let root = if cfg!(windows) { r"C:\p" } else { "/p" };
        assert!(matches!(
            parse(&["workspace", "--registry", registry, "status"]).unwrap(),
            Some(LocalCommand::Workspace {
                action: WorkspaceAction::Status { .. },
                ..
            })
        ));
        assert!(
            parse(&[
                "workspace",
                "--registry",
                registry,
                "client-config",
                "--mode",
                "global",
                "--socket",
                socket,
                "--root",
                root
            ])
            .is_err()
        );
        assert!(
            parse(&[
                "workspace",
                "--registry",
                registry,
                "client-config",
                "--mode",
                "fixed"
            ])
            .is_err()
        );
    }
}

#[cfg(any(unix, windows))]
#[test]
fn cli_outputs_parseable_toml_without_loading_provider_or_creating_socket() {
    use std::process::Command;
    let (temp, registry, root, _, _) = fixture();
    let socket = temp.path().join("no-service.sock");
    let before = fs::read(&registry).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_agent_llm_mm"))
        .current_dir(&root)
        .env(
            "AGENT_LLM_MM_CONFIG",
            temp.path().join("DOES-NOT-EXIST.toml"),
        )
        .args(["workspace", "--registry"])
        .arg(&registry)
        .args(["client-config", "--mode", "fixed", "--socket"])
        .arg(&socket)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = std::str::from_utf8(&output.stdout).unwrap();
    let config: toml::Value = toml::from_str(text).unwrap();
    assert_eq!(
        config["mcp_servers"]["memory-ledger"]["command"]
            .as_str()
            .unwrap(),
        env!("CARGO_BIN_EXE_agent_llm_mm")
    );
    assert_eq!(before, fs::read(&registry).unwrap());
    assert!(!socket.exists());
    assert!(fs::read_dir(&root).unwrap().next().is_none());
}
