"""Synthetic CLI -> stdio bridge -> live scoped-core integration smoke.

Run with an already-built binary, for example:
    LEDGER_BIN=/absolute/path/agent_llm_mm python3 tests/test_codex_result_feedback.py

This does not run Codex, install hooks, inspect transcripts, or establish native
host delivery/authenticity. All state is disposable, the provider is mock, and
the only transport is the ledger's local Unix socket. No build is performed.
"""

import json
import os
from pathlib import Path
import queue
import signal
import stat
import subprocess
import tempfile
import threading
import time
import unittest
import uuid
from urllib.parse import quote


class ScopedBridge:
    """Minimal bounded MCP client; raw protocol data is never written to disk."""

    def __init__(self, binary, socket, workspace, root, env):
        self.process = subprocess.Popen(
            [str(binary), "bridge", "--socket", str(socket),
             "--workspace-id", workspace, "--workspace-root", str(root),
             "--client-label", "synthetic-result-feedback-test"],
            cwd=root, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL, text=True, encoding="utf-8",
        )
        self.responses = queue.Queue(maxsize=128)
        self.next_id = 0
        self.reader = threading.Thread(target=self._read, daemon=True)
        self.reader.start()
        try:
            response = self.request("initialize", {
                "protocolVersion": "2025-03-26", "capabilities": {},
                "clientInfo": {"name": "synthetic-result-feedback-test", "version": "1"},
            })
            if "error" in response:
                raise AssertionError("scoped bridge initialization was rejected")
            if response["result"]["protocolVersion"] != "2025-03-26":
                raise AssertionError("unexpected MCP protocol version")
            self._send({"jsonrpc": "2.0", "method": "notifications/initialized"})
        except BaseException:
            self.close()
            raise

    def _read(self):
        try:
            for line in iter(lambda: self.process.stdout.readline(1048577), ""):
                if len(line) > 1048576:
                    break
                self.responses.put(json.loads(line), timeout=1)
        except (ValueError, queue.Full, OSError):
            pass
        finally:
            try:
                self.responses.put(None, timeout=1)
            except queue.Full:
                pass

    def _send(self, value):
        self.process.stdin.write(json.dumps(value) + "\n")
        self.process.stdin.flush()

    def request(self, method, params):
        self.next_id += 1
        request_id = self.next_id
        self._send({"jsonrpc": "2.0", "id": request_id,
                    "method": method, "params": params})
        end = time.monotonic() + 10
        while time.monotonic() < end:
            try:
                response = self.responses.get(timeout=max(.01, end - time.monotonic()))
            except queue.Empty:
                raise AssertionError("scoped MCP response timed out") from None
            if response is None:
                raise AssertionError("scoped bridge closed before its response")
            if response.get("id") == request_id:
                return response
        raise AssertionError("scoped MCP response timed out")

    def call(self, name, arguments):
        response = self.request("tools/call", {"name": name, "arguments": arguments})
        if "error" in response or response.get("result", {}).get("isError") is True:
            raise AssertionError("scoped tool failed: " + name + ": " + json.dumps(response))
        result = response["result"]
        if "structuredContent" not in result:
            raise AssertionError("scoped tool omitted structuredContent: " + name)
        return result["structuredContent"]

    def close(self):
        if not self.process.stdin.closed:
            self.process.stdin.close()
        try:
            self.process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            self.process.terminate()
            try:
                self.process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=3)
        self.reader.join(timeout=2)
        self.process.stdout.close()


