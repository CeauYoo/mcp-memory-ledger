use agent_llm_mm::adapters::codex_actions::deployment::{PlacementStatus, TargetPlatform, inspect};
use std::process::Command;

fn status(platform: TargetPlatform, path: &str) -> PlacementStatus {
    inspect(platform, std::array::from_fn(|_| path.to_owned())).status
}

#[test]
fn ordinary_paths_are_never_verified_or_deployable() {
    for (platform, path) in [
        (TargetPlatform::Macos, "/Users/test/local"),
        (TargetPlatform::Linux, "/var/lib/ledger"),
        (TargetPlatform::Windows, r"c:\Local\Ledger"),
    ] {
        let report = inspect(platform, std::array::from_fn(|_| path.into()));
        assert_eq!(report.status, PlacementStatus::Unknown);
        assert!(!report.ready_to_deploy);
        assert!(!report.filesystem_verified);
        assert!(!report.sync_location_verified);
        assert!(!report.installation_performed);
        assert_eq!(
            report.placements.iter().map(|p| p.role).collect::<Vec<_>>(),
            ["config", "dependencies", "database", "logs", "build"]
        );
    }
}

#[test]
fn windows_lexical_rules_are_independent_of_running_os() {
    for path in [
        r"c:\users\test\oNeDrIvE - Work\db",
        "C:/Users/test/DropBox/db",
        r"C:\Users\test\iCloudDrive\db",
    ] {
        assert_eq!(
            status(TargetPlatform::Windows, path),
            PlacementStatus::Rejected,
            "{path}"
        );
    }
    let report = inspect(
        TargetPlatform::Windows,
        std::array::from_fn(|_| r"c:\Local\Ledger".into()),
    );
    assert_eq!(
        report.placements[0].normalized_path.as_deref(),
        Some("C:/Local/Ledger")
    );
    assert_eq!(
        status(TargetPlatform::Macos, r"c:\Local\Ledger"),
        PlacementStatus::Rejected
    );
    assert_eq!(
        status(TargetPlatform::Windows, "/var/lib/ledger"),
        PlacementStatus::Rejected
    );
}

#[test]
fn uncertain_windows_namespaces_stay_unknown_without_normalization() {
    for path in [
        r"\\server\share\ledger",
        r"\\?\C:\ledger",
        r"\\.\C:\ledger",
        "//server/share/ledger",
    ] {
        let report = inspect(
            TargetPlatform::Windows,
            std::array::from_fn(|_| path.into()),
        );
        assert_eq!(report.status, PlacementStatus::Unknown);
        assert!(
            report
                .placements
                .iter()
                .all(|p| p.normalized_path.is_none())
        );
    }
}

#[test]
fn reject_relative_traversal_reserved_and_ambiguous_windows_paths() {
    for path in [
        "ledger",
        "C:ledger",
        r"\ledger",
        "C:/",
        "C:/local/../db",
        "C:/local/./db",
        "C:/local//db",
        "C:/local/db/",
        "C:/local/DB.",
        "C:/local/DB ",
        "C:/local/db:stream",
        "C:/NUL/db",
        "C:/con.txt/db",
        "C:/LPT1/db",
        "C:/COM¹/db",
        "C:/CONIN$/db",
        "C:/CONOUT$/db",
        "C:/bad*/db",
        "C:/bad\n/db",
    ] {
        assert_eq!(
            status(TargetPlatform::Windows, path),
            PlacementStatus::Rejected,
            "{path}"
        );
    }
}

#[test]
fn macos_known_provider_markers_are_rejected_conservatively() {
    for path in [
        "/Users/test/Library/Mobile Documents/com~apple~CloudDocs/db",
        "/Users/test/Library/CloudStorage/OneDrive-Personal/db",
        "/Users/test/Dropbox/db",
        "/Users/test/Dropbox (Team)/db",
        "/Users/test/OneDrive - Team/db",
        "/Users/test/iCloud Drive/db",
        "/Users/test/dropbox/%UNEXPANDED%/db",
    ] {
        assert_eq!(
            status(TargetPlatform::Macos, path),
            PlacementStatus::Rejected,
            "{path}"
        );
    }
    for path in [
        "/Users/test/DropboxBackup/db",
        "/Users/test/custom-sync-root/db",
        "/Volumes/redirected/db",
        "/Users/$USER/db",
        "/Users/test/%ROOT%/db",
    ] {
        assert_eq!(
            status(TargetPlatform::Macos, path),
            PlacementStatus::Unknown,
            "{path}"
        );
    }
}

