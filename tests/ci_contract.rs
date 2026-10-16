use std::fs;

#[test]
fn ci_covers_linux_and_macos_quality_gates() {
    let workflow = fs::read_to_string(".github/workflows/ci.yml").expect("CI workflow");

    for expected in [
        "ubuntu-latest",
        "macos-latest",
        "cargo fmt --all -- --check",
        "cargo clippy --all-targets --all-features -- -D warnings",
        "./scripts/test-tier.sh full",
        "./scripts/status-sync-check.sh",
    ] {
        assert!(
            workflow.contains(expected),
            "CI workflow must contain {expected}"
        );
    }
    assert!(workflow.contains("windows-latest"));
    assert!(workflow.contains("scripts/local-memory-smoke.py"));
    for suite in [
        "correction_atomicity",
        "feedback_provenance",
        "feedback_candidates",
        "experience_workflow",
        "experience_use_feedback",
        "dashboard_admin",
        "indexed_text_recall",
        "mcp_stdio",
        "schema7_migration",
        "self_model_versions",
        "version_api",
        "native_model_protocols",
        "openai_compatible_model",
        "provider_config",
        "workspace_registry",
        "workspace_client_config",
        "workspace_adoption",
        "mcp_session_policy",
        "local_core_cli",
        "local_core",
        "database_backup",
    ] {
        assert!(
            workflow.contains(&format!("--test {suite}")),
            "missing native CI suite {suite}"
        );
    }
    assert!(workflow.contains("scripts/evaluate-memory-loop.py"));
}

#[test]
fn cargo_and_rustup_share_the_verified_toolchain_floor() {
    let manifest = fs::read_to_string("Cargo.toml").expect("Cargo manifest");
    let toolchain = fs::read_to_string("rust-toolchain.toml").expect("toolchain manifest");

    assert!(manifest.contains("rust-version = \"1.95\""));
    assert!(toolchain.contains("channel = \"1.95.0\""));
    assert!(toolchain.contains("components = [\"clippy\", \"rustfmt\"]"));
}

#[test]
fn ci_executes_source_bound_packages_and_real_windows_wrapper() {
    let workflow = fs::read_to_string(".github/workflows/ci.yml").expect("CI workflow");
    assert!(workflow.contains("github.event.pull_request.head.sha || github.sha"));
    for contract in [
        "scripts/test-windows-wrapper.py",
        "test_portable_package.py",
        "scripts/portable-package.py build",
        "scripts/portable-package.py verify",
        "--expected-commit",
        "--expected-tree",
    ] {
        assert!(
            workflow.contains(contract),
            "missing executable CI contract: {contract}"
        );
    }
    assert_eq!(
        workflow
            .matches("scripts/portable-package.py build")
            .count(),
        2
    );
    assert_eq!(
        workflow
            .matches("scripts/portable-package.py verify")
            .count(),
        2
    );
    // Each Windows Python command is its own step: PowerShell otherwise lets a
    // later successful native command mask an earlier nonzero exit status.
    let windows = workflow
        .split("  windows-runtime:")
        .nth(1)
        .expect("Windows job");
    for (step, command) in [
        (
            "Verify feedback and experience proxy workflow",
            "python scripts/evaluate-memory-loop.py",
        ),
        (
            "Test offline evaluation fixtures",
            "python -m unittest discover",
        ),
        (
            "Verify temporal scope and export workflow",
            "python scripts/temporal-scope-export-smoke.py",
        ),
    ] {
        assert!(windows.contains(&format!("- name: {step}\n        run: {command}")));
    }
    // CI may retain narrowly selected synthetic verification evidence, never
    // publish a candidate binary/archive, database, private config or raw log.
    assert!(workflow.contains("scripts/verify-local-core.py"));
    assert!(workflow.contains("test_local_core_verifier.py"));
    assert!(workflow.contains("test_windows_local_core_verifier.py"));
    assert!(workflow.contains("scripts/dashboard-admin-browser-check.mjs"));
    assert!(!windows.contains("scripts/verify-local-core.py"));
    assert_eq!(workflow.matches("actions/upload-artifact@").count(), 2);
    assert!(windows.contains("test_windows_local_core_verifier.py"));
    assert!(windows.contains("target/portable-package-verify/local-core/summary.json"));
    let upload = workflow
        .split("uses: actions/upload-artifact@")
        .nth(1)
        .expect("synthetic verification evidence upload")
        .split("  windows-runtime:")
        .next()
        .unwrap();
    let paths = upload
        .split("          path: |\n")
        .nth(1)
        .expect("explicit evidence paths");
    let paths: Vec<_> = paths
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    assert_eq!(
        paths,
        [
            "target/core-verification/summary.json",
            "target/reports/codex-native-host.json",
            "target/portable-package-verify/summary.json",
            "target/portable-package-verify/SHA256SUMS",
            "target/portable-package-verify/local-core/summary.json",
            "${{ runner.temp }}/admin-browser-verification/report.json",
            "${{ runner.temp }}/admin-browser-verification/cleanup.json",
            "${{ runner.temp }}/admin-browser-verification/*.png",
        ]
    );
    assert!(!workflow.contains("continue-on-error:"));
}

#[test]
fn native_host_probe_is_pinned_local_and_keeps_only_safe_summary() {
    let workflow = fs::read_to_string(".github/workflows/ci.yml").unwrap();
    let step = workflow
        .split("- name: Verify native Codex task context")
        .nth(1)
        .unwrap()
        .split("      - name:")
        .next()
        .unwrap();
    assert!(step.contains("if: matrix.os == 'ubuntu-latest'"));
    assert!(step.contains("rust-v0.159.2"));
    assert!(step.contains("9e2d29a713b94478b240dec2f10e11324cd05fad76dc43e7c639bdf8a1337a6b"));
    assert!(step.contains("sha256sum --check --strict"));
    assert!(step.contains("--allow-synthetic-local-turn"));
    assert!(!step.contains("continue-on-error"));
}
