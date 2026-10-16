//! Read-only local registration inspection and client TOML rendering.
//! No transport connection, provider configuration, ledger access or file writes.
use super::workspace_registry::{ScopeGrant, WorkspaceRegistration, WorkspaceRegistry};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientMode {
    Fixed,
    Global,
}

#[derive(Debug, Serialize)]
pub struct RootStatus {
    pub path: PathBuf,
    pub valid: bool,
    pub reason: Option<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct WorkspaceStatus {
    pub id: String,
    pub project_namespace: String,
    pub active: bool,
    pub bound: bool,
    pub roots: Vec<RootStatus>,
    pub configured_personal_grants: Vec<ScopeGrant>,
    pub effective_personal_grants: Vec<ScopeGrant>,
    pub configuration_available: bool,
    pub refusal_reasons: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct StatusReport {
    pub registry: PathBuf,
    pub platform_supported: bool,
    pub core_runtime: &'static str,
    pub total: usize,
    pub available: usize,
    pub workspaces: Vec<WorkspaceStatus>,
}

pub fn inspect_workspace(workspace: &WorkspaceRegistration) -> WorkspaceStatus {
    let roots: Vec<_> = workspace
        .roots
        .iter()
        .map(|root| {
            let reason = match root.canonicalize() {
                Ok(canonical) if canonical != *root => Some("root canonical path changed"),
                Ok(_) if !root.is_dir() => Some("root is not a directory"),
                Ok(_) => None,
                Err(_) => Some("root missing or inaccessible"),
            };
            RootStatus {
                path: root.clone(),
                valid: reason.is_none(),
                reason,
            }
        })
        .collect();
    let mut refusal_reasons = Vec::new();
    if !cfg!(any(unix, windows)) {
        refusal_reasons
            .push("local core/bridge unsupported on this platform; legacy serve remains available");
    }
    if !workspace.active {
        refusal_reasons.push("workspace is disabled");
    }
    if roots.is_empty() {
        refusal_reasons.push("workspace has no bound roots");
    } else if !roots.iter().any(|root| root.valid) {
        refusal_reasons.push("workspace has no valid existing root");
    }
    let configuration_available = refusal_reasons.is_empty();
    WorkspaceStatus {
        id: workspace.id.clone(),
        project_namespace: workspace.project_namespace.clone(),
        active: workspace.active,
        bound: !roots.is_empty(),
        roots,
        configured_personal_grants: workspace.personal_grants.clone(),
        effective_personal_grants: if configuration_available {
            workspace.personal_grants.clone()
        } else {
            Vec::new()
        },
        configuration_available,
        refusal_reasons,
    }
}

/// No selector means all registrations; a root selector is exact after canonicalization.
pub fn inspect_registry(
    registry_path: &Path,
    id: Option<&str>,
    root: Option<&Path>,
) -> Result<StatusReport> {
    let registry = WorkspaceRegistry::load(registry_path)?;
    ensure!(
        id.is_none() || root.is_none(),
        "choose either --id or --root"
    );
    let selected = if id.is_some() || root.is_some() {
        vec![select_workspace(&registry, id, root)?.clone()]
    } else {
        registry.workspaces
    };
    let workspaces: Vec<_> = selected.iter().map(inspect_workspace).collect();
    Ok(StatusReport {
        registry: registry_path.to_owned(),
        platform_supported: cfg!(any(unix, windows)),
        core_runtime: "not checked (local configuration only; no connection or database access)",
        total: workspaces.len(),
        available: workspaces
            .iter()
            .filter(|w| w.configuration_available)
            .count(),
        workspaces,
    })
}

fn select_workspace<'a>(
    registry: &'a WorkspaceRegistry,
    id: Option<&str>,
    root: Option<&Path>,
) -> Result<&'a WorkspaceRegistration> {
    ensure!(
        id.is_none() || root.is_none(),
        "choose either --id or --root"
    );
    if let Some(id) = id {
        return registry
            .workspaces
            .iter()
            .find(|w| w.id == id)
            .context("workspace id is not registered");
    }
    let root = root.context("workspace selection requires --id or an exact root")?;
    let root = root
        .canonicalize()
        .context("selected root missing or inaccessible")?;
    ensure!(root.is_dir(), "selected root must be a directory");
    registry
        .workspaces
        .iter()
        .find(|w| w.roots.contains(&root))
        .context(
            "exact root is not registered; register or bind it explicitly (no ancestor fallback)",
        )
}

#[derive(Serialize)]
struct ClientConfig {
    mcp_servers: BTreeMap<String, ClientServer>,
}
#[derive(Serialize)]
struct ClientServer {
    command: String,
    args: Vec<String>,
}

