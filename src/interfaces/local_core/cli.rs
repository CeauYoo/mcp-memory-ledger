//! Pure argument parsing for the local core and its explicitly bound stdio bridge.
//!
//! This layer never loads configuration, reads environment variables, resolves
//! paths, or grants access. Registry and connection validation happen downstream.
use std::{collections::BTreeMap, path::PathBuf};

use anyhow::{Result, bail, ensure};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalCommand {
    Core {
        socket: PathBuf,
        registry: PathBuf,
        admin: Option<AdminOptions>,
    },
    Bridge {
        socket: PathBuf,
        workspace_id: String,
        workspace_root: PathBuf,
        client_label: String,
    },
    BridgeFromRegistry {
        socket: PathBuf,
        registry: PathBuf,
        client_label: String,
    },
    Workspace {
        registry: PathBuf,
        action: WorkspaceAction,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminOptions {
    pub bind: std::net::SocketAddr,
    pub token_file: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceAction {
    Register {
        root: PathBuf,
    },
    Bind {
        id: String,
        root: PathBuf,
    },
    Grant {
        id: String,
        namespace: String,
        read: bool,
        write: bool,
    },
    List,
}

const CORE_FLAGS: &[&str] = &[
    "--socket",
    "--registry",
    "--admin-bind",
    "--admin-token-file",
];
const BRIDGE_FLAGS: &[&str] = &[
    "--registry",
    "--socket",
    "--workspace-id",
    "--workspace-root",
    "--client-label",
];
const WORKSPACE_FLAGS: &[&str] = &[
    "--registry",
    "--root",
    "--id",
    "--namespace",
    "--read",
    "--write",
];

type Flags = BTreeMap<&'static str, String>;

/// Parses arguments including the program name, leaving legacy commands alone.
///
/// Recognized commands fail closed on invalid arguments. Error messages never
/// interpolate supplied values or unknown flag names, which may be sensitive.
pub fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Option<LocalCommand>> {
    let mut args = args.into_iter();
    let _program = args.next();
    let command = match args.next().as_deref() {
        Some("core") => {
            let (mut flags, _) = collect_flags(args, CORE_FLAGS, false)?;
            let admin = match (
                flags.remove("--admin-bind"),
                flags.remove("--admin-token-file"),
            ) {
                (None, None) => None,
                (Some(bind), Some(token_file)) => {
                    let bind: std::net::SocketAddr = bind.parse().map_err(|_| {
                        anyhow::anyhow!("admin bind requires a loopback IP address and port")
                    })?;
                    ensure!(bind.ip().is_loopback(), "admin bind must be loopback");
                    Some(AdminOptions {
                        bind,
                        token_file: absolute_path(token_file)?,
                    })
                }
                _ => bail!("admin bind and token file must be supplied together"),
            };
            LocalCommand::Core {
                admin,
                socket: absolute_path(take_required(&mut flags, "--socket")?)?,
                registry: absolute_path(take_required(&mut flags, "--registry")?)?,
            }
        }
        Some("bridge") => {
            let (mut flags, _) = collect_flags(args, BRIDGE_FLAGS, false)?;
            let client_label = flags
                .remove("--client-label")
                .unwrap_or_else(|| "codex".to_string());
            validate_client_label(&client_label)?;
            let socket = absolute_path(take_required(&mut flags, "--socket")?)?;
            if let Some(registry) = flags.remove("--registry") {
                require_only(&flags, &[])?;
                LocalCommand::BridgeFromRegistry {
                    socket,
                    registry: absolute_path(registry)?,
                    client_label,
                }
            } else {
                LocalCommand::Bridge {
                    socket,
                    workspace_id: canonical_uuid(take_required(&mut flags, "--workspace-id")?)?,
                    workspace_root: absolute_path(take_required(&mut flags, "--workspace-root")?)?,
                    client_label,
                }
            }
        }
        Some("workspace") => parse_workspace(args)?,
        _ => return Ok(None),
    };
    Ok(Some(command))
}

fn parse_workspace(args: impl Iterator<Item = String>) -> Result<LocalCommand> {
    let (mut flags, action) = collect_flags(args, WORKSPACE_FLAGS, true)?;
    let registry = absolute_path(take_required(&mut flags, "--registry")?)?;
    let action = match action {
        Some("register") => {
            require_only(&flags, &["--root"])?;
            WorkspaceAction::Register {
                root: absolute_path(take_required(&mut flags, "--root")?)?,
            }
        }
        Some("bind") => {
            require_only(&flags, &["--id", "--root"])?;
            WorkspaceAction::Bind {
                id: canonical_uuid(take_required(&mut flags, "--id")?)?,
                root: absolute_path(take_required(&mut flags, "--root")?)?,
            }
        }
        Some("grant") => {
            require_only(&flags, &["--id", "--namespace", "--read", "--write"])?;
            let namespace = take_required(&mut flags, "--namespace")?;
            ensure!(
                !namespace.trim().is_empty() && !namespace.chars().any(char::is_control),
                "namespace must be nonempty and contain no control characters"
            );
            WorkspaceAction::Grant {
                id: canonical_uuid(take_required(&mut flags, "--id")?)?,
                namespace,
                read: boolean(take_required(&mut flags, "--read")?)?,
                write: boolean(take_required(&mut flags, "--write")?)?,
            }
        }
        Some("list") => {
            require_only(&flags, &[])?;
            WorkspaceAction::List
        }
        _ => bail!("workspace requires one supported action"),
    };
    Ok(LocalCommand::Workspace { registry, action })
}

fn collect_flags(
    args: impl Iterator<Item = String>,
    allowed: &'static [&'static str],
    workspace_action: bool,
) -> Result<(Flags, Option<&'static str>)> {
    let mut args = args;
    let mut flags = Flags::new();
    let mut action = None;
    while let Some(argument) = args.next() {
        if argument.starts_with('-') {
            let Some(flag) = allowed.iter().copied().find(|flag| *flag == argument) else {
                bail!("unknown local command flag");
            };
            ensure!(!flags.contains_key(flag), "duplicate local command flag");
            let value = args
                .next()
                .ok_or_else(|| anyhow::anyhow!("local command flag requires a value"))?;
            ensure!(
                !value.is_empty() && !value.starts_with("--"),
                "local command flag requires a value"
            );
            flags.insert(flag, value);
        } else {
            ensure!(workspace_action, "unexpected local command argument");
            ensure!(action.is_none(), "workspace requires exactly one action");
            action = Some(match argument.as_str() {
                "register" => "register",
                "bind" => "bind",
                "grant" => "grant",
                "list" => "list",
                _ => bail!("unsupported workspace action"),
            });
        }
    }
    Ok((flags, action))
}

fn take_required(flags: &mut Flags, key: &'static str) -> Result<String> {
    flags
        .remove(key)
        .ok_or_else(|| anyhow::anyhow!("missing required local command flag"))
}

fn require_only(flags: &Flags, allowed: &[&str]) -> Result<()> {
    ensure!(
        flags.keys().all(|key| allowed.contains(key)),
        "flag is not supported for this workspace action"
    );
    Ok(())
}

fn absolute_path(value: String) -> Result<PathBuf> {
    ensure!(!value.contains('\0'), "path must not contain NUL");
    let path = PathBuf::from(value);
    ensure!(path.is_absolute(), "local command paths must be absolute");
    Ok(path)
}

fn canonical_uuid(value: String) -> Result<String> {
    let id = Uuid::parse_str(&value)
        .map_err(|_| anyhow::anyhow!("workspace id must be a canonical UUID"))?;
    ensure!(
        id.hyphenated().to_string() == value,
        "workspace id must be a canonical lowercase hyphenated UUID"
    );
    Ok(value)
}

fn validate_client_label(label: &str) -> Result<()> {
    ensure!(
        !label.is_empty()
            && label.len() <= 64
            && label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')),
        "client label must contain 1 to 64 ASCII letters, digits, dots, underscores or hyphens"
    );
    Ok(())
}

fn boolean(value: String) -> Result<bool> {
    match value.as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => bail!("permission flags require true or false"),
    }
}
