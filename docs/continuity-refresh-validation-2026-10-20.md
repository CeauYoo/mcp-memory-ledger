# Explicit controller refresh and output receipts: validation

Date: 2026-10-20. Scope: local cloud-only follow-on to the restored third slice. This remains a technical demo/MVP and **partial T07/T08**, not a completed independent host-delivery or automatic in-turn refresh system.

## Implemented

- Explicit `codex-context refresh` for an already-bound session, fresh scoped bridge reads, bounded full context or caller-fingerprint cache response. No background watcher and no new native hooks.
- Cache hits cannot bypass active state, current version, mandatory snapshot, full-envelope budget or artifact observations. Content identity includes the binding, requirements, checkpoint, optional selection and artifact outcomes.
- Adapter-only schema 2 and explicit transactional schema-1 migration. Expected source tables/columns are checked; unknown versions, damaged schema-1 tables and conflicting additions fail rather than replace state. Existing receipts are preserved without fabricated historical output hashes.
- Exact newline-inclusive UTF-8 output SHA256, content fingerprint, kind and byte count saved before stdout; stage advances only after successful write/flush. Broken-pipe output remains prepared/core_built, even after reopening. No generic delivered setter.

## Verification status

- Initial full Linux cloud regression: `cargo test --offline --all-features --no-fail-fast` completed **1008 passed, 14 failed, 0 ignored** across 68 top-level runners. Every failure was in `local_core` at listener startup (`Operation not permitted`, AF_UNIX); no native-refresh assertion ran in that environment. This run preceded the last migration/broken-pipe hardening and is not presented as a full regression of the final source.
- Final-source strict `cargo clippy --offline --all-targets --all-features -- -D warnings`: passed.
- Final-source formatting and `git diff --check`: passed. Status sync: 33 checks passed. Unchanged dashboard Node suite: 13 passed.
- Final affected-target rerun: `cargo test --offline --all-features --lib --test codex_context_adapter --test local_core --no-fail-fast` completed **110 library tests passed, 14 adapter integration tests passed, 1 local-core test passed and 14 local-core tests failed at AF_UNIX listener startup**. The final library run includes all 12 adapter unit tests, including the actual anonymous broken-pipe and damaged/conflicting migration cases. Exit 101 is solely the environment-blocked local-core target; its refresh assertions were not executed.
- Build scheduling: one queued run was cancelled to release a shared-target lock inversion with a separate suite's recursive shell-wrapper build. The final affected run above ran afterward in an exclusive slot. No test result was manufactured from the cancellation.

Counts use the last test-result summary in each top-level `Running` / `Doc-tests` block. Three nested one-test `sqlite_lifecycle` subprocess summaries are excluded (raw summation would incorrectly report 1011 passes). Full regression and final affected-target checks are separate runs; they must not be summed. An environment-denied assertion is not a pass. The native integration code covers first/unchanged refresh, full hook replay after cache hit, paused/revoked refresh errors with empty stdout, and exact output linkage, but requires a permitted socket environment to execute.

No real paid model requests, credentials, user-computer access, deployment, merge or publication were performed. Native host acceptance, model adoption and result efficacy remain unverified/unknown. UI and its Chinese/English/Japanese resources were not changed. The adapter database remains outside the main ledger backup/export.

## Remaining work

T07 still needs a supported host controller's automatic in-turn polling/push and acceptance integration. T08 still needs independently sourced host/session/context evidence in a trusted observation path; adapter stdout and caller-provided JSON cannot establish it. Existing synthetic host-gate results for other commits are not inherited. Windows native hook rendering/installation is unchanged and remains outside this slice.
