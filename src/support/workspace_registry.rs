//! Local workspace identities and explicit scope grants.
//!
//! Registry writes are administrative operations, never model-supplied tool calls.
//! A session grant must be resolved again for each request to observe revocation.
//! This is a same-user local trust boundary, not protection from a malicious process
//! running as the same operating-system user.

mod model;
mod storage;

pub use model::{Registry, ScopeGrant, SessionGrant, WorkspaceRegistration, WorkspaceRegistry};
pub use storage::{
    bind_workspace_root, register_workspace, resolve_workspace, resolve_workspace_id,
    set_personal_grant, set_workspace_active, unbind_workspace_root,
};

mod adoption;
pub use adoption::{AdoptionReport, adopt_workspace, preview_adoption};
