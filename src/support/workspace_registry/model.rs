use std::{
    collections::HashSet,
    path::{Component, Path, PathBuf},
};

use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub(crate) const MAX_REGISTRY_BYTES: u64 = 1024 * 1024;
const MAX_WORKSPACES: usize = 1024;
const MAX_ROOTS: usize = 32;
const MAX_GRANTS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceRegistry {
    pub version: u32,
    pub workspaces: Vec<WorkspaceRegistration>,
}

pub type Registry = WorkspaceRegistry;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceRegistration {
    pub id: String,
    pub roots: Vec<PathBuf>,
    pub project_namespace: String,
    pub personal_grants: Vec<ScopeGrant>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeGrant {
    pub namespace: String,
    pub read: bool,
    pub write: bool,
}

/// Created by the trusted registry, never accepted as authority from MCP arguments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionGrant {
    pub workspace_id: String,
    pub project_namespace: String,
    pub readable_namespaces: Vec<String>,
    pub writable_namespaces: Vec<String>,
}

impl SessionGrant {
    pub fn allows_read(&self, namespace: &str) -> bool {
        self.readable_namespaces
            .iter()
            .any(|scope| scope == namespace)
    }

    pub fn allows_write(&self, namespace: &str) -> bool {
        self.writable_namespaces
            .iter()
            .any(|scope| scope == namespace)
    }
}

impl Default for WorkspaceRegistry {
    fn default() -> Self {
        Self {
            version: 1,
            workspaces: Vec::new(),
        }
    }
}

impl WorkspaceRegistry {
    /// Load and validate a private registry. Missing files are errors for readers.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        super::storage::load(path.as_ref())
    }

    /// Pure structural validation. Existing roots may later move or be offline;
    /// only an explicit root resolution accesses the filesystem.
    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == 1, "unsupported workspace registry version");
        ensure!(
            self.workspaces.len() <= MAX_WORKSPACES,
            "too many workspaces"
        );
        let mut ids = HashSet::new();
        let mut roots = HashSet::new();
        for workspace in &self.workspaces {
            validate_id(&workspace.id)?;
            ensure!(ids.insert(&workspace.id), "duplicate workspace id");
            ensure!(
                workspace.project_namespace == format!("project/{}", workspace.id),
                "workspace namespace must match its stable id"
            );
            ensure!(
                !workspace.roots.is_empty() && workspace.roots.len() <= MAX_ROOTS,
                "workspace requires 1 to {MAX_ROOTS} roots"
            );
            for root in &workspace.roots {
                validate_absolute_path(root)?;
                ensure!(roots.insert(root), "root is bound more than once");
            }
            ensure!(
                workspace.personal_grants.len() <= MAX_GRANTS,
                "too many personal grants"
            );
            let mut namespaces = HashSet::new();
            for grant in &workspace.personal_grants {
                validate_personal_namespace(&grant.namespace)?;
                ensure!(grant.read, "stored personal grants must permit reading");
                ensure!(
                    namespaces.insert(&grant.namespace),
                    "duplicate personal grant"
                );
            }
        }
        Ok(())
    }

    pub fn session_for_id(&self, id: &str) -> Result<SessionGrant> {
        self.validate()?;
        validate_id(id)?;
        let workspace = self
            .workspaces
            .iter()
            .find(|workspace| workspace.id == id)
            .ok_or_else(|| anyhow::anyhow!("workspace id is not registered"))?;
        Ok(workspace.session_grant())
    }

    pub(crate) fn register_canonical(
        &mut self,
        root: PathBuf,
        new_id: String,
    ) -> Result<WorkspaceRegistration> {
        self.validate()?;
        validate_absolute_path(&root)?;
        if let Some(workspace) = self
            .workspaces
            .iter()
            .find(|workspace| workspace.roots.contains(&root))
        {
            return Ok(workspace.clone());
        }
        ensure!(
            self.workspaces.len() < MAX_WORKSPACES,
            "too many workspaces"
        );
        validate_id(&new_id)?;
        ensure!(
            !self
                .workspaces
                .iter()
                .any(|workspace| workspace.id == new_id),
            "new workspace id is already registered"
        );
        let id = new_id;
        let workspace = WorkspaceRegistration {
            project_namespace: format!("project/{id}"),
            id,
            roots: vec![root],
            personal_grants: Vec::new(),
        };
        self.workspaces.push(workspace.clone());
        Ok(workspace)
    }

    pub(crate) fn bind_canonical(
        &mut self,
        id: &str,
        root: PathBuf,
    ) -> Result<WorkspaceRegistration> {
        self.validate()?;
        validate_absolute_path(&root)?;
        if let Some(existing) = self
            .workspaces
            .iter()
            .find(|workspace| workspace.roots.contains(&root))
        {
            ensure!(
                existing.id == id,
                "root is already bound to another workspace"
            );
            return Ok(existing.clone());
        }
        let workspace = self.find_mut(id)?;
        ensure!(
            workspace.roots.len() < MAX_ROOTS,
            "too many workspace roots"
        );
        workspace.roots.push(root);
        Ok(workspace.clone())
    }

    pub(crate) fn set_grant(
        &mut self,
        id: &str,
        namespace: &str,
        read: bool,
        write: bool,
    ) -> Result<WorkspaceRegistration> {
        self.validate()?;
        validate_personal_namespace(namespace)?;
        ensure!(
            !write || read,
            "personal write permission requires read permission"
        );
        let workspace = self.find_mut(id)?;
        let existing = workspace
            .personal_grants
            .iter()
            .position(|grant| grant.namespace == namespace);
        match (read, existing) {
            (false, Some(index)) => {
                workspace.personal_grants.remove(index);
            }
            (false, None) => {}
            (true, Some(index)) => {
                workspace.personal_grants[index] = ScopeGrant {
                    namespace: namespace.to_owned(),
                    read,
                    write,
                }
            }
            (true, None) => {
                ensure!(
                    workspace.personal_grants.len() < MAX_GRANTS,
                    "too many personal grants"
                );
                workspace.personal_grants.push(ScopeGrant {
                    namespace: namespace.to_owned(),
                    read,
                    write,
                });
            }
        }
        Ok(workspace.clone())
    }

    fn find_mut(&mut self, id: &str) -> Result<&mut WorkspaceRegistration> {
        validate_id(id)?;
        self.workspaces
            .iter_mut()
            .find(|workspace| workspace.id == id)
            .ok_or_else(|| anyhow::anyhow!("workspace id is not registered"))
    }
}

