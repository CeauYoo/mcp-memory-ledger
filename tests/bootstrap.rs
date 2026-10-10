use agent_llm_mm::{
    RunOutput,
    interfaces::mcp::server::AUTO_REFLECTION_RUNTIME_HOOKS,
    run_command, run_doctor, startup_transport_from_default_config,
    support::{
        cli::{AppCommand, DoctorMode, command_from_args},
        config::{AppConfig, DATABASE_URL_ENV_VAR, ModelConfig, ModelProviderKind, TransportKind},
    },
};
use std::{
    collections::HashMap,
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Mutex, OnceLock, mpsc},
    time::Duration,
    vec,
};
use tempfile::tempdir;

#[test]
fn default_config_uses_stdio_transport() {
    let config = AppConfig::default();
    assert_eq!(config.transport, TransportKind::Stdio);
}

#[test]
fn default_config_uses_file_backed_sqlite_database() {
    let config = AppConfig::default();

    assert_ne!(config.database_url, "sqlite::memory:");
    assert!(config.database_url.starts_with("sqlite://"));
}

#[test]
#[cfg(target_os = "macos")]
fn default_config_uses_user_scoped_database_under_application_support() {
    let temp_dir = tempdir().expect("temp dir");
    let home_dir = temp_dir.path().join("home");
    let _guard = EnvGuard::apply([
        EnvChange::Unset(DATABASE_URL_ENV_VAR),
        EnvChange::Unset("XDG_DATA_HOME"),
        EnvChange::Set("HOME", home_dir.to_string_lossy().into_owned()),
    ]);

    let config = AppConfig::default();

    assert_eq!(
        config.database_url,
        sqlite_url(
            &home_dir
                .join("Library")
                .join("Application Support")
                .join("agent-llm-mm")
                .join("agent-llm-mm.sqlite")
        )
    );
}

#[test]
#[cfg(all(unix, not(target_os = "macos")))]
fn default_config_uses_xdg_data_home_for_user_scoped_database() {
    let temp_dir = tempdir().expect("temp dir");
    let home_dir = temp_dir.path().join("home");
    let xdg_data_home = temp_dir.path().join("xdg-data");
    let _guard = EnvGuard::apply([
        EnvChange::Unset(DATABASE_URL_ENV_VAR),
        EnvChange::Set("HOME", home_dir.to_string_lossy().into_owned()),
        EnvChange::Set(
            "XDG_DATA_HOME",
            xdg_data_home.to_string_lossy().into_owned(),
        ),
    ]);

    let config = AppConfig::default();

    assert_eq!(
        config.database_url,
        sqlite_url(
            &xdg_data_home
                .join("agent-llm-mm")
                .join("agent-llm-mm.sqlite")
        )
    );
}

#[test]
#[cfg(windows)]
fn default_config_uses_local_app_data_for_user_scoped_database() {
    let temp_dir = tempdir().expect("temp dir");
    let local_app_data = temp_dir.path().join("AppData").join("Local");
    let _guard = EnvGuard::apply([
        EnvChange::Unset(DATABASE_URL_ENV_VAR),
        EnvChange::Set(
            "LOCALAPPDATA",
            local_app_data.to_string_lossy().into_owned(),
        ),
    ]);

    let config = AppConfig::default();

    assert_eq!(
        config.database_url,
        sqlite_url(
            &local_app_data
                .join("agent-llm-mm")
                .join("agent-llm-mm.sqlite")
        )
    );
}

#[test]
fn default_config_prefers_explicit_database_url_env_override() {
    let _guard = EnvGuard::apply([
        EnvChange::Set(
            DATABASE_URL_ENV_VAR,
            "sqlite:///tmp/agent-llm-mm-explicit.sqlite".to_string(),
        ),
        EnvChange::Unset("XDG_DATA_HOME"),
    ]);

    let config = AppConfig::default();

    assert_eq!(
        config.database_url,
        "sqlite:///tmp/agent-llm-mm-explicit.sqlite"
    );
}

#[test]
fn startup_transport_uses_default_config_stdio() {
    assert_eq!(
        startup_transport_from_default_config(),
        TransportKind::Stdio
    );
}

