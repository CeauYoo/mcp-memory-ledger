#!/usr/bin/env python3
"""Offline installed-binary MCP workflow. Uses only Python's standard library.
This is a local installation simulation, not fresh-machine or release approval.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import queue
import shutil
import sqlite3
import subprocess
import threading
import time


def compact(value):
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"))


class Client:
    def __init__(self, binary, config, evidence):
        self.evidence = evidence
        env = {k: v for k, v in os.environ.items() if not k.startswith("AGENT_LLM_MM_")}
        env["AGENT_LLM_MM_CONFIG"] = str(config)
        # Explicit config environment avoids caller configuration and working-directory defaults.
        self.log = open(config.parent / (config.stem + ".stderr.log"), "a", encoding="utf-8")
        self.proc = subprocess.Popen([str(binary), "serve"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.log,
            text=True, encoding="utf-8", env=env)
        self.lines = queue.Queue()
        def reader():
            for line in self.proc.stdout:
                self.lines.put(line)
            self.lines.put(None)
        threading.Thread(target=reader, daemon=True).start()
        self.counter = 0
        self.request("initialize", {"protocolVersion": "2025-03-26", "capabilities": {},
                     "clientInfo": {"name": "ledger-offline-smoke", "version": "1"}})
        self.send({"jsonrpc": "2.0", "method": "notifications/initialized"})

    def send(self, message):
        self.proc.stdin.write(compact(message) + "\n")
        self.proc.stdin.flush()

    def request(self, method, params):
        self.counter += 1
        request = {"jsonrpc": "2.0", "id": self.counter, "method": method, "params": params}
        self.send(request)
        deadline = time.monotonic() + 30
        while True:
            line = self.lines.get(timeout=max(0.01, deadline - time.monotonic()))
            if line is None:
                raise RuntimeError("MCP process closed before reply; inspect local stderr log")
            reply = json.loads(line)  # Non-JSON stdout contamination fails the test.
            if reply.get("id") == self.counter:
                break
        self.evidence.append({"request": request, "response": reply})
        if "error" in reply:
            raise RuntimeError(compact(reply["error"]))
        return reply["result"]

    def tool(self, name, arguments):
        result = self.request("tools/call", {"name": name, "arguments": arguments})
        if result.get("isError"):
            raise RuntimeError(compact(result))
        return result["structuredContent"]

    def close(self):
        self.proc.stdin.close()
        try:
            self.proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait()
        self.log.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    source = args.binary.resolve(strict=True)
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    if any(output.iterdir()):
        raise SystemExit("output directory must be empty (will never overwrite existing evidence)")
    start = time.monotonic()
    installed = output / ("agent_llm_mm.exe" if os.name == "nt" else "agent_llm_mm")
    shutil.copy2(source, installed)
    database = output / "memory.sqlite"
    config = output / "memory.toml"
    def write_config(path):
        config.write_text('database_url = ' + json.dumps('sqlite://' + path.as_posix()) +
                          '\n[model]\nprovider = "mock"\n', encoding="utf-8")
    write_config(database)
    env = {k: v for k, v in os.environ.items() if not k.startswith("AGENT_LLM_MM_")}
    env["AGENT_LLM_MM_CONFIG"] = str(config)
    init = subprocess.run([str(installed), "init"], env=env,
                          capture_output=True, text=True, encoding="utf-8", check=True)
    (output / "init.json").write_text(init.stdout, encoding="utf-8")
    evidence = []
    client = Client(installed, config, evidence)
    try:
        names = {tool["name"] for tool in client.request("tools/list", {})["tools"]}
        assert {"recall_memory", "build_task_context", "supersede_memory"} <= names
        event_id = None
        for namespace, count in [("project/a", 10), ("project/b", 5), ("project/c", 5)]:
            for index in range(count):
                claims = [{"owner": "World", "namespace": namespace, "subject": "city",
                           "predicate": "is", "object": "北京", "mode": "Observed"}] if index == 0 else []
                payload = {"request_id": f"{namespace}-{index}", "event": {"owner": "World",
                           "namespace": namespace, "kind": "Observation", "summary": f"北京咖啡记忆 {index}"},
                           "claim_drafts": claims, "episode_reference": f"episode:{namespace}"}
                result = client.tool("ingest_interaction", payload)
                if namespace == "project/a" and index == 0:
                    event_id = result["event_id"]
                    replay = client.tool("ingest_interaction", payload)
                    assert replay["event_id"] == event_id and replay["replayed"]
    finally:
        client.close()
    client = Client(installed, config, evidence)
    try:
        for query in ["北京", "咖啡", "记忆"]:
            recalled = client.tool("recall_memory", {"namespace": "project/a", "query": query})
            assert recalled["records"]
            assert all(x["record"]["namespace"] == "project/a" for x in recalled["records"])
        old = event_id + ":claim:0"
        record = client.tool("get_memory", {"namespace": "project/a", "record_type": "Claim", "id": old})
        assert record["record"]["provenance"]["evidence_event_references"]
        feedback = {"source_kind": "tool_reported", "producer": "offline-smoke-fixture",
                    "observed_target": "city", "observed_version": "1", "expected": "北京", "actual": "上海",
                    "verification_method": "synthetic deterministic fixture", "verification_result": "failed",
                    "limitations": ["Caller-supplied synthetic observation, not authenticated truth"],
                    "evidence_refs": [event_id]}
        observed = client.tool("ingest_interaction", {"request_id": "city-feedback", "event": {
            "owner": "World", "namespace": "project/a", "kind": "Observation", "summary": "city correction 上海",
            "feedback": feedback}, "claim_drafts": [], "episode_reference": "episode:feedback"})
        observation_id = observed["event_id"]
        inspected = client.tool("get_memory", {"namespace": "project/a", "id": observation_id})
        assert inspected["record"]["feedback"]["source_kind"] == "tool_reported"
        payload = {"request_id": "correct-city", "namespace": "project/a", "claim_reference": old,
                   "replacement_claim": {"owner": "World", "namespace": "project/a", "subject": "city",
                   "predicate": "is", "object": "上海", "mode": "Observed"},
                   "replacement_evidence_event_ids": [observation_id], "summary": "Correct city with explicit evidence"}
        correction = client.tool("supersede_memory", payload)
        assert client.tool("supersede_memory", payload) == correction
        history = client.tool("get_reflection_history", {"namespace": "project/a", "claim_reference": old})
        assert history["reflections"]
        current = client.tool("recall_memory", {"namespace": "project/a", "query": "上海"})
        assert current["records"][0]["record"]["status"] == "Active"
        assert current["records"][0]["record"]["object"] == "上海"
        context = client.tool("build_task_context", {"namespace": "project/a", "query": "上海 北京", "max_bytes": 2048})
        assert len(compact(context).encode("utf-8")) == context["serialized_bytes"] <= 2048
    finally:
        client.close()
    backup = output / "backup.sqlite"
    restored = output / "restored.sqlite"
    with sqlite3.connect(database) as src, sqlite3.connect(backup) as dst:
        src.backup(dst)
    # Separate, never-existing restore target; no live path switching or overwriting.
    with sqlite3.connect(backup) as src, sqlite3.connect(restored) as dst:
        src.backup(dst)
    with sqlite3.connect(database) as src, sqlite3.connect(restored) as dst:
        assert list(src.iterdump()) == list(dst.iterdump())
        assert dst.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
        assert dst.execute("PRAGMA foreign_key_check").fetchall() == []
    write_config(restored)
    doctor = subprocess.run([str(installed), "doctor", "--read-only"], env=env,
                            capture_output=True, text=True, encoding="utf-8", check=True)
    (output / "doctor-restored.json").write_text(doctor.stdout, encoding="utf-8")
    client = Client(installed, config, evidence)
    try:
        after = client.tool("recall_memory", {"namespace": "project/a", "query": "上海"})
        assert after == current
    finally:
        client.close()
    (output / "mcp-transcript.json").write_text(json.dumps(evidence, ensure_ascii=False, indent=2), encoding="utf-8")
    summary = {"status": "passed", "kind": "installed_binary_local_simulation", "provider": "mock",
               "remote_model_calls": 0, "real_fresh_machine_evidence": False,
               "binary_sha256": hashlib.sha256(installed.read_bytes()).hexdigest(),
               "duration_seconds": round(time.monotonic() - start, 3), "scope_leaks": 0,
               "restart": True, "correction_replay": True, "backup_restore": True}
    (output / "summary.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
    print(json.dumps(summary))


if __name__ == "__main__":
    main()
