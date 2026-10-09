# Bounded offline memory workflow (e14a59e baseline)

Historical snapshot only: this page preserves the earlier v4 candidate contract at e14a59e. Do not use its startup commands or targetless-reflection wording as the current contract. Start with [the current quickstart](quickstart.md), [workflow](runnable-memory-workflow.md), [schema6 scope rules](reflection-scope-history.md), and [database operations](database-operations.md). The current additive v5 workflow, FTS index, balanced recall and feedback/experience lifecycle are documented in [the A–E runtime guide](memory-feedback-experience.md). Where this historical page says scans/no FTS or procedural expansion deferred, the newer guide supersedes that specific boundary. Legacy retry/scope and installation cautions continue to apply.

This is a local-first technical MVP candidate, not a production release or autonomous controller. A model is not needed for recording, recall, correction, history, or context packing. The configured owner/namespace boundaries are data isolation, not authentication between hostile local clients.

## Start from an existing binary

Set `AGENT_LLM_MM_DATABASE_URL` to a new file-backed SQLite URL, run `agent_llm_mm init`, then `agent_llm_mm doctor --read-only`. Register the same binary and environment as a stdio MCP server. The no-argument invocation remains `serve`; serving never initializes or migrates a database silently.

For an existing database, first run read-only doctor. Schema v4 adds optional feedback metadata. Explicit `migrate` makes a backup anchor, rehearses migration, then validates structural and data readback while excluding other SQLite writers. Never replace the live database with a backup in place: restore to a new path, verify, and switch the configured path deliberately. Structurally modified current-version databases fail closed and require explicit repair; they are not silently rebuilt. Failed initialization retains its reserved file for diagnosis.

## Source quickstart and client configuration

For Linux/macOS, this source path requires the pinned Rust toolchain; the copied-binary smoke above/below does not. Start in the cloned repository:

```sh
git clone --branch dev_work_dots https://github.com/CeauYoo/mcp-memory-ledger.git
cd mcp-memory-ledger
cargo build --release --bin agent_llm_mm
mkdir -p "$HOME/.local/share/mcp-memory-ledger"
export AGENT_LLM_MM_DATABASE_URL="sqlite://$HOME/.local/share/mcp-memory-ledger/memory.sqlite"
./target/release/agent_llm_mm init
./target/release/agent_llm_mm doctor --read-only
```

If that database already exists, do not retry init or delete it. Inspect with read-only doctor; use explicit `migrate` only when doctor reports an older supported schema. Migration creates its own backup anchor and rehearsal. Writer admission fails fast on contention; after acquiring its reservation, migration allows a bounded five-second wait for transient reader locks, including at COMMIT. For an additional user-controlled backup, see [backup/restore instructions](development-macos.md) and the tested `scripts/backup-sqlite.sh` / `scripts/restore-sqlite.sh`; restore to a new path and inspect before switching. Windows setup/PowerShell instructions remain in [development-windows.md](development-windows.md), with native-runtime evidence tracked separately from shell-wrapper parity.

Use absolute paths when registering the binary. A typical stdio client configuration is:

```json
{
  "mcpServers": {
    "memory-ledger": {
      "command": "/absolute/path/mcp-memory-ledger/target/release/agent_llm_mm",
      "args": ["serve"],
      "env": {
        "AGENT_LLM_MM_DATABASE_URL": "sqlite:///absolute/path/memory.sqlite"
      }
    }
  }
}
```

Replace both paths with the same initialized database and actual binary; do not paste placeholders unchanged. The database environment is required in the client too, not only in the terminal that initialized it. For Codex TOML registration, use [the existing config example](../examples/codex-mcp-config.toml) and [local MCP integration guide](local-mcp-integration-2026-03-26.md). No provider key is required for deterministic memory tools; the default mock profile is offline. Reconnect the client after changing its configuration, then call `recall_memory` with an explicit project namespace.

## Record and retry safely

`ingest_interaction` accepts an optional `request_id`: 1–128 UTF-8 bytes, no surrounding whitespace or control characters. The key is scoped to operation and namespace. Reuse the same key and exact typed payload/order after a lost response. A committed retry returns the same `event_id` and `replayed: true`; changed payload is rejected. Trigger hints are included in request identity, and successful replay does not run automatic reflection again. Calls without a key remain distinct writes.

`supersede_memory` accepts the same optional key. It returns the original reflection/replacement IDs on replay. Active and disputed Claims may be corrected; superseded Claims are terminal. Target, replacement, and evidence scope are checked inside the same transaction as the conditional state transition. There is one successor, no hard deletion, and history remains inspectable. Legacy `run_reflection` self-governance may still cite global-world observations; explicit scoped `supersede_memory` never inherits that exception.

Compatibility change: direct `run_reflection` previously allowed project-scoped evidence to support global `self` Claim revisions. That path now rejects project→global-self evidence, before any replacement, status change, reflection audit, or success receipt. This prevents one project's observations silently changing global self Claims. Keep project-specific Claims/evidence in that project. If a global self revision is intended, explicitly record an appropriate global observation in `world` (or same-scope `self` evidence) and retain its limitations; do not merely relabel private project data or treat this as authenticated approval. The global-world exception exists only for the legacy direct reflection contract, not scoped correction.