/// Render a copyable config. Global mode must validate the actual launch cwd.
/// The socket need not exist: this is configuration readiness, not service health.
pub fn render_client_config(
    registry_path: &Path,
    socket: &Path,
    binary: &Path,
    mode: ClientMode,
    id: Option<&str>,
    root: Option<&Path>,
    cwd: &Path,
) -> Result<String> {
    ensure!(
        cfg!(any(unix, windows)),
        "local core/bridge requires Unix or Windows"
    );
    let registry = WorkspaceRegistry::load(registry_path)?;
    let selected_root = match mode {
        ClientMode::Global => {
            ensure!(
                id.is_none() && root.is_none(),
                "global mode uses launch cwd; --id/--root are not allowed"
            );
            Some(cwd)
        }
        ClientMode::Fixed => root.or_else(|| if id.is_none() { Some(cwd) } else { None }),
    };
    let workspace = select_workspace(&registry, id, selected_root)?;
    let status = inspect_workspace(workspace);
    ensure!(
        status.configuration_available,
        "cannot generate client config: {}",
        status.refusal_reasons.join("; ")
    );
    let path_text = |path: &Path| -> Result<String> {
        ensure!(
            path.is_absolute(),
            "client configuration paths must be absolute"
        );
        Ok(path
            .to_str()
            .context("client configuration paths must be UTF-8")?
            .to_owned())
    };
    ensure!(binary.is_file(), "client binary must exist as a file");
    let mut args = vec!["bridge".into(), "--socket".into(), path_text(socket)?];
    match mode {
        ClientMode::Global => args.extend(["--registry".into(), path_text(registry_path)?]),
        ClientMode::Fixed => {
            let fixed_root = if let Some(root) = selected_root {
                root.canonicalize()?
            } else {
                status
                    .roots
                    .iter()
                    .find(|root| root.valid)
                    .expect("checked availability")
                    .path
                    .clone()
            };
            args.extend([
                "--workspace-id".into(),
                workspace.id.clone(),
                "--workspace-root".into(),
                path_text(&fixed_root)?,
            ]);
        }
    }
    args.extend(["--client-label".into(), "codex".into()]);
    let config = ClientConfig {
        mcp_servers: BTreeMap::from([(
            "memory-ledger".into(),
            ClientServer {
                command: path_text(binary)?,
                args,
            },
        )]),
    };
    let warning = match mode {
        ClientMode::Global => {
            "# Global registry-cwd mode: each launch must use an exact registered, enabled root.\n# Clients that do not pass the project cwd must use fixed mode.\n"
        }
        ClientMode::Fixed => {
            "# Fixed workspace: reconnect/regenerate explicitly when switching workspace.\n"
        }
    };
    Ok(format!(
        "# Generated read-only; core runtime was not checked. No client file was changed.\n{warning}{}",
        toml::to_string_pretty(&config)?
    ))
}

pub const WORKSPACE_HELP: &str = "Workspace administration (same-user local MVP)
  workspace --registry <absolute-file> register --root <absolute-dir>
  workspace --registry <absolute-file> adopt-preview|adopt --root <absolute-dir> --namespace project/<legacy-name> --database <existing-file>
  workspace --registry <absolute-file> bind|unbind --id <UUID> --root <absolute-dir>
  workspace --registry <absolute-file> enable|disable --id <UUID>
  workspace --registry <absolute-file> grant --id <UUID> --namespace user/<name> --read true|false --write true|false
  workspace --registry <absolute-file> list
  workspace --registry <absolute-file> inspect [--id <UUID> | --root <absolute-dir>]
  workspace --registry <absolute-file> status [--id <UUID> | --root <absolute-dir>]
  workspace --registry <absolute-file> client-config --socket <absolute-socket> --mode fixed [--id <UUID> | --root <absolute-dir>]
  workspace --registry <absolute-file> client-config --socket <absolute-socket> --mode global

adopt-preview reports basic-facts-v1 support and blockers without writes; adopt rechecks under core/SQLite locks.
Advanced reflection/experience/candidate/trigger histories anywhere are conservatively unsupported.
Adoption preserves DB/receipts, creates a new UUID mapping in registry v2, and never switches clients.
inspect defaults to exact current directory; status defaults to all registrations.
client-config prints TOML only; it does not change client files or grants.
fixed defaults to exact current directory; --id selects its first valid bound root.
global validates current directory, and every client launch must use an exact available registered root.
No ancestor fallback, automatic registration, service start or runtime health check.
New core/bridge use Unix sockets on macOS/Linux and local-only named pipes on Windows (candidate).";
