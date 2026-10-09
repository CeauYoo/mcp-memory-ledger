#!/usr/bin/env python3
"""Fixed offline MCP retrieval/correction diagnostics; never invokes a real model."""
import argparse
from datetime import datetime, timezone
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import sys
sys.dont_write_bytecode = True
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("memory_smoke", ROOT / "scripts/local-memory-smoke.py")
smoke = importlib.util.module_from_spec(spec)
spec.loader.exec_module(smoke)
compact = smoke.compact


def write_json(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def prepare(binary, directory):
    db = directory / "memory.sqlite"
    config = directory / "memory.toml"
    config.write_text('database_url = ' + json.dumps('sqlite://' + db.as_posix()) +
                      '\n[model]\nprovider = "mock"\n', encoding="utf-8")
    env = {k: v for k, v in os.environ.items() if not k.startswith("AGENT_LLM_MM_")}
    env["AGENT_LLM_MM_CONFIG"] = str(config)
    subprocess.run([str(binary), "init"], env=env, capture_output=True, check=True)
    return db, config


def claim(task, value, namespace=None):
    return {"owner": "World", "namespace": namespace or task["namespace"],
            "subject": task["subject"], "predicate": "is", "object": value, "mode": "Observed"}


def seed(client, task):
    result = client.tool("ingest_interaction", {"request_id": task["id"], "event": {
        "owner": "World", "namespace": task["namespace"], "kind": "Observation",
        "summary": task["subject"] + " initial observation " + task["before"]},
        "claim_drafts": [claim(task, task["before"])], "episode_reference": "episode:" + task["id"]})
    # Same lexical content across a scope boundary is an intentional isolation trap.
    foreign = "project/eval-foreign"
    client.tool("ingest_interaction", {"request_id": task["id"], "event": {
        "owner": "World", "namespace": foreign, "kind": "Observation",
        "summary": task["subject"] + " private foreign result"},
        "claim_drafts": [claim(task, "FOREIGN", foreign)], "episode_reference": "episode:foreign"})
    return "claim:" + result["event_id"] + ":claim:0", result["event_id"]


def observe_feedback(client, task, event, target, version):
    feedback = {"source_kind": "tool_reported", "producer": "fixed-offline-fixture-v1",
        "observed_target": target, "observed_version": version,
        "expected": task["before"], "actual": task["after"],
        "verification_method": "synthetic exact-value fixture (not authenticated external reality)",
        "verification_result": "failed", "limitations": [],
        "evidence_refs": []}
    observation = client.tool("ingest_interaction", {"request_id": "feedback-" + task["id"],
        "event": {"owner": "World", "namespace": task["namespace"], "kind": "Observation",
                  "summary": task["subject"] + " corrected observation " + task["after"], "feedback": feedback},
        "claim_drafts": [], "episode_reference": "episode:feedback-" + task["id"]})
    return observation["event_id"]


def correct(client, task, old, feedback_id, version, legacy=False):
    if legacy:
        payload = {"request_id": "correction-" + task["id"], "namespace": task["namespace"],
            "claim_reference": old, "replacement_claim": claim(task, task["after"]),
            "replacement_evidence_event_ids": [feedback_id], "summary": "Fixed synthetic external-feedback correction"}
        result = client.tool("supersede_memory", payload)
        assert client.tool("supersede_memory", payload) == result
    else:
        proposed = client.tool("propose_feedback_candidate", {"namespace": task["namespace"],
            "target_claim_reference": old, "expected_target_version": version,
            "replacement_object": task["after"], "evidence_event_ids": [feedback_id],
            "summary": "Fixed synthetic external-feedback correction", "request_id": "propose-" + task["id"]})
        action = {"namespace": task["namespace"], "candidate_id": proposed["candidate_id"], "request_id": "validate-" + task["id"]}
        validated = client.tool("validate_feedback_candidate", action)
        assert validated["state"] == "validated" and validated["validation"]["passed"], validated
        action["request_id"] = "commit-" + task["id"]
        result = client.tool("commit_feedback_candidate", action)
        assert result["state"] == "committed"
        assert client.tool("commit_feedback_candidate", action) == result, "commit replay must be idempotent"
    historical = client.tool("get_memory", {"namespace": task["namespace"], "record_type": "Claim", "id": old})
    assert historical["record"]["status"] == "Superseded", "old claim must remain inspectable and terminal"
    return feedback_id


def blocked_feedback_workflow(client, task, old, version):
    results = {}
    for negative in ("missing_evidence", "limited_evidence"):
        evidence = []
        if negative == "limited_evidence":
            observed = client.tool("ingest_interaction", {"request_id": "limited-observation", "event": {
                "owner": "World", "namespace": task["namespace"], "kind": "Observation", "summary": "Limited synthetic observation",
                "feedback": {"source_kind": "tool_reported", "producer": "fixed-offline-fixture-v1", "observed_target": old,
                    "observed_version": version, "expected": task["before"], "actual": task["after"],
                    "verification_method": "synthetic exact-value fixture", "verification_result": "failed",
                    "limitations": ["Only applies to a different environment"], "evidence_refs": []}},
                "claim_drafts": [], "episode_reference": "episode:limited-feedback"})
            evidence = [observed["event_id"]]
        candidate = client.tool("propose_feedback_candidate", {"namespace": task["namespace"],
            "target_claim_reference": old, "expected_target_version": version,
            "replacement_object": task["after"], "evidence_event_ids": evidence,
            "summary": "Deliberately unsupported fixture proposal", "request_id": negative + "-proposal"})
        action = {"namespace": task["namespace"], "candidate_id": candidate["candidate_id"], "request_id": negative + "-validate"}
        blocked = client.tool("validate_feedback_candidate", action)
        assert blocked["state"] == "blocked" and not blocked["validation"]["passed"], blocked
        action["request_id"] = negative + "-commit"
        try:
            denied = client.request("tools/call", {"name": "commit_feedback_candidate", "arguments": action})
            assert denied.get("isError"), "unsupported candidate must not commit"
        except RuntimeError as error:
            assert "validated" in str(error) and "-32602" in str(error), str(error)
        rejected = client.tool("reject_feedback_candidate", dict(action, request_id=negative + "-reject", reason="Unsupported evidence"))
        assert rejected["state"] == "rejected"
        unchanged = client.tool("get_feedback_target_version", {"namespace": task["namespace"], "target_claim_reference": old})
        assert unchanged["target_version"] == version
        results[negative] = {"blocked": True, "commit_denied": True, "rejected": True, "target_unchanged": True,
                             "validation_codes": [r["code"] for r in blocked["validation"]["reasons"]]}
    return results


def experience_workflow(client, task, feedback_id):
    ns = task["namespace"]
    episode_id = "episode:rich-feedback-evaluation"
    episode = {"namespace": ns, "request_id": "record-rich-episode", "episode_id": episode_id,
        "content": {"title": "Evaluation correction episode", "objective": "Correct the observed deployment city",
            "actions": ["Inspect fixture report", "Validate candidate", "Commit correction"],
            "observations": ["Synthetic external fixture disagreed with initial Claim"],
            "outcome": "Corrected Claim recalled", "lesson": "Check exact target version before a correction",
            "limitations": ["Synthetic evidence, no general semantic guarantee"], "source_event_refs": [feedback_id]}}
    created = client.tool("record_episode", episode)
    replay = client.tool("record_episode", episode)
    assert replay["record"] == created["record"] and replay["replayed"]
    outcomes = {}
    for kind in ("semantic", "procedural"):
        cid = "experience:evaluation-" + kind
        content = {"kind": kind, "title": "fixtureknowledge " + kind,
            "statement": "Check an exact Claim version before correction", "steps": [] if kind == "semantic" else ["Inspect target", "Check evidence", "Request explicit correction"],
            "limitations": ["Inert knowledge, no execution authority"], "source_episode_ids": [episode_id]}
        current = client.tool("propose_experience_candidate", {"namespace": ns, "request_id": "experience-propose-" + kind, "candidate_id": cid, "content": content})["record"]
        assert current["status"] == "pending"
        def recall():
            result = client.tool("recall_experience_candidates", {"namespace": ns, "query": "fixtureknowledge", "max_bytes": 4096})
            assert len(compact(result).encode()) == result["used_bytes"] <= 4096
            return result["candidates"]
        assert cid not in {r["candidate_id"] for r in recall()}
        current = client.tool("set_experience_candidate_status", {"namespace": ns, "request_id": "activate-" + kind,
            "candidate_id": cid, "expected_version": current["version"], "status": "active"})["record"]
        active_version = current["version"]
        assert cid in {r["candidate_id"] for r in recall()}
        revised = dict(content, statement="Revised knowledge remains pending until reviewed")
        current = client.tool("revise_experience_candidate", {"namespace": ns, "request_id": "revise-" + kind,
            "candidate_id": cid, "expected_version": current["version"], "content": revised})["record"]
        assert current["status"] == "pending" and cid not in {r["candidate_id"] for r in recall()}
        current = client.tool("set_experience_candidate_status", {"namespace": ns, "request_id": "reject-" + kind,
            "candidate_id": cid, "expected_version": current["version"], "status": "rejected"})["record"]
        current = client.tool("rollback_experience_candidate", {"namespace": ns, "request_id": "rollback-" + kind,
            "candidate_id": cid, "expected_version": current["version"], "target_version": active_version})["record"]
        assert current["status"] == "pending" and current["content"] == content
        assert cid not in {r["candidate_id"] for r in recall()}
        historical = client.tool("get_experience_candidate", {"namespace": ns, "candidate_id": cid, "version": active_version})
        assert historical["status"] == "active"
        outcomes[kind] = {"final_version": current["version"], "final_status": current["status"], "historical_active_preserved": True}
    return outcomes


def metrics(task, records, relevant_ids, stale_ids, elapsed_ms, context_bytes, k):
    ids = [x["record"]["id"] for x in records[:k]]
    relevant = set(relevant_ids)
    hits = [i + 1 for i, rid in enumerate(ids) if rid in relevant]
    claims = [x["record"] for x in records if "object" in x["record"]]
    # A deliberately weak, fully specified deterministic selector, NOT model task success.
    selected = next((x for x in claims if x["subject"] == task["subject"]), None)
    return {"task_id": task["id"], "language": task["language"], "stratum": task["stratum"],
        "recall_at_k": len(set(ids) & relevant) / len(relevant) if relevant else None,
        "reciprocal_rank": 1 / hits[0] if hits else 0,
        "stale_claim_exposure_count": sum(x in stale_ids for x in ids),
        "stale_conclusion_used_proxy": bool(selected and selected["object"] == task["before"]),
        "exact_answer_proxy": bool(selected and selected["object"] == task["after"]),
        "scope_leak_count": sum(x["record"]["namespace"] != task["namespace"] for x in records),
        "provenance_missing_count": sum(not x["record"].get("provenance") for x in records),
        "context_bytes": context_bytes, "observed_latency_ms": elapsed_ms,
        "returned_ids": ids, "selected_answer": selected["object"] if selected else None,
        "model_task_success": None, "token_usage": None, "token_cost": None}


def evaluate(client, tasks, ids, variant, k, budget):
    rows = []
    for task in tasks:
        started = time.perf_counter()
        if variant in ("A_no_memory", "C_minus_retrieval"):
            result = {"records": []}
            size = 0
        else:
            result = client.tool("build_task_context", {"namespace": task["namespace"],
                "query": task["query"], "limit": k,
                "max_bytes": 262144 if variant == "C_minus_byte_budget" else budget})
            size = len(compact(result).encode("utf-8"))
            assert size == result["serialized_bytes"]
            assert size <= (262144 if variant == "C_minus_byte_budget" else budget)
        old, new = ids[task["id"]]
        corrected = variant.startswith("C") and variant != "C_minus_correction"
        gold = [new if corrected else old]
        row = metrics(task, result["records"], gold, {old},
                      (time.perf_counter() - started) * 1000, size, k)
        row["variant"] = variant
        row["context"] = result
        rows.append(row)
    return rows


def query_scenarios(client, tasks, ids, fixtures):
    by_task = {task["id"]: task for task in tasks}
    rows = []
    for case in fixtures["cases"]:
        task = by_task[case["task_id"]]
        started = time.perf_counter()
        result = client.tool("recall_memory", {"namespace": task["namespace"], "query": case["query"], "limit": 5})
        records = result["records"]
        returned = [r["record"]["id"] for r in records]
        gold = ids[task["id"]][1]
        actual_hit = gold in returned
        assert all(r["record"]["namespace"] == task["namespace"] for r in records)
        assert actual_hit == case["expected_hit"], (case, returned)
        if not case["expected_hit"]:
            assert not records, (case, returned)
        rows.append(dict(case, actual_hit=actual_hit, returned_ids=returned,
            reciprocal_rank=1 / (returned.index(gold) + 1) if actual_hit else 0,
            observed_latency_ms=(time.perf_counter() - started) * 1000,
            response_bytes=len(compact(result).encode())))
    return rows


def summarize(rows):
    result = {}
    for variant in sorted({row["variant"] for row in rows}):
        selected = [r for r in rows if r["variant"] == variant]
        result[variant] = {"task_count": len(selected)}
        for metric in ("recall_at_k", "reciprocal_rank", "stale_conclusion_used_proxy", "exact_answer_proxy",
                       "stale_claim_exposure_count", "scope_leak_count", "provenance_missing_count", "context_bytes", "observed_latency_ms"):
            result[variant]["mean_" + metric] = statistics.mean(r[metric] for r in selected)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--legacy-supersede", action="store_true", help="Explicit historical-binary compatibility; skips candidate/E lifecycle checks")
    parser.add_argument("--query-scenarios", type=Path, default=ROOT / "tests/fixtures/memory-evaluation/query-scenarios-v1.json")
    parser.add_argument("--fixtures", type=Path, default=ROOT / "tests/fixtures/memory-evaluation/tasks.json")
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    args.output.mkdir(parents=True, exist_ok=True)
    if any(args.output.iterdir()):
        parser.error("output must be empty; evidence is never overwritten")
    raw = args.fixtures.read_bytes()
    fixtures = json.loads(raw)
    tasks = fixtures["tasks"]
    assert len({t["id"] for t in tasks}) == len(tasks)
    query_raw = args.query_scenarios.read_bytes()
    query_fixtures = json.loads(query_raw)
    if not {case["task_id"] for case in query_fixtures["cases"]} <= {task["id"] for task in tasks}:
        parser.error("query scenarios reference tasks absent from --fixtures")
    evidence, rows, ids = [], [], {}
    with tempfile.TemporaryDirectory(prefix="ledger-evaluation-") as temporary:
        _, config = prepare(binary, Path(temporary))
        client = smoke.Client(binary, config, evidence)
        try:
            seeds = {task["id"]: seed(client, task) for task in tasks}
            ids = {key: (value[0], value[0]) for key, value in seeds.items()}
            for variant in ("A_no_memory", "B_recall"):
                rows.extend(evaluate(client, tasks, ids, variant, fixtures["k"], fixtures["max_context_bytes"]))
            versions = {task["id"]: ("legacy-fixture" if args.legacy_supersede else client.tool("get_feedback_target_version", {"namespace": task["namespace"], "target_claim_reference": seeds[task["id"]][0]})["target_version"]) for task in tasks}
            rejection = None if args.legacy_supersede else blocked_feedback_workflow(client, tasks[0], seeds[tasks[0]["id"]][0], versions[tasks[0]["id"]])
            feedback_ids = {task["id"]: observe_feedback(client, task, seeds[task["id"]][1], seeds[task["id"]][0], versions[task["id"]]) for task in tasks}
            rows.extend(evaluate(client, tasks, ids, "C_minus_correction", fixtures["k"], fixtures["max_context_bytes"]))
            for task in tasks:
                old, event = seeds[task["id"]]
                feedback = correct(client, task, old, feedback_ids[task["id"]], versions[task["id"]], args.legacy_supersede)
                # Resolve new IDs from claim/evidence, independent of generated UUID spelling.
                results = client.tool("recall_memory", {"namespace": task["namespace"], "query": task["subject"], "limit": 100})
                replacements = [r["record"] for r in results["records"] if r["record"].get("subject") == task["subject"]
                                and r["record"].get("object") == task["after"]]
                assert len(replacements) == 1
                assert "event:" + feedback in replacements[0]["provenance"]["evidence_event_references"]
                ids[task["id"]] = (old, replacements[0]["id"])
            for variant in ("C_feedback_corrected", "C_minus_retrieval", "C_minus_byte_budget"):
                rows.extend(evaluate(client, tasks, ids, variant, fixtures["k"], fixtures["max_context_bytes"]))
            scenarios = query_scenarios(client, tasks, ids, query_fixtures)
            experience = None if args.legacy_supersede else experience_workflow(client, tasks[0], feedback_ids[tasks[0]["id"]])
        finally:
            client.close()
    assert all(r["scope_leak_count"] == 0 and r["provenance_missing_count"] == 0 for r in rows)
    report = {"kind": "deterministic_offline_proxy", "recorded_at_utc": datetime.now(timezone.utc).isoformat(), "remote_model_calls": 0,
        "same_model_experiment": False, "correction_path": "legacy_supersede" if args.legacy_supersede else "feedback_candidate", "experience_lifecycle": experience, "unsupported_feedback_checks": rejection, "token_cost_evidence": False, "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "fixture_sha256": hashlib.sha256(raw).hexdigest(), "query_scenario_sha256": hashlib.sha256(query_raw).hexdigest(),
        "query_scenario_count": len(scenarios), "query_scenario_selection": query_fixtures["selection"],
        "query_scenario_categories": {category: {"cases": len(group), "expected_hits": sum(r["expected_hit"] for r in group),
            "actual_hits": sum(r["actual_hit"] for r in group), "negative_false_positives": sum(bool(r["returned_ids"]) for r in group if not r["expected_hit"]),
            "mean_reciprocal_rank_positive_cases": statistics.mean(r["reciprocal_rank"] for r in group if r["expected_hit"]) if any(r["expected_hit"] for r in group) else None}
            for category in sorted({r["category"] for r in scenarios})
            for group in [[r for r in scenarios if r["category"] == category]]}, "platform": platform.platform(),
        "selection": fixtures["selection"], "k": fixtures["k"], "max_context_bytes": fixtures["max_context_bytes"],
        "summary": summarize(rows), "by_stratum": {s: summarize([r for r in rows if r["stratum"] == s]) for s in {t["stratum"] for t in tasks}},
        "limitations": ["Synthetic preselected fixtures; not real task distribution or model gains.",
            "Correction is supplied by harness; this run does not evaluate a model producing a justified correction.",
            "C_minus_correction has feedback observations but retains old claims; C_minus_retrieval removes recall; C_minus_byte_budget uses the API maximum, not unlimited memory.",
            "Timing includes local MCP round trip, no model inference; bytes are not tokens.",
            "A uses an empty-context deterministic selector and therefore cannot measure a model's prior knowledge."]}
    write_json(args.output / "report.json", report)
    write_json(args.output / "results.json", rows)
    write_json(args.output / "query-scenarios.json", scenarios)
    write_json(args.output / "mcp-transcript.json", evidence)
    requests = [{"schema_version": 1, "task_id": r["task_id"], "variant": r["variant"],
                 "fixture_sha256": report["fixture_sha256"], "task_prompt": next(t["prompt"] for t in tasks if t["id"] == r["task_id"]),
                 "context": r["context"], "network_execution_authorized": False} for r in rows]
    write_json(args.output / "adapter-requests.json", requests)
    print(compact(report))


if __name__ == "__main__":
    main()
