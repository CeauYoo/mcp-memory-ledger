//! Conservative, bounded parsing of the supported apply_patch grammar.
//!
//! Path checks describe a point-in-time observation only. They do not hold file
//! handles through execution and do NOT provide TOCTOU / race-free confinement.
//! Different-device descendants and hardlinked file targets are rejected. Same-device
//! bind mounts and network filesystem identity remain unverified.
//! Windows is unsupported until reparse points and path identity can be checked.
use anyhow::{Context, Result, bail, ensure};
use std::fs;
use std::path::{Component, Path, PathBuf};

pub const MAX_PATCH_BYTES: usize = 1024 * 1024;
pub const MAX_PATCH_FILES: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchOperation {
    Add,
    Update,
    Delete,
    Move,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchTarget {
    pub operation: PatchOperation,
    pub path: PathBuf,
}

fn target(operation: PatchOperation, value: &str) -> Result<PatchTarget> {
    ensure!(
        !value.is_empty() && value.trim() == value,
        "empty or padded patch path"
    );
    ensure!(
        !value.chars().any(char::is_control),
        "control character in patch path"
    );
    Ok(PatchTarget {
        operation,
        path: PathBuf::from(value),
    })
}

/// Supports exact Begin/End, Add, Delete, Update, optional Move, @@ hunks,
/// and a terminal End of File marker. Shell wrappers and other extensions fail.
/// Both paths of a move are emitted, source first. No input is executed.
pub fn parse_targets(patch: &str) -> Result<Vec<PatchTarget>> {
    ensure!(patch.len() <= MAX_PATCH_BYTES, "patch exceeds byte limit");
    ensure!(
        !patch.contains('\0') && !patch.contains('\r'),
        "unsupported patch control bytes"
    );
    let lines: Vec<_> = patch.lines().collect();
    ensure!(
        lines.first() == Some(&"*** Begin Patch"),
        "missing Begin Patch"
    );
    ensure!(
        lines.last() == Some(&"*** End Patch"),
        "missing End Patch or trailing data"
    );
    let mut i = 1;
    let mut files = 0;
    let mut targets = Vec::new();
    while i + 1 < lines.len() {
        files += 1;
        ensure!(files <= MAX_PATCH_FILES, "patch exceeds file limit");
        let header = lines[i];
        i += 1;
        if let Some(path) = header.strip_prefix("*** Add File: ") {
            targets.push(target(PatchOperation::Add, path)?);
            // Empty additions are supported. Every content line must be prefixed.
            while i + 1 < lines.len() && lines[i].starts_with('+') {
                i += 1;
            }
        } else if let Some(path) = header.strip_prefix("*** Delete File: ") {
            targets.push(target(PatchOperation::Delete, path)?);
        } else if let Some(path) = header.strip_prefix("*** Update File: ") {
            let mut source = target(PatchOperation::Update, path)?;
            if i + 1 < lines.len() {
                if let Some(destination) = lines[i].strip_prefix("*** Move to: ") {
                    source.operation = PatchOperation::Move;
                    targets.push(source);
                    targets.push(target(PatchOperation::Move, destination)?);
                    i += 1;
                } else {
                    targets.push(source);
                }
            } else {
                targets.push(source);
            }
            let mut hunks = 0;
            while i + 1 < lines.len() && (lines[i] == "@@" || lines[i].starts_with("@@ ")) {
                hunks += 1;
                i += 1;
                let start = i;
                while i + 1 < lines.len()
                    && matches!(lines[i].as_bytes().first(), Some(b' ' | b'+' | b'-'))
                {
                    i += 1;
                }
                ensure!(i > start, "empty update hunk");
                if i + 1 < lines.len() && lines[i] == "*** End of File" {
                    i += 1;
                    break;
                }
            }
            ensure!(hunks > 0, "update requires a prefixed, nonempty @@ hunk");
        } else {
            bail!("malformed or unsupported patch header/body");
        }
    }
    ensure!(
        i == lines.len() - 1 && files > 0,
        "empty or malformed patch"
    );
    Ok(targets)
}

fn validate_path(path: &Path) -> Result<()> {
    let raw = path.to_str().context("non-UTF-8 path unsupported")?;
    ensure!(!raw.is_empty(), "empty path");
    ensure!(
        !raw.starts_with("//") && !raw.contains('\\') && !raw.contains(':'),
        "UNC, device, or Windows path unsupported"
    );
    ensure!(
        !raw.chars().any(char::is_control),
        "control character in path"
    );
    for component in path.components() {
        ensure!(
            !matches!(component, Component::ParentDir | Component::Prefix(_)),
            "traversal or platform prefix unsupported"
        );
    }
    Ok(())
}

/// Resolve through existing ancestors, rejecting every symlink component and
/// non-directory ancestor. Missing descendants are appended only after checking
/// the nearest existing parent. Does not create directories or files.
fn checked_absolute(path: &Path) -> Result<PathBuf> {
    validate_path(path)?;
    ensure!(path.is_absolute(), "absolute path required");
    let mut result = PathBuf::new();
    let mut missing = false;
    for component in path.components() {
        if component == Component::CurDir {
            continue;
        }
        if !result.as_os_str().is_empty() && !missing {
            ensure!(
                fs::symlink_metadata(&result)?.is_dir(),
                "non-directory path ancestor"
            );
        }
        result.push(component.as_os_str());
        if missing {
            continue;
        }
        match fs::symlink_metadata(&result) {
            Ok(meta) => {
                ensure!(
                    !meta.file_type().is_symlink(),
                    "symlink path component unsupported"
                );
                ensure!(
                    meta.is_file() || meta.is_dir(),
                    "special file target unsupported"
                );
                result = fs::canonicalize(&result).context("cannot canonicalize path component")?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => missing = true,
            Err(error) => return Err(error).context("cannot inspect path component"),
        }
    }
    Ok(result)
}

/// Check existing components at and below the workspace boundary. Ancestors
/// above the workspace may legitimately be on another device.
#[cfg(unix)]
fn check_workspace_device(path: &Path, root: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let expected_device = fs::symlink_metadata(root)?.dev();
    for ancestor in path.ancestors().take_while(|p| p.starts_with(root)) {
        match fs::symlink_metadata(ancestor) {
            Ok(meta) => {
                ensure!(
                    !meta.file_type().is_symlink(),
                    "symlink appeared during path check"
                );
                ensure!(
                    meta.dev() == expected_device,
                    "cross-device workspace path unsupported"
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("cannot inspect workspace device"),
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_workspace_device(_path: &Path, _root: &Path) -> Result<()> {
    bail!("workspace device identity unsupported on this platform")
}

/// Check every target, including both move paths, under the canonical workspace
/// and outside every forbidden root. This is not an execution authorization.
pub fn resolve_targets(
    targets: &[PatchTarget],
    cwd: &Path,
    root: &Path,
    forbidden_roots: &[PathBuf],
) -> Result<Vec<PathBuf>> {
    ensure!(
        cfg!(unix),
        "checked writes unsupported: platform path/reparse identity unavailable"
    );
    ensure!(
        !targets.is_empty() && targets.len() <= MAX_PATCH_FILES * 2,
        "invalid target count"
    );
    let root = checked_absolute(root)?;
    ensure!(
        fs::metadata(&root)?.is_dir(),
        "workspace root is not an existing directory"
    );
    let cwd = checked_absolute(cwd)?;
    ensure!(
        fs::metadata(&cwd)?.is_dir() && cwd.starts_with(&root),
        "cwd outside workspace or not a directory"
    );
    check_workspace_device(&cwd, &root)?;
    let forbidden: Vec<_> = forbidden_roots
        .iter()
        .map(|p| checked_absolute(p))
        .collect::<Result<_>>()?;
    let mut resolved = Vec::with_capacity(targets.len());
    let mut move_destination_pending = false;
    for (index, target) in targets.iter().enumerate() {
        let move_source = target.operation == PatchOperation::Move && !move_destination_pending;
        if move_destination_pending {
            ensure!(
                target.operation == PatchOperation::Move,
                "unpaired move source"
            );
            move_destination_pending = false;
        } else if move_source {
            ensure!(
                targets
                    .get(index + 1)
                    .is_some_and(|t| t.operation == PatchOperation::Move),
                "unpaired move source"
            );
            move_destination_pending = true;
        }
        validate_path(&target.path)?;
        let path = if target.path.is_absolute() {
            target.path.clone()
        } else {
            cwd.join(&target.path)
        };
        let path = checked_absolute(&path)?;
        ensure!(
            path != root && path.starts_with(&root),
            "patch target outside workspace"
        );
        ensure!(
            !forbidden.iter().any(|p| path.starts_with(p)),
            "patch target in forbidden root"
        );
        check_workspace_device(&path, &root)?;
        match fs::symlink_metadata(&path) {
            Ok(meta) => {
                ensure!(
                    target.operation != PatchOperation::Add,
                    "add target already exists; overwrite requires update operation"
                );
                ensure!(
                    target.operation != PatchOperation::Move || move_source,
                    "move destination already exists; overwrite unsupported"
                );
                ensure!(
                    meta.is_file() && !meta.file_type().is_symlink(),
                    "patch target is not a regular file"
                );
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    ensure!(meta.nlink() == 1, "hardlinked patch target unsupported");
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                ensure!(
                    !move_source
                        && !matches!(
                            target.operation,
                            PatchOperation::Update | PatchOperation::Delete
                        ),
                    "update/delete/move source does not exist"
                );
            }
            Err(error) => return Err(error).context("cannot inspect final target"),
        }
        resolved.push(path);
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_operations_and_move_paths() {
        let targets = parse_targets("*** Begin Patch\n*** Add File: a\n+x\n*** Update File: b\n@@\n-old\n+new\n*** Delete File: c\n*** Update File: d\n*** Move to: e\n@@ context\n x\n*** End of File\n*** End Patch\n").unwrap();
        assert_eq!(targets.len(), 5);
        assert_eq!(targets[0].operation, PatchOperation::Add);
        assert_eq!(targets[1].operation, PatchOperation::Update);
        assert_eq!(targets[2].operation, PatchOperation::Delete);
        assert_eq!(targets[3].operation, PatchOperation::Move);
        assert_eq!(targets[3].path, PathBuf::from("d"));
        assert_eq!(targets[4].path, PathBuf::from("e"));
    }

    #[test]
    fn rejects_malformed_and_unsupported_bodies() {
        for body in [
            "",
            "*** Update File: a",
            "*** Update File: a\n@@",
            "*** Update File: a\n@@\nunprefixed",
            "*** Update File: a\n*** Delete File: b",
            "*** Add File: a\nnot prefixed",
            "*** Delete File: a\n+unexpected",
            "*** Move to: a",
            "*** Update File: a\n@@\n+x\n*** Fake Header: b",
            "*** Update File: a\n@@\n+x\n*** End of File\n@@\n+y",
        ] {
            assert!(
                parse_targets(&format!("*** Begin Patch\n{body}\n*** End Patch")).is_err(),
                "{body}"
            );
        }
        assert!(
            parse_targets("*** Begin Patch\n*** Add File: a\n*** End Patch\ntrailing").is_err()
        );
        assert!(parse_targets(&"x".repeat(MAX_PATCH_BYTES + 1)).is_err());
        assert!(
            parse_targets(&format!(
                "*** Begin Patch\n{}*** End Patch",
                "*** Add File: a\n".repeat(MAX_PATCH_FILES + 1)
            ))
            .is_err()
        );
    }

    #[test]
    fn content_cannot_smuggle_headers() {
        let targets = parse_targets("*** Begin Patch\n*** Update File: a\n@@\n+*** Delete File: outside\n *** Add File: another\n*** End Patch").unwrap();
        assert_eq!(targets.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn confines_new_paths_and_checks_every_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let parse = |name: &str| {
            parse_targets(&format!(
                "*** Begin Patch\n*** Add File: {name}\n+x\n*** End Patch"
            ))
            .unwrap()
        };
        assert_eq!(
            resolve_targets(&parse("missing/child/new"), &root, &root, &[]).unwrap(),
            vec![root.join("missing/child/new")]
        );
        for name in [
            "../outside",
            "/outside",
            "//server/share",
            "C:\\device",
            "a/../b",
        ] {
            assert!(
                resolve_targets(&parse(name), &root, &root, &[]).is_err(),
                "{name}"
            );
        }
        assert!(
            resolve_targets(&parse("private/a"), &root, &root, &[root.join("private")]).is_err()
        );
        let mixed = parse_targets(
            "*** Begin Patch\n*** Add File: safe\n+x\n*** Add File: ../bad\n+y\n*** End Patch",
        )
        .unwrap();
        assert!(resolve_targets(&mixed, &root, &root, &[]).is_err());
        fs::write(root.join("source"), "old").unwrap();
        let moved = parse_targets("*** Begin Patch\n*** Update File: source\n*** Move to: ../outside\n@@\n-old\n+new\n*** End Patch").unwrap();
        assert!(resolve_targets(&moved, &root, &root, &[]).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_ancestors_and_targets() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        fs::create_dir(root.join("real")).unwrap();
        symlink(root.join("real"), root.join("link")).unwrap();
        for path in ["link", "link/new"] {
            let targets = vec![PatchTarget {
                operation: PatchOperation::Add,
                path: path.into(),
            }];
            assert!(resolve_targets(&targets, &root, &root, &[]).is_err());
        }
    }
    #[cfg(unix)]
    #[test]
    fn rejects_hardlinks_missing_move_source_and_unpaired_moves() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        fs::write(root.join("source"), "old").unwrap();
        fs::hard_link(root.join("source"), root.join("alias")).unwrap();
        let linked = vec![PatchTarget {
            operation: PatchOperation::Update,
            path: "source".into(),
        }];
        assert!(resolve_targets(&linked, &root, &root, &[]).is_err());
        let missing = parse_targets("*** Begin Patch\n*** Update File: missing\n*** Move to: new\n@@\n-old\n+new\n*** End Patch").unwrap();
        assert!(resolve_targets(&missing, &root, &root, &[]).is_err());
        let unpaired = vec![PatchTarget {
            operation: PatchOperation::Move,
            path: "source".into(),
        }];
        assert!(resolve_targets(&unpaired, &root, &root, &[]).is_err());
        fs::remove_file(root.join("alias")).unwrap();
        let valid = parse_targets("*** Begin Patch\n*** Add File: added\n+x\n*** Update File: source\n*** Move to: new\n@@\n-old\n+new\n*** End Patch").unwrap();
        assert!(resolve_targets(&valid, &root, &root, &[]).is_ok());
    }
    #[cfg(unix)]
    #[test]
    fn rejects_add_and_move_destination_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        fs::write(root.join("source"), "old").unwrap();
        fs::write(root.join("occupied"), "keep").unwrap();
        let add =
            parse_targets("*** Begin Patch\n*** Add File: occupied\n+replacement\n*** End Patch")
                .unwrap();
        assert!(resolve_targets(&add, &root, &root, &[]).is_err());
        let moved = parse_targets("*** Begin Patch\n*** Update File: source\n*** Move to: occupied\n@@\n-old\n+new\n*** End Patch").unwrap();
        assert!(resolve_targets(&moved, &root, &root, &[]).is_err());
        let update = parse_targets(
            "*** Begin Patch\n*** Update File: occupied\n@@\n-keep\n+new\n*** End Patch",
        )
        .unwrap();
        assert!(resolve_targets(&update, &root, &root, &[]).is_ok());
        assert_eq!(fs::read_to_string(root.join("occupied")).unwrap(), "keep");
    }
}
