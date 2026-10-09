# MCP Memory Ledger

Local-first MCP memory for AI agents, backed by SQLite and evidence-gated self-revision.

Database safety: current-schema inspection validates canonical columns, keys, foreign keys, constraints and indexes. Initialization reserves its path atomically; migration holds a SQLite write lock through backup and readback. A malformed current-version database requires explicit repair, and failed initialization retains files for diagnosis.

Languages: English | [Simplified Chinese](docs/README.zh-CN.md) | [Japanese](docs/README.ja.md)

MCP Memory Ledger is a Rust MCP `stdio` memory service for local AI clients. It records interactions, evidence, claims, self snapshots, and reflection audits in SQLite so an agent can use durable memory inside explicit, inspectable boundaries instead of relying only on a single prompt context.

The project grew out of a discussion about information loss, durable memory, and how selected parts of the past can safely constrain future agent behavior. The [origin and mainline principles](docs/origin-and-principles.md) are now the entry point for deciding what belongs in this repository.

The current project is best understood as a technical MVP for local agent memory, MCP integration, SQLite persistence, and governed self-revision. It is not a production autonomous-agent platform. Remote team mode, multi-tenancy, packaged installers, daemon write capabilities, and production security boundaries remain gated roadmap work.

The [formalization improvement plan](docs/formalization-improvement-plan-2026-08-25.md) consolidates the current product, engineering, security, release, and mainline-sync gaps. It is an acceptance map, not evidence that Local Alpha or production readiness has already been achieved; the active project plan remains the only execution queue.

## Offline usability candidate

The MCP surface now adds `recall_memory` and `build_task_context` (12 tools total). Record and correct with optional durable `request_id` replay, inspect bounded structured Event feedback, retrieve short Chinese/English literal queries without a model, and pack complete provenance-bearing records under an exact compact-JSON byte budget.

Compatibility note: direct reflection no longer permits project evidence to revise global self Claims; project-specific facts stay scoped, while the documented legacy global-world observation exception remains.

See the [workflow and contracts](docs/local-memory-usability.md) for retry rules, schema-v4 migration, feedback trust limits, current-vs-historical retrieval, context byte accounting, and an executable installed-binary restart/backup/restore simulation. Text recall currently scans scoped data; it has no embeddings or large-corpus performance claim. Formal Alpha/fresh-machine/release gates remain open.

## Features