@unittest.skipUnless(os.name == "posix", "integration smoke requires a Unix local socket")
class CodexResultFeedbackIntegration(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        selected = os.environ.get("LEDGER_BIN")
        if not selected:
            raise unittest.SkipTest("set LEDGER_BIN to an already-built ledger binary")
        binary = Path(selected)
        if not binary.is_absolute() or not binary.is_file() or not os.access(binary, os.X_OK):
            raise AssertionError("LEDGER_BIN must name an absolute executable file")
        cls.binary = binary.resolve()

    def setUp(self):
        # A short, private path also fits Darwin's Unix socket length limit.
        self.temp = tempfile.TemporaryDirectory(prefix="result-feedback-", dir="/tmp")
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name).resolve()
        self.base.chmod(0o700)
        self.home = self.base / "home"
        self.project = self.base / "project"
        for path in (self.home, self.project, self.base / "bin", self.base / "config"):
            path.mkdir(mode=0o700)
        self.config = self.base / "ledger.toml"
        self.database = self.base / "ledger.sqlite"
        self.config.write_text(
            'transport="stdio"\ndatabase_url='
            + json.dumps("sqlite://" + quote(self.database.as_posix(), safe="/:"))
            + '\n[model]\nprovider="mock"\n[dashboard]\nenabled=false\n'
              '[daemon]\nenabled=false\n',
            encoding="utf-8",
        )
        self.config.chmod(0o600)
        # Construct from scratch, never inherit provider secrets or host config.
        self.env = {
            "PATH": str(self.base / "bin"),
            "HOME": str(self.home),
            "CODEX_HOME": str(self.base / "config"),
            "XDG_CONFIG_HOME": str(self.base / "config"),
            "XDG_DATA_HOME": str(self.home / "data"),
            "XDG_CACHE_HOME": str(self.home / "cache"),
            "TMPDIR": str(self.base), "TMP": str(self.base), "TEMP": str(self.base),
            "TOKIO_WORKER_THREADS": "2",
            "AGENT_LLM_MM_CONFIG": str(self.config),
        }
        self.registry = self.base / "registry.json"
        self.socket = self.base / "c.sock"
        self.state = self.base / "context.sqlite"
        self.outbox = self.base / "results.sqlite"
        self.session = "synthetic-session"
        self.task_id = "synthetic-task"
        self.candidate_id = "synthetic-candidate"
        self.use_id = "synthetic-adopted-use"
        self.source = {"kind": "caller_reported", "reference": None}
        self.core = None
        self.bridges = []
        self.addCleanup(self.stop_core)
        self.cli("init")
        self.workspace = self.cli(
            "workspace", "register", "--registry", self.registry, "--root", self.project)
        self.namespace = self.workspace["project_namespace"]
        self.start_core()
        self.bridge = self.new_bridge()
        self.seed_scoped_ledger()
        self.cli("codex-context", "init", "--state", self.state)
        self.cli(
            "codex-context", "bind", "--state", self.state, "--registry", self.registry,
            "--socket", self.socket, "--root", self.project,
            "--workspace-id", self.workspace["id"], "--task-id", self.task_id,
            "--session-id", self.session,
        )
        self.cli("codex-results", "init", "--outbox", self.outbox)
        token = uuid.uuid4().hex
        self.canaries = [kind + "_RAW_CANARY_" + token for kind in (
            "INPUT", "OUTPUT", "TRANSCRIPT", "MODEL", "PERMISSION", "AGENT_ID", "AGENT_TYPE")]

    def process(self, *args, payload=None):
        return subprocess.run(
            [str(self.binary), *map(str, args)], env=self.env, cwd=self.project,
            input=None if payload is None else json.dumps(payload),
            capture_output=True, text=True, encoding="utf-8", timeout=25,
        )

    def cli(self, *args, payload=None):
        result = self.process(*args, payload=payload)
        self.assertEqual(result.returncode, 0, result.stderr)
        try:
            return json.loads(result.stdout) if result.stdout.strip() else None
        except ValueError:
            self.fail("ledger CLI did not return JSON for " + str(args[0]))

    def start_core(self):
        self.core_errors = bytearray()
        self.core = subprocess.Popen(
            [str(self.binary), "core", "--registry", str(self.registry),
             "--socket", str(self.socket)],
            env=self.env, cwd=self.project, stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
        )
        process = self.core
        errors = self.core_errors

        def drain_errors():
            while True:
                chunk = process.stderr.read(1024)
                if not chunk:
                    return
                errors.extend(chunk[:max(0, 8192 - len(errors))])

        self.core_reader = threading.Thread(target=drain_errors, daemon=True)
        self.core_reader.start()
        end = time.monotonic() + 10
        while time.monotonic() < end:
            if self.core.poll() is not None:
                self.core_reader.join(timeout=1)
                self.fail("isolated core exited before socket readiness: "
                          + self.core_errors.decode(errors="replace"))
            if self.socket.exists() and stat.S_ISSOCK(self.socket.stat().st_mode):
                return
            time.sleep(.02)
        self.fail("isolated local core socket was not ready")

    def stop_core(self):
        for bridge in self.bridges:
            bridge.close()
        self.bridges.clear()
        if self.core is not None:
            if self.core.poll() is None:
                # The core handles Ctrl-C to drain writes and remove its socket.
                self.core.send_signal(signal.SIGINT)
                try:
                    self.core.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    self.core.kill()
                    self.core.wait(timeout=3)
            self.core_reader.join(timeout=2)
            self.core.stderr.close()
            self.core = None

    def new_bridge(self):
        # Successful calls also prove bridge transport does not load model config.
        env = dict(self.env, AGENT_LLM_MM_CONFIG=str(self.base / "missing-provider.toml"))
        bridge = ScopedBridge(
            self.binary, self.socket, self.workspace["id"], self.project, env)
        self.bridges.append(bridge)
        return bridge

    def scoped(self, name, **arguments):
        return self.bridge.call(name, dict(arguments, namespace=self.namespace))

    def requirement(self, requirement_id):
        return {"requirement_id": requirement_id, "kind": "prohibition",
                "text": "Keep this synthetic operation within its disposable workspace.",
                "state": "reported", "source": self.source, "replaces": None}

    def seed_scoped_ledger(self):
        task = self.scoped(
            "create_task", task_id=self.task_id, request_id="create-task",
            title="Synthetic result feedback", requirements=[self.requirement("initial")],
            source=self.source, reason="Local-only integration fixture")
        self.assertEqual(task["record"]["version"], 1)
        event = self.scoped(
            "ingest_interaction", request_id="seed-event",
            event={"owner": "World", "namespace": self.namespace, "kind": "Observation",
                   "summary": "Synthetic source observation, not a real tool run"},
            claim_drafts=[])
        self.scoped(
            "record_episode", request_id="seed-episode", episode_id="synthetic-episode",
            content={
                "title": "Synthetic source episode", "objective": "Exercise scoped feedback",
                "actions": ["No real tool execution"], "observations": ["Fixture only"],
                "outcome": "No real-world outcome established", "lesson": "Retain provenance",
                "limitations": ["Synthetic fixture"],
                "source_event_refs": ["event:" + event["event_id"]],
            })
        self.scoped(
            "propose_experience_candidate", request_id="seed-candidate",
            candidate_id=self.candidate_id, content={
                "kind": "semantic", "title": "Synthetic experience",
                "statement": "Keep synthetic result metadata separate from outcome claims",
                "steps": [], "limitations": ["Not validated against a native host"],
                "source_episode_ids": ["synthetic-episode"],
            })
        active = self.scoped(
            "set_experience_candidate_status", request_id="activate-candidate",
            candidate_id=self.candidate_id, expected_version=1, status="active")
        self.assertEqual(active["record"]["version"], 2)
        original = self.scoped(
            "record_experience_use", request_id="seed-use", use_id=self.use_id,
            task_id=self.task_id, task_version=1, expected_task_version=1,
            candidate_id=self.candidate_id, candidate_version=2, expected_candidate_version=2,
            decision="adopted", reason="Explicit synthetic caller selection",
            conditions=[{"condition": "Disposable local fixture",
                         "assessment": "caller_reported_satisfied"}],
            action_event_refs=[], outcome_event_refs=[], verification="not_performed",
            evaluation_method="No independent evaluation",
            limitations=["Synthetic fixture, not native-host evidence"], related_use_id=None)
        self.original_observation = original["record"]["observation"]
        self.assertTrue(self.uses()[0]["currently_reusable"])

    def hook_input(self, event, tool_id="synthetic-action"):
        payload = {
            "session_id": self.session, "turn_id": "synthetic-turn",
            "cwd": str(self.project), "hook_event_name": event,
            "tool_name": "apply_patch", "tool_use_id": tool_id,
            "tool_input": {"command": self.canaries[0]},
            # A deliberately nonexistent synthetic path. Never create/read a transcript.
            "transcript_path": str(self.base / self.canaries[2]),
            "model": self.canaries[3], "permission_mode": "default",
            "agent_id": self.canaries[5], "agent_type": self.canaries[6],
        }
        if event == "PostToolUse":
            payload["tool_response"] = {"output": self.canaries[1], "exit_code": 0}
        return payload

    def result_args(self, action):
        return (
            "codex-results", action, "--outbox", self.outbox,
            "--state", self.state, "--registry", self.registry, "--socket", self.socket,
        )

    def link(self):
        self.assertEqual(self.cli(
            *self.result_args("link"), "--use-id", self.use_id,
            payload=self.hook_input("PreToolUse")), {})
        linked = self.inspect()
        self.assertFalse(linked["captured"])
        self.assertFalse(linked["completed"])
        self.assertEqual(linked["linked_use_id"], self.use_id)
        self.assertEqual((linked["task_version"], linked["candidate_version"]), (1, 2))

    def inspect(self):
        return self.cli(
            "codex-results", "inspect", "--outbox", self.outbox,
            "--session-id", self.session, "--tool-use-id", "synthetic-action")

    def uses(self):
        return self.scoped("list_experience_uses", task_id=self.task_id, limit=50)

    def assert_no_raw_content_persisted(self):
        # Include SQLite WAL/journal files, adapter state, and all other fixture files.
        for path in self.base.rglob("*"):
            if path.is_file():
                content = path.read_bytes()
                for canary in self.canaries:
                    self.assertNotIn(canary.encode(), content,
                                     "raw hook content persisted in " + path.name)
        self.assertFalse((self.base / self.canaries[2]).exists())
        self.assertFalse((self.base / "config" / "hooks.json").exists())

    def test_completed_feedback_keeps_historical_versions_and_omits_raw_payload(self):
        self.link()
        revised = self.scoped(
            "revise_task", task_id=self.task_id, request_id="advance-task", expected_version=1,
            changes=[{"operation": "add", "requirement": self.requirement("later")}],
            source=self.source, reason="Advance head after explicit pre-action linkage")
        self.assertEqual(revised["record"]["version"], 2)
        withdrawn = self.scoped(
            "set_experience_candidate_status", request_id="withdraw-candidate",
            candidate_id=self.candidate_id, expected_version=2, status="rejected")
        self.assertEqual(withdrawn["record"]["version"], 3)

        post = self.hook_input("PostToolUse")
        self.assertEqual(self.cli(*self.result_args("capture"), payload=post), {})
        summary = self.inspect()
        self.assertTrue(summary["captured"])
        self.assertTrue(summary["completed"])
        self.assertEqual(summary["origin"], "unverified")
        self.assertEqual(summary["outcome"], "unknown")
        self.assertFalse(summary["independently_verified"])
        self.assertFalse(summary["execution_permission"])

        uses = self.uses()
        self.assertEqual(len(uses), 2)
        original = next(row for row in uses
                        if row["record"]["observation"]["use_id"] == self.use_id)
        self.assertEqual(original["record"]["observation"], self.original_observation)
        result = next(row for row in uses
                      if row["record"]["observation"]["related_use_id"] == self.use_id)
        observation = result["record"]["observation"]
        self.assertEqual(observation["namespace"], self.namespace)
        self.assertEqual(observation["task_id"], self.task_id)
        self.assertEqual(observation["candidate_id"], self.candidate_id)
        self.assertEqual((observation["task_version"], observation["candidate_version"]), (1, 2))
        self.assertEqual(observation["decision"], "adopted")
        self.assertEqual(observation["verification"], "not_performed")
        self.assertEqual(observation["conditions"][0]["assessment"], "unknown")
        self.assertFalse(result["currently_reusable"])
        self.assertEqual(observation["outcome_event_refs"], ["event:" + summary["event_id"]])
        exported = self.scoped("export_memory")
        event = next(row for row in exported["events"] if row["event_id"] == summary["event_id"])
        self.assertEqual(event["event"]["feedback"]["source_kind"], "tool_reported")
        self.assertEqual(event["event"]["feedback"]["verification_result"], "not_performed")
        self.assertEqual(event["event"]["feedback"]["observed_version"], "1")
        self.assertEqual(len(exported["events"]), 2)

        # Exact replay is idempotent, including after the candidate was withdrawn.
        self.assertEqual(self.cli(*self.result_args("capture"), payload=post), {})
        retried = self.cli(
            *self.result_args("retry"), "--session-id", self.session,
            "--tool-use-id", "synthetic-action")
        self.assertTrue(retried["completed"])
        self.assertEqual(self.inspect(), summary)
        self.assertEqual(self.uses(), uses)
        self.assertEqual(self.scoped("export_memory"), exported)
        self.assert_no_raw_content_persisted()

    def test_offline_capture_stays_pending_and_revocation_blocks_delivery(self):
        self.link()
        before_uses = self.uses()
        before_export = self.scoped("export_memory")
        self.stop_core()
        post = self.hook_input("PostToolUse")
        failed_delivery = self.process(*self.result_args("capture"), payload=post)
        # Public capture output remains fail-open without falsely marking delivery.
        self.assertEqual(failed_delivery.returncode, 0)
        self.assertEqual(json.loads(failed_delivery.stdout), {})
        self.assertIn("incomplete", failed_delivery.stderr)
        pending = self.inspect()
        self.assertTrue(pending["captured"])
        self.assertFalse(pending["completed"])
        self.assertIsNone(pending["event_id"])
        self.assertFalse(pending["use_request_prepared"])

        self.start_core()
        self.bridge = self.new_bridge()
        self.cli("workspace", "disable", "--registry", self.registry,
                 "--id", self.workspace["id"])
        # Check revocation on an already-initialized scoped core session too.
        denied = self.bridge.request(
            "tools/call", {"name": "list_experience_uses",
                          "arguments": {"namespace": self.namespace, "task_id": self.task_id}})
        self.assertTrue("error" in denied or denied.get("result", {}).get("isError") is True)
        retry = self.process(
            *self.result_args("retry"), "--session-id", self.session,
            "--tool-use-id", "synthetic-action")
        self.assertNotEqual(retry.returncode, 0)
        self.assertIn("operation incomplete", retry.stderr)
        self.assertEqual(self.cli(*self.result_args("capture"), payload=post), {})
        self.assertEqual(self.inspect(), pending)

        # Restore only this disposable fixture grant to inspect the ledger.
        # No retry is issued while checking that revocation prevented any write.
        self.cli("workspace", "enable", "--registry", self.registry,
                 "--id", self.workspace["id"])
        self.assertEqual(self.uses(), before_uses)
        self.assertEqual(self.scoped("export_memory"), before_export)
        self.assert_no_raw_content_persisted()

        # A later explicit authorized retry delivers the retained exact observation.
        recovered = self.cli(
            *self.result_args("retry"), "--session-id", self.session,
            "--tool-use-id", "synthetic-action")
        self.assertTrue(recovered["completed"])
        self.assertEqual(len(self.uses()), 2)
        self.assertEqual(len(self.scoped("export_memory")["events"]), 2)
        self.assert_no_raw_content_persisted()


if __name__ == "__main__":
    unittest.main()
