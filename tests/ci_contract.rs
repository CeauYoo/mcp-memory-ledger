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
        "indexed_text_recall",
        "mcp_stdio",
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
    assert!(
        !workflow.contains("upload-artifact"),
        "local candidate checks must not publish artifacts"
    );
}
