# Combined local candidate validation — 2026-10-20

Validated source head: `182647242e6da4d180c1e3f9db494b6da2f7a0de`. This report and its status links are documentation-only additions. Published base remains `4ac6cfca1a7c69327d8186957c191aa9ed243e93`; this combined candidate is locally committed, not published, merged, released or deployed.

## Included scope

This candidate combines explicit context refresh/output identities, typed caller-reported environment matching, lexical deployment placement preview, unsupported-platform hook-preview safety, explicit Schema 11 owner-channel provenance, optional semantic index v2 and unified task selection, and the explicitly linked minimized Codex result-feedback outbox. Original UI and Chinese/English/Japanese support remain intact.

Cross-slice corrections preserve owner provenance through optional-selection failure/timeouts and schema migration/backup/restore/index reset. The final mandatory task/checkpoint recheck occurs after all other awaited reads, including low-budget paths; late owner changes prevent a stale snapshot being declared resumable. Final time-applicability checks follow that await. This remains an optimistic point-in-time check, not cross-store atomicity or execution authority.

Unified selection uses four bounded indexed source-kind seeks, at most 64 vectors each and one query embedding. A regression with 305 irrelevant claims, 305 irrelevant events and 1,000 malformed foreign-generation vectors retains relevant Episode/active-experience participation. Within-kind suffixes can remain unexamined; coverage explicitly remains partial, not global top-k. Public index-search pagination is unchanged.

## Exact verification

The final combined source used its own build target, without reusing sibling project package artifacts. Cargo used the existing offline registry cache, disabled debug info and incremental artifacts, and ran sequentially to avoid nested-build lock interference.

- `cargo test --offline --all-features --no-fail-fast`: **1117 passed, 14 failed, 0 ignored** across 75 top-level runner summaries, including empty runners. Lifecycle child-process summaries are not double-counted. Exit status was 101; this is **not a full pass**.
- All 14 failures are the existing `local_core` runtime cases: AF_UNIX listener setup returns `Operation not permitted (os error 1)` at the shared fixture before the affected assertions. No other test target failed. Production socket/path safeguards were not weakened to bypass the execution environment.
- Node dashboard tests: **13 passed**.
- Python offline safety/package/probe tests: **87 passed** (9 native-host probe, 17 local-core verifier, 47 portable-package, 14 Windows verifier). These are test doubles and safety-contract checks, not real native-host acceptance.

`cargo clippy --offline --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`, `git diff --check` and the status-sync checker all passed in the final sequential check. The status-sync check covers 33 historical completed-item gates, not completion of the current 24-task implementation plan.

## Remaining external and product gates

- New exact-head Ubuntu/macOS/Windows CI and archive checks have not run. Historical base CI is not inherited. The macOS backup fixture now uses a canonical temporary root and production URL encoding, but the corrected native result is pending.
- Real scoped-core T14 smoke remains blocked at Unix-socket setup in this environment. Native Codex PostToolUse acceptance and result semantics remain unverified; no further bypass attempts were made. Prepared CI coverage is not evidence it ran.
- Local-owner attestation is not authenticated human origin or a signed configured-host channel. Same-user shell access is inside the existing trust boundary.
- Explicit refresh/output receipts do not prove automatic in-turn delivery, host acceptance or model adoption. Result metadata is not proof of success or causal improvement.
- Index rebuilding remains explicit and opt-in; source mutations invalidate it. Default retrieval is unchanged. Windows hook rendering and checked writes remain unsupported rather than emitting an installable placeholder.
- Deployment preview does not inspect the user's real filesystem or prove non-sync ownership. No user computer operation, installation, credential use, paid provider call or real-model quality experiment occurred.

All source changes and local commit history are recoverable independently of publication. The implementation plan remains partially complete; publication/native execution, authenticated host support, installation and separately authorized model experiments are among the remaining gates, not silently completed tasks.