- **Local MCP memory service**: exposes `ingest_interaction`, `search_memory`, `get_memory`, `get_reflection_history`, `get_self_model_history`, `get_evidence_relation`, `supersede_memory`, `build_self_snapshot`, `decide_with_snapshot`, and `run_reflection` over MCP `stdio`.
- **Scoped event, claim, episode, and reflection recall**: `search_memory` requires an explicit namespace. It defaults to bounded recent-first event records; additive `record_type = Claim` returns scoped claims, `record_type = Episode` returns a scope-projected episode record, and `record_type = Reflection` returns reflections attributed only through same-scope Claim endpoints. Record-only reflections stay invisible. Additive `record_types` can request a scoped union of those existing tagged records with a stable recorded_at / type / id order. All deterministic paths are provider-free and scope-filter exact records; mixed-scope Claim revision edges are hidden in full.
- **Scoped stable-ID lookup**: `get_memory(namespace, id, record_type?)` returns one complete Event, Claim, Episode, or scoped Reflection record. Omitting `record_type` preserves Event behavior. Claim, Episode, and Reflection lookup require their explicit type. Episode and Reflection treat `id` as an opaque exact persisted reference. Event and Claim accept canonical or raw IDs. Missing, cross-namespace, and record-only Reflection IDs return `record: null` without widening.
- **Scoped Claim reflection history**: `get_reflection_history(namespace, claim_reference, limit?)` walks the bidirectional revision chain reachable from one exact scoped Claim and returns newest-first reflection records. Missing, cross-scope, or mixed-scope paths stay empty/hidden; the provider-free read remains bounded to 1–100 records.
- **Scoped identity/commitment revision audit**: `get_self_model_history(namespace, history_type, limit?)` returns newest-first identity or commitment patches persisted on claim-attributed reflections. Record-only updates stay hidden. This is an audit trail, not a versioned identity/commitment ledger.
- **Scoped evidence-relation runtime**: `get_evidence_relation(namespace, trigger_window_event_ids, selected_evidence_event_ids?, selection_basis?)` intersects a caller-provided trigger window with same-scope events, then reports selected versus available-not-selected rows. Missing and cross-scope trigger IDs are omitted; selected IDs outside the scoped window fail closed. The path does not rank or widen.
- **Scoped audited supersede**: `supersede_memory(namespace, claim_reference, replacement_claim, replacement_evidence_event_ids, summary)` replaces one same-scope claim through the existing `run_reflection` transaction. The old claim stays as `Superseded`; missing or cross-scope targets fail closed. This is not a second durable write path.
- **SQLite persistence**: stores events, claims, evidence, reflection audits, trigger ledger entries, and operation logs.
- **Evidence-gated self-revision**: claim, identity, and commitment updates must be backed by explicit evidence and governance rules. `run_reflection` remains the only durable write path for identity, commitment, and reflection changes.
- **Bounded scoped snapshots**: the M0.2 path accepts an explicit namespace, optional evidence manifest, and inclusive time window; it applies owner/namespace filtering and stable recent-first ordering in SQLite, and feeds automatic reflection only from the frozen trigger scope and evidence window.
- **Bounded local operations**: includes operation-log lookup, backup / restore helpers, redacted diagnostics, explicit `init` / `migrate`, and a no-write `doctor --read-only` default. `serve` refuses missing or stale databases instead of changing them implicitly.
- **Local runtime safety gate**: an enabled unauthenticated dashboard accepts only localhost/loopback hosts, while CLI tracing is initialized on stderr so MCP/JSON stdout stays protocol-only.
- **Reproducible source gate**: Rust `1.95.0` is pinned, and Linux/macOS CI runs formatting, all-feature Clippy, the full test tier, and status synchronization.
- **Provider integration**: supports `mock`, `openai-compatible`, and OpenRouter configuration paths. Provider secrets should stay in private local config or environment variables.

## Use Cases

- Add MCP memory to a local AI client.
- Study how an agent can update long-term memory through explicit evidence.
- Validate a minimal loop for self snapshots, reflection, and commitment gates.
- Use a Rust + SQLite + MCP `stdio` project as an engineering reference.

## Quick Start

macOS:

```zsh
./scripts/agent-llm-mm.sh bootstrap-local
./scripts/agent-llm-mm.sh init
./scripts/agent-llm-mm.sh doctor --read-only
./scripts/agent-llm-mm.sh serve
```

Windows:

```powershell
pwsh -File .\scripts\agent-llm-mm.ps1 bootstrap-local
pwsh -File .\scripts\agent-llm-mm.ps1 init
pwsh -File .\scripts\agent-llm-mm.ps1 doctor --read-only
pwsh -File .\scripts\agent-llm-mm.ps1 serve
```

`bootstrap-local` creates a local config template from the development example. It does not overwrite existing files, create secrets, or start the service.

Platform and integration guides:

- [macOS development guide](docs/development-macos.md)
- [Windows development guide](docs/development-windows.md)
- [Local MCP integration guide](docs/local-mcp-integration-2026-03-26.md)

## Demo

Run the reproducible self-revision demo:

```zsh
./scripts/run-self-revision-demo.sh
```

The demo starts a deterministic local `openai-compatible` stub provider, runs the canonical scenario through the real MCP `stdio` service, and writes its report to `target/reports/self-revision-demo/...`.

## Local Diagnostics

Generate a redacted support bundle:

```zsh
./scripts/generate-support-bundle.sh target/support-bundles/manual-check
```

The support bundle contains redacted JSON summaries only. It does not copy the full SQLite database, raw TOML, provider payloads, or raw `.log` files. To export a focused log excerpt or a specific MCP tool call, pass `--log-file` or `--correlation-id` explicitly.

## Current Boundaries

Implemented:

- MCP `stdio` main flow
- SQLite persistence with owner / namespace constraints
- Audited claim replacement through `run_reflection`
- Minimal identity and commitment revision
- Trigger-ledger-backed automatic self-revision MVP
- Read-only dashboard, `doctor`, local support bundle, and local gate summary scripts
- Explicit SQLite schema version / migration ledger, transactional legacy migration, pre-write backup and restore rehearsal
- Loopback-only enabled dashboard configuration, stderr-only tracing, and Linux/macOS source CI on pinned Rust `1.95.0`
- M1.1.1 scoped event recall through the additive `search_memory` MCP tool; cross-namespace exact-ID matches return empty and semantic memory tables remain unchanged by reads
- M1.1.2 scoped claim recall through additive `search_memory(record_type = Claim)`; omitted `claim_status` defaults to `Active`, claim results retain canonical evidence/episode and direct reflection revision links, and event/time filters are rejected because claims have no stored `recorded_at`
- M1.1.3 scoped episode provenance recall through explicit `search_memory(record_type = Episode)`; SQLite derives each record only from same-scope events, orders records by the latest same-scope event tuple, returns the persisted episode reference unchanged, and exposes canonical same-scope Event/Claim provenance
- M1.1.4 scoped reflection provenance recall through explicit `search_memory(record_type = Reflection)`; a row is in scope only via a same-scope superseded Claim with an in-scope or absent replacement, and record-only reflections cannot inherit a namespace
- M1.2.1 scoped event lookup through additive `get_memory`; a missing or cross-namespace stable ID returns `record: null` without widening
- M1.2.2 scoped claim lookup through explicit `get_memory(record_type = Claim)`; exact lookup accepts canonical/raw Claim IDs and returns Active, Disputed, or Superseded claims with the same provenance shape as Claim search
- M1.2.3 scoped Claim reflection history through the seventh MCP tool, `get_reflection_history`; canonical/raw Claim anchors reach a bounded newest-first bidirectional revision chain, while missing/cross-scope anchors and mixed-scope edges do not widen or leak
- M1.1.5 scoped evidence-relation runtime through the eighth MCP tool, `get_evidence_relation`; a caller trigger window is intersected with same-scope events, selected IDs must stay inside that window, and the existing no-widening / binary-weight report is returned with canonical event references
- M1.1.6 scoped cross-type union through additive `search_memory.record_types`; omitted type still searches Event, union rejects type-specific filters, and mixed results keep the existing tagged record JSON
- M1.2.4 scoped Episode lookup through explicit `get_memory(record_type = Episode)`; the persisted episode reference stays opaque, and missing or cross-scope IDs return `record: null`
- M1.2.5 scoped Reflection lookup through explicit `get_memory(record_type = Reflection)`; missing, cross-scope, and record-only IDs all return `record: null`
- M1.2.6 scoped identity/commitment revision audit through the ninth MCP tool, `get_self_model_history`; it reads existing reflection audit columns and does not version the current-state tables
- M1.2.7 scoped audited supersede through the tenth MCP tool, `supersede_memory`; it reuses `run_reflection`, marks the old claim `Superseded`, and does not hard delete
- M1.0.1 scoped identity evidence-to-Episode counting: supporting-Episode lookup now requires an explicit `MemoryScope` and keeps only Claim/Event endpoints that match that owner + namespace before any identity-revision count
- M1.0.2 mixed-scope Claim revision-edge redaction: Claim search/get hide the whole edge when either endpoint leaves the requested scope, so Reflection IDs and the other Claim ID cannot leak as metadata
- M1.0.3 owner/namespace write-read reachability: new writes use the namespace-derived owner matrix and reject `Owner::Unknown`; read-only doctor inventories leftover Unknown rows without rewriting them

Partially implemented:

