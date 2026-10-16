#[cfg(windows)]
#[path = "support/windows_private.rs"]
mod windows_private;

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Barrier},
    thread,
};

use agent_llm_mm::support::workspace_registry::{
    WorkspaceRegistry, bind_workspace_root, register_workspace, resolve_workspace,
    resolve_workspace_id, set_personal_grant,
};
use tempfile::TempDir;

fn fixture() -> (TempDir, PathBuf, PathBuf) {
    let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/workspace-core/registry-tests");
    fs::create_dir_all(&parent).unwrap();
    let temp = tempfile::tempdir_in(parent).unwrap();
    #[cfg(windows)]
    windows_private::private_directory(temp.path());
    let root = temp.path().join("project");
    fs::create_dir(&root).unwrap();
    let registry = temp.path().join("registry.json");
    (temp, registry, root)
}

fn private_write(path: &Path, bytes: impl AsRef<[u8]>) {
    fs::write(path, bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
}

#[test]
fn registration_reuses_canonical_root_and_does_not_infer_related_projects() {
    let (temp, path, root) = fixture();
    let first = register_workspace(&path, &root).unwrap();
    let alias = root.join(".");
    assert_eq!(register_workspace(&path, alias).unwrap(), first);
    let sibling = temp.path().join("another clone");
    fs::create_dir(&sibling).unwrap();
    let second = register_workspace(&path, &sibling).unwrap();
    assert_ne!(first.id, second.id);
    assert_eq!(first.project_namespace, format!("project/{}", first.id));
    let grant = resolve_workspace(&path, &root).unwrap();
    assert!(grant.allows_read(&first.project_namespace));
    assert!(grant.allows_write(&first.project_namespace));
    assert!(!grant.allows_read("user/default"));
    assert!(!grant.allows_read(&second.project_namespace));
    assert_eq!(grant.readable_namespaces.len(), 1);
}

#[test]
fn explicit_binding_survives_a_move_without_reassigning_identity() {
    let (temp, path, root) = fixture();
    let original = register_workspace(&path, &root).unwrap();
    let moved = temp.path().join("moved");
    fs::rename(&root, &moved).unwrap();
    assert!(resolve_workspace(&path, &root).is_err());
    assert!(resolve_workspace(&path, &moved).is_err());
    let bound = bind_workspace_root(&path, &original.id, &moved).unwrap();
    assert_eq!(bound.id, original.id);
    assert_eq!(bound.roots.len(), 2);
    assert_eq!(
        resolve_workspace(&path, &moved).unwrap().workspace_id,
        original.id
    );
    let other = temp.path().join("other");
    fs::create_dir(&other).unwrap();
    let second = register_workspace(&path, &other).unwrap();
    let bytes = fs::read(&path).unwrap();
    assert!(bind_workspace_root(&path, &second.id, &moved).is_err());
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[test]
fn no_ancestor_scope_or_missing_registry_fallback() {
    let (_temp, path, root) = fixture();
    assert!(WorkspaceRegistry::load(&path).is_err());
    assert!(resolve_workspace(&path, &root).is_err());
    assert!(
        set_personal_grant(
            &path,
            uuid::Uuid::new_v4().to_string(),
            "user/default",
            true,
            false
        )
        .is_err()
    );
    register_workspace(&path, &root).unwrap();
    let child = root.join("child");
    fs::create_dir(&child).unwrap();
    assert!(resolve_workspace(&path, &child).is_err());
    assert!(register_workspace(&path, root.join("absent")).is_err());
    assert!(register_workspace(&path, Path::new(".")).is_err());
    let file = root.join("a-file");
    fs::write(&file, "synthetic").unwrap();
    assert!(register_workspace(&path, file).is_err());
}

#[test]
fn personal_scopes_require_explicit_read_write_and_observe_revocation() {
    let (_temp, path, root) = fixture();
    let workspace = register_workspace(&path, &root).unwrap();
    assert!(set_personal_grant(&path, &workspace.id, "user/default", false, true).is_err());
    for bad in [
        "self",
        "world",
        "project/foreign",
        "user/",
        "user/*",
        "user/../x",
        "user/a/b",
        "user/..",
        "user/a b",
    ] {
        assert!(
            set_personal_grant(&path, &workspace.id, bad, true, false).is_err(),
            "accepted {bad}"
        );
    }
    set_personal_grant(&path, &workspace.id, "user/default", true, false).unwrap();
    let read = resolve_workspace_id(&path, &workspace.id).unwrap();
    assert!(read.allows_read("user/default"));
    assert!(!read.allows_write("user/default"));
    set_personal_grant(&path, &workspace.id, "user/default", true, true).unwrap();
    assert!(
        resolve_workspace_id(&path, &workspace.id)
            .unwrap()
            .allows_write("user/default")
    );
    set_personal_grant(&path, &workspace.id, "user/default", false, false).unwrap();
    assert!(
        !resolve_workspace_id(&path, &workspace.id)
            .unwrap()
            .allows_read("user/default")
    );
    assert!(
        WorkspaceRegistry::load(&path).unwrap().workspaces[0]
            .personal_grants
            .is_empty()
    );
}

#[test]
fn damaged_or_unsupported_registry_is_never_overwritten() {
    let (_temp, path, root) = fixture();
    for data in [
        "{broken",
        r#"{"version":99,"workspaces":[]}"#,
        r#"{"version":1,"workspaces":[],"unexpected":true}"#,
    ] {
        private_write(&path, data);
        assert!(register_workspace(&path, &root).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), data);
        assert!(!path.with_file_name("registry.json.lock").exists());
    }
    private_write(&path, vec![b' '; 1024 * 1024 + 1]);
    assert!(WorkspaceRegistry::load(&path).is_err());
}

#[test]
fn structural_validation_rejects_forged_scopes_and_duplicate_identity() {
    let (_temp, path, root) = fixture();
    register_workspace(&path, &root).unwrap();
    let registry = WorkspaceRegistry::load(&path).unwrap();
    let mut invalid = registry.clone();
    invalid.workspaces[0].project_namespace = "user/default".into();
    assert!(invalid.validate().is_err());
    invalid = registry.clone();
    invalid.workspaces.push(invalid.workspaces[0].clone());
    assert!(invalid.validate().is_err());
    invalid = registry.clone();
    invalid.workspaces[0]
        .roots
        .push(root.canonicalize().unwrap());
    assert!(invalid.validate().is_err());
    invalid = registry.clone();
    invalid.workspaces[0].id = uuid::Uuid::nil().to_string();
    assert!(invalid.validate().is_err());
    invalid = registry;
    invalid.workspaces[0].personal_grants.push(
        agent_llm_mm::support::workspace_registry::ScopeGrant {
            namespace: "user/default".into(),
            read: false,
            write: true,
        },
    );
    assert!(invalid.validate().is_err());
}

#[test]
fn concurrent_updates_retain_every_successful_registration_and_grant() {
    let (temp, path, root) = fixture();
    let workspace = register_workspace(&path, &root).unwrap();
    let barrier = Arc::new(Barrier::new(8));
    let handles: Vec<_> = (0..8)
        .map(|index| {
            let path = path.clone();
            let id = workspace.id.clone();
            let barrier = barrier.clone();
            let new_root = temp.path().join(format!("root-{index}"));
            fs::create_dir(&new_root).unwrap();
            thread::spawn(move || {
                barrier.wait();
                register_workspace(&path, new_root).unwrap();
                set_personal_grant(&path, id, format!("user/synthetic-{index}"), true, false)
                    .unwrap();
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
    let registry = WorkspaceRegistry::load(&path).unwrap();
    assert_eq!(registry.workspaces.len(), 9);
    assert_eq!(
        resolve_workspace_id(&path, workspace.id)
            .unwrap()
            .readable_namespaces
            .len(),
        9
    );
}

#[test]
fn unknown_lock_is_reported_and_preserved() {
    let (_temp, path, root) = fixture();
    let lock = path.with_file_name("registry.json.lock");
    private_write(&lock, "synthetic unknown owner");
    let error = register_workspace(&path, &root).unwrap_err().to_string();
    assert!(error.contains("busy or stale"));
    assert_eq!(fs::read_to_string(lock).unwrap(), "synthetic unknown owner");
    assert!(!path.exists());
}

#[cfg(unix)]
#[test]
fn unix_registry_is_private_and_rejects_symlinks_hardlinks_and_unsafe_parent() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let (temp, path, root) = fixture();
    let workspace = register_workspace(&path, &root).unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let root_alias = temp.path().join("root-alias");
    symlink(&root, &root_alias).unwrap();
    assert_eq!(
        register_workspace(&path, root_alias).unwrap().id,
        workspace.id
    );
    let alias = temp.path().join("registry-alias.json");
    symlink(&path, &alias).unwrap();
    assert!(WorkspaceRegistry::load(&alias).is_err());
    assert!(register_workspace(&alias, &root).is_err());
    let hardlink = temp.path().join("hardlink.json");
    fs::hard_link(&path, &hardlink).unwrap();
    assert!(WorkspaceRegistry::load(&path).is_err());
    fs::remove_file(hardlink).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(WorkspaceRegistry::load(&path).is_err());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let parent_alias = temp.path().join("parent-alias");
    symlink(temp.path(), &parent_alias).unwrap();
    assert!(WorkspaceRegistry::load(parent_alias.join("registry.json")).is_err());
    fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o777)).unwrap();
    assert!(WorkspaceRegistry::load(&path).is_err());
    fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700)).unwrap();
}

