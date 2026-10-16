use std::path::PathBuf;

use agent_llm_mm::interfaces::local_core::cli::{LocalCommand, WorkspaceAction, parse_args};

const ID: &str = "26f02b7d-778d-4e4c-8ea6-53fc349fa650";

fn path(name: &str) -> String {
    let base = if cfg!(windows) {
        PathBuf::from(r"C:\synthetic-cli-no-io")
    } else {
        PathBuf::from("/synthetic-cli-no-io")
    };
    base.join(name).to_string_lossy().into_owned()
}

fn parse(args: &[&str]) -> anyhow::Result<Option<LocalCommand>> {
    parse_args(
        std::iter::once("agent_llm_mm")
            .chain(args.iter().copied())
            .map(str::to_owned),
    )
}

fn bridge_args(socket: &str, root: &str) -> Vec<String> {
    [
        "agent_llm_mm",
        "bridge",
        "--socket",
        socket,
        "--workspace-id",
        ID,
        "--workspace-root",
        root,
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

#[test]
fn leaves_legacy_and_unknown_commands_to_existing_parser() {
    assert_eq!(parse_args(Vec::<String>::new()).unwrap(), None);
    for args in [
        vec![],
        vec!["serve"],
        vec!["doctor", "--read-only"],
        vec!["init"],
        vec!["migrate"],
        vec!["future-command", "--flag"],
    ] {
        assert_eq!(parse(&args).unwrap(), None);
    }
}

#[test]
fn parses_core_without_resolving_nonexistent_paths() {
    let socket = path("socket");
    let registry = path("registry");
    assert_eq!(
        parse(&["core", "--registry", &registry, "--socket", &socket]).unwrap(),
        Some(LocalCommand::Core {
            admin: None,
            socket: socket.into(),
            registry: registry.into(),
        })
    );
}

#[test]
fn bridge_uses_explicit_binding_and_default_client_label() {
    let socket = path("socket");
    let root = path("project");
    assert_eq!(
        parse_args(bridge_args(&socket, &root)).unwrap(),
        Some(LocalCommand::Bridge {
            socket: socket.into(),
            workspace_id: ID.to_string(),
            workspace_root: root.into(),
            client_label: "codex".to_string(),
        })
    );
}

#[test]
fn accepts_client_label_ascii_boundaries() {
    for label in ["Codex.agent_1-2".to_string(), "x".repeat(64)] {
        let mut args = bridge_args(&path("socket"), &path("root"));
        args.extend(["--client-label".to_string(), label.clone()]);
        let Some(LocalCommand::Bridge { client_label, .. }) = parse_args(args).unwrap() else {
            panic!("expected bridge");
        };
        assert_eq!(client_label, label);
    }
}

#[test]
fn rejects_client_labels_outside_safe_ascii_contract() {
    for label in [
        "".to_string(),
        "x".repeat(65),
        "项目".to_string(),
        "client label".to_string(),
        "client/name".to_string(),
        "client\nname".to_string(),
    ] {
        let mut args = bridge_args(&path("socket"), &path("root"));
        args.extend(["--client-label".to_string(), label]);
        assert!(parse_args(args).is_err());
    }
}

#[test]
fn parses_all_workspace_actions_with_registry_before_or_after_action() {
    let registry = path("registry");
    let root = path("root");
    for (args, expected) in [
        (
            vec![
                "workspace",
                "--registry",
                &registry,
                "register",
                "--root",
                &root,
            ],
            WorkspaceAction::Register {
                root: root.clone().into(),
            },
        ),
        (
            vec![
                "workspace",
                "bind",
                "--root",
                &root,
                "--id",
                ID,
                "--registry",
                &registry,
            ],
            WorkspaceAction::Bind {
                id: ID.to_string(),
                root: root.clone().into(),
            },
        ),
        (
            vec![
                "workspace",
                "--registry",
                &registry,
                "grant",
                "--id",
                ID,
                "--namespace",
                "user/default",
                "--read",
                "true",
                "--write",
                "false",
            ],
            WorkspaceAction::Grant {
                id: ID.to_string(),
                namespace: "user/default".to_string(),
                read: true,
                write: false,
            },
        ),
        (
            vec!["workspace", "list", "--registry", &registry],
            WorkspaceAction::List,
        ),
    ] {
        assert_eq!(
            parse(&args).unwrap(),
            Some(LocalCommand::Workspace {
                registry: registry.clone().into(),
                action: expected
            })
        );
    }
}

#[test]
fn rejects_unknown_duplicate_missing_and_action_inappropriate_flags() {
    let socket = path("socket");
    let registry = path("registry");
    let root = path("root");
    for args in [
        vec!["core"],
        vec!["core", "--socket"],
        vec!["core", "--socket", &socket],
        vec!["core", "--socket", "--registry", &registry],
        vec![
            "core",
            "--socket",
            &socket,
            "--registry",
            &registry,
            "--socket",
            &socket,
        ],
        vec![
            "core",
            "--socket",
            &socket,
            "--registry",
            &registry,
            "--unknown",
            "x",
        ],
        vec![
            "core",
            "--socket",
            &socket,
            "--registry",
            &registry,
            "extra",
        ],
        vec!["bridge", "--socket", &socket, "--workspace-id", ID],
        vec![
            "bridge",
            "--socket",
            &socket,
            "--workspace-id",
            ID,
            "--workspace-root",
            &root,
            "--registry",
            &registry,
        ],
        vec!["workspace", "--registry", &registry],
        vec!["workspace", "--registry", &registry, "unknown"],
        vec!["workspace", "--registry", &registry, "list", "register"],
        vec![
            "workspace",
            "--registry",
            &registry,
            "list",
            "--root",
            &root,
        ],
        vec!["workspace", "--registry", &registry, "register"],
        vec![
            "workspace",
            "--registry",
            &registry,
            "register",
            "--root",
            &root,
            "--id",
            ID,
        ],
        vec![
            "workspace",
            "--registry",
            &registry,
            "bind",
            "--root",
            &root,
        ],
        vec![
            "workspace",
            "--registry",
            &registry,
            "list",
            "--registry",
            &registry,
        ],
        vec![
            "workspace",
            "--registry",
            &registry,
            "grant",
            "--id",
            ID,
            "--namespace",
            "user/default",
            "--read",
            "true",
        ],
    ] {
        assert!(parse(&args).is_err());
    }
}

#[test]
fn rejects_relative_paths_for_every_path_argument() {
    let socket = path("socket");
    let registry = path("registry");
    let root = path("root");
    for args in [
        vec!["core", "--socket", "socket", "--registry", &registry],
        vec!["core", "--socket", &socket, "--registry", "registry"],
        vec![
            "bridge",
            "--socket",
            "socket",
            "--workspace-id",
            ID,
            "--workspace-root",
            &root,
        ],
        vec![
            "bridge",
            "--socket",
            &socket,
            "--workspace-id",
            ID,
            "--workspace-root",
            "root",
        ],
        vec!["workspace", "--registry", "registry", "list"],
        vec![
            "workspace",
            "--registry",
            &registry,
            "register",
            "--root",
            "root",
        ],
        vec![
            "workspace",
            "--registry",
            &registry,
            "bind",
            "--id",
            ID,
            "--root",
            "root",
        ],
    ] {
        assert!(parse(&args).is_err());
    }
}

#[test]
fn rejects_nul_in_paths() {
    assert!(
        parse(&[
            "core",
            "--socket",
            &format!("{}\0", path("socket")),
            "--registry",
            &path("registry")
        ])
        .is_err()
    );
}

#[test]
fn rejects_noncanonical_ids_for_bridge_bind_and_grant() {
    let socket = path("socket");
    let root = path("root");
    let registry = path("registry");
    for id in [
        "not-a-uuid".to_string(),
        ID.to_uppercase(),
        ID.replace('-', ""),
        format!("urn:uuid:{ID}"),
        format!("{{{ID}}}"),
    ] {
        for args in [
            vec![
                "bridge",
                "--socket",
                &socket,
                "--workspace-id",
                &id,
                "--workspace-root",
                &root,
            ],
            vec![
                "workspace",
                "--registry",
                &registry,
                "bind",
                "--id",
                &id,
                "--root",
                &root,
            ],
            vec![
                "workspace",
                "--registry",
                &registry,
                "grant",
                "--id",
                &id,
                "--namespace",
                "user/default",
                "--read",
                "true",
                "--write",
                "false",
            ],
        ] {
            assert!(parse(&args).is_err());
        }
    }
}

#[test]
fn permission_booleans_must_be_explicit_and_exact() {
    let registry = path("registry");
    for (read, write) in [
        ("true", "false"),
        ("false", "true"),
        ("false", "false"),
        ("true", "true"),
    ] {
        assert!(
            parse(&[
                "workspace",
                "--registry",
                &registry,
                "grant",
                "--id",
                ID,
                "--namespace",
                "user/default",
                "--read",
                read,
                "--write",
                write
            ])
            .is_ok()
        );
    }
    for (read, write) in [
        ("1", "false"),
        ("true", "0"),
        ("yes", "false"),
        ("true", "False"),
    ] {
        assert!(
            parse(&[
                "workspace",
                "--registry",
                &registry,
                "grant",
                "--id",
                ID,
                "--namespace",
                "user/default",
                "--read",
                read,
                "--write",
                write
            ])
            .is_err()
        );
    }
}

#[test]
fn rejects_empty_and_control_character_namespaces() {
    for namespace in ["", "   ", "user/\nprivate", "user/\0private"] {
        assert!(
            parse(&[
                "workspace",
                "--registry",
                &path("registry"),
                "grant",
                "--id",
                ID,
                "--namespace",
                namespace,
                "--read",
                "true",
                "--write",
                "false"
            ])
            .is_err()
        );
    }
}

#[test]
fn diagnostics_never_echo_untrusted_flags_values_or_actions() {
    let canary = "SYNTHETIC_PRIVATE_INPUT_31";
    let socket = path("socket");
    let registry = path("registry");
    for args in [
        vec![
            "core",
            "--socket",
            &socket,
            "--registry",
            &registry,
            "--SYNTHETIC_PRIVATE_INPUT_31",
            canary,
        ],
        vec!["workspace", "--registry", &registry, canary],
        vec![
            "bridge",
            "--socket",
            &socket,
            "--workspace-id",
            canary,
            "--workspace-root",
            &socket,
        ],
        vec![
            "bridge",
            "--socket",
            &socket,
            "--workspace-id",
            ID,
            "--workspace-root",
            &socket,
            "--client-label",
            "SYNTHETIC_PRIVATE_INPUT_31/private",
        ],
    ] {
        let error = parse(&args).unwrap_err();
        assert!(!format!("{error:#}").contains(canary));
        assert!(!format!("{error:?}").contains(canary));
    }
}

#[test]
fn bridge_registry_mode_is_global_and_never_combines_with_explicit_binding() {
    let socket = path("socket");
    let registry = path("registry.json");
    assert_eq!(
        parse(&["bridge", "--socket", &socket, "--registry", &registry]).unwrap(),
        Some(LocalCommand::BridgeFromRegistry {
            socket: socket.clone().into(),
            registry: registry.clone().into(),
            client_label: "codex".into()
        })
    );
    assert!(
        parse(&[
            "bridge",
            "--socket",
            &socket,
            "--registry",
            &registry,
            "--workspace-id",
            ID
        ])
        .is_err()
    );
    assert!(
        parse(&[
            "bridge",
            "--socket",
            &socket,
            "--registry",
            &registry,
            "--workspace-root",
            &path("root")
        ])
        .is_err()
    );
}

#[test]
fn admin_cli_requires_explicit_loopback_and_capability_file_pair() {
    let socket = path("socket");
    let registry = path("registry");
    let token = path("token");
    let base = ["core", "--socket", &socket, "--registry", &registry];
    for extra in [
        vec!["--admin-bind", "127.0.0.1:9988"],
        vec!["--admin-token-file", &token],
        vec!["--admin-bind", "0.0.0.0:9988", "--admin-token-file", &token],
    ] {
        assert!(parse(&[base.to_vec(), extra].concat()).is_err());
    }
    assert!(matches!(
        parse(
            &[
                base.to_vec(),
                vec![
                    "--admin-bind",
                    "127.0.0.1:9988",
                    "--admin-token-file",
                    &token
                ]
            ]
            .concat()
        )
        .unwrap(),
        Some(LocalCommand::Core { admin: Some(_), .. })
    ));
}

#[test]
fn parses_workspace_lifecycle_and_rejects_unrelated_flags() {
    let registry = path("registry");
    let root = path("deleted-root");
    for (action, expected) in [
        ("disable", WorkspaceAction::Disable { id: ID.into() }),
        ("enable", WorkspaceAction::Enable { id: ID.into() }),
    ] {
        assert_eq!(
            parse(&["workspace", "--registry", &registry, action, "--id", ID]).unwrap(),
            Some(LocalCommand::Workspace {
                registry: registry.clone().into(),
                action: expected
            })
        );
        assert!(
            parse(&[
                "workspace",
                "--registry",
                &registry,
                action,
                "--id",
                ID,
                "--root",
                &root
            ])
            .is_err()
        );
    }
    assert_eq!(
        parse(&[
            "workspace",
            "--registry",
            &registry,
            "unbind",
            "--id",
            ID,
            "--root",
            &root
        ])
        .unwrap(),
        Some(LocalCommand::Workspace {
            registry: registry.into(),
            action: WorkspaceAction::Unbind {
                id: ID.into(),
                root: root.into()
            }
        })
    );
}