impl WorkspaceRegistration {
    fn session_grant(&self) -> SessionGrant {
        let mut readable_namespaces = vec![self.project_namespace.clone()];
        let mut writable_namespaces = vec![self.project_namespace.clone()];
        for grant in &self.personal_grants {
            if grant.read {
                readable_namespaces.push(grant.namespace.clone());
            }
            if grant.write {
                writable_namespaces.push(grant.namespace.clone());
            }
        }
        SessionGrant {
            workspace_id: self.id.clone(),
            project_namespace: self.project_namespace.clone(),
            readable_namespaces,
            writable_namespaces,
        }
    }
}

pub(crate) fn validate_id(id: &str) -> Result<()> {
    let uuid = Uuid::parse_str(id).map_err(|_| anyhow::anyhow!("invalid workspace id"))?;
    ensure!(
        !uuid.is_nil() && uuid.to_string() == id,
        "workspace id must be a canonical non-nil UUID"
    );
    Ok(())
}

pub(crate) fn validate_absolute_path(path: &Path) -> Result<()> {
    let text = path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("workspace paths must be UTF-8"))?;
    ensure!(
        !text.is_empty() && text.len() <= 4096 && !text.chars().any(char::is_control),
        "invalid workspace path length or characters"
    );
    ensure!(path.is_absolute(), "workspace paths must be absolute");
    ensure!(
        !path
            .components()
            .any(|part| matches!(part, Component::CurDir | Component::ParentDir)),
        "workspace paths must be normalized"
    );
    Ok(())
}

pub(crate) fn validate_personal_namespace(namespace: &str) -> Result<()> {
    let Some(id) = namespace.strip_prefix("user/") else {
        bail!("personal grants require a user namespace")
    };
    ensure!(
        !id.is_empty()
            && id.len() <= 128
            && id != "."
            && id != ".."
            && id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')),
        "invalid personal namespace identifier"
    );
    Ok(())
}
