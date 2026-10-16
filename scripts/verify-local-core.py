#!/usr/bin/env python3
"""Isolated local-core/stdio-bridge verification; no Cargo, model or user configuration.

Supply an absolute installed/candidate binary and a new absolute output directory.
All writable state lives beneath output. Unix local transport is required.
This is a direct MCP protocol test, not natural-language or new-machine acceptance.
"""
import argparse
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import queue
import shutil
import signal
import socket
import sqlite3
import subprocess
import sys
import threading
import time
import uuid
from urllib.parse import quote

CONFIG_ENV = "AGENT_LLM_MM_CONFIG"
MAX_MESSAGE_BYTES = 2 * 1024 * 1024
REQUIRED_TOOLS = {"ingest_interaction", "recall_memory", "get_memory", "supersede_memory", "get_reflection_history"}
WRITE_TOOLS = {"ingest_interaction", "supersede_memory"}


class VerificationError(RuntimeError):
    """Static diagnostic identifier; never contain tool responses or credentials."""


def require(condition, code):
    if not condition:
        raise VerificationError(code)


def write_json(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def validate_paths(binary, output):
    require(binary.is_absolute() and output.is_absolute(), "absolute_binary_and_output_required")
    binary = binary.resolve(strict=True)
    require(binary.is_file() and os.access(binary, os.X_OK), "binary_must_be_executable_file")
    require(not output.exists() and not output.is_symlink(), "output_must_not_exist")
    output = output.resolve()
    require(not output.exists(), "output_must_not_exist")
    require(hasattr(socket, "AF_UNIX") and os.name == "posix", "unix_local_transport_required")
    validate_socket_path(output / "s" / "c.sock")
    return binary, output


def validate_socket_path(path):
    # Darwin sockaddr_un.sun_path includes at most 104 bytes including terminator.
    limit = 103 if sys.platform == "darwin" else 107
    require(len(os.fsencode(path)) <= limit, "output_path_too_long_for_unix_socket_choose_shorter_directory")


def make_environment(output, config=None):
    """Construct from scratch: never inspect or inherit provider/auth variables."""
    env = {
        "PATH": str(output / "bin"),
        "HOME": str(output / "home"),
        "TMPDIR": str(output / "tmp"),
        "TMP": str(output / "tmp"),
        "TEMP": str(output / "tmp"),
        "XDG_DATA_HOME": str(output / "home" / "data"),
        "XDG_CONFIG_HOME": str(output / "home" / "config"),
        "XDG_CACHE_HOME": str(output / "home" / "cache"),
    }
    if os.name == "nt":
        # Only native loader essentials; never inherit provider/auth settings.
        env.update({key: value for key, value in os.environ.items()
                    if key.upper() in ("SYSTEMROOT", "WINDIR", "SYSTEMDRIVE")})
        env["USERPROFILE"] = str(output / "home")
    if config is not None:
        env[CONFIG_ENV] = str(config)
    return env


def write_mock_config(path, database):
    # SQLite URLs use URI escaping, so spaces and Unicode remain unambiguous.
    database_url = "sqlite://" + quote(database.as_posix(), safe="/:")
    path.write_text(
        "transport=\"stdio\"\ndatabase_url=" + json.dumps(database_url) +
        "\n[model]\nprovider=\"mock\"\n[dashboard]\nenabled=false\n[daemon]\nenabled=false\n",
        encoding="utf-8",
    )
    path.chmod(0o600)


class OwnedProcess:
    def __init__(self, process, label, role, handles):
        self.process, self.label, self.role, self.handles = process, label, role, handles

    def close(self):
        process = self.process
        if process.poll() is None:
            if self.role == "bridge" and process.stdin is not None:
                try:
                    process.stdin.close()
                except (OSError, ValueError):
                    pass
            else:
                process.terminate()
            try:
                process.wait(timeout=4)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=4)
        for handle in self.handles:
            handle.close()
        for handle in (process.stdin, process.stdout):
            if handle is not None and not handle.closed:
                handle.close()


