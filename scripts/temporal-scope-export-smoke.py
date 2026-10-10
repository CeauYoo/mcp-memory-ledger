#!/usr/bin/env python3
"""Provider-free schema6 temporal/history/export MCP smoke with synthetic data."""
import argparse
from contextlib import closing
from datetime import datetime
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("memory_smoke", Path(__file__).with_name("local-memory-smoke.py"))
smoke = importlib.util.module_from_spec(spec)
spec.loader.exec_module(smoke)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    if any(output.iterdir()):
        raise SystemExit("output must be empty")
    database, config = output / "memory.sqlite", output / "memory.toml"
    config.write_text('database_url = ' + json.dumps('sqlite://' + database.as_posix()) + '\n[model]\nprovider = "mock"\n', encoding="utf-8")
    env = {k: v for k, v in os.environ.items() if not k.startswith("AGENT_LLM_MM_")}
    env["AGENT_LLM_MM_CONFIG"] = str(config)
    subprocess.run([str(binary), "init"], env=env, check=True, capture_output=True)
    trace = []
    client = smoke.Client(binary, config, trace)
    try:
        observed = "2020-02-03T08:00:00.123456789+08:00"
        payload = {"request_id": "temporal-1", "observed_at": observed,
                   "event": {"owner": "World", "namespace": "project/a", "kind": "Observation", "summary": "北京 temporal observation"},
                   "claim_drafts": [{"owner": "World", "namespace": "project/a", "subject": "city", "predicate": "is", "object": "北京", "mode": "Observed"}]}
        first = client.tool("ingest_interaction", payload)
        replay = client.tool("ingest_interaction", payload)
        assert replay["event_id"] == first["event_id"] and replay["replayed"]
        event_ref = "event:" + first["event_id"]
        event = client.tool("get_memory", {"namespace": "project/a", "id": event_ref})["record"]
        claim = client.tool("get_memory", {"namespace": "project/a", "id": "claim:" + first["event_id"] + ":claim:0", "record_type": "Claim"})["record"]
        assert event["observed_at"] == claim["observed_at"] == observed
        assert event["recorded_at"] == claim["recorded_at"]
        assert datetime.fromisoformat(event["recorded_at"].replace("Z", "+00:00")) != datetime.fromisoformat(observed)
        reflection = client.tool("run_reflection", {"reflection": {"summary": "scoped record-only observation"}, "origin_namespace": "project/a", "replacement_evidence_event_ids": [event_ref]})
        own = client.tool("get_memory", {"namespace": "project/a", "record_type": "Reflection", "id": reflection["reflection_id"]})
        assert own["record"] and own["record"]["scope"]["status"] == "verified"
        foreign = client.tool("get_memory", {"namespace": "project/b", "record_type": "Reflection", "id": reflection["reflection_id"]})
        assert foreign["record"] is None
        for extra in [{"identity_update": {"canonical_claims": ["forbidden new global patch"]}}, {"commitment_updates": []}]:
            try:
                client.tool("run_reflection", {"reflection": {"summary": "denied"}, "origin_namespace": "project/a", "replacement_evidence_event_ids": [event_ref], **extra})
            except RuntimeError:
                pass
            else:
                raise AssertionError("targetless global update must fail")
        def count():
            with closing(sqlite3.connect(database)) as db, db:
                return db.execute("SELECT count(*) FROM operation_log").fetchone()[0]
        before = count()
        exported = client.tool("export_memory", {"namespace": "project/a", "max_bytes": 16384})
        assert count() == before
        assert exported["database_schema_version"] >= 6 and not exported["replayable_backup"]
        assert exported["snapshot_consistency"] == "single_read_transaction"
        assert exported["used_bytes"] == len(smoke.compact(exported).encode("utf-8")) <= 16384
        assert len(exported["events"]) == len(exported["claims"]) == len(exported["reflections"]) == 1
        assert exported["events"][0]["observed_at"] == observed
        try:
            client.tool("export_memory", {"namespace": "project/a", "max_bytes": 1024})
        except RuntimeError:
            pass
        else:
            raise AssertionError("bounded export must reject oversize document")
        assert count() == before, "failed export must not append diagnostics"
        # Synthetic import rows make equal-time ties deterministic and verify
        # that the MCP Runtime forwards union-specific ordering before LIMIT.
        with closing(sqlite3.connect(database)) as db, db:
            for suffix in ("z", "a", "m"):
                db.execute("INSERT INTO events(event_id,recorded_at,owner,namespace,kind,summary) VALUES (?,?,'world','project/order','observation','tie')", ("order-" + suffix, "2026-01-01T00:00:00Z"))
        browse = client.tool("search_memory", {"namespace": "project/order", "record_type": "Event", "limit": 1})
        union = client.tool("search_memory", {"namespace": "project/order", "record_types": ["Event", "Claim"], "limit": 1})
        assert browse["records"][0]["id"] == "event:order-m"
        assert union["records"][0]["id"] == "event:order-z"
        (output / "export.json").write_text(json.dumps(exported, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    finally:
        client.close()
    (output / "transcript.json").write_text(json.dumps(trace, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    summary = {"status": "passed", "kind": "synthetic_temporal_scope_export", "provider": "mock", "remote_model_calls": 0, "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "checks": ["observed_vs_recorded", "immutable_replay", "targetless_scoped_history", "no_targetless_global_patch", "readonly_bounded_export", "union_prelimit_ordering"]}
    (output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
    print(smoke.compact(summary))


if __name__ == "__main__":
    main()