#[cfg(unix)]
#[test]
fn non_utf8_roots_fail_closed() {
    use std::os::unix::ffi::OsStringExt;
    let (temp, path, _root) = fixture();
    let root = temp.path().join(std::ffi::OsString::from_vec(vec![0xff]));
    // macOS itself rejects non-UTF-8 names; the API must reject before filesystem lookup.
    assert!(register_workspace(&path, &root).is_err());
    assert!(!path.exists());
}

#[test]
fn lifecycle_preserves_identity_grants_and_requires_explicit_enable() {
    use agent_llm_mm::support::workspace_registry::{set_workspace_active, unbind_workspace_root};
    let (temp, path, root) = fixture();
    let original = register_workspace(&path, &root).unwrap();
    let granted = set_personal_grant(&path, &original.id, "user/default", true, true).unwrap();
    let disabled = set_workspace_active(&path, &original.id, false).unwrap();
    assert!(!disabled.active);
    assert_eq!(disabled.personal_grants, granted.personal_grants);
    assert!(resolve_workspace(&path, &root).is_err());
    assert!(resolve_workspace_id(&path, &original.id).is_err());
    let bytes = fs::read(&path).unwrap();
    assert_eq!(
        set_workspace_active(&path, &original.id, false).unwrap(),
        disabled
    );
    assert_eq!(register_workspace(&path, &root).unwrap(), disabled);
    assert_eq!(fs::read(&path).unwrap(), bytes);
    let moved = temp.path().join("moved");
    fs::rename(&root, &moved).unwrap();
    assert!(set_workspace_active(&path, &original.id, true).is_err());
    let unbound = unbind_workspace_root(&path, &original.id, &root).unwrap();
    assert!(unbound.roots.is_empty());
    assert!(!unbound.active);
    assert_eq!(
        unbind_workspace_root(&path, &original.id, &root).unwrap(),
        unbound
    );
    assert!(set_workspace_active(&path, &original.id, true).is_err());
    let rebound = bind_workspace_root(&path, &original.id, &moved).unwrap();
    assert!(!rebound.active);
    assert!(resolve_workspace(&path, &moved).is_err());
    let enabled = set_workspace_active(&path, &original.id, true).unwrap();
    assert!(enabled.active);
    assert_eq!(enabled.id, original.id);
    assert_eq!(enabled.personal_grants, granted.personal_grants);
    assert!(
        resolve_workspace(&path, &moved)
            .unwrap()
            .allows_write("user/default")
    );
}