#[test]
fn cli_defaults_to_serve_when_no_subcommand_is_provided() {
    let command = command_from_args(vec!["agent_llm_mm".to_string()]).expect("command");

    assert_eq!(command, AppCommand::Serve);
}

#[test]
fn cli_accepts_doctor_subcommand() {
    let command =
        command_from_args(vec!["agent_llm_mm".to_string(), "doctor".to_string()]).expect("command");

    assert_eq!(command, AppCommand::Doctor(DoctorMode::ReadOnly));
}

#[test]
fn cli_accepts_explicit_database_lifecycle_and_doctor_modes() {
    assert_eq!(
        command_from_args(vec!["agent_llm_mm".into(), "init".into()]).expect("init"),
        AppCommand::Init
    );
    assert_eq!(
        command_from_args(vec!["agent_llm_mm".into(), "migrate".into()]).expect("migrate"),
        AppCommand::Migrate
    );
    assert_eq!(
        command_from_args(vec![
            "agent_llm_mm".into(),
            "doctor".into(),
            "--read-only".into(),
        ])
        .expect("read-only doctor"),
        AppCommand::Doctor(DoctorMode::ReadOnly)
    );
    assert_eq!(
        command_from_args(vec![
            "agent_llm_mm".into(),
            "doctor".into(),
            "--allow-bootstrap".into(),
        ])
        .expect("bootstrap doctor"),
        AppCommand::Doctor(DoctorMode::AllowBootstrap)
    );
}

#[test]
fn cli_rejects_unknown_subcommand() {
    let error = command_from_args(vec!["agent_llm_mm".to_string(), "wat".to_string()])
        .expect_err("unknown command should fail");

    assert!(error.to_string().contains("unsupported command"));
}

#[test]
fn shell_entry_pins_main_binary_when_auxiliary_bins_exist() {
    let script = fs::read_to_string("scripts/agent-llm-mm.sh").expect("script should be readable");
    let powershell = fs::read_to_string("scripts/agent-llm-mm.ps1")
        .expect("PowerShell script should be readable");

    assert!(
        script.contains("cargo run --quiet --bin agent_llm_mm --"),
        "script must select the main binary explicitly when auxiliary src/bin targets exist"
    );
    assert!(
        powershell.contains("cargo run --quiet --bin agent_llm_mm --"),
        "PowerShell script must select the main binary explicitly when auxiliary src/bin targets exist"
    );
}

#[test]
fn cli_tracing_writes_to_stderr_without_corrupting_json_stdout() {
    let temp_dir = tempdir().expect("temp dir");
    let database_url = sqlite_url(&temp_dir.path().join("tracing-doctor.sqlite"));
    let output = Command::new(env!("CARGO_BIN_EXE_agent_llm_mm"))
        .args(["doctor", "--read-only"])
        .current_dir(temp_dir.path())
        .env(DATABASE_URL_ENV_VAR, database_url)
        .env("RUST_LOG", "agent_llm_mm=debug")
        .output()
        .expect("doctor process");

    assert!(
        output.status.success(),
        "doctor failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout must contain only doctor JSON");
    assert_eq!(stdout["database_lifecycle"]["status"], "missing");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("CLI command selected"));
    assert!(!stderr.contains("\"database_lifecycle\""));
}

