//! Opt-in same-user local core. Legacy `serve` remains a separate compatibility
//! mode; a bridge never loads provider configuration or opens a ledger.
#[cfg(any(unix, windows))]
mod admin_start;
pub mod cli;
#[cfg(any(unix, windows))]
mod framing;
#[cfg(unix)]
#[path = "unix.rs"]
mod platform;
#[cfg(windows)]
#[path = "windows.rs"]
mod platform;
#[cfg(any(unix, windows))]
mod runtime;
#[cfg(any(unix, windows))]
mod session;
#[cfg(any(unix, windows))]
pub use runtime::{CoreHandle, start_core};

use crate::support::workspace_registry::{
    WorkspaceRegistry, bind_workspace_root, register_workspace, set_personal_grant,
    set_workspace_active, unbind_workspace_root,
};
use anyhow::Result;
use cli::{LocalCommand, WorkspaceAction};

pub async fn run_command(command: LocalCommand) -> Result<()> {
    match command {
        LocalCommand::WorkspaceHelp => {
            println!(
                "{}",
                crate::support::workspace_client_config::WORKSPACE_HELP
            );
            Ok(())
        }
        LocalCommand::Workspace { registry, action } => {
            let result = match action {
                WorkspaceAction::Adopt {
                    root,
                    namespace,
                    database,
                    preview,
                } => {
                    let report = if preview {
                        crate::support::workspace_registry::preview_adoption(
                            &registry, &root, &namespace, &database,
                        )
                        .await?
                    } else {
                        crate::support::workspace_registry::adopt_workspace(
                            &registry, &root, &namespace, &database,
                        )
                        .await?
                    };
                    serde_json::to_value(report)?
                }
                WorkspaceAction::Inspect { id, root } => {
                    let root = if id.is_none() && root.is_none() {
                        Some(std::env::current_dir()?)
                    } else {
                        root
                    };
                    serde_json::to_value(
                        crate::support::workspace_client_config::inspect_registry(
                            &registry,
                            id.as_deref(),
                            root.as_deref(),
                        )?,
                    )?
                }
                WorkspaceAction::Status { id, root } => serde_json::to_value(
                    crate::support::workspace_client_config::inspect_registry(
                        &registry,
                        id.as_deref(),
                        root.as_deref(),
                    )?,
                )?,
                WorkspaceAction::ClientConfig {
                    mode,
                    socket,
                    id,
                    root,
                } => {
                    let output = crate::support::workspace_client_config::render_client_config(
                        &registry,
                        &socket,
                        &std::env::current_exe()?,
                        mode,
                        id.as_deref(),
                        root.as_deref(),
                        &std::env::current_dir()?,
                    )?;
                    print!("{output}");
                    return Ok(());
                }
                WorkspaceAction::Register { root } => {
                    serde_json::to_value(register_workspace(registry, root)?)?
                }
                WorkspaceAction::Bind { id, root } => {
                    serde_json::to_value(bind_workspace_root(registry, id, root)?)?
                }
                WorkspaceAction::Unbind { id, root } => {
                    serde_json::to_value(unbind_workspace_root(registry, id, root)?)?
                }
                WorkspaceAction::Disable { id } => {
                    serde_json::to_value(set_workspace_active(registry, id, false)?)?
                }
                WorkspaceAction::Enable { id } => {
                    serde_json::to_value(set_workspace_active(registry, id, true)?)?
                }
                WorkspaceAction::Grant {
                    id,
                    namespace,
                    read,
                    write,
                } => {
                    serde_json::to_value(set_personal_grant(registry, id, namespace, read, write)?)?
                }
                WorkspaceAction::List => serde_json::to_value(WorkspaceRegistry::load(registry)?)?,
            };
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(())
        }
        command => run_transport(command).await,
    }
}

#[cfg(not(any(unix, windows)))]
async fn run_transport(_: LocalCommand) -> Result<()> {
    anyhow::bail!(
        "local core/bridge requires macOS, Linux, or Windows; use legacy serve on this platform"
    )
}

#[cfg(any(unix, windows))]
async fn run_transport(command: LocalCommand) -> Result<()> {
    match command {
        LocalCommand::Core {
            socket,
            registry,
            admin,
        } => {
            let config = crate::support::config::AppConfig::load().map_err(anyhow::Error::msg)?;
            let admin = admin
                .map(|options| admin_start::prepare(options, &registry))
                .transpose()?;
            let core = start_core(config, socket, registry).await?;
            let dashboard = match admin {
                Some((config, gate)) => match admin_start::start(config, gate, &core).await {
                    Ok(handle) => {
                        eprintln!("local core admin dashboard: {}", handle.base_url());
                        Some(handle)
                    }
                    Err(error) => {
                        core.stop().await;
                        return Err(error);
                    }
                },
                None => None,
            };
            tracing::info!("local core ready; workspace bridge connections accepted");
            tokio::signal::ctrl_c().await?;
            drop(dashboard);
            core.stop().await;
            Ok(())
        }
        LocalCommand::Bridge {
            socket,
            workspace_id,
            workspace_root,
            client_label,
        } => runtime::run_bridge(socket, workspace_id, workspace_root, client_label).await,
        LocalCommand::BridgeFromRegistry {
            socket,
            registry,
            client_label,
        } => {
            let root = std::env::current_dir()?;
            let grant = crate::support::workspace_registry::resolve_workspace(&registry, &root)
                .map_err(|_| anyhow::anyhow!("launch directory is not an available registered workspace; explicitly run workspace --registry <registry> register --root <project-root>, then reconnect from that exact directory"))?;
            runtime::run_bridge(socket, grant.workspace_id, root, client_label).await
        }
        LocalCommand::Workspace { .. } | LocalCommand::WorkspaceHelp => {
            unreachable!("handled before transport")
        }
    }
}