#[test]
fn unbind_is_exact_and_zero_roots_never_authorizes_a_session() {
    use agent_llm_mm::support::workspace_registry::{set_workspace_active, unbind_workspace_root};
    let (temp, path, root) = fixture();
    let workspace = register_workspace(&path, &root).unwrap();
    let similar = temp.path().join("project-other");
    fs::create_dir(&similar).unwrap();
    bind_workspace_root(&path, &workspace.id, &similar).unwrap();
    unbind_workspace_root(&path, &workspace.id, root.join("child")).unwrap();
    assert_eq!(
        WorkspaceRegistry::load(&path).unwrap().workspaces[0]
            .roots
            .len(),
        2
    );
    unbind_workspace_root(&path, &workspace.id, &root).unwrap();
    assert!(resolve_workspace(&path, &root).is_err());
    assert!(resolve_workspace(&path, &similar).is_ok());
    let empty = unbind_workspace_root(&path, &workspace.id, &similar).unwrap();
    assert!(
        empty.active,
        "unbind must preserve explicit activation choice"
    );
    assert!(empty.roots.is_empty());
    assert!(resolve_workspace_id(&path, &workspace.id).is_err());
    assert!(set_workspace_active(&path, &workspace.id, true).is_err());
    let unknown = uuid::Uuid::new_v4().to_string();
    assert!(
        unbind_workspace_root(&path, &unknown, &root)
            .unwrap_err()
            .to_string()
            .contains("not registered")
    );
    assert!(set_workspace_active(&path, &unknown, false).is_err());
    assert!(unbind_workspace_root(&path, &workspace.id, "relative").is_err());
    let foreign = register_workspace(&path, &root).unwrap();
    assert_ne!(foreign.id, workspace.id);
    assert!(unbind_workspace_root(&path, &workspace.id, &root).is_err());
}

