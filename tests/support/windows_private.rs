//! Windows-only security setup for a brand-new, empty synthetic test directory.
//! Never use this helper on a user directory or an already-populated fixture.
#![cfg(windows)]

use std::{fs, path::Path, process::Command};

pub fn private_directory(path: &Path) {
    let metadata = fs::symlink_metadata(path).unwrap();
    assert!(metadata.is_dir() && !metadata.file_type().is_symlink());
    assert!(fs::read_dir(path).unwrap().next().is_none());
    let system_root = std::env::var_os("SystemRoot").expect("Windows system directory");
    let powershell = Path::new(&system_root).join("System32/WindowsPowerShell/v1.0/powershell.exe");
    // Use Framework ACL APIs directly; inherited pwsh modules cannot be loaded
    // into the fixed Windows PowerShell 5.1 executable.
    let output = Command::new(powershell)
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"])
        .arg(
            r#"$ErrorActionPreference = 'Stop';
$PSModuleAutoLoadingPreference = 'None';
$sid = [System.Security.Principal.WindowsIdentity]::GetCurrent().User;
$acl = [System.Security.AccessControl.DirectorySecurity]::new();
$acl.SetOwner($sid);
$acl.SetAccessRuleProtection($true, $false);
$rule = [System.Security.AccessControl.FileSystemAccessRule]::new(
    $sid, 'FullControl', 'ContainerInherit, ObjectInherit', 'None', 'Allow');
$acl.AddAccessRule($rule);
[System.IO.Directory]::SetAccessControl($env:LEDGER_SYNTHETIC_PRIVATE_DIRECTORY, $acl);"#,
        )
        .env_remove("PSModulePath")
        .env("LEDGER_SYNTHETIC_PRIVATE_DIRECTORY", path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "private synthetic Windows directory setup failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