- M0.2 is complete only for the explicit scoped-snapshot boundary described above. Legacy calls that omit `namespace` remain unscoped for MCP compatibility, and the repository still has no complete user-facing recall contract.
- `build_self_snapshot` now accepts an additive `namespace` input. The server derives the matching owner and pushes exact owner/namespace filtering through the application/query-port/SQLite path for claims, event references, and episode references. `MemoryScope` accepts only a matching owner+namespace pair or the fully empty legacy compatibility state; partial and mismatched deserialized scopes are rejected. Calls that omit `namespace` retain the legacy unscoped behavior for MCP compatibility, so callers that require isolation must pass it explicitly.
- `build_self_snapshot` also accepts an additive explicit `evidence_manifest`, which requires an explicit `namespace` so it cannot enter the legacy unscoped compatibility path and is capped at 256 entries before query construction. DTO, application, and store boundaries fail closed; snapshot inputs are validated before optional auto-reflection runs. Bare event IDs and `event:<id>` references normalize to one canonical representation with order-preserving linear-time deduplication; SQLite intersects the manifest with the server-owned owner/namespace scope, and an empty intersection stays empty without widening. Tool operation logs use the snapshot namespace, while separate auto-reflection diagnostics retain their own namespace.
- Optional `recorded_after` / `recorded_before` now add an inclusive snapshot time window. Either bound requires an explicit `namespace`; reversed bounds are rejected before optional auto-reflection. SQLite intersects owner/namespace, manifest, and time in the evidence query, normalizes project-canonical timestamps and common legacy RFC3339 `Z` / offset variants into a fixed-width UTC key while preserving nine fractional-second digits, and returns recent-first evidence with a stable row-id tie-break. Episodes are selected and ordered by their latest qualifying event tuple, while claims remain scope-only because they do not currently carry a recorded timestamp. An explicit window with no matches stays empty without widening; legacy unbounded snapshot calls remain available but now use recent-first SQLite ordering.
- Scoped snapshot v2 now also feeds automatic reflection from an explicit server-derived `MemoryScope`, the current trigger evidence manifest, and the inclusive bounds derived from that manifest. Candidate detection and its dependent episode read use the same owner/namespace/window; no qualifying intersection leaves auto-reflection untriggered rather than falling back to historical data. Explicit MCP `build_self_snapshot` calls retain their legacy-compatible path.
- In the active reflection runtime, MCP `replacement_evidence_event_ids`, application explicit/query evidence merging, and model-proposed auto-reflection evidence accept either raw IDs or `event:<id>`. They parse through `EventReference`, reject blank/empty/repeated-prefix forms, and deduplicate by underlying raw ID while preserving first-seen order. Store lookups, evidence links, reflection audit `supporting_evidence_event_ids`, and auto-reflection diagnostic `*_event_ids` deliberately remain raw for compatibility; canonical `event:<id>` is reserved for reference-shaped fields.
- The read-only evidence-relation and episode-summary projections now apply the same parsing and ordered raw-ID deduplication to their trigger/selected/episode/linked `*_event_ids`, before subset checks and count/rank derivation. Projection JSON readback deliberately remains raw IDs. M1.1.5 additionally exposes the evidence-relation report as a scoped MCP runtime path with canonical `event:<id>` rows.
- The deterministic offline self-revision demo now exposes its generated baseline event in `timeline.json` as a canonical `event_reference`; malformed MCP event IDs fail the runner instead of entering artifacts. Snapshot evidence is already reference-shaped, while the stub's empty `proposed_evidence_event_ids` and SQLite `supporting_evidence_event_ids` remain explicit raw-ID compatibility fields. Repository-wide normalization remains partial: support bundle and other excluded surfaces are not changed by these slices.
- `decide_with_snapshot` still uses an action-string contract and is not a full decision engine. M0.3 replaces caller-provided commitments with the current server-side commitment store before invoking the model, and applies the same commitment gate to both the requested and provider-selected actions. A selected action rejected by policy is returned as blocked with no `decision` payload. A non-blocked action keeps the compatible `model_decision` / `{ "action": "..." }` shape but now reports `decision_authority = experimental_non_authoritative` and `policy_scope = server_commitment_gate_only`; `gate.blocked = false` is not a full policy-passed verdict.
- Automatic identity revision replaced the old global-count heuristic with distinct episodes reached through selected active-claim IDs, persisted `evidence_links`, and `episode_events`. Unrelated global episodes no longer raise the support count. M1.0.1 now binds both Claim and Evidence Event endpoints to one explicit `MemoryScope` before grouping or counting, so a persisted cross-namespace evidence link cannot change the identity-revision judgment.
- Governance validation failures write only a rejected trigger audit. Failures while appending the handled trigger ledger or committing the reflection transaction roll back pending identity, commitment, claim/evidence, reflection, and handled-ledger changes; a separate rejected audit is then recorded outside the failed transaction. This is locally verified failure atomicity, not crash-recovery or distributed transaction support.
- Identity, claims, evidence, and episodes in the decision snapshot are still caller-provided; there is no server-created snapshot handle or complete policy/provenance binding yet.
- Episodes are still lightweight scope-projected records over `episode_events -> events`, not a durable Episode entity or complete autobiographical memory model. The new search slice does not persist or claim `objective`, `outcome`, or `lesson` fields.
- The runtime read interface now covers complete Event/Claim search and lookup records, scoped Episode search and lookup, scoped Reflection search and lookup, a scoped cross-type union, a bounded Claim-linked reflection-history slice, a scoped identity/commitment revision audit, and a scoped evidence-relation report. `supersede_memory` adds a scoped Claim correction facade over `run_reflection` and does not create a second durable write path. These paths do not cover a versioned identity/commitment ledger or record-only reflection history. Record-only reflections stay invisible because they have no Claim scope. The Episode, Reflection, self-model audit, evidence-relation, union, and supersede slices add no schema migration or index and retain an MVP table-scan performance boundary.
- M1.0 scope/data-integrity gates are complete for new writes. Leftover `Owner::Unknown` rows remain schema-legal but invisible to namespace-derived scoped reads until a separately approved rewrite.
- Provider live evidence proves configuration and connectivity only. It does not prove model quality, SLA, or production readiness.
- Local alpha gates still depend on external evidence such as a real fresh-machine run, Windows parity, and a human release decision.
- The repository remains on `rmcp 0.5.0`. An isolated `2.2.0` compatibility probe is documented as no-go for an in-place M0.5 bump because one handler error contract regressed; the future upgrade must remain capability-neutral.