class Processes:
    def __init__(self, binary, output, evidence):
        self.binary, self.output, self.evidence = binary, output, evidence
        self.owned = []

    def spawn(self, label, role, args, config=None, pipe=False):
        require(label.replace("-", "").isalnum(), "invalid_internal_process_label")
        stderr = (self.output / "logs" / (label + ".stderr.log")).open("wb")
        stdout = subprocess.PIPE if pipe else (self.output / "logs" / (label + ".stdout.log")).open("wb")
        handles = [stderr] + ([] if pipe else [stdout])
        try:
            process = subprocess.Popen(
                [str(self.binary), *map(str, args)], cwd=self.output,
                env=make_environment(self.output, config),
                stdin=subprocess.PIPE if pipe else subprocess.DEVNULL,
                stdout=stdout, stderr=stderr, text=pipe, encoding="utf-8" if pipe else None,
            )
        except BaseException:
            for handle in handles:
                handle.close()
            raise
        owned = OwnedProcess(process, label, role, handles)
        self.owned.append(owned)
        self.evidence["processes"].append({"label": label, "role": role, "pid": process.pid})
        return owned

    def command(self, label, args, config=None):
        owned = self.spawn(label, "command", args, config)
        try:
            owned.process.wait(timeout=20)
            require(owned.process.returncode == 0, "cli_command_failed")
            data = (self.output / "logs" / (label + ".stdout.log")).read_text(encoding="utf-8")
            return json.loads(data) if data.strip() else None
        finally:
            owned.close()

    def cleanup(self):
        errors = []
        for owned in reversed(self.owned):
            try:
                owned.close()
            except (OSError, subprocess.TimeoutExpired):
                errors.append(owned.label)
        return {"passed": not errors and all(p.process.poll() is not None for p in self.owned),
                "unclosed_test_processes": errors}


def wait_ready(owned, address, evidence, timeout=15):
    """Only connection readiness is polled; application writes are never retried."""
    deadline = time.monotonic() + timeout
    polls = 0
    while time.monotonic() < deadline:
        require(owned.process.poll() is None, "core_exited_before_ready")
        polls += 1
        try:
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
                connection.settimeout(.25)
                connection.connect(str(address))
            evidence["readiness"].append({"process": owned.label, "polls": polls})
            return
        except (OSError, TimeoutError):
            time.sleep(.05)
    raise VerificationError("core_socket_ready_timeout")


def crash_core(owned, evidence):
    require(owned.role == "core", "only_owned_test_core_may_be_crashed")
    require(owned.process.poll() is None, "test_core_already_exited")
    owned.process.kill()
    owned.process.wait(timeout=5)
    require(owned.process.returncode == -signal.SIGKILL, "test_core_sigkill_not_confirmed")
    evidence["crash"] = {"test_core_pid": owned.process.pid, "signal": "SIGKILL", "confirmed": True}


