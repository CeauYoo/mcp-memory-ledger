//! Read-only, target-platform lexical placement review. No filesystem or sync
//! provider inspection: an ordinary-looking path is UNKNOWN, never verified safe.
use anyhow::{Result, bail};
use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TargetPlatform {
    Macos,
    Windows,
    Linux,
}
impl TargetPlatform {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "macos" => Ok(Self::Macos),
            "windows" => Ok(Self::Windows),
            "linux" => Ok(Self::Linux),
            _ => bail!("target platform must be macos, windows, or linux"),
        }
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlacementStatus {
    Rejected,
    Unknown,
}

#[derive(Debug, Serialize)]
pub struct Placement {
    pub role: &'static str,
    /// Omitted rather than truncated when over the input byte limit.
    pub requested_path: Option<String>,
    pub requested_path_original_bytes: usize,
    pub normalized_path: Option<String>,
    pub status: PlacementStatus,
    pub reason: &'static str,
}

#[derive(Debug, Serialize)]
pub struct DeploymentPreview {
    pub schema_version: u32,
    pub target_platform: TargetPlatform,
    pub status: PlacementStatus,
    pub mode: &'static str,
    pub installation_performed: bool,
    pub filesystem_verified: bool,
    pub sync_location_verified: bool,
    pub ready_to_deploy: bool,
    pub placements: Vec<Placement>,
    pub required_target_checks: [&'static str; 4],
}

/// All inputs describe directories, not files. This function does no I/O and
/// never interprets paths using the machine on which the preview is running.
pub fn inspect(platform: TargetPlatform, paths: [String; 5]) -> DeploymentPreview {
    let placements: Vec<_> = ["config", "dependencies", "database", "logs", "build"]
        .into_iter()
        .zip(paths)
        .map(|(role, path)| inspect_path(platform, role, path))
        .collect();
    DeploymentPreview {
        schema_version: 1,
        target_platform: platform,
        status: if placements
            .iter()
            .any(|p| p.status == PlacementStatus::Rejected)
        {
            PlacementStatus::Rejected
        } else {
            PlacementStatus::Unknown
        },
        mode: "lexical_preview_only_not_installed",
        installation_performed: false,
        filesystem_verified: false,
        sync_location_verified: false,
        ready_to_deploy: false,
        placements,
        required_target_checks: [
            "Resolve target symlinks, junctions, reparse points and mount aliases without writing.",
            "Inspect actual sync-provider roots and redirected folders on the target; names alone are insufficient.",
            "Verify directory ownership, permissions, placement collisions and available space on the target.",
            "Obtain deployment authorization and a separately reviewed backup/rollback plan before any writes.",
        ],
    }
}

fn inspect_path(platform: TargetPlatform, role: &'static str, path: String) -> Placement {
    let (normalized_path, status, reason) = classify(platform, &path);
    Placement {
        role,
        requested_path_original_bytes: path.len(),
        requested_path: (path.len() <= 4096).then_some(path),
        normalized_path,
        status,
        reason,
    }
}

fn classify(
    platform: TargetPlatform,
    path: &str,
) -> (Option<String>, PlacementStatus, &'static str) {
    use PlacementStatus::{Rejected, Unknown};
    if path.is_empty() || path.len() > 4096 || path.chars().any(char::is_control) {
        return (None, Rejected, "empty_overlong_or_control_character_path");
    }
    let windows = platform == TargetPlatform::Windows;
    let text = if windows {
        path.replace('\\', "/")
    } else {
        path.to_owned()
    };
    // UNC, device namespaces and POSIX double-slash semantics require target
    // inspection. Do not collapse them into an ordinary local path.
    if text.starts_with("//") {
        return (
            None,
            Unknown,
            "network_device_or_double_slash_root_unverified",
        );
    }
    let body = if windows {
        let bytes = text.as_bytes();
        if bytes.len() < 4 || !bytes[0].is_ascii_alphabetic() || bytes[1..3] != *b":/" {
            return (None, Rejected, "absolute_drive_path_required");
        }
        &text[3..]
    } else {
        if !text.starts_with('/') {
            return (None, Rejected, "absolute_posix_path_required");
        }
        &text[1..]
    };
    let components: Vec<_> = body.split('/').collect();
    if components
        .iter()
        .any(|c| c.is_empty() || *c == "." || *c == "..")
    {
        return (None, Rejected, "root_empty_or_traversal_component");
    }
    if components.iter().any(|c| {
        let lower = c.to_ascii_lowercase();
        lower == "dropbox"
            || lower.starts_with("dropbox (")
            || lower == "onedrive"
            || lower.starts_with("onedrive -")
            || lower == "icloud drive"
            || lower == "iclouddrive"
            || lower == "cloudstorage"
            || lower == "mobile documents"
    }) {
        return (None, Rejected, "known_sync_directory_marker");
    }
    if windows && components.iter().any(|c| windows_ambiguous_component(c)) {
        return (None, Rejected, "windows_reserved_or_ambiguous_component");
    }
    if components.iter().any(|c| c.contains(['$', '%', '~'])) {
        // The one literal iCloud identifier below must still be classified as
        // a known sync marker, not mistaken for shell expansion.
        if components
            .iter()
            .any(|c| c.eq_ignore_ascii_case("com~apple~CloudDocs"))
        {
            return (None, Rejected, "known_sync_directory_marker");
        }
        return (
            None,
            Unknown,
            "expansion_or_ambiguous_literal_requires_target_review",
        );
    }
    // Keep case on POSIX (including macOS, whose volume case behavior is not
    // established). Canonicalize only Windows drive spelling and separators.
    let normalized = if windows {
        format!("{}:{}", text[..1].to_ascii_uppercase(), &text[2..])
    } else {
        text
    };
    (
        Some(normalized),
        Unknown,
        "no_known_marker_filesystem_and_sync_ownership_unverified",
    )
}

fn windows_ambiguous_component(component: &str) -> bool {
    if component.ends_with([' ', '.']) || component.contains([':', '<', '>', '"', '|', '?', '*']) {
        return true;
    }
    let stem = component
        .split('.')
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|prefix| {
        stem.strip_prefix(prefix).is_some_and(|suffix| {
            matches!(
                suffix,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    })
}