#[test]
fn wrapper_scripts_reject_unsupported_modes_with_exit_code_two() {
    let script = fs::read_to_string("scripts/agent-llm-mm.sh").expect("script should be readable");
    let powershell = fs::read_to_string("scripts/agent-llm-mm.ps1")
        .expect("PowerShell script should be readable");
    if command_exists("bash") {
        let shell_output = Command::new("bash")
            .args(["scripts/agent-llm-mm.sh", "nope"])
            .output()
            .expect("shell wrapper should run");

        assert_eq!(
            shell_output.status.code(),
            Some(2),
            "shell wrapper should reject unsupported modes with exit code 2; stderr={}",
            String::from_utf8_lossy(&shell_output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&shell_output.stderr).contains(
                "usage: ./scripts/agent-llm-mm.sh [serve|init|migrate|doctor|bootstrap-local] [config_path]"
            ),
            "shell wrapper should print supported mode/config path contract"
        );
    }
    if command_exists("pwsh") {
        let powershell_output = Command::new("pwsh")
            .args(["-NoProfile", "-File", "scripts/agent-llm-mm.ps1", "nope"])
            .output()
            .expect("PowerShell wrapper should run");

        assert_eq!(
            powershell_output.status.code(),
            Some(2),
            "PowerShell wrapper should reject unsupported modes with exit code 2; stderr={}",
            String::from_utf8_lossy(&powershell_output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&powershell_output.stderr).contains(
                "usage: pwsh -File .\\scripts\\agent-llm-mm.ps1 [serve|init|migrate|doctor|bootstrap-local] [config_path]"
            ),
            "PowerShell wrapper should print supported mode/config path contract"
        );
    }
    assert!(
        powershell.contains("exit 2"),
        "PowerShell wrapper should reject unsupported modes with exit code 2"
    );
    assert!(
        script.contains(
            "usage: ./scripts/agent-llm-mm.sh [serve|init|migrate|doctor|bootstrap-local] [config_path]"
        ),
        "shell wrapper should document the supported mode/config path contract"
    );
    assert!(
        powershell.contains(
            "usage: pwsh -File .\\scripts\\agent-llm-mm.ps1 [serve|init|migrate|doctor|bootstrap-local] [config_path]"
        ),
        "PowerShell wrapper should document the supported mode/config path contract"
    );
    assert!(
        script.contains("agent-llm-mm.local.toml"),
        "shell wrapper should document the default bootstrap-local target"
    );
    assert!(
        powershell.contains("agent-llm-mm.local.toml"),
        "PowerShell wrapper should document the default bootstrap-local target"
    );
    assert!(
        script.contains("cd \"$project_root\"")
            && powershell.contains("Push-Location -LiteralPath $projectRoot"),
        "wrapper scripts should resolve relative bootstrap-local targets from the repository root"
    );
}

#[test]
fn bash_bootstrap_local_copies_dev_example_to_requested_target() {
    if !command_exists("bash") {
        return;
    }
    let temp_dir = tempdir().expect("temp dir");
    let target = temp_dir.path().join("agent llm mm.local.toml");

    let output = Command::new("bash")
        .args([
            "scripts/agent-llm-mm.sh",
            "bootstrap-local",
            target.to_str().expect("utf-8 target"),
        ])
        .output()
        .expect("shell wrapper should run");

    assert!(
        output.status.success(),
        "bootstrap-local should create missing target; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        read_file(&target),
        read_file("examples/agent-llm-mm.dev.example.toml")
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("Next commands:"),
        "bootstrap-local should print next commands"
    );
    let escaped_target = target.to_string_lossy().replace(' ', "\\ ");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&format!("init {escaped_target}")),
        "bootstrap-local should print a shell-safe init command for paths with spaces; stdout={stdout}"
    );
    assert!(
        stdout.contains(&format!("doctor --read-only {escaped_target}")),
        "bootstrap-local should print a shell-safe doctor command for paths with spaces; stdout={stdout}"
    );
    assert!(
        stdout.contains(&format!("serve {escaped_target}")),
        "bootstrap-local should print a shell-safe serve command for paths with spaces; stdout={stdout}"
    );
}