Important legacy boundary: direct `run_reflection` can still attach global identity/commitment patches to a project Claim, and existing automatic/targetless self-revision can derive global patches from governed project evidence. Those experimental semantics are preserved; this candidate does not redesign them or claim universal global self-model isolation. Use scoped `supersede_memory` for ordinary correction: its input has no global self-model patch, and regression tests verify identity/commitments remain unchanged. Review project-driven global proposals carefully. Namespace isolation is not multi-user authentication or authorization, and scoped-path test results do not imply all legacy paths are isolated.

All successful application ingest/reflection writes append a minimal durable receipt in the same transaction. Receipt rows use `operation_log.actor_id = durable_write_receipt_v1` and reserved `write-receipt:v1:` IDs. They contain request hashes and result IDs, not raw keys or request text. Do not prune or alter them: they are part of retry correctness, not disposable diagnostics. Failure to store the receipt rolls back the write. Low-level store/import APIs are not covered by the application correction contract.

## Structured observations

An ingested Event may contain `feedback` with:

- `source_kind`: `caller_reported`, `tool_reported`, or `model_asserted`
- `producer`, `observed_target`, optional `observed_version`
- `expected`, `actual`, `verification_method`
- `verification_result`: `passed`, `failed`, `inconclusive`, or `not_performed`
- `limitations`, `evidence_refs`

Text fields are bounded to 4096 UTF-8 bytes each; lists to 32 entries. Referenced evidence must already exist in the same owner/namespace and is checked transactionally. Feedback is returned by Event get/search/recall and remains part of the original immutable application observation. A source label is supplied by the caller: it does not authenticate a tool or prove semantic truth. Corrections append new evidence and history; no external action is authorized by feedback.

## Recall and bounded task context

Use `recall_memory(namespace, query, limit?)` for local literal-text retrieval. `search_memory` retains its historical filter/browse ordering. Recall searches active Claim subject/predicate/object and Event summaries, with exact scope filtering before candidate limits. It uses literal substring OR matches for up to eight whitespace-separated terms, at most 512 UTF-8 query bytes. ASCII is case-insensitive; CJK is exact substring matching, including 北京, 咖啡, and 记忆. Punctuation, `%`, `_`, and quotes are literal. Empty queries are rejected.

Claims rank before Events, then matched-term count descending and stable ID ascending. Returned records carry complete provenance and a match count. Superseded/disputed Claims are omitted, but historical raw Event observations can still match and must not be treated as current authoritative Claims. This is not semantic/vector search, fuzzy search, stemming, or full Unicode case folding. Text scans are not indexed; no latency or large-corpus performance claim is made. Episode/Reflection text retrieval is deferred.

`build_task_context(namespace, query, limit?, max_bytes?)` uses the same recall, then packs complete records with provenance. Default `max_bytes` is 16384, maximum 262144. The hard cap measures the exact compact result JSON UTF-8 bytes, including all metadata and the `serialized_bytes` field. It excludes enclosing MCP/JSON-RPC transport fields. It is not a token budget. Oversized records are omitted rather than cutting text or references; later smaller records can fit. A budget too small for the empty metadata envelope is rejected. Omission counts distinguish byte exclusions from further matching candidates. Fetch full records by scoped `get_memory` when needed.

## Reproduce the local installation simulation

From a source checkout with the pinned toolchain, build the binary once:

```sh
cargo build --bin agent_llm_mm
python3 scripts/local-memory-smoke.py --binary target/debug/agent_llm_mm --output target/reports/local-memory-smoke
```

On Windows, use `python` and `target/debug/agent_llm_mm.exe`. Choose an empty output directory; the script refuses overwrite. It copies the binary to that directory and runs it without Cargo, using synthetic data and a mock provider. It exercises initialization, tools/list, three namespaces, keyed ingest/replay, restart, CJK recall, provenance, structured feedback, correction/replay/history, exact context size, backup, restore to a new path, read-only doctor, and post-restore recall.

It saves a synthetic MCP transcript, installed binary checksum, doctor output, and summary. SQLite backup uses Python's SQLite backup API, not a live file copy. Local duration is recorded, but this is explicitly a local installation simulation, not an actual fresh-machine test or a promise of a ten-minute installation. Existing backup/restore shell tooling remains separately regression-tested.

CI adds the same workflow on Linux/macOS plus a separate Windows native-runtime subset. A configured job is not platform evidence until that exact commit's run passes. Windows shell-wrapper parity, binary release publishing, real fresh-machine evidence, live-provider certification, and human release approval remain open.

## Design references checked 2026-10-09

These are narrow architectural references, not imported code or benchmark claims:

- [Mem0](https://github.com/mem0ai/mem0): keep scope identity separate from mutable metadata and evaluate retrieval independently.
- [Graphiti edge schema](https://github.com/getzep/graphiti/blob/main/graphiti_core/edges.py): explicit source/validity/revision provenance. Ledger retains SQLite and auditable claim supersession.
- [Letta MemFS](https://docs.letta.com/concepts/memfs): compact useful context plus retrievable references. No cloud dependency or self-modifying harness is added.
- [Official MCP memory server](https://github.com/modelcontextprotocol/servers/blob/main/src/memory/index.ts): predictable local tools and serialized safe writes. Ledger uses SQLite transactions and durable retry receipts.
- [Basic Memory](https://github.com/basicmachines-co/basic-memory): local inspectability and CJK usability. Its AGPL implementation is not copied into this Apache-2.0 codebase; literal matching here is independently implemented.

Full-text indexes, mandatory embeddings, model-judged paid evaluations, remote/team services, autonomous control and procedural-memory expansion remain deferred.