class McpClient:
    def __init__(self, owned, output, evidence, timeout=15):
        self.owned, self.output, self.evidence, self.timeout = owned, output, evidence, timeout
        self.counter = 0
        self.messages = queue.Queue(maxsize=256)
        self.log_path = output / "logs" / (owned.label + ".mcp.jsonl")
        def reader():
            try:
                with self.log_path.open("w", encoding="utf-8") as log:
                    while True:
                        line = owned.process.stdout.readline(MAX_MESSAGE_BYTES)
                        if not line:
                            self.messages.put(None)
                            return
                        if len(line.encode("utf-8")) >= MAX_MESSAGE_BYTES or not line.endswith("\n"):
                            self.messages.put(VerificationError("mcp_response_size_or_framing_invalid"))
                            return
                        log.write(line); log.flush()
                        self.messages.put(json.loads(line))
            except (OSError, ValueError):
                self.messages.put(VerificationError("mcp_response_invalid_json"))
        self.reader = threading.Thread(target=reader, daemon=True)
        self.reader.start()

    def request(self, method, params, intent="read"):
        self.counter += 1
        request_id = self.counter
        self.evidence["rpc_calls"].append({
            "client": self.owned.label, "id": request_id, "method": method,
            "tool": params.get("name") if method == "tools/call" else None, "intent": intent,
        })
        # Exactly one send. A timeout or broken pipe fails this run, never resends.
        payload = {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}
        try:
            self.owned.process.stdin.write(json.dumps(payload, ensure_ascii=False) + "\n")
            self.owned.process.stdin.flush()
        except (OSError, ValueError):
            raise VerificationError("mcp_request_send_failed") from None
        deadline = time.monotonic() + self.timeout
        while True:
            remaining = deadline - time.monotonic()
            require(remaining > 0, "mcp_response_timeout")
            try:
                response = self.messages.get(timeout=remaining)
            except queue.Empty:
                raise VerificationError("mcp_response_timeout") from None
            if isinstance(response, VerificationError):
                raise response
            require(response is not None, "mcp_closed_before_response")
            require(isinstance(response, dict), "mcp_response_not_object")
            if response.get("id") == request_id and "method" not in response:
                return response
            require("id" not in response or "method" not in response, "unexpected_mcp_server_request")

    def initialize(self):
        response = self.request("initialize", {"protocolVersion": "2025-03-26", "capabilities": {},
            "clientInfo": {"name": "isolated-local-core-verifier", "version": "1"}})
        require("result" in response and "error" not in response, "mcp_initialize_failed")
        self.owned.process.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
        self.owned.process.stdin.flush()
        inventory = self.request("tools/list", {})
        tools = inventory.get("result", {}).get("tools", [])
        require(REQUIRED_TOOLS <= {tool.get("name") for tool in tools}, "required_memory_tools_missing")

    def tool_response(self, name, arguments, intent=None):
        return self.request("tools/call", {"name": name, "arguments": arguments},
                            intent or ("initial_write" if name in WRITE_TOOLS else "read"))

    def tool(self, name, arguments, intent=None):
        response = self.tool_response(name, arguments, intent)
        require("error" not in response, "mcp_tool_rpc_error")
        result = response.get("result", {})
        require(not result.get("isError", False), "mcp_tool_result_error")
        content = result.get("structuredContent")
        if content is None:
            texts = [item.get("text") for item in result.get("content", []) if item.get("type") == "text"]
            require(len(texts) == 1, "mcp_tool_structured_content_missing")
            try:
                content = json.loads(texts[0])
            except (ValueError, TypeError):
                raise VerificationError("mcp_tool_content_invalid") from None
        require(isinstance(content, dict), "mcp_tool_content_not_object")
        return content

    def close(self):
        self.owned.close()
        self.reader.join(timeout=1)


def is_authorization_denial(response):
    error = response.get("error", {})
    return error.get("code") == -32602 and error.get("message") == "tool request is not permitted for this session"


def draft(namespace, index, value):
    return {"owner": "World", "namespace": namespace, "subject": "SYNTHETIC_CORE_PROBE_" + index,
            "predicate": "synthetic_venue", "object": json.dumps(value), "mode": "Observed"}


def ingest_payload(namespace, index, value):
    request_id = "synthetic-core-" + index
    return {"request_id": request_id,
            "event": {"owner": "World", "namespace": namespace, "kind": "Observation",
                      "summary": "SYNTHETIC_CORE_PROBE " + index + " " + value},
            "claim_drafts": [draft(namespace, index, value)], "episode_reference": None}


