"""Offline verifier safety/receipt regressions. Never execute a candidate or model."""
from contextlib import closing, redirect_stdout
import importlib.util
import io
import json
import os
from pathlib import Path
import signal
import sqlite3
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

REPO = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("local_core_verifier", REPO / "scripts" / "verify-local-core.py")
verifier = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(verifier)


class VerifierTests(unittest.TestCase):
    def setUp(self):
        parent = REPO / "target" / "workspace-core" / "verifier-unit-tests"
        parent.mkdir(parents=True, exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(dir=parent)
        self.root = Path(self.temp.name)
        (self.root / "logs").mkdir()

    def tearDown(self):
        self.temp.cleanup()

    def ledger(self, path):
        with closing(sqlite3.connect(path)) as db:
            db.executescript("""
            CREATE TABLE events(event_id TEXT PRIMARY KEY, summary TEXT);
            CREATE TABLE claims(claim_id TEXT PRIMARY KEY, value TEXT);
            CREATE TABLE reflections(reflection_id TEXT PRIMARY KEY);
            CREATE TABLE operation_log(operation_id TEXT PRIMARY KEY, actor_id TEXT, status TEXT);
            INSERT INTO events VALUES ('synthetic-event', 'SYNTHETIC');
            INSERT INTO claims VALUES ('synthetic-claim', 'SYNTHETIC');
            INSERT INTO reflections VALUES ('synthetic-reflection');
            INSERT INTO operation_log VALUES ('synthetic-receipt', 'durable_write_receipt_v1', 'ok');
            """)
            db.commit()

    def client(self, responses):
        process = SimpleNamespace(stdin=io.StringIO(), stdout=io.StringIO("".join(json.dumps(r) + "\n" for r in responses)))
        owned = SimpleNamespace(process=process, label="fake-bridge")
        evidence = {"rpc_calls": []}
        client = verifier.McpClient(owned, self.root, evidence, timeout=.05)
        self.addCleanup(client.reader.join, 1)
        self.addCleanup(process.stdin.close)
        self.addCleanup(process.stdout.close)
        return client, process, evidence

    def test_environment_never_inherits_credentials_or_config(self):
        with patch.dict(os.environ, {"OPENAI_API_KEY": "SYNTHETIC_SECRET_CANARY", "AGENT_LLM_MM_CONFIG": "SYNTHETIC_USER_CONFIG", "PATH": "SYNTHETIC_HOST_PATH"}):
            bridge = verifier.make_environment(self.root)
            core = verifier.make_environment(self.root, self.root / "mock.toml")
        self.assertEqual(bridge["PATH"], str(self.root / "bin"))
        self.assertNotIn("OPENAI_API_KEY", bridge)
        self.assertNotIn(verifier.CONFIG_ENV, bridge)
        self.assertEqual(core[verifier.CONFIG_ENV], str(self.root / "mock.toml"))
        self.assertNotIn("SYNTHETIC_SECRET_CANARY", json.dumps(core))
        self.assertTrue(all(str(self.root) in value for value in core.values()))

    def test_mock_config_is_explicit_and_escapes_database_path(self):
        config = self.root / "mock.toml"
        verifier.write_mock_config(config, self.root / "synthetic space 名.sqlite")
        text = config.read_text()
        self.assertIn('provider="mock"', text)
        self.assertIn('[dashboard]\nenabled=false', text)
        self.assertIn('[daemon]\nenabled=false', text)
        self.assertIn('%20', text)
        self.assertNotIn('api_key', text)
        self.assertNotIn('\nbase_url=', text)

    def test_rejects_relative_or_existing_output_without_overwriting(self):
        binary = self.root / "candidate"
        binary.write_bytes(b"inert synthetic fixture")
        binary.chmod(0o700)
        sentinel = self.root / "keep"
        sentinel.write_text("preserve")
        for b, output in [(Path("candidate"), self.root / "new"), (binary, Path("relative")), (binary, self.root)]:
            with self.assertRaises(verifier.VerificationError):
                verifier.validate_paths(b, output)
        self.assertEqual(sentinel.read_text(), "preserve")

    def test_socket_length_guard_does_not_fallback_to_external_temp_directory(self):
        with self.assertRaisesRegex(verifier.VerificationError, "choose_shorter_directory"):
            verifier.validate_socket_path(Path("/" + "x" * 200 + "/c.sock"))
        verifier.validate_socket_path(Path("/synthetic/c.sock"))

    def test_payload_has_explicit_namespace_no_reflection_hints_and_stable_id(self):
        first = verifier.ingest_payload("project/synthetic-a", "A_00", "SYNTHETIC")
        self.assertEqual(first, verifier.ingest_payload("project/synthetic-a", "A_00", "SYNTHETIC"))
        self.assertEqual(first["event"]["namespace"], "project/synthetic-a")
        self.assertEqual(first["claim_drafts"][0]["namespace"], "project/synthetic-a")
        self.assertNotIn("trigger_hints", first)
        self.assertIsNone(first["episode_reference"])
        self.assertNotIn("feedback", first["event"])

    def test_failed_mcp_write_is_sent_once_and_not_retried(self):
        client, process, evidence = self.client([{"jsonrpc": "2.0", "id": 1, "error": {"code": -32602, "message": "synthetic failure"}}])
        with self.assertRaisesRegex(verifier.VerificationError, "mcp_tool_rpc_error"):
            client.tool("ingest_interaction", {"request_id": "synthetic-write"})
        messages = process.stdin.getvalue().splitlines()
        self.assertEqual(len(messages), 1)
        self.assertEqual(json.loads(messages[0])["params"]["arguments"]["request_id"], "synthetic-write")
        self.assertEqual(len(evidence["rpc_calls"]), 1)
        self.assertEqual(evidence["rpc_calls"][0]["intent"], "initial_write")

    def test_explicit_replay_is_distinguished_from_retry(self):
        response = {"jsonrpc": "2.0", "id": 1, "result": {"structuredContent": {"event_id": "synthetic", "replayed": True}}}
        client, process, evidence = self.client([response])
        result = client.tool("ingest_interaction", {"request_id": "synthetic-write"}, "intentional_replay_after_crash")
        self.assertTrue(result["replayed"])
        self.assertEqual(len(process.stdin.getvalue().splitlines()), 1)
        self.assertEqual(evidence["rpc_calls"][0]["intent"], "intentional_replay_after_crash")

    def test_eof_write_failure_does_not_retry(self):
        client, process, evidence = self.client([])
        with self.assertRaisesRegex(verifier.VerificationError, "mcp_closed_before_response"):
            client.tool("ingest_interaction", {"request_id": "synthetic-write"})
        self.assertEqual(len(process.stdin.getvalue().splitlines()), 1)
        self.assertEqual(len(evidence["rpc_calls"]), 1)

    def test_arbitrary_tool_error_is_not_authorization_evidence(self):
        self.assertFalse(verifier.is_authorization_denial({"error": {"code": -32602, "message": "invalid parameters"}}))
        self.assertFalse(verifier.is_authorization_denial({"result": {"structuredContent": {"records": []}}}))
        self.assertFalse(verifier.is_authorization_denial({"error": {"code": -32600, "message": "session workspace binding is unavailable"}}))
        self.assertTrue(verifier.is_authorization_denial({"error": {"code": -32602, "message": "tool request is not permitted for this session"}}))

    def test_backup_preserves_source_and_new_path_integrity_and_contents(self):
        source, restored = self.root / "source.sqlite", self.root / "restore.sqlite"
        self.ledger(source)
        source_bytes = source.read_bytes()
        result = verifier.backup_to_new_path(source, restored)
        self.assertEqual(result["source"], result["restored"])
        self.assertEqual(result["source"]["logical_counts"], {"events": 1, "claims": 1, "reflections": 1, "durable_write_receipts": 1})
        self.assertEqual(source.read_bytes(), source_bytes)
        with closing(sqlite3.connect(restored)) as db:
            db.execute("UPDATE claims SET value='SYNTHETIC_CHANGED'")
            db.commit()
        self.assertNotEqual(verifier.readonly_snapshot(source)["content_sha256"], verifier.readonly_snapshot(restored)["content_sha256"])
        self.assertEqual(source.read_bytes(), source_bytes)

    def test_backup_refuses_same_or_existing_destination(self):
        source, existing = self.root / "source.sqlite", self.root / "existing.sqlite"
        self.ledger(source)
        existing.write_bytes(b"preserve synthetic destination")
        for destination in (source, existing):
            with self.assertRaises(verifier.VerificationError):
                verifier.backup_to_new_path(source, destination)
        self.assertEqual(existing.read_bytes(), b"preserve synthetic destination")

    def test_snapshot_does_not_create_missing_database(self):
        missing = self.root / "missing.sqlite"
        with self.assertRaises(verifier.VerificationError):
            verifier.readonly_snapshot(missing)
        self.assertFalse(missing.exists())

    @unittest.skipUnless(hasattr(signal, "SIGKILL"), "Unix SIGKILL is required")
    def test_crash_uses_only_owned_core_handle(self):
        class Process:
            pid = 123
            returncode = None
            kills = 0
            def poll(self):
                return self.returncode
            def kill(self):
                self.kills += 1
                self.returncode = -signal.SIGKILL
            def wait(self, timeout):
                return self.returncode
        process = Process()
        with self.assertRaises(verifier.VerificationError):
            verifier.crash_core(SimpleNamespace(role="bridge", process=process), {})
        self.assertEqual(process.kills, 0)
        evidence = {}
        verifier.crash_core(SimpleNamespace(role="core", process=process), evidence)
        self.assertEqual(process.kills, 1)
        self.assertTrue(evidence["crash"]["confirmed"])

    def test_cleanup_only_visits_registered_handles_even_after_error(self):
        closed = []
        processes = verifier.Processes(self.root / "inert", self.root, {"processes": []})
        def fake(label, fail=False):
            def close():
                closed.append(label)
                if fail:
                    raise OSError("synthetic failure")
            return SimpleNamespace(label=label, close=close, process=SimpleNamespace(poll=lambda: 0))
        processes.owned = [fake("owned-one"), fake("owned-two", True), fake("owned-three")]
        result = processes.cleanup()
        self.assertEqual(closed, ["owned-three", "owned-two", "owned-one"])
        self.assertFalse(result["passed"])
        self.assertEqual(result["unclosed_test_processes"], ["owned-two"])

    def test_failure_always_records_cleanup_without_raw_exception_text(self):
        output = self.root / "run"
        binary = self.root / "inert"
        binary.write_bytes(b"inert synthetic fixture")
        with patch.object(verifier, "validate_paths", return_value=(binary, output)), \
             patch.object(verifier.Workflow, "run", side_effect=RuntimeError("SYNTHETIC_PRIVATE_EXCEPTION")), \
             patch.object(verifier.Workflow, "cleanup", return_value={"passed": True, "unclosed_test_processes": []}) as cleanup:
            evidence = verifier.verify(binary, output)
        cleanup.assert_called_once()
        self.assertEqual(evidence["status"], "failed")
        self.assertEqual(evidence["failure_code"], "RuntimeError")
        self.assertNotIn("SYNTHETIC_PRIVATE_EXCEPTION", (output / "summary.json").read_text())
        self.assertFalse(evidence["new_machine_verified"])
        self.assertFalse(evidence["natural_language_client_verified"])
        self.assertEqual(evidence["model_requests"], 0)

    def test_candidate_binary_change_cannot_be_reported_as_passed(self):
        output = self.root / "changed-binary-run"
        binary = self.root / "inert"
        binary.write_bytes(b"synthetic original")
        with patch.object(verifier, "validate_paths", return_value=(binary, output)), \
             patch.object(verifier.Workflow, "run", side_effect=lambda: binary.write_bytes(b"synthetic changed")), \
             patch.object(verifier.Workflow, "cleanup", return_value={"passed": True, "unclosed_test_processes": []}):
            evidence = verifier.verify(binary, output)
        self.assertEqual(evidence["status"], "failed")
        self.assertEqual(evidence["failure_code"], "candidate_binary_changed_during_verification")
        self.assertNotEqual(evidence["binary_sha256"], evidence["binary_sha256_after"])

    def test_main_failure_does_not_print_raw_exception(self):
        output = io.StringIO()
        with patch.object(verifier, "verify", side_effect=RuntimeError("SYNTHETIC_PRIVATE_EXCEPTION")), redirect_stdout(output):
            status = verifier.main(["--binary", str(self.root / "inert"), "--output", str(self.root / "new")])
        self.assertEqual(status, 1)
        self.assertNotIn("SYNTHETIC_PRIVATE_EXCEPTION", output.getvalue())
        self.assertEqual(json.loads(output.getvalue())["status"], "failed")


if __name__ == "__main__":
    unittest.main()