#[test]
fn bash_bootstrap_local_resolves_relative_targets_from_repo_root() {
    if !command_exists("bash") {
        return;
    }
    let temp_dir = tempdir().expect("temp dir");
    let repo_root = std::env::current_dir().expect("repo root");
    let unique_name = temp_dir
        .path()
        .file_name()
        .expect("temp dir name")
        .to_string_lossy();
    let relative_target = format!("target/bootstrap-local-relative-{unique_name}.toml");
    let repo_target = repo_root.join(&relative_target);
    let caller_target = temp_dir.path().join(&relative_target);
    let _ = fs::remove_file(&repo_target);

    let output = Command::new("bash")
        .arg(repo_root.join("scripts/agent-llm-mm.sh"))
        .args(["bootstrap-local", relative_target.as_str()])
        .current_dir(temp_dir.path())
        .output()
        .expect("shell wrapper should run");

    assert!(
        output.status.success(),
        "bootstrap-local should resolve relative targets from the repository root; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        repo_target.exists(),
        "relative bootstrap-local target should be created under the repository root"
    );
    assert!(
        !caller_target.exists(),
        "relative bootstrap-local target should not be created under the caller cwd"
    );
    assert_eq!(
        read_file(&repo_target),
        read_file("examples/agent-llm-mm.dev.example.toml")
    );

    fs::remove_file(repo_target).expect("cleanup repo-relative target");
}

#[test]
fn bash_bootstrap_local_refuses_to_overwrite_existing_target() {
    if !command_exists("bash") {
        return;
    }
    let temp_dir = tempdir().expect("temp dir");
    let target = temp_dir.path().join("agent-llm-mm.local.toml");
    fs::write(&target, "existing config").expect("seed target");

    let output = Command::new("bash")
        .args([
            "scripts/agent-llm-mm.sh",
            "bootstrap-local",
            target.to_str().expect("utf-8 target"),
        ])
        .output()
        .expect("shell wrapper should run");

    assert!(
        !output.status.success(),
        "bootstrap-local should reject existing target"
    );
    assert_eq!(read_file(&target), "existing config");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("already exists"),
        "bootstrap-local should explain overwrite refusal"
    );
}

#[test]
#[cfg(unix)]
fn bash_bootstrap_local_refuses_dangling_symlink_target() {
    if !command_exists("bash") {
        return;
    }
    let temp_dir = tempdir().expect("temp dir");
    let target = temp_dir.path().join("agent-llm-mm.local.toml");
    let missing_destination = temp_dir.path().join("missing-destination.toml");
    std::os::unix::fs::symlink(&missing_destination, &target).expect("seed dangling symlink");

    let output = Command::new("bash")
        .args([
            "scripts/agent-llm-mm.sh",
            "bootstrap-local",
            target.to_str().expect("utf-8 target"),
        ])
        .output()
        .expect("shell wrapper should run");

    assert!(
        !output.status.success(),
        "bootstrap-local should reject dangling symlink targets"
    );
    assert!(
        target
            .symlink_metadata()
            .expect("symlink metadata")
            .file_type()
            .is_symlink(),
        "bootstrap-local must not replace a dangling symlink target"
    );
    assert!(
        !target.exists(),
        "test target should remain a dangling symlink"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("already exists"),
        "bootstrap-local should treat dangling symlinks as occupied targets"
    );
}

#[test]
fn bootstrap_local_copy_uses_no_clobber_primitives() {
    let script = fs::read_to_string("scripts/agent-llm-mm.sh").expect("script should be readable");
    let powershell = fs::read_to_string("scripts/agent-llm-mm.ps1")
        .expect("PowerShell script should be readable");

    assert!(
        script.contains("noclobber"),
        "shell bootstrap-local should use no-clobber creation instead of check-then-copy"
    );
    assert!(
        powershell.contains("[System.IO.File]::Copy") && powershell.contains("$false"),
        "PowerShell bootstrap-local should use File.Copy with overwrite=false"
    );
    assert!(
        powershell.contains("$quotedTargetPath"),
        "PowerShell bootstrap-local should quote generated next commands"
    );
}

#[test]
fn bash_bootstrap_local_rejects_missing_parent_directory() {
    if !command_exists("bash") {
        return;
    }
    let temp_dir = tempdir().expect("temp dir");
    let target = temp_dir
        .path()
        .join("missing-parent")
        .join("agent-llm-mm.local.toml");

    let output = Command::new("bash")
        .args([
            "scripts/agent-llm-mm.sh",
            "bootstrap-local",
            target.to_str().expect("utf-8 target"),
        ])
        .output()
        .expect("shell wrapper should run");

    assert!(
        !output.status.success(),
        "bootstrap-local should reject missing parent directories"
    );
    assert!(
        !target.exists(),
        "bootstrap-local must not create missing parent directories"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("parent directory does not exist"),
        "bootstrap-local should explain missing parent directory"
    );
}