Not implemented:

- Full memory layering
- Versioned identity/commitment ledger, record-only reflection history, and Event/Episode/Reflection correction
- Richer evidence ranking / weighting
- Production-grade remote, team, or multi-tenant mode
- Daemon write capabilities and autonomous background operation
- Installers, service managers, auto-updaters, and release certification

See [project status](docs/project-status.md), the [roadmap](docs/roadmap.md), and the [active 2026-07-10 project plan](docs/plans/2026-07-10-product-replan.md) for the current implementation boundary and execution order.

## Documentation

### Understand the project

1. [Project origin and mainline principles](docs/origin-and-principles.md)
2. [Positioning](docs/positioning.md)
3. [Current implementation status](docs/project-status.md)
4. [Now / Next / Later roadmap](docs/roadmap.md)
5. [Formalization improvement and mainline sync plan](docs/formalization-improvement-plan-2026-08-25.md)
6. [Active project plan](docs/plans/2026-07-10-product-replan.md)

The active plan is the only current execution queue. Historical checklists are
preserved for traceability, but they do not define current work.

### Build and verify

- [macOS development guide](docs/development-macos.md)
- [Windows development guide](docs/development-windows.md)
- [Local MCP integration guide](docs/local-mcp-integration-2026-03-26.md)
- [Testing guide](docs/testing-guide-2026-03-24.md)

### Reference and history

- [Complete document map](docs/document-map.md)
- [Historical archive](docs/archive.md)

## Verification

Tests are split into `fast`, `core`, and `full` tiers. Release evidence, packaging,
and provider-certification tooling is opt-in through the `release-tools` feature.

Common local checks:

```zsh
./scripts/test-tier.sh fast
./scripts/test-tier.sh core
./scripts/status-sync-check.sh
./scripts/agent-llm-mm.sh doctor
git diff --check
```

Use `./scripts/test-tier.sh full` for release-tool changes and final full-feature
verification. For provider, dashboard, release-evidence, or support-bundle changes,
follow the relevant layered checks in the [testing guide](docs/testing-guide-2026-03-24.md).

## Naming

The public project name is MCP Memory Ledger. The current Rust crate, binary, scripts, config examples, and some historical docs still use `agent_llm_mm` / `agent-llm-mm` as compatibility identifiers.

## Acknowledgements

This repository has been developed, reviewed, and documented with active support from OpenAI Codex as a collaborative development tool. Thanks to OpenAI for the tooling and research ecosystem that made this workflow possible.

## License

This project is licensed under the Apache License 2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).

Copyright 2026 yooyui
