# Original-plan continuation: temporal facts, Reflection scope, selective export

This stage follows the original 70-row traceability matrix, after the published schema5 baseline `9a9d3be`. It closes actionable temporal/scope/export gaps rather than treating the first usable candidate as completion of the document.

## Requirement delta

- **S01/S06:** new Claims have recording time; caller observation time is distinct and optional. Historical unknown Claim times stay null. Original timestamp text is preserved; indexed opaque nanosecond sort keys normalize valid offsets without fabricating invalid/unknown dates. Neither field means effective validity or expiry.
- **S02/S03:** durable Reflection origin/affected scopes and evidence relations are independent source-of-truth tables, with foreign keys and reverse indexes. Targetless known history can be read only through its safe origin scope; mixed, missing and ambiguous history remains quarantined. Affected scope never grants alternate visibility or mutation authority. Feedback payload JSON remains appropriate bounded source data; no speculative normalization of every nested field.
- **S04:** scope/time indexes and union-specific per-type ordering before LIMIT are implemented. Single-type legacy browse ordering is preserved.
- **Retention/export:** retain-all remains the conservative non-destructive policy. New `export_memory` produces a bounded, scoped, read-only interchange document, not a restore backup or secret-redaction guarantee. Every original Claim evidence edge participates in closure: unsafe provenance omits the whole dependent Claim and cascades, rather than presenting a falsely complete partial history. Success and failure do not append diagnostic writes.
- **O03:** the reproducible harness now optionally exercises actual local HTTP summary/events/operation-log endpoints concurrently with real MCP writes/recall. This measures HTTP behavior, not visual browser rendering or production certification.
- **M01–M03/D10:** targeted in-crate module extraction and bounded observable context-state diagnostics are the next independent implementation stage. They are not marked complete here.

## Compatibility and migration evidence

Schema6 migration uses the existing reserved-write, pre-migration backup, rehearsal and canonical-readback lifecycle. It preserves source rows and raw dates, safely backfills only attributable history, and rebuilds only derived FTS projections. Existing request receipts retain their omitted-observation payload hash; the Claim version-v1 fingerprint remains frozen to its original fields. Caller-supplied source labels remain unauthenticated claims, not proof that a reported result is true.

Relevant suites: `schema6_migration`, `sqlite_lifecycle`, `temporal_metadata`, `sqlite_temporal_store`, `reflection_scope_history`, `scoped_ledger_export`, `mcp_stdio`; executable end-to-end flow: `scripts/temporal-scope-export-smoke.py`.

Local all-feature verification passed 613 Rust tests, formatting, all-target/all-feature Clippy with warnings denied, status-sync (32 completed gates), and diff checks. Final default runtime binary SHA-256 is `3045e57b2f1490fea5be62f2acf716f307f0841279298f40a094c81f15001a91`; source/build manifests and independent workflow/measurement results are in [schema6 evidence](../evaluation-evidence/schema6/README.md). Exact published-head CI must be checked separately.

The original fixed tasks, budgets and gold records are unchanged. Richer temporal metadata consumes context bytes: some fixed B contexts fit the gold Claim but no longer its supporting Event. Capacity results retain slower cases as well as improvements. Earlier schema5 reports remain historical evidence, not proof for schema6. All experiments here use synthetic isolated data and mock/local deterministic outcomes; real same-model efficacy, token costs, user-machine installation and human release approval remain separate gates.
