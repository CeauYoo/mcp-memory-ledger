#!/usr/bin/env python3
"""Safe temporary-database MCP capacity observations; no performance pass threshold."""
import argparse
from datetime import datetime, timezone
import concurrent.futures
import hashlib
import importlib.util
import json
import math
from pathlib import Path
import platform
import sqlite3
import statistics
import sys
import tempfile
import time
sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("evaluation", Path(__file__).with_name("evaluate-memory-loop.py"))
evaluation = importlib.util.module_from_spec(spec)
spec.loader.exec_module(evaluation)


def percentile(values, q):
    return sorted(values)[max(0, math.ceil(len(values) * q) - 1)]


def describe(values):
    return {"samples": len(values), "p50": statistics.median(values), "p95": percentile(values, .95),
            "min": min(values), "max": max(values)}


def rss(pid):
    try:
        return int(next(line.split()[1] for line in Path(f"/proc/{pid}/status").read_text().splitlines()
                        if line.startswith("VmRSS:"))) * 1024
    except (OSError, StopIteration):
        return None


def seed(db, size):
    """Exactly size event+claim records, 20 scopes; 100 revisions form one real history chain."""
    pairs = size // 2
    started = time.perf_counter()
    digest = hashlib.sha256()
    with sqlite3.connect(db) as connection:
        connection.execute("PRAGMA foreign_keys=ON")
        for i in range(pairs):
            ns = "project/capacity-00" if i < 100 else f"project/capacity-{i % 20:02d}"
            subject = "revision_chain" if i < 100 else f"target_{i:06d}"
            summary = f"{subject} 北京咖啡 capacity observation {i}"
            event = (f"bench-event-{i:06d}", "2026-01-01T00:00:00Z", "world", ns, "observation", summary)
            claim = (f"bench-claim-{i:06d}", "world", ns, subject, "is", str(i), "observed",
                     "superseded" if i < 99 else "active")
            connection.execute("INSERT INTO events(event_id,recorded_at,owner,namespace,kind,summary) VALUES (?,?,?,?,?,?)", event)
            connection.execute("INSERT INTO claims(claim_id,owner,namespace,subject,predicate,object,mode,status) VALUES (?,?,?,?,?,?,?,?)", claim)
            connection.execute("INSERT INTO evidence_links(claim_id,event_id) VALUES (?,?)", (claim[0], event[0]))
            connection.execute("INSERT INTO episode_events(episode_reference,event_id) VALUES (?,?)", (f"episode:bench-{i // 100}", event[0]))
            digest.update(evaluation.compact([event, claim]).encode())
            if 0 < i < 100:
                connection.execute("INSERT INTO reflections(reflection_id,recorded_at,summary,superseded_claim_id,replacement_claim_id,supporting_evidence_event_ids) VALUES (?,?,?,?,?,?)",
                    (f"bench-reflection-{i:06d}", event[1], "Synthetic revision chain", f"bench-claim-{i-1:06d}", claim[0], json.dumps([event[0]])))
        assert connection.execute("PRAGMA foreign_key_check").fetchall() == []
    return {"event_claim_records": pairs * 2, "namespaces": 20, "revision_chain_claims": 100,
            "seed_seconds": time.perf_counter() - started, "data_sha256": digest.hexdigest(),
            "writes": "Direct synthetic bulk SQL, not application write-throughput evidence"}


def plans(db, term):
    """Diagnostic primitive plans; not claimed to be the complete application SQL."""
    queries = [("literal_claim_primitive", "SELECT claim_id FROM claims WHERE owner='world' AND namespace=? AND status='active' AND instr(lower(subject),?)>0", ("project/capacity-00", term))]
    with sqlite3.connect(db) as connection:
        if connection.execute("SELECT 1 FROM sqlite_master WHERE name='text_recall_fts'").fetchone():
            queries.append(("fts_candidate_primitive", "SELECT d.record_id FROM text_recall_fts JOIN text_recall_documents d ON d.doc_id=text_recall_fts.rowid WHERE text_recall_fts MATCH ? AND d.owner='world' AND d.namespace=?", ('"' + term + '"', "project/capacity-00")))
        return [{"name": name, "sql": sql, "parameters": args,
                 "plan": connection.execute("EXPLAIN QUERY PLAN " + sql, args).fetchall()} for name, sql, args in queries]


