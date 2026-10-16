#!/usr/bin/env python3
"""Native Windows named-pipe core acceptance using only an installed binary.

The shared protocol workflow writes exclusively synthetic data beneath a new
private output directory. No Rust PATH, provider credentials, user configuration,
service installation, user process discovery, or natural-language client claim.
"""
import argparse
import ctypes
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import time

# Importing the bundled helper must not create files beside the candidate.
sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location(
    "shared_local_core_verifier", Path(__file__).with_name("verify-local-core.py"))
shared = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(shared)
require, VerificationError = shared.require, shared.VerificationError


def drive_type(anchor):
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    get_drive_type = kernel.GetDriveTypeW
    get_drive_type.argtypes = [ctypes.c_wchar_p]
    get_drive_type.restype = ctypes.c_uint
    return get_drive_type(anchor)


def require_local_drive(path):
    # Check before resolve/stat/mkdir: a drive letter may be mapped to SMB.
    require(not str(path).startswith("\\\\"), "local_drive_required")
    require(drive_type(path.anchor) in (2, 3), "local_drive_required")


def validate_paths(binary, output):
    require(os.name == "nt", "native_windows_runtime_required")
    require(binary.is_absolute() and output.is_absolute(), "absolute_binary_and_output_required")
    require_local_drive(binary)
    require_local_drive(output)
    binary = binary.resolve(strict=True)
    require(binary.is_file(), "binary_must_be_executable_file")
    require(not output.exists() and not output.is_symlink(), "output_must_not_exist")
    output = output.resolve()
    require(not output.exists(), "output_must_not_exist")
    # All runtime state is local; neither UNC paths nor device namespaces are a
    # supported fixture destination for this same-user named-pipe verifier.
    require(not str(output).startswith("\\\\"), "local_output_directory_required")
    return binary, output


def prepare_private_output(output):
    # Reserve a new directory before setting its ACL. No existing directory,
    # user file, parent ACL, or broad inherited permission is ever modified.
    output.mkdir(parents=True, exist_ok=False)
    require(not any(output.iterdir()), "new_output_directory_must_be_empty")
    system_root = next((value for key, value in os.environ.items()
                        if key.upper() == "SYSTEMROOT"), None)
    require(system_root is not None, "windows_system_directory_required")
    powershell = Path(system_root) / "System32/WindowsPowerShell/v1.0/powershell.exe"
    require(powershell.is_file(), "windows_acl_setup_tool_unavailable")
    env = {key: value for key, value in os.environ.items()
           if key.upper() in ("SYSTEMROOT", "WINDIR", "SYSTEMDRIVE")}
    env.update({"PATH": "", "LEDGER_SYNTHETIC_PRIVATE_DIRECTORY": str(output)})
    script = """$ErrorActionPreference = 'Stop';
$PSModuleAutoLoadingPreference = 'None';
$sid = [System.Security.Principal.WindowsIdentity]::GetCurrent().User;
$acl = [System.Security.AccessControl.DirectorySecurity]::new();
$acl.SetOwner($sid);
$acl.SetAccessRuleProtection($true, $false);
$rule = [System.Security.AccessControl.FileSystemAccessRule]::new(
    $sid, 'FullControl', 'ContainerInherit, ObjectInherit', 'None', 'Allow');
$acl.AddAccessRule($rule);
[System.IO.Directory]::SetAccessControl($env:LEDGER_SYNTHETIC_PRIVATE_DIRECTORY, $acl);
"""
    result = subprocess.run([str(powershell), "-NoLogo", "-NoProfile", "-NonInteractive",
                             "-Command", script], env=env, capture_output=True,
                            check=False, timeout=30)
    require(result.returncode == 0, "private_synthetic_directory_acl_setup_failed")


def terminate_owned_core(owned, evidence):
    require(owned.role == "core", "only_owned_test_core_may_be_terminated")
    require(owned.process.poll() is None, "test_core_already_exited")
    # Popen.kill uses the handle of precisely the child we started. No name/PID
    # scan or systemwide kill, and no Unix signal claim on Windows.
    owned.process.kill()
    owned.process.wait(timeout=5)
    require(owned.process.returncode is not None, "test_core_termination_not_confirmed")
    evidence["crash"] = {"test_core_pid": owned.process.pid,
                         "mechanism": "TerminateProcess", "confirmed": True}


