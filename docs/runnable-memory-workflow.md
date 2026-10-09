# Run the complete local memory workflow

This is a local technical MVP. Use the platform setup first: [macOS](development-macos.md) or [Windows](development-windows.md). [Client registration](local-mcp-integration-2026-03-26.md) remains separate from the executable protocol tests below.

## 1. Run isolated, provider-free examples

Build once with `cargo build --bin agent_llm_mm`. Use your platform's Python launcher and binary path (`target/debug/agent_llm_mm`, or `.exe` on Windows). These scripts generate explicit mock configuration and isolated synthetic databases; each output directory must be new and empty.

```text
python scripts/evaluate-memory-loop.py --binary <built-binary> --output <new-output-directory>
python scripts/local-memory-smoke.py --binary <built-binary> --output <another-new-directory>
python scripts/temporal-scope-export-smoke.py --binary <built-binary> --output <another-new-directory>
```

On systems whose launcher is `python3`, substitute that launcher. The evaluator runs feedback → proposal → deterministic validation → commit → recalled correction, then rich Episode → semantic/procedural candidate → activation → recall → revision/rejection/rollback. The other scripts verify restart/replay/backup restoration and temporal/scoped-history/read-only export. They fail on broken assertions. They are not real-model efficacy or fresh-machine installation evidence. Raw outputs stay local unless you deliberately choose to share them.

## 2. Use the same flow through your MCP client

Keep one explicit namespace, for example `project/demo`, throughout:

1. Ingest an Event and supported Claim; retain returned IDs. Optional `observed_at` is the caller's observation time; server recording time is separate. Omitted historical times remain unknown.
2. Read `get_feedback_target_version` for the exact canonical Claim reference.
3. Ingest truthful structured feedback bound to that target/fingerprint and old/new object values. `tool_reported` is a caller label, not authentication. Missing, inconclusive or restricted evidence cannot be made trustworthy by labeling it.
4. Propose and validate a feedback candidate. Inspect its decision. Commit only a supported candidate; reject or revise insufficient proposals. A changed target or nonempty free-text limitation blocks this automatic commit contract.
5. Recall or build context, and inspect the original/replacement evidence and history. Retry durable writes with the same request_id and identical business payload; changed payload under that key is rejected.
6. Record a rich Episode linked to same-scope source Events. Propose semantic/procedural candidates, inspect them and explicitly activate only the desired recall version. Revision/rollback creates a pending version requiring activation again; no procedure execution or permission change follows.

Exact argument shapes and examples: [feedback and experience](memory-feedback-experience.md). Do not invent IDs/fingerprints; carry returned values forward.

## 3. Build bounded, inspectable working context

`build_task_context` takes namespace, query, limit and max_bytes. It packs complete Claim/Event records first, then bounded source-linked rich Episode snapshots, then optional observable diagnostics. All fields, provenance, omissions and an optional caller receipt count toward the compact JSON UTF-8 cap. It is not a token or transport-envelope limit.

Disputed/Superseded diagnostics use explicitly incomplete bounded samples. Differing returned values are possible multi-valued conflicts, not inferred contradictions. Recording age is not expiry. Missing or omitted diagnostics do not prove consistency. See [context contract](context-diagnostics.md).

Optionally supply `caller_budget` on recall_memory, build_task_context or run_reflection with limits/used retrieval, reflection and retry counts. Carry next_used forward from success or admitted-error receipts; stop when allowed=false and inspect stop_reason. Explicit unchanged/insufficient evidence can stop reflection without blocking evidence retrieval. This is cooperative caller state, not a persistent quota or new session. Omission/null retains legacy behavior. See [budget request example and error accounting](caller-operation-budget.md).

## 4. Inspect, export and recover deliberately

- Targetless Reflection history requires an explicit safe origin scope; affected scope never becomes an alternative visibility grant. [Scope/history contract](reflection-scope-history.md).
- `export_memory` returns a bounded read-only same-scope interchange document. It excludes unsafe dependent provenance as a whole and does not log writes, even on failure. It is not a backup or automatic secret-redaction service. [Export contract](scoped-export.md).
- Keep explicit migration backups and restore only to a new path. Serving never silently migrates. Inspect derived-index damage read-only, and rebuild explicitly. Retain-all histories/receipts remain the policy; no automatic deletion is introduced.

## What the evidence establishes

The fixed provider-free evaluator, source-consistent capacity observations, migration/replay regressions and exact-head CI establish bounded implementation behavior. They do not establish actual model task improvement, token savings, production throughput, user-client/fresh-machine installation or human release approval. [Final original-plan reconciliation](plans/2026-10-09-original-plan-final-reconciliation.md) records those boundaries and each stage's verification.
