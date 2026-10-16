//! Explicit local-owner dashboard setup. No credential issuance or persistence.
use super::{CoreHandle, cli::AdminOptions};
use crate::{
    interfaces::dashboard::{
        AdminGate, DashboardHandle, DashboardRuntimeInfo, start_admin_dashboard_service,
    },
    support::{config::DashboardConfig, workspace_registry::WorkspaceRegistry},
};
use anyhow::Result;
#[cfg(unix)]
use anyhow::ensure;
use std::path::Path;

pub(super) fn prepare(
    options: AdminOptions,
    registry: &Path,
) -> Result<(DashboardConfig, AdminGate)> {
    WorkspaceRegistry::load(registry)?;
    #[cfg(unix)]
    let capability = {
        use std::os::unix::fs::MetadataExt;
        read_capability(&options.token_file, std::fs::metadata(registry)?.uid())?
    };
    #[cfg(windows)]
    let capability =
        crate::support::windows_security::read_private_file(&options.token_file, 4096)?;
    let gate = AdminGate::new(capability.trim_end_matches(['\r', '\n']))?;
    Ok((
        DashboardConfig {
            enabled: true,
            host: options.bind.ip().to_string(),
            port: options.bind.port(),
            required: true,
            ..DashboardConfig::default()
        },
        gate,
    ))
}

#[cfg(unix)]
fn read_capability(path: &Path, uid: u32) -> Result<String> {
    use std::{
        fs::{self, File},
        io::Read,
        os::unix::fs::MetadataExt,
    };
    for ancestor in path.ancestors() {
        ensure!(
            !fs::symlink_metadata(ancestor)?.file_type().is_symlink(),
            "admin capability path must not contain symlinks"
        );
    }
    let before = fs::symlink_metadata(path)?;
    ensure!(
        before.is_file()
            && before.uid() == uid
            && before.nlink() == 1
            && before.mode() & 0o077 == 0,
        "admin capability file must be an owned private regular file"
    );
    let file = File::open(path)?;
    let after = file.metadata()?;
    ensure!(
        before.dev() == after.dev() && before.ino() == after.ino(),
        "admin capability file changed while opening"
    );
    let mut value = String::new();
    file.take(4097).read_to_string(&mut value)?;
    ensure!(value.len() <= 4096, "admin capability exceeds size limit");
    Ok(value)
}

pub(super) async fn start(
    config: DashboardConfig,
    gate: AdminGate,
    core: &CoreHandle,
) -> Result<DashboardHandle> {
    start_admin_dashboard_service(
        config,
        DashboardRuntimeInfo {
            service_name: "MCP Memory Ledger".into(),
            transport: "local-core".into(),
            provider: "not-used-by-workspace-tools".into(),
            dashboard_enabled: true,
            read_only: true,
        },
        core.observer(),
        Some(core.store()),
        gate,
    )
    .await
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    #[test]
    fn capability_file_is_private_bounded_and_never_follows_aliases() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let path = root.join("synthetic-capability");
        fs::write(&path, "synthetic-test-only-capability\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let uid = fs::metadata(&path).unwrap().uid();
        assert!(read_capability(&path, uid).is_ok());
        let alias = root.join("alias");
        symlink(&path, &alias).unwrap();
        assert!(read_capability(&alias, uid).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_capability(&path, uid).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        fs::write(&path, "x".repeat(4097)).unwrap();
        assert!(read_capability(&path, uid).is_err());
        fs::write(&path, "synthetic").unwrap();
        fs::hard_link(&path, root.join("hard-link")).unwrap();
        assert!(read_capability(&path, uid).is_err());
    }
}