def record_startup_failure(output, owned, evidence):
    """Export fixed diagnostic codes only, never stderr, paths or config values."""
    labels = {
        "owner_mismatch": b"private state must be owned by the current Windows user",
        "parent_acl_rejected": b"local state parent must be owned",
        "acl_outside_trust_boundary": b"private state DACL grants access outside",
        "invalid_dacl": b"private state must have a valid non-NULL DACL",
        "unsupported_ace": b"unsupported private-state DACL entry",
        "acl_inspection_failed": b"inspect private state ACL",
        "reparse_point": b"private state must not be a reparse point",
        "hard_link_alias": b"private state must not have hard-link aliases",
        "nonlocal_drive": b"local-core state requires a fixed or removable local drive",
        "database_acl_rejected": b"ledger must be a current-user-owned private regular file",
        "lock_already_owned": b"another core or adoption owns this database or endpoint",
        "parent_missing": b"local state parent must exist",
        "missing_database": b"core requires an existing file-backed ledger",
        "pipe_creation_failed": b"create exclusive local-core named pipe",
        "endpoint_already_exists": b"Windows core endpoint path must be unused",
        "dashboard_enabled": b"core requires dashboard.enabled=false",
        "daemon_enabled": b"core requires daemon.enabled=false",
    }
    diagnostic = {"process": owned.label, "exit_code": owned.process.returncode}
    try:
        with (output / "logs" / (owned.label + ".stderr.log")).open("rb") as stream:
            data = stream.read(65537)
        diagnostic["stderr_truncated"] = len(data) > 65536
        data = data[:65536]
        diagnostic["categories"] = [code for code, literal in labels.items() if literal in data]
        diagnostic["os_error_codes"] = [code for code in (1, 2, 3, 5, 32, 87, 123, 183, 231, 233, 535)
                                        if ("os error " + str(code) + ")").encode() in data]
        if not diagnostic["categories"]:
            diagnostic["categories"] = ["unclassified_startup_failure"]
    except OSError:
        diagnostic["categories"] = ["stderr_unavailable"]
    evidence.setdefault("startup_failures", []).append(diagnostic)


class WindowsWorkflow(shared.Workflow):
    def create_working_directory(self, folder):
        # CPython's Windows mkdir(0700) installs OWNER RIGHTS (OW), rather
        # than inheriting the parent's explicit current-user SID ACL.
        # Set the existing synthetic fixture policy on every new empty child.
        prepare_private_output(folder)

    def core(self, label, config):
        owned = self.processes.spawn(label, "core", ["core", "--socket", self.address,
                                      "--registry", self.registry], config)
        # A named pipe has no filesystem socket node. Readiness is an actual
        # fixed-workspace bridge handshake plus initialize/tools/list, never an
        # endpoint-exists check and never an application write retry.
        registrations = json.loads(self.registry.read_text(encoding="utf-8"))["workspaces"]
        workspace = registrations[0]
        deadline, polls = time.monotonic() + 20, 0
        while time.monotonic() < deadline:
            if owned.process.poll() is not None:
                record_startup_failure(self.output, owned, self.evidence)
                raise VerificationError("core_exited_before_ready")
            polls += 1
            probe_label = label + "-probe-" + str(polls)
            probe = self.processes.spawn(probe_label, "bridge", ["bridge", "--socket", self.address,
                "--workspace-id", workspace["id"], "--workspace-root", workspace["roots"][0],
                "--client-label", "readiness-probe"], pipe=True)
            client = shared.McpClient(probe, self.output, self.evidence, timeout=.75)
            try:
                client.initialize()
                self.evidence["readiness"].append({"process": label, "polls": polls,
                                                   "method": "bridge_initialize_and_tools_list"})
                return owned
            except (VerificationError, OSError, ValueError):
                time.sleep(.05)
            finally:
                client.close()
        raise VerificationError("named_pipe_core_ready_timeout")

    def crash(self, owned):
        terminate_owned_core(owned, self.evidence)


def verify(binary, output):
    evidence = shared.verify(binary, output, workflow_class=WindowsWorkflow,
                             path_validator=validate_paths, output_initializer=prepare_private_output)
    evidence["transport"] = "windows_local_named_pipe"
    evidence["native_windows_runtime"] = True
    shared.write_json(output / "summary.json", evidence)
    return evidence


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        evidence = verify(args.binary, args.output)
    except (Exception, KeyboardInterrupt) as error:
        print(json.dumps({"status": "failed", "failure_code": str(error)
                          if isinstance(error, VerificationError) else type(error).__name__,
                          "kind": "isolated_runtime_verification", "new_machine_verified": False}))
        return 1
    print(json.dumps({key: evidence[key] for key in (
        "kind", "status", "transport", "new_machine_verified", "natural_language_client_verified",
        "model_requests", "implicit_write_retries", "explicit_replays_sent", "mcp_tool_calls", "cleanup")}))
    return 0 if evidence["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
