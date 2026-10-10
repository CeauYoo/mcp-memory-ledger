# Schema7 bounded self-model version results

Date: 2026-10-10. Status: locally implemented and verified on an isolated worktree
based on `66924c3f13d0793871875a4bf0af49d9e5efea5c`; this report does not claim
publication, exact-commit CI or a human release decision. The project remains a
local-first technical MVP.

## Delivered contract

The [version contract](../self-model-versions.md) defines one append-only global
identity/commitment sequence, truthful initialization/migration baseline 0,
per-component source pointers, ordered source-safe diffs, optional expected-version
guards and explicit component-selective compensating rollback. Existing
`run_reflection` remains the production global write boundary. Global writes,
Claim changes, normalized provenance, versions and successful receipts share one
transaction. Version reads verify projections and filter provenance in one read
snapshot. Doctor diagnostics acknowledge these governed paths and local portable
build/unpack support while preserving blocked automatic memory-layer writes and
incomplete full user-client compatibility evidence.

Rollback preserves the Claim anchor/evidence contract and unselected components;
without a replacement the anchor becomes disputed. Explicit caller confirmation
is not human authentication. Supplied evidence need not be newly created and its
content is not independently certified. Global metadata opt-in acknowledges the
counter side channel; it does not establish tenant isolation.

Migration captures actual current projections and preserves old audit/receipt
bytes. It never reconstructs a historical timeline. Drift, missing history and
weakened schema fail closed; serve/index repair never rebaseline. Compensation is
not schema downgrade. Old binaries require their matching pre-migration backup
restored to a new path for manual recovery.

## Source-bound local evidence

The source manifest covers every file under `src/**`, plus `Cargo.toml`,
`Cargo.lock` and `rust-toolchain.toml`: 146 files, SHA-256
`ee6ebf3d3fea1911b8092cc81ec367e72c2b58c5c17cc302317cbb7049363630`.
It is the SHA-256 of the sorted compact UTF-8 JSON mapping relative paths to file
SHA-256 values, without a trailing newline. This identifies the tested source;
it is not a substitute for a future exact-commit CI result.

The Rust 1.95 default-feature dev binary used for separate protocol/workflow
checks has SHA-256
`fd10c2ead864697c9fbbea20566f412042dc4313f825b3af2b740da1a0e1af49`.
Build paths can affect dev-binary identity; reproduce the behavior from source,
rather than treating that local artifact as a published package.

Verified:

- `cargo fmt --all -- --check`: passed.
- `cargo clippy --offline --all-targets --all-features -- -D warnings`: passed.
- `cargo test --offline --all-features --no-fail-fast`: 673 passed across 50
  result groups; zero failed, ignored or filtered.
- Schema7 migration/structure suite: 11 tests, including actual v6-shaped
  migration, unknown effective time, raw-byte preservation, failed rehearsal,
  missing baseline, drift, CHECK/FK/UNIQUE/trigger weakening and REPLACE guards.
- Global version/governance suite: 21 tests, including concurrent guards,
  selective compensation, both-endpoint provenance, ordered/duplicate semantics,
  old serialization, keyed replay, drift, source-audit mismatch, nonbaseline
  backup/restore replay and scoped export privacy. Failure injections include
  version/Reflection/trigger/receipt statements and actual deferred-FK COMMIT.
- Scoped version API suite: six tests for explicit metadata opt-in, bounded
  record count, safe ordered before/after patches, foreign-value redaction,
  inherited/unknown/baseline filtering and durable provenance rechecks.
- MCP subprocess suite: all 67 tests passed, including new version schema,
  rollback, replay, pagination, process restart and generic drift failures.
- Nine Python evaluation-fixture tests and 33 portable-package contract tests:
  passed. The source-bound package build and unpack check also passed as recorded
  below.
- An independent read-only reviewer found no actionable governance, atomicity,
  privacy or compatibility issues. This is source review, not additional runtime
  or platform certification.

