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
    assert!(
        workflow
            .contains("--test correction_atomicity --test feedback_provenance --test mcp_stdio")
    );
}

#[test]
fn cargo_and_rustup_share_the_verified_toolchain_floor() {
    let manifest = fs::read_to_string("Cargo.toml").expect("Cargo manifest");
    let toolchain = fs::read_to_string("rust-toolchain.toml").expect("toolchain manifest");

    assert!(manifest.contains("rust-version = \"1.95\""));
    assert!(toolchain.contains("channel = \"1.95.0\""));
    assert!(toolchain.contains("components = [\"clippy\", \"rustfmt\"]"));
}