#[test]
fn powershell_bootstrap_local_matches_bash_behavior_when_pwsh_exists() {
    if !command_exists("pwsh") {
        return;
    }
    let temp_dir = tempdir().expect("temp dir");
    let target = temp_dir.path().join("agent-llm-mm.local.toml");

    let create_output = Command::new("pwsh")
        .args([
            "-NoProfile",
            "-File",
            "scripts/agent-llm-mm.ps1",
            "bootstrap-local",
            target.to_str().expect("utf-8 target"),
        ])
        .output()
        .expect("PowerShell wrapper should run");

    assert!(
        create_output.status.success(),
        "PowerShell bootstrap-local should create missing target; stderr={}",
        String::from_utf8_lossy(&create_output.stderr)
    );
    assert_eq!(
        read_file(&target),
        read_file("examples/agent-llm-mm.dev.example.toml")
    );
    assert!(
        String::from_utf8_lossy(&create_output.stdout).contains("Next commands:"),
        "PowerShell bootstrap-local should print next commands"
    );

    let overwrite_output = Command::new("pwsh")
        .args([
            "-NoProfile",
            "-File",
            "scripts/agent-llm-mm.ps1",
            "bootstrap-local",
            target.to_str().expect("utf-8 target"),
        ])
        .output()
        .expect("PowerShell wrapper should run");

    assert!(
        !overwrite_output.status.success(),
        "PowerShell bootstrap-local should reject existing target"
    );
    assert_eq!(
        read_file(&target),
        read_file("examples/agent-llm-mm.dev.example.toml")
    );
    assert!(
        String::from_utf8_lossy(&overwrite_output.stderr).contains("already exists"),
        "PowerShell bootstrap-local should explain overwrite refusal"
    );
}