def run_size(binary, size, repeats, directory):
    db, config = evaluation.prepare(binary, directory)
    dataset = seed(db, size)
    target = ((size // 2 - 1) // 20) * 20
    queries = {"selective_ascii": f"target_{target:06d}", "common_short_cjk": "北京", "revision_chain": "revision_chain", "absent": "missing_needle"}
    transcript = []
    client = evaluation.smoke.Client(binary, config, transcript)
    report = {"dataset": dataset, "database_bytes_before_reads": db.stat().st_size, "queries": {}}
    try:
        for name, query in queries.items():
            times, sizes = [], []
            for _ in range(2):  # explicitly discarded warmups
                client.tool("recall_memory", {"namespace": "project/capacity-00", "query": query, "limit": 20})
            for _ in range(repeats):
                started = time.perf_counter()
                result = client.tool("recall_memory", {"namespace": "project/capacity-00", "query": query, "limit": 20})
                times.append((time.perf_counter() - started) * 1000)
                sizes.append(len(evaluation.compact(result).encode()))
                assert all(r["record"]["namespace"] == "project/capacity-00" for r in result["records"])
                assert all(r["record"].get("status", "Active") == "Active" for r in result["records"])
            report["queries"][name] = {"query": query, "mcp_latency_ms": describe(times),
                "response_bytes": describe(sizes), "strategy": result["strategy"], "returned_records": len(result["records"]),
                "observed_server_rss_bytes": rss(client.proc.pid)}
        report["query_plans"] = plans(db, queries["selective_ascii"])
        # Three independent MCP connections to the same temporary DB: read, writer, dashboard-like browse.
        writer = evaluation.smoke.Client(binary, config, [])
        browse = evaluation.smoke.Client(binary, config, [])
        try:
            def ingest(i):
                started = time.perf_counter()
                writer.tool("ingest_interaction", {"request_id": f"concurrent-{i}", "event": {
                    "owner": "World", "namespace": "project/capacity-00", "kind": "Observation",
                    "summary": f"concurrent observation {i}"}, "claim_drafts": [], "episode_reference": "episode:concurrent"})
                return (time.perf_counter() - started) * 1000
            def reader():
                started = time.perf_counter()
                client.tool("recall_memory", {"namespace": "project/capacity-00", "query": "北京", "limit": 20})
                return (time.perf_counter() - started) * 1000
            def dashboard():
                started = time.perf_counter()
                browse.tool("search_memory", {"namespace": "project/capacity-00", "record_types": ["Event", "Claim"], "limit": 20})
                return (time.perf_counter() - started) * 1000
            concurrent_samples = {"write": [], "recall": [], "dashboard_like_browse": []}
            with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:
                for i in range(10):
                    futures = [pool.submit(ingest, i), pool.submit(reader), pool.submit(dashboard)]
                    for label, future in zip(concurrent_samples, futures):
                        concurrent_samples[label].append(future.result())
            report["concurrent_mcp_latency_ms"] = {k: describe(v) for k, v in concurrent_samples.items()}
            # Intentional bounded external writer reservation. This is observed request delay, not internal lock telemetry.
            with sqlite3.connect(db) as lock:
                lock.execute("BEGIN IMMEDIATE")
                with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
                    future = pool.submit(ingest, 999)
                    started = time.perf_counter()
                    time.sleep(.05)
                    lock.rollback()
                    held_ms = (time.perf_counter() - started) * 1000
                    try:
                        report["lock_probe"] = {"held_reservation_ms": held_ms, "write_request_ms": future.result(), "result": "committed"}
                    except Exception as error:
                        report["lock_probe"] = {"held_reservation_ms": held_ms, "result": "rejected", "error": str(error)}
        finally:
            writer.close()
            browse.close()
        report["database_bytes_after_reads_and_writes"] = sum(p.stat().st_size for p in directory.glob("memory.sqlite*"))
        report["observed_server_rss_bytes_after"] = rss(client.proc.pid)
    finally:
        client.close()
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--sizes", type=int, nargs="+", default=[10000, 100000])
    parser.add_argument("--repeats", type=int, default=20)
    parser.add_argument("--label", default="unversioned-build")
    args = parser.parse_args()
    if any(n < 400 or n % 40 for n in args.sizes) or args.repeats < 2 or len(set(args.sizes)) != len(args.sizes):
        parser.error("sizes must be unique multiples of 40 >= 400; repeats >= 2")
    args.output.mkdir(parents=True, exist_ok=True)
    if any(args.output.iterdir()):
        parser.error("output must be empty")
    binary = args.binary.resolve(strict=True)
    report = {"kind": "synthetic_local_capacity_observation", "recorded_at_utc": datetime.now(timezone.utc).isoformat(), "label": args.label,
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "platform": platform.platform(),
        "python_sqlite_version": sqlite3.sqlite_version, "warmup_requests_per_query": 2,
        "percentile_definition": "p95 nearest-rank; p50 median", "remote_model_calls": 0,
        "limitations": ["Local MCP timings include transport/serialization/provenance reads; no throughput SLA.",
            "Warm cache, one hardware/runtime session; compare only matching data hashes, build profiles and conditions.",
            "RSS is a Linux process snapshot, not peak memory; null means unavailable.",
            "Lock probe request delay includes work beyond lock waiting; internal lock wait is not instrumented.",
            "Dashboard-like browse means search_memory union; no browser/dashboard rendering measured.",
            "EXPLAIN primitives use Python SQLite, which can differ from the binary bundled SQLite/planner.",
            "Dataset has N/2 events + N/2 claims and 99 reflection rows; extra relation/index rows are not included in N."]}
    report["sizes"] = {}
    for size in args.sizes:
        with tempfile.TemporaryDirectory(prefix=f"ledger-capacity-{size}-") as temporary:
            report["sizes"][str(size)] = run_size(binary, size, args.repeats, Path(temporary))
        evaluation.write_json(args.output / f"capacity-{size}.json", report["sizes"][str(size)])
    evaluation.write_json(args.output / "report.json", report)
    print(evaluation.compact(report))


if __name__ == "__main__":
    main()