def readonly_snapshot(path):
    require(path.is_file(), "synthetic_database_missing")
    with closing(sqlite3.connect(path.as_uri() + "?mode=ro", uri=True, timeout=5)) as db:
        db.execute("PRAGMA query_only=ON")
        integrity = [row[0] for row in db.execute("PRAGMA integrity_check")]
        require(integrity == ["ok"], "database_integrity_check_failed")
        require(not list(db.execute("PRAGMA foreign_key_check")), "database_foreign_key_check_failed")
        counts, digest = {}, hashlib.sha256()
        tables = [row[0] for row in db.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")]
        for table in tables:
            quoted = '"' + table.replace('"', '""') + '"'
            rows = list(db.execute("SELECT * FROM " + quoted))
            counts[table] = len(rows)
            encoded = [json.dumps(list(row), ensure_ascii=False, separators=(",", ":"), default=lambda v: {"hex": v.hex()}) for row in rows]
            digest.update(json.dumps([table, sorted(encoded)], ensure_ascii=False).encode("utf-8"))
        receipts = db.execute("SELECT COUNT(*) FROM operation_log WHERE actor_id='durable_write_receipt_v1' AND status='ok'").fetchone()[0]
        logical = {name: counts[name] for name in ("events", "claims", "reflections")}
        logical["durable_write_receipts"] = receipts
        return {"integrity": "ok", "counts": counts, "logical_counts": logical, "content_sha256": digest.hexdigest()}


def backup_to_new_path(source, destination):
    require(source.resolve() != destination.resolve(), "backup_destination_must_be_new_path")
    require(not destination.exists(), "backup_destination_must_not_exist")
    before = readonly_snapshot(source)
    with closing(sqlite3.connect(source.as_uri() + "?mode=ro", uri=True, timeout=5)) as origin:
        with closing(sqlite3.connect(destination)) as restored:
            origin.backup(restored)
    destination.chmod(0o600)
    after = readonly_snapshot(source)
    restored = readonly_snapshot(destination)
    require(before == after, "backup_modified_source_database")
    require(restored == before, "backup_content_mismatch")
    return {"source": before, "restored": restored, "read_only_source_verified": True}


class Workflow:
    def __init__(self, binary, output, evidence):
        self.binary, self.output, self.evidence = binary, output, evidence
        self.processes = Processes(binary, output, evidence)
        self.clients = []
        self.serial = 0
        self.address = output / "s" / "c.sock"
        self.registry = output / "registry.json"
        self.database = output / "memory.sqlite"
        self.config = output / "mock.toml"
        self.restored_database = output / "restore" / "memory.sqlite"
        self.restored_config = output / "restore" / "mock.toml"

    def check(self, condition, label):
        require(condition, label)
        self.evidence["checks"].append(label)

    def core(self, label, config):
        owned = self.processes.spawn(label, "core", ["core", "--socket", self.address, "--registry", self.registry], config)
        wait_ready(owned, self.address, self.evidence)
        return owned

    def crash(self, owned):
        crash_core(owned, self.evidence)

    def bridge(self, workspace, root):
        self.serial += 1
        label = "bridge-" + str(self.serial)
        owned = self.processes.spawn(label, "bridge", ["bridge", "--socket", self.address,
            "--workspace-id", workspace["id"], "--workspace-root", root, "--client-label", label], pipe=True)
        client = McpClient(owned, self.output, self.evidence)
        self.clients.append(client)
        client.initialize()
        return client

    def recall(self, client, namespace, query="SYNTHETIC_CORE_PROBE"):
        return client.tool("recall_memory", {"namespace": namespace, "query": query, "limit": 100})

    def get(self, client, namespace, kind, reference):
        return client.tool("get_memory", {"namespace": namespace, "record_type": kind, "id": reference})["record"]

    def deny(self, client, tool, arguments):
        response = client.tool_response(tool, arguments, "negative_authorization")
        self.check(is_authorization_denial(response), "authorization_denied_" + tool + "_" + client.owned.label)
        self.check("SYNTHETIC_CANARY_B" not in json.dumps(response), "denial_did_not_expose_foreign_canary_" + tool)

    def create_working_directory(self, folder):
        folder.mkdir(mode=0o700)

    def run(self):
        for name in ("bin", "home", "tmp", "logs", "s", "restore", "workspace-a", "workspace-b"):
            folder = self.output / name
            self.create_working_directory(folder)
        env = make_environment(self.output)
        self.check(all(shutil.which(name, path=env["PATH"]) is None for name in ("cargo", "rustc", "rustup")), "runtime_path_contains_no_cargo_or_rust")
        self.check(CONFIG_ENV not in env, "bridge_environment_has_no_config_override")
        write_mock_config(self.config, self.database)
        self.processes.command("initialize", ["init"], self.config)
        a_root, b_root = self.output / "workspace-a", self.output / "workspace-b"
        a = self.processes.command("register-a", ["workspace", "--registry", self.registry, "register", "--root", a_root])
        b = self.processes.command("register-b", ["workspace", "--registry", self.registry, "register", "--root", b_root])
        for workspace in (a, b):
            self.check(isinstance(workspace, dict) and str(uuid.UUID(workspace["id"])) == workspace["id"], "canonical_registered_workspace_" + workspace["id"])
            self.check(workspace["project_namespace"] == "project/" + workspace["id"], "workspace_namespace_matches_registered_id")
        self.check(a["id"] != b["id"], "independent_workspace_ids")
        self.evidence["workspaces"] = {"a": a["id"], "b": b["id"]}
        a_ns, b_ns = a["project_namespace"], b["project_namespace"]
        core = self.core("core-initial", self.config)
        ca, cb = self.bridge(a, a_root), self.bridge(b, b_root)
        saved = {"a": [], "b": []}
        first_payload = None
        for label, count, client, namespace in (("A", 10, ca, a_ns), ("B", 5, cb, b_ns)):
            for index in range(count):
                key = label + "_" + str(index).zfill(2)
                payload = ingest_payload(namespace, key, "SYNTHETIC_CANARY_" + key)
                result = client.tool("ingest_interaction", payload)
                self.check(not result.get("replayed", False), "initial_write_not_replayed_" + key)
                saved[label.lower()].append(result["event_id"])
                if key == "A_00":
                    first_payload = payload
        ca.close()
        ca = self.bridge(a, a_root)
        recalled_a = self.recall(ca, a_ns)
        recalled_b = self.recall(cb, b_ns)
        self.check(all(event + ":claim:0" in json.dumps(recalled_a) for event in saved["a"]), "ten_a_claims_recalled_after_bridge_reconnect")
        self.check(all(event + ":claim:0" in json.dumps(recalled_b) for event in saved["b"]), "five_b_claims_recalled")
        self.check("SYNTHETIC_CANARY_B" not in json.dumps(recalled_a) and "SYNTHETIC_CANARY_A" not in json.dumps(recalled_b), "recall_namespace_canaries_isolated")
        old_ref, foreign_ref = saved["a"][0] + ":claim:0", saved["b"][0] + ":claim:0"
        old = self.get(ca, a_ns, "Claim", old_ref)
        source_ref = "event:" + saved["a"][0].removeprefix("event:")
        self.check(old["status"] == "Active" and source_ref in old["provenance"]["evidence_event_references"], "original_claim_has_event_provenance")
        self.check(self.get(ca, a_ns, "Event", saved["a"][0]) is not None, "original_source_event_retrievable")
        correction_source = ca.tool("ingest_interaction", {"request_id": "synthetic-core-correction-source",
            "event": {"owner": "World", "namespace": a_ns, "kind": "Observation", "summary": "SYNTHETIC_CORE_PROBE explicit synthetic correction"},
            "claim_drafts": [], "episode_reference": None})
        correction = ca.tool("supersede_memory", {"request_id": "synthetic-core-correction", "namespace": a_ns,
            "claim_reference": old_ref, "replacement_claim": draft(a_ns, "A_00", "SYNTHETIC_CORRECTED_A_00"),
            "replacement_evidence_event_ids": [correction_source["event_id"]], "summary": "Synthetic verifier explicit correction"})
        new_ref = correction["replacement_claim_id"]
        self.check(self.get(ca, a_ns, "Claim", old_ref)["status"] == "Superseded", "old_claim_preserved_as_superseded")
        self.check(self.get(ca, a_ns, "Claim", new_ref)["status"] == "Active", "replacement_claim_active")
        history_args = {"namespace": a_ns, "claim_reference": old_ref}
        history = ca.tool("get_reflection_history", history_args)
        self.check(bool(history["reflections"]) and new_ref in json.dumps(history), "explicit_correction_history_present")
        self.check(self.get(ca, a_ns, "Claim", foreign_ref) is None, "foreign_claim_id_hidden_in_own_namespace")
        self.check(self.get(ca, a_ns, "Event", saved["b"][0]) is None, "foreign_event_id_hidden_in_own_namespace")
        self.check(not ca.tool("get_reflection_history", {"namespace": a_ns, "claim_reference": foreign_ref})["reflections"], "foreign_history_hidden_in_own_namespace")
        self.deny(ca, "recall_memory", {"namespace": b_ns, "query": "SYNTHETIC_CORE_PROBE", "limit": 100})
        self.deny(ca, "get_memory", {"namespace": b_ns, "record_type": "Claim", "id": foreign_ref})
        self.deny(ca, "get_reflection_history", {"namespace": b_ns, "claim_reference": foreign_ref})
        self.deny(ca, "ingest_interaction", ingest_payload(b_ns, "FORBIDDEN_WRITE", "SYNTHETIC_DENIED_WRITE"))
        self.deny(cb, "recall_memory", {"namespace": a_ns, "query": "SYNTHETIC_CORE_PROBE", "limit": 100})
        before_crash = readonly_snapshot(self.database)
        self.check(before_crash["logical_counts"]["events"] == 16 and before_crash["logical_counts"]["claims"] == 16,
                   "only_expected_synthetic_records_before_crash")
        self.evidence["before_crash"] = before_crash
        self.crash(core)
        ca.close(); cb.close()
        core = self.core("core-after-crash", self.config)
        ca, cb = self.bridge(a, a_root), self.bridge(b, b_root)
        replay = ca.tool("ingest_interaction", first_payload, "intentional_replay_after_crash")
        self.check(replay["event_id"] == saved["a"][0] and replay.get("replayed") is True, "completed_write_replayed_after_core_termination_without_new_event")
        self.check(self.get(ca, a_ns, "Claim", old_ref)["status"] == "Superseded", "replay_did_not_resurrect_superseded_claim")
        self.check(self.get(ca, a_ns, "Claim", new_ref)["status"] == "Active", "correction_survived_core_crash")
        recalled_after_crash_b = self.recall(cb, b_ns)
        self.check(all(event + ":claim:0" in json.dumps(recalled_after_crash_b) for event in saved["b"]), "b_claims_survived_core_crash")
        after_replay = readonly_snapshot(self.database)
        self.check(after_replay["logical_counts"] == before_crash["logical_counts"], "replay_preserved_events_claims_reflections_and_receipt_counts")
        ca.close(); cb.close(); core.close()
        self.evidence["backup"] = backup_to_new_path(self.database, self.restored_database)
        self.check(True, "backup_to_new_path_readonly_integrity_counts_and_content_verified")
        write_mock_config(self.restored_config, self.restored_database)
        core = self.core("core-restored", self.restored_config)
        ca, cb = self.bridge(a, a_root), self.bridge(b, b_root)
        restored = self.get(ca, a_ns, "Claim", new_ref)
        self.check(restored["status"] == "Active" and json.loads(restored["object"]) == "SYNTHETIC_CORRECTED_A_00", "restored_core_recalled_corrected_value")
        evidence_ref = "event:" + correction_source["event_id"].removeprefix("event:")
        self.check(evidence_ref in restored["provenance"]["evidence_event_references"], "restored_correction_provenance_preserved")
        self.check(self.get(ca, a_ns, "Event", correction_source["event_id"]) is not None, "restored_evidence_event_retrievable")
        self.check(self.get(ca, a_ns, "Claim", old_ref)["status"] == "Superseded", "restored_old_claim_not_deleted")
        self.check(ca.tool("get_reflection_history", history_args) == history, "restored_correction_history_matches")
        self.check("SYNTHETIC_CANARY_A" not in json.dumps(self.recall(cb, b_ns)), "restored_namespace_canaries_isolated")
        self.deny(ca, "recall_memory", {"namespace": b_ns, "query": "SYNTHETIC_CORE_PROBE", "limit": 100})
        ca.close(); cb.close(); core.close()
        restored_snapshot = readonly_snapshot(self.restored_database)
        self.check(restored_snapshot["logical_counts"] == before_crash["logical_counts"], "restored_core_preserved_logical_record_counts")
        self.check(readonly_snapshot(self.database) == self.evidence["backup"]["source"], "restored_core_did_not_modify_source_database")
        self.evidence["restored_after_queries"] = restored_snapshot

    def cleanup(self):
        for client in reversed(self.clients):
            try:
                client.close()
            except (OSError, subprocess.TimeoutExpired):
                pass
        return self.processes.cleanup()


def prepare_output(output):
    output.mkdir(parents=True, mode=0o700)
    output.chmod(0o700)


def verify(binary, output, *, workflow_class=None, path_validator=None, output_initializer=None):
    binary, output = (path_validator or validate_paths)(binary, output)
    (output_initializer or prepare_output)(output)
    evidence = {"kind": "isolated_runtime_verification", "status": "running",
        "new_machine_verified": False, "natural_language_client_verified": False,
        "protocol_client": "stdlib_direct_mcp_stdio", "model_provider": "mock", "model_requests": 0,
        "credentials_loaded": False, "user_configuration_changed": False, "implicit_write_retries": 0,
        "planned_explicit_replays": 1, "crash_scenario": "after_acknowledged_writes",
        "in_flight_write_crash_verified": False, "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "processes": [], "readiness": [], "checks": [], "rpc_calls": []}
    workflow = (workflow_class or Workflow)(binary, output, evidence)
    try:
        workflow.run()
        evidence["binary_sha256_after"] = hashlib.sha256(binary.read_bytes()).hexdigest()
        require(evidence["binary_sha256_after"] == evidence["binary_sha256"], "candidate_binary_changed_during_verification")
        evidence["status"] = "passed"
    except (Exception, KeyboardInterrupt) as error:
        evidence["status"] = "failed"
        evidence["failure_code"] = str(error) if isinstance(error, VerificationError) else type(error).__name__
    finally:
        evidence["cleanup"] = workflow.cleanup()
        if not evidence["cleanup"]["passed"]:
            evidence["status"] = "failed"
            evidence.setdefault("failure_code", "test_process_cleanup_failed")
        evidence["mcp_tool_calls"] = sum(call["method"] == "tools/call" for call in evidence["rpc_calls"])
        evidence["explicit_replays_sent"] = sum(call["intent"] == "intentional_replay_after_crash" for call in evidence["rpc_calls"])
        write_json(output / "summary.json", evidence)
    return evidence


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        evidence = verify(args.binary, args.output)
    except (Exception, KeyboardInterrupt) as error:
        print(json.dumps({"status": "failed", "failure_code": str(error) if isinstance(error, VerificationError) else type(error).__name__,
                          "kind": "isolated_runtime_verification", "new_machine_verified": False, "model_requests": 0}))
        return 1
    print(json.dumps({key: evidence[key] for key in (
        "kind", "status", "new_machine_verified", "natural_language_client_verified", "model_requests",
        "implicit_write_retries", "explicit_replays_sent", "mcp_tool_calls", "cleanup")}, ensure_ascii=False))
    return 0 if evidence["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