fn command_exists(command: &str) -> bool {
    Command::new(command)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn read_file(path: impl Into<PathBuf>) -> String {
    fs::read_to_string(path.into()).expect("file should be readable")
}

#[tokio::test]
async fn doctor_read_only_reports_missing_database_without_bootstrapping() {
    let temp_dir = tempdir().expect("temp dir");
    let database_path = temp_dir.path().join("doctor.sqlite");
    let database_url = format!(
        "sqlite://{}",
        database_path.to_string_lossy().replace('\\', "/")
    );
    let config = AppConfig {
        transport: TransportKind::Stdio,
        database_url: database_url.clone(),
        model_provider: ModelProviderKind::Mock,
        model_config: ModelConfig::Mock,
        dashboard: Default::default(),
        ..Default::default()
    };

    let report = run_doctor(config).await.expect("doctor should pass");

    assert_eq!(report.transport, TransportKind::Stdio);
    assert_eq!(report.database_url, database_url);
    assert_eq!(report.provider, ModelProviderKind::Mock);
    assert_eq!(report.base_url, None);
    assert_eq!(report.model, None);
    assert_eq!(report.status, "attention_required");
    assert_eq!(report.database_lifecycle.status, "missing");
    assert!(!report.database_lifecycle.bootstrap_performed);
    assert!(
        !database_path.exists(),
        "read-only doctor must not create sqlite database"
    );
}

#[tokio::test]
async fn doctor_allow_bootstrap_explicitly_initializes_missing_database() {
    let temp_dir = tempdir().expect("temp dir");
    let database_path = temp_dir.path().join("doctor-bootstrap.sqlite");
    let config = AppConfig {
        database_url: sqlite_url(&database_path),
        ..Default::default()
    };

    let output = run_command(AppCommand::Doctor(DoctorMode::AllowBootstrap), config)
        .await
        .expect("bootstrap doctor")
        .expect("doctor output");
    let RunOutput::Doctor(report) = output else {
        panic!("expected doctor output");
    };

    assert_eq!(report.status, "ok");
    assert_eq!(report.database_lifecycle.status, "current");
    assert_eq!(
        report.database_lifecycle.operation,
        "doctor_allow_bootstrap"
    );
    assert!(report.database_lifecycle.bootstrap_performed);
    assert!(database_path.is_file());
}

#[tokio::test]
async fn doctor_reports_self_revision_runtime_coverage() {
    let temp_dir = tempdir().expect("temp dir");
    let database_url = format!(
        "sqlite://{}",
        temp_dir
            .path()
            .join("doctor-runtime-coverage.sqlite")
            .to_string_lossy()
            .replace('\\', "/")
    );
    let config = AppConfig {
        database_url,
        ..Default::default()
    };

    let report = run_doctor(config).await.expect("doctor should pass");

    assert_eq!(
        report.auto_reflection_runtime_hooks,
        vec![
            "ingest_interaction:failure".to_string(),
            "ingest_interaction:conflict".to_string(),
            "decide_with_snapshot:conflict".to_string(),
            "build_self_snapshot:periodic".to_string(),
        ]
    );
    assert_eq!(report.self_revision_write_path, "run_reflection");

    let serialized = serde_json::to_value(&report).expect("doctor report JSON");
    assert_eq!(
        serialized["auto_reflection_runtime_hooks"],
        serde_json::json!([
            "ingest_interaction:failure",
            "ingest_interaction:conflict",
            "decide_with_snapshot:conflict",
            "build_self_snapshot:periodic"
        ])
    );
    assert_eq!(serialized["self_revision_write_path"], "run_reflection");
}

#[tokio::test]
async fn doctor_reports_dashboard_config_without_starting_dashboard() {
    let temp_dir = tempdir().expect("temp dir");
    let database_url = format!(
        "sqlite://{}",
        temp_dir
            .path()
            .join("doctor-dashboard.sqlite")
            .to_string_lossy()
            .replace('\\', "/")
    );
    let config = AppConfig {
        transport: TransportKind::Stdio,
        database_url,
        model_provider: ModelProviderKind::Mock,
        model_config: ModelConfig::Mock,
        dashboard: agent_llm_mm::support::config::DashboardConfig {
            enabled: true,
            host: "127.0.0.1".to_string(),
            port: 8787,
            base_path: "/agent-llm-mm".to_string(),
            event_capacity: 2000,
            sse_enabled: true,
            open_browser: false,
            required: true,
        },
        ..Default::default()
    };

    let report = run_doctor(config).await.expect("doctor should pass");

    assert!(report.dashboard_enabled);
    assert_eq!(report.dashboard_host, "127.0.0.1");
    assert_eq!(report.dashboard_port, 8787);
    assert_eq!(report.dashboard_base_path, "/agent-llm-mm");
    assert!(report.dashboard_required);
}

#[tokio::test]
async fn default_command_serves_stdio_after_explicit_database_init() {
    let temp_dir = tempdir().expect("temp dir");
    let database_url = sqlite_url(&temp_dir.path().join("stdio.sqlite"));
    agent_llm_mm::adapters::sqlite::initialize_database(&database_url)
        .await
        .expect("initialize isolated stdio database");

    // Do not inherit the test runner's stdin, local config, or process-wide env
    // mutations from other bootstrap tests. The default CLI command is serve.
    let mut child = StdioChild(
        Command::new(env!("CARGO_BIN_EXE_agent_llm_mm"))
            .current_dir(temp_dir.path())
            .env_clear()
            .env(DATABASE_URL_ENV_VAR, &database_url)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn default stdio command"),
    );
    let mut stdin = child.0.stdin.take().expect("child stdin");
    let stdout = child.0.stdout.take().expect("child stdout");
    let (sender, receiver) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if sender.send(line).is_err() {
                break;
            }
        }
    });

    writeln!(
        stdin,
        "{}",
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": {"name": "bootstrap-test", "version": "0.1.0"}
            }
        })
    )
    .expect("send initialize");
    stdin.flush().expect("flush initialize");
    let initialize = receive_stdio_message(&receiver);
    assert_eq!(initialize["id"], 1);
    assert!(initialize.get("error").is_none(), "{initialize}");
    assert!(
        initialize["result"]["serverInfo"].is_object(),
        "{initialize}"
    );

    writeln!(
        stdin,
        "{}\n{}",
        serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}})
    )
    .expect("send initialized and tools/list");
    stdin.flush().expect("flush tools/list");
    let tools = receive_stdio_message(&receiver);
    assert_eq!(tools["id"], 2);
    assert!(tools.get("error").is_none(), "{tools}");
    assert!(
        tools["result"]["tools"]
            .as_array()
            .is_some_and(|tools| !tools.is_empty()),
        "{tools}"
    );
    assert!(child.0.try_wait().expect("check server status").is_none());

    drop(stdin);
    drop(child);
    reader.join().expect("join stdout reader");
}