#[test]
fn all_five_placements_are_independently_checked() {
    for index in 0..5 {
        let mut paths = std::array::from_fn(|n| format!("/local/role{n}"));
        paths[index] = "/Users/test/Dropbox/db".into();
        let report = inspect(TargetPlatform::Macos, paths);
        assert_eq!(report.status, PlacementStatus::Rejected);
        assert_eq!(report.placements[index].status, PlacementStatus::Rejected);
        assert_eq!(
            report
                .placements
                .iter()
                .filter(|p| p.status == PlacementStatus::Rejected)
                .count(),
            1
        );
    }
}

#[test]
fn cli_reports_json_without_creating_directories() {
    let temp = tempfile::tempdir().unwrap();
    let paths: Vec<_> = ["config", "dependencies", "database", "logs", "build"]
        .iter()
        .map(|name| temp.path().join(name))
        .collect();
    let mut command = Command::new(env!("CARGO_BIN_EXE_agent_llm_mm"));
    command.args([
        "codex-actions",
        "deployment-path-preview",
        "--target-platform",
        // TempDir supplies host-native paths, not synthetic Linux paths.
        if cfg!(windows) { "windows" } else { "linux" },
    ]);
    for (flag, path) in [
        "--config-dir",
        "--dependencies-dir",
        "--database-dir",
        "--logs-dir",
        "--build-dir",
    ]
    .iter()
    .zip(&paths)
    {
        command.arg(flag).arg(path);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["status"], "unknown");
    assert_eq!(report["ready_to_deploy"], false);
    assert_eq!(report["placements"].as_array().unwrap().len(), 5);
    assert!(paths.iter().all(|p| !p.exists()));
}

#[test]
fn target_platform_is_mandatory_and_validated() {
    assert!(TargetPlatform::parse("host").is_err());
    for args in [
        vec!["codex-actions", "deployment-path-preview"],
        vec![
            "codex-actions",
            "deployment-path-preview",
            "--target-platform",
            "unknown",
        ],
    ] {
        assert!(
            !Command::new(env!("CARGO_BIN_EXE_agent_llm_mm"))
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
}

#[test]
fn posix_ambiguity_and_input_bounds_are_conservative() {
    for platform in [TargetPlatform::Macos, TargetPlatform::Linux] {
        for path in [
            "",
            "/",
            "relative/db",
            "/local/../db",
            "/local/./db",
            "/local//db",
            "/local/db/",
            "/local/\0db",
        ] {
            assert_eq!(
                status(platform, path),
                PlacementStatus::Rejected,
                "{path:?}"
            );
        }
        assert_eq!(
            status(platform, &format!("/{}", "a".repeat(4096))),
            PlacementStatus::Rejected
        );
        assert_eq!(status(platform, "//local/db"), PlacementStatus::Unknown);
    }
}

#[test]
fn preview_is_deterministic_and_does_not_resolve_environment_tokens() {
    let paths = std::array::from_fn(|n| format!("/local/$HOME/role{n}"));
    let first = inspect(TargetPlatform::Macos, paths.clone());
    let second = inspect(TargetPlatform::Macos, paths);
    assert_eq!(
        serde_json::to_string(&first).unwrap(),
        serde_json::to_string(&second).unwrap()
    );
    assert!(
        first
            .placements
            .iter()
            .all(|p| p.normalized_path.is_none() && p.status == PlacementStatus::Unknown)
    );
}

#[test]
fn oversized_unicode_paths_are_omitted_and_report_output_is_bounded() {
    let path = format!("/{}", "界".repeat(100_000));
    let report = inspect(TargetPlatform::Macos, std::array::from_fn(|_| path.clone()));
    assert_eq!(report.status, PlacementStatus::Rejected);
    for placement in &report.placements {
        assert_eq!(placement.requested_path_original_bytes, 300_001);
        assert!(placement.requested_path.is_none());
        assert!(placement.normalized_path.is_none());
    }
    assert!(serde_json::to_vec(&report).unwrap().len() < 4096);
    let boundary = format!("/{}", "界".repeat(1365));
    assert_eq!(boundary.len(), 4096);
    let accepted = inspect(
        TargetPlatform::Macos,
        std::array::from_fn(|_| boundary.clone()),
    );
    assert_eq!(accepted.status, PlacementStatus::Unknown);
    assert_eq!(
        accepted.placements[0].requested_path.as_deref(),
        Some(boundary.as_str())
    );
    assert_eq!(
        accepted.placements[0].normalized_path.as_deref(),
        Some(boundary.as_str())
    );
    let over_boundary = format!("{boundary}x");
    let rejected = inspect(
        TargetPlatform::Macos,
        std::array::from_fn(|_| over_boundary.clone()),
    );
    assert!(rejected.placements[0].requested_path.is_none());
}
