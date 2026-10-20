# Typed environment matching: local follow-on slice

Base: the exact restored third-batch source published as `4ac6cfca1a7c69327d8186957c191aa9ed243e93`. This change is developed and committed separately from remote publication and CI.

## Implemented contract

- Optional typed environment requirements in persisted applicability declarations; exact key/value conjunction with strings, booleans and signed 64-bit integers.
- Optional per-request reported environment in enhanced recall. Known mismatches are omitted before document embedding and rechecked after hydration. Missing facts remain unknown; free-text conditions are not interpreted.
- Separate `environment_match` annotation, explicitly describing reported-value comparison rather than authentication, authority or natural-language applicability.
- Typed declaration fields default empty and are omitted when serialized empty, preserving literal pre-slice request replay. No database schema migration is needed for this optional JSON payload extension.
- Second-pass applicability exclusions are counted consistently; metadata annotations are reset before each reread to avoid carrying stale match evidence.

## Verification

Focused cloud tests: 14 enhanced recall tests, 5 task-resume retrieval tests, 22 SQLite lifecycle tests, and 3 typed-environment domain unit tests passed (44 top-level tests; lifecycle child-process output is not counted again).

Coverage includes known mismatch, unknown missing facts, exact types, invalid bounds/values, conjunctive mismatch precedence, legacy persisted request replay, optional wire compatibility, exclusion before synthetic document embedding, declaration changes between ranking and hydration, stale content-digest annotations, and free-text conditions remaining unknown.

An independent read-only review identified the second-pass counter inconsistency. It was corrected and regression-tested. The review is focused, not comprehensive security certification.

Strict all-target/all-feature Clippy, final formatting and diff validation passed against the final source. Full T20 cloud regression subsequently completed: 1009 passed, 14 failed, 0 ignored when counting the final result of each top-level target (nested lifecycle subprocess summaries are not double-counted). All 14 failures are the local_core AF_UNIX listener EPERM restriction. This is not a full pass. Combined-slice regression and new exact-head native CI remain separate gates; historical third-batch CI does not certify this change.

## Boundaries

Inputs are caller reports, not authenticated host observations. Task-resume currently supplies no environment map, so missing typed conditions remain unknown. This is a usable exact-match slice of T20, not all remaining plan tasks. Existing Rust callers constructing public structs need to initialize the new fields; JSON callers can omit them. No provider inference, user Mac access, deployment, merge or release occurred.

## macOS fixture correction carried forward

Third-batch exact-head CI run 39383250045 exposed `task_ledger::full_database_backup_restore_preserves_tasks_checkpoints_and_receipts` using the noncanonical macOS `/var` temporary root. The fixture now canonicalizes its synthetic temporary directory before deriving database/backup/restore paths. No production symlink rejection was weakened. The targeted Linux fixture check passed after this test-only change; macOS native verification remains pending the next exact-head CI.