fn receive_stdio_message(receiver: &mpsc::Receiver<std::io::Result<String>>) -> serde_json::Value {
    let line = receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("stdio response before timeout or EOF")
        .expect("read stdio response");
    serde_json::from_str(&line).expect("JSON-RPC response")
}

struct StdioChild(Child);

impl Drop for StdioChild {
    fn drop(&mut self) {
        // Reap the process on success, assertion failure, and response timeout.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
async fn doctor_read_only_does_not_create_missing_parent_directories() {
    let temp_dir = tempdir().expect("temp dir");
    let database_path = temp_dir.path().join("missing-parent").join("serve.sqlite");
    let database_url = format!(
        "sqlite://{}",
        database_path.to_string_lossy().replace('\\', "/")
    );
    let config = AppConfig {
        transport: TransportKind::Stdio,
        database_url: database_url.clone(),
        model_provider: ModelProviderKind::Mock,
        model_config: ModelConfig::Mock,
        dashboard: Default::default(),
        ..Default::default()
    };

    let report = run_doctor(config).await.expect("doctor should pass");

    assert_eq!(report.database_url, database_url);
    assert_eq!(report.database_lifecycle.status, "missing");
    assert!(
        !database_path.exists() && !database_path.parent().expect("parent").exists(),
        "read-only doctor must not create the database or missing parent directories"
    );
}

#[tokio::test]
async fn serve_command_fails_fast_for_malformed_database_url() {
    let config = AppConfig {
        transport: TransportKind::Stdio,
        database_url: "postgres://not-a-sqlite-url".to_string(),
        model_provider: ModelProviderKind::Mock,
        model_config: ModelConfig::Mock,
        dashboard: Default::default(),
        ..Default::default()
    };

    let result = run_command(AppCommand::Serve, config).await;

    assert!(
        result.is_err(),
        "malformed database url should return an error"
    );
}

#[test]
fn runtime_hook_contract_is_exactly_four_hooks() {
    assert_eq!(
        AUTO_REFLECTION_RUNTIME_HOOKS,
        [
            "ingest_interaction:failure",
            "ingest_interaction:conflict",
            "decide_with_snapshot:conflict",
            "build_self_snapshot:periodic",
        ]
    );
}

fn sqlite_url(path: &Path) -> String {
    format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"))
}

enum EnvChange {
    Set(&'static str, String),
    Unset(&'static str),
}

struct EnvGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
    previous: HashMap<&'static str, Option<String>>,
}

impl EnvGuard {
    fn apply<const N: usize>(changes: [EnvChange; N]) -> Self {
        static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

        let lock = ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("env lock");

        let mut previous = HashMap::new();
        for change in changes {
            let key = match &change {
                EnvChange::Set(key, _) | EnvChange::Unset(key) => *key,
            };
            previous
                .entry(key)
                .or_insert_with(|| std::env::var(key).ok());

            match change {
                EnvChange::Set(key, value) => unsafe {
                    std::env::set_var(key, value);
                },
                EnvChange::Unset(key) => unsafe {
                    std::env::remove_var(key);
                },
            }
        }

        Self {
            _lock: lock,
            previous,
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, value) in &self.previous {
            match value {
                Some(value) => unsafe {
                    std::env::set_var(key, value);
                },
                None => unsafe {
                    std::env::remove_var(key);
                },
            }
        }
    }
}