The frozen default binary separately passed a synthetic local MCP flow with 32
tools and versions 0–4: updates, ordered diffs, metadata opt-in, selective rollback,
foreign-component preservation without disclosure, stale rejection, receipt
replay, cursor pagination, scoped export, restart and SQLite backup/restore replay.
Existing installed-copy memory and temporal/scope/export workflows also passed.
These workflows use isolated mock configuration and make no remote model calls.
The unchanged fixed evaluator also passed against the final binary: recall and
corrected-answer proxies remain 8/10, with both unsupported paraphrase misses
retained. All 50 separately counted query variants matched expectations, with zero
scope/provenance assertion failures. Mean context bytes remain 1339.9 for recall,
1298.4 after correction and 3361.8 for the API-max budget ablation. These are
synthetic mechanism measurements, not independent real tasks, token savings,
causal model efficacy or a new capacity/latency certification.

An early run overlapped source/binary rebuilds and failed subprocess startup;
a separate controlled old-guard/new-binary probe reproduced structural refusal.
The exact early cause was not captured because the helper discarded stderr. A
legacy test's migration-list expectation also needed row 7. The final aggregate above was rerun after source stabilization and
passes completely; earlier failed logs are not passing evidence.

## Independent actual prior-binary upgrade

An independent probe unpacked the actual schema6 portable binary built from the
base commit above (SHA-256
`81462af016a37a88700a5bc41e9f8b4092c67404f95c44c587e60e82144d2888`).
It created a real v6 database with Events, Claims, ordered duplicate identity,
unsorted commitments and receipts, then migrated it using the new binary.
Original semantic rows and both ingest/reflection operation-log receipt bytes
were preserved, baseline
historical effective time stayed null, old keyed ingest replayed, a guarded v1
write succeeded and baseline previous values remained redacted. The old unkeyed
global reflection is not claimed to support public retry. The pre-migration
backup matched old rows; the old binary diagnosed unsupported schema7 and refused
serve without row changes while still validating its matching backup. Restoring
the v7 database retained version readback. This is actual prior-binary upgrade
coverage with synthetic local data, not a fresh-machine or user-client claim.

## Exact-source portable package check

A local validation snapshot `10f19fbbdaa5558e804552cbf59509d1d0933a5c`
(tree `c9e74dde0380c5b05d96588d22f9e998a96a0117`) built the native Linux x86_64
optimized package from a clean committed-source archive using pinned Rust1.95.
The package SHA-256 was
`49cfc4fc7fbe9caf24846d20c3af1b6a44d83503e4c41f04fe1e61b9e56d6a81`.
This local validation commit is not a claim that the remote branch has advanced.

Exact commit/tree and payload checks passed, followed by actual unpacked-binary
init, unchanged read-only doctor, MCP initialize/tool listing, ingest/restart/
scoped recall, inspection/correction/history, SQLite integrity and backup into
a new path with restored doctor/recall equality. The child PATH exposed neither
Cargo nor rustc; provider was mock, with zero remote model calls and zero namespace
leaks. This was a controlled same-cloud-host installation simulation, not a
real fresh-machine or actual user-client acceptance. Package files and raw
runtime output remain local. Recording this result changes documentation only;
future published heads still require their own exact-head platform CI.

## Remaining gates and non-claims

Full snapshots add storage cost. The read limit bounds returned records, not total
bytes or scan work. There is no semantic retrieval, external-action reversal,
new retention policy, automatic rollback, new global authority or autonomous
execution. Historical schema6 metrics and manifests remain unchanged.

Actual user-client acceptance, fresh-machine timing, final-source native platform
CI, separately authorized live-model efficacy and human release approval remain
open. Windows native CI now explicitly includes the new suites; adding their
names is not evidence of a completed Windows run. No remote branch update,
artifact upload, tag, release or model-provider experiment is claimed by this local verification.
Raw transcripts, binaries, databases and detailed local logs remain local.
