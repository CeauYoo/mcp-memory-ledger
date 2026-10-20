# Local owner-channel task attestation (partial T03)

This technical MVP adds an explicit local CLI channel, not authenticated human authorship. A process with the same OS user's shell/filesystem access can invoke it. UID/SID, client labels, possession of the database, and this command do **not** distinguish a human from a model. It does not add `verified_user`, signing keys, credentials, a network endpoint, MCP tools, or dashboard/admin write capabilities. Configured-host signed import is a future integration decision. Imported requirements remain knowledge, never permission to execute, deploy, spend, disclose data, or change confirmation policy.

## Protocol

Configure the intended database as for existing local task commands. The database must already be explicitly migrated to schema 11; this command never initializes or migrates it.

```text
agent_llm_mm task-owner-attest --json '{"namespace":"project/demo","task_id":"task-one","import_event_id":"owner-event-001","expected_version":1,"evidence_text":"Exact operator-supplied evidence","change":{"operation":"append","requirement_id":"owner-constraint-001","kind":"prohibition","exact_text":"Do not deploy without fresh approval."}}'
```

The existing task must be active, the expected revision must match, and the requirement ID must be new across its entire history. Append accepts exactly one explicitly structured requirement with one of the existing requirement kinds. Text is preserved as supplied, including leading/trailing whitespace, Unicode, newlines and tabs. There is no extraction, interpretation, splitting, normalization or implicit replacement. Evidence and requirement text are independently supplied exact strings: the command does not prove a quotation occurs in evidence or that evidence came from a human. Model interpretation through ordinary task routes remains a candidate. The operator explicitly chooses the exact structured change.

A withdrawal is also explicit, requires the currently live imported requirement's exact text, and preserves its provenance/history:

```json
{"namespace":"project/demo","task_id":"task-one","import_event_id":"owner-event-002","expected_version":2,"evidence_text":"Exact operator-supplied withdrawal evidence","change":{"operation":"withdraw","requirement_id":"owner-constraint-001","exact_text":"Do not deploy without fresh approval."}}
```

No implicit replacement or owner lifecycle override exists in this slice. Ordinary revisions may add independent requirements/candidates, but cannot withdraw or replace any imported ID, even after owner withdrawal. IDs cannot be recycled. While a live imported requirement exists, ordinary lifecycle changes to paused/completed/cancelled are rejected. Explicit owner withdrawal of every live imported requirement is necessary before normal lifecycle deactivation; that does not authorize executing anything. Checkpoints record observations only and cannot alter task requirements.

## Persistence and replay

Schema 11 adds `task_owner_events` and `task_owner_provenance` under the existing explicit backup/rehearsal migration workflow. Event input stores the exact parsed request fields, including evidence and structured change. Per-requirement provenance stores import ID/version, kind, exact text/evidence and any withdrawal event/version. Reads reconcile provenance with bounded event history and requirement state/kind/text; task snapshots and resume context return server-owned provenance. Ordinary request schemas cannot inject it.

The local event ID is the durable global deduplication key, independent of ordinary caller request IDs (the attestation request has no request_id). Repeating exactly the same request returns the original historical revision without another write, even after later revisions or restart. Reusing an event ID with different content or scope fails with a generic conflict and never returns another namespace's receipt. Global ID collisions can deny a chosen ID, so use unique IDs. Changed expected_version is changed content, not an identical retry.

`BEGIN IMMEDIATE`, task revision compare-and-swap, event/provenance insertion, and checkpoint-independent revision changes commit together. Any failure rolls back the entire operation. Full database backup retains provenance and replay identity. Doctor/open verifies owner provenance integrity. Scoped export rejects namespaces with owner event history until its format can preserve that authority; legacy adoption still rejects task histories. Direct same-user database rewriting is outside the trust boundary, although detected inconsistency fails closed.

Bounds: 131072 UTF-8 JSON bytes per request/task projection, 4096 bytes per requirement, 16384 bytes of evidence per event, existing bounded identifiers and 128 retained requirements. Metadata itself consumes task/context budget; overflow fails closed rather than silently omitting constraints and claiming completeness. No actual user data is imported by tests.

## Verification

```text
cargo test --test owner_attestation --test task_ledger --test sqlite_lifecycle --test schema7_migration --test enhanced_recall
```

The focused suite covers exact text, candidates versus imported requirements, replay/restart/duplicate concurrency, collision/cross-scope refusal, stale CAS, transactional rollback, ordinary replace/withdraw/re-add/lifecycle bypasses, retained withdrawal history, corruption refusal, schema10→11 migration and full SQLite backup/reopen. This is local deterministic evidence, not a configured-host integration or proof of human approval.

### Local validation record (2026-10-20)

On Linux, source commit `c26cdd3` passed the focused command above plus `--test mcp_session_policy`: owner_attestation **9**, task_ledger **23**, sqlite_lifecycle **22**, schema7_migration **11**, enhanced_recall **14**, and mcp_session_policy **21** tests, **100 total** (excluding lifecycle subprocess repeats). The actual local CLI, task context provenance, and production backup→restore helper were exercised with disposable fixture databases only. `cargo fmt --all -- --check`, `git diff --check`, and `cargo clippy --offline --lib --test owner_attestation -- -D warnings` passed.

The broader `cargo test --offline --lib --tests` attempt did **not** pass as a whole: it stopped in the pre-existing local_core runtime suite, where 14 Unix-socket startup cases failed at `tests/local_core/runtime.rs:93` with `Operation not permitted (os error 1)` in this execution sandbox. This is a verified environment blocker, not evidence of successful bridge runtime validation. No transport/security setting was changed to bypass it. No signed host ingestion, deployment, actual user-data import, or remote publication was performed.
