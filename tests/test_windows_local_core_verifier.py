"""Offline Windows verifier contracts. Mocked checks are never native evidence."""
from contextlib import redirect_stdout
import importlib.util
import io
import json
import os
from pathlib import Path, PureWindowsPath
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

SPEC = importlib.util.spec_from_file_location("windows_core_verifier",
    Path(__file__).resolve().parents[1] / "scripts/verify-windows-local-core.py")
verifier = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(verifier)


class WindowsVerifierTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def test_refuses_non_windows_host_instead_of_claiming_runtime_success(self):
        with patch.object(verifier.os, "name", "posix"):
            with self.assertRaisesRegex(verifier.VerificationError, "native_windows_runtime_required"):
                verifier.validate_paths(self.root / "inert", self.root / "new")

    def test_mapped_output_drive_is_rejected_before_resolve_or_creation(self):
        binary = PureWindowsPath(r"C:\synthetic\binary.exe")
        output = PureWindowsPath(r"Z:\synthetic\new")
        with patch.object(verifier.os, "name", "nt"), \
             patch.object(verifier, "drive_type", side_effect=[3, 4]) as check:
            with self.assertRaisesRegex(verifier.VerificationError, "local_drive_required"):
                verifier.validate_paths(binary, output)
        self.assertEqual(check.call_count, 2)
        for kind in (0, 1, 4, 5, 6):
            with patch.object(verifier, "drive_type", return_value=kind):
                with self.assertRaises(verifier.VerificationError):
                    verifier.require_local_drive(output)
        with patch.object(verifier, "drive_type") as check:
            with self.assertRaises(verifier.VerificationError):
                verifier.require_local_drive(PureWindowsPath(r"\\server\share\new"))
        check.assert_not_called()

    def test_windows_database_url_has_drive_path_and_single_uri_encoding(self):
        config = self.root / "mock.toml"
        verifier.shared.write_mock_config(config, PureWindowsPath(r"C:\synthetic space\100% 名.sqlite"))
        text = config.read_text(encoding="utf-8")
        self.assertIn('database_url="sqlite://C:/synthetic%20space/100%25%20', text)
        self.assertNotIn("sqlite:///C:", text)
        self.assertNotIn("%2525", text)
        self.assertIn('provider="mock"', text)

    def test_child_environment_keeps_only_windows_loader_essentials(self):
        source = {"SystemRoot": "SYNTHETIC_WINDOWS", "WINDIR": "SYNTHETIC_WINDOWS",
                  "OPENAI_API_KEY": "SYNTHETIC_SECRET", "AGENT_LLM_MM_CONFIG": "USER_CONFIG",
                  "PATH": "USER_TOOLCHAIN_PATH", "PYTHONPATH": "USER_CODE",
                  "openai_api_key": "SYNTHETIC_SECRET", "pSmOdUlEpAtH": "USER_MODULES"}
        with patch.dict(os.environ, source, clear=True), patch.object(verifier.shared.os, "name", "nt"):
            env = verifier.shared.make_environment(self.root)
        # os.environ uppercases keys on Windows; child env dict casing on a
        # mocked Unix host is incidental. Assert the Windows key semantics.
        env = {key.upper(): value for key, value in env.items()}
        self.assertEqual(env["SYSTEMROOT"], "SYNTHETIC_WINDOWS")
        self.assertEqual(env["WINDIR"], "SYNTHETIC_WINDOWS")
        self.assertEqual(env["PATH"], str(self.root / "bin"))
        self.assertEqual(env["USERPROFILE"], str(self.root / "home"))
        for key in ("OPENAI_API_KEY", "AGENT_LLM_MM_CONFIG", "PYTHONPATH", "PSMODULEPATH"):
            self.assertNotIn(key, env)

    def test_nested_working_directories_use_explicit_current_user_acl(self):
        workflow = verifier.WindowsWorkflow(self.root / "inert.exe", self.root, {})
        with patch.object(verifier, "prepare_private_output") as create:
            for name in ("bin", "home", "tmp", "logs", "s", "restore", "workspace-a", "workspace-b"):
                folder = self.root / name
                workflow.create_working_directory(folder)
                create.assert_called_with(folder)
        self.assertEqual(create.call_count, 8)

    def test_startup_diagnostics_export_only_fixed_codes(self):
        logs = self.root / "logs"
        logs.mkdir()
        path = logs / "core-initial.stderr.log"
        path.write_bytes(b"SYNTHETIC_SECRET C:/private/config.toml\n"
                         b"private state must be owned by the current Windows user\n"
                         b"Access denied (os error 5)")
        owned = SimpleNamespace(label="core-initial", process=SimpleNamespace(returncode=1))
        evidence = {}
        verifier.record_startup_failure(self.root, owned, evidence)
        diagnostic = evidence["startup_failures"][0]
        self.assertEqual(diagnostic["categories"], ["owner_mismatch"])
        self.assertEqual(diagnostic["os_error_codes"], [5])
        self.assertEqual(diagnostic["exit_code"], 1)
        self.assertNotIn("SYNTHETIC_SECRET", json.dumps(evidence))
        self.assertNotIn("private/config", json.dumps(evidence))
        path.write_bytes(b"SYNTHETIC_SECRET" * 6000)
        verifier.record_startup_failure(self.root, owned, evidence)
        self.assertTrue(evidence["startup_failures"][-1]["stderr_truncated"])
        self.assertEqual(evidence["startup_failures"][-1]["categories"], ["unclassified_startup_failure"])
        path.unlink()
        verifier.record_startup_failure(self.root, owned, evidence)
        self.assertEqual(evidence["startup_failures"][-1]["categories"], ["stderr_unavailable"])

    def test_existing_output_cannot_be_overwritten(self):
        binary = self.root / "inert.exe"
        binary.write_bytes(b"inert fixture")
        sentinel = self.root / "preserve"
        sentinel.write_bytes(b"preserved")
        with patch.object(verifier.os, "name", "nt"), patch.object(verifier, "require_local_drive"):
            with self.assertRaisesRegex(verifier.VerificationError, "output_must_not_exist"):
                verifier.validate_paths(binary, self.root)
        self.assertEqual(sentinel.read_bytes(), b"preserved")

    def test_private_acl_setup_only_targets_new_empty_synthetic_directory(self):
        powershell = self.root / "System32/WindowsPowerShell/v1.0/powershell.exe"
        powershell.parent.mkdir(parents=True)
        powershell.write_bytes(b"inert fixture")
        output = self.root / "new"
        with patch.dict(os.environ, {"SystemRoot": str(self.root), "OPENAI_API_KEY": "SYNTHETIC_SECRET",
                                     "PSModulePath": "SYNTHETIC_INCOMPATIBLE_PWSH_MODULES"}, clear=True), \
             patch.object(verifier.subprocess, "run", return_value=SimpleNamespace(returncode=0)) as run:
            verifier.prepare_private_output(output)
        self.assertTrue(output.is_dir())
        self.assertEqual(list(output.iterdir()), [])
        self.assertEqual(run.call_args.args[0][0], str(powershell))
        self.assertIn("-NoProfile", run.call_args.args[0])
        self.assertIn("SetAccessRuleProtection($true, $false)", run.call_args.args[0][-1])
        self.assertIn("[System.IO.Directory]::SetAccessControl(", run.call_args.args[0][-1])
        self.assertIn("$PSModuleAutoLoadingPreference = 'None'", run.call_args.args[0][-1])
        self.assertNotIn("Set-Acl", run.call_args.args[0][-1])
        self.assertNotIn("PSMODULEPATH", {key.upper() for key in run.call_args.kwargs["env"]})
        self.assertEqual(run.call_args.kwargs["env"]["LEDGER_SYNTHETIC_PRIVATE_DIRECTORY"], str(output))
        self.assertNotIn("OPENAI_API_KEY", run.call_args.kwargs["env"])
        self.assertEqual(run.call_args.kwargs["env"]["PATH"], "")
        with patch.object(verifier.subprocess, "run") as run:
            with self.assertRaises(FileExistsError):
                verifier.prepare_private_output(output)
        run.assert_not_called()

    def test_acl_setup_failure_is_fatal_and_does_not_echo_raw_output(self):
        powershell = self.root / "System32/WindowsPowerShell/v1.0/powershell.exe"
        powershell.parent.mkdir(parents=True)
        powershell.write_bytes(b"inert fixture")
        with patch.dict(os.environ, {"SystemRoot": str(self.root)}, clear=True), \
             patch.object(verifier.subprocess, "run", return_value=SimpleNamespace(returncode=1, stderr=b"SYNTHETIC_SECRET")):
            with self.assertRaisesRegex(verifier.VerificationError, "private_synthetic_directory_acl_setup_failed"):
                verifier.prepare_private_output(self.root / "new")

    def test_termination_is_limited_to_owned_core_handle_and_reaped(self):
        process = Mock(pid=123, returncode=1)
        process.poll.return_value = None
        evidence = {}
        with self.assertRaises(verifier.VerificationError):
            verifier.terminate_owned_core(SimpleNamespace(role="bridge", process=process), evidence)
        process.kill.assert_not_called()
        verifier.terminate_owned_core(SimpleNamespace(role="core", process=process), evidence)
        process.kill.assert_called_once_with()
        process.wait.assert_called_once_with(timeout=5)
        self.assertEqual(evidence["crash"]["mechanism"], "TerminateProcess")
        self.assertTrue(evidence["crash"]["confirmed"])
        self.assertNotIn("signal", evidence["crash"])

    def test_exited_core_is_not_killed_again(self):
        process = Mock(returncode=1)
        process.poll.return_value = 1
        with self.assertRaisesRegex(verifier.VerificationError, "test_core_already_exited"):
            verifier.terminate_owned_core(SimpleNamespace(role="core", process=process), {})
        process.kill.assert_not_called()

    def test_readiness_retries_only_initialize_not_application_writes(self):
        evidence = {"processes": [], "readiness": [], "rpc_calls": []}
        workflow = verifier.WindowsWorkflow(self.root / "inert.exe", self.root, evidence)
        workflow.registry.write_text(json.dumps({"workspaces": [
            {"id": "synthetic-id", "roots": [str(self.root / "project")]}]}))
        core = SimpleNamespace(process=SimpleNamespace(poll=lambda: None))
        probes = [SimpleNamespace(label="probe-1"), SimpleNamespace(label="probe-2")]
        workflow.processes.spawn = Mock(side_effect=[core, *probes])
        clients = [Mock(), Mock()]
        clients[0].initialize.side_effect = verifier.VerificationError("not_ready")
        with patch.object(verifier.shared, "McpClient", side_effect=clients), \
             patch.object(verifier.time, "sleep"):
            result = workflow.core("core-synthetic", self.root / "mock.toml")
        self.assertIs(result, core)
        for client in clients:
            client.initialize.assert_called_once_with()
            client.close.assert_called_once_with()
            client.tool.assert_not_called()
        self.assertEqual(evidence["readiness"][0]["polls"], 2)
        self.assertEqual(evidence["readiness"][0]["method"], "bridge_initialize_and_tools_list")

    def test_native_wrapper_retains_shared_failure_and_cleanup_status(self):
        evidence = {"status": "failed", "cleanup": {"passed": False}}
        with patch.object(verifier.shared, "verify", return_value=evidence) as shared_verify:
            result = verifier.verify(self.root / "inert", self.root)
        self.assertEqual(result["status"], "failed")
        self.assertFalse(result["cleanup"]["passed"])
        self.assertEqual(result["transport"], "windows_local_named_pipe")
        self.assertIs(shared_verify.call_args.kwargs["workflow_class"], verifier.WindowsWorkflow)
        self.assertIs(shared_verify.call_args.kwargs["output_initializer"], verifier.prepare_private_output)

    def test_main_failure_is_sanitized(self):
        output = io.StringIO()
        with patch.object(verifier, "verify", side_effect=RuntimeError("SYNTHETIC_PRIVATE_ERROR")), redirect_stdout(output):
            status = verifier.main(["--binary", str(self.root / "inert"), "--output", str(self.root / "new")])
        self.assertEqual(status, 1)
        self.assertNotIn("SYNTHETIC_PRIVATE_ERROR", output.getvalue())
        self.assertEqual(json.loads(output.getvalue())["status"], "failed")


if __name__ == "__main__":
    unittest.main()
