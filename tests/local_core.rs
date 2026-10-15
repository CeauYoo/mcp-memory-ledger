//! Isolated protocol acceptance of the installed-style stdio bridge and local core.
//! These synthetic tests do not establish natural-language client tool selection.
#[cfg(unix)]
#[path = "local_core/unix.rs"]
mod unix;

#[cfg(windows)]
#[test]
fn windows_bridge_reports_unsupported_without_opening_configuration_or_network() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_agent_llm_mm"))
        .env_clear()
        .args([
            "bridge",
            "--socket",
            r"C:\synthetic\core.sock",
            "--workspace-id",
            "02e2c6e0-874b-46a4-bfc0-44d275ce3674",
            "--workspace-root",
            r"C:\synthetic\project",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires Unix sockets"));
}