#[test]
fn legacy_registry_without_active_remains_readable_and_enabled() {
    let (_temp, path, root) = fixture();
    let original = register_workspace(&path, &root).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["workspaces"][0]
        .as_object_mut()
        .unwrap()
        .remove("active");
    private_write(&path, serde_json::to_vec(&value).unwrap());
    assert!(WorkspaceRegistry::load(&path).unwrap().workspaces[0].active);
    assert_eq!(
        resolve_workspace(&path, &root).unwrap().workspace_id,
        original.id
    );
}

#[test]
fn concurrent_lifecycle_updates_do_not_lose_independent_changes() {
    use agent_llm_mm::support::workspace_registry::{set_workspace_active, unbind_workspace_root};
    let (temp, path, root) = fixture();
    let workspace = register_workspace(&path, &root).unwrap();
    let barrier = Arc::new(Barrier::new(8));
    let mut handles = Vec::new();
    for index in 0..8 {
        let path = path.clone();
        let id = workspace.id.clone();
        let barrier = barrier.clone();
        let root = if index == 0 {
            root.clone()
        } else {
            temp.path().join(format!("alias-{index}"))
        };
        if index > 0 {
            fs::create_dir(&root).unwrap();
        }
        handles.push(thread::spawn(move || {
            barrier.wait();
            match index {
                0 => {
                    unbind_workspace_root(&path, &id, root).unwrap();
                }
                1 => {
                    set_workspace_active(&path, &id, false).unwrap();
                }
                _ => {
                    bind_workspace_root(&path, &id, root).unwrap();
                }
            }
            set_personal_grant(&path, &id, format!("user/concurrent-{index}"), true, false)
                .unwrap();
        }));
    }
    for handle in handles {
        handle.join().unwrap();
    }
    let current = &WorkspaceRegistry::load(&path).unwrap().workspaces[0];
    assert!(!current.active);
    assert_eq!(current.roots.len(), 6);
    assert!(!current.roots.contains(&root));
    assert_eq!(current.personal_grants.len(), 8);
}

#[cfg(windows)]
#[test]
fn windows_unbind_accepts_original_absolute_spelling_after_directory_is_removed() {
    use agent_llm_mm::support::workspace_registry::unbind_workspace_root;
    let (_temp, path, root) = fixture();
    let workspace = register_workspace(&path, &root).unwrap();
    assert!(workspace.roots[0].to_str().unwrap().starts_with(r"\\?\"));
    let ordinary = root.to_str().unwrap().replace('\\', "/");
    fs::remove_dir(&root).unwrap();
    let removed = unbind_workspace_root(&path, &workspace.id, &ordinary).unwrap();
    assert!(removed.roots.is_empty());
    assert_eq!(
        unbind_workspace_root(&path, &workspace.id, &ordinary).unwrap(),
        removed
    );
    assert!(resolve_workspace_id(&path, &workspace.id).is_err());
}
