# Follow-on slice validation — 2026-10-20

Publication base: `4ac6cfca1a7c69327d8186957c191aa9ed243e93`. Local integration combines typed environment matching, explicit context refresh/output identities, read-only target-platform deployment placement preview, and the macOS task-backup fixture correction. Semantic-index and owner-attested import work are NOT included in this candidate.

## Implemented, bounded capabilities

- T20: exact typed caller-reported environment matching in enhanced recall. Missing facts remain unknown; a known mismatch is excluded before document embedding and after authoritative rehydration. Natural-language conditions, trusted host observations and automatic task-resume environment collection remain unsupported.
- T07/T08: explicit scoped controller refresh with fresh status/version/snapshot checks, stable content fingerprints, exact newline-inclusive stdout identities and prepared/written receipt stages. Explicit transactional adapter-state schema 1→2 migration preserves old receipts without inventing hashes. No automatic in-turn push or independently verified host-accepted receipt.
- T24: explicit macOS/Windows/Linux lexical preview for five placement roles. Known sync markers/invalid paths reject; ordinary paths remain unknown, never ready-to-deploy. Oversized echoed inputs are omitted with their byte length. No actual target filesystem/sync ownership inspection, installation or deployment.
- The synthetic `task_ledger` backup fixture now canonicalizes the temporary root before deriving its three file paths. This addresses the `/var` alias failure in third-batch macOS CI without relaxing production path confinement.

## Exact local verification

A new dedicated target directory was used for the combined source. No compiled package artifacts from sibling working copies were reused. Dependencies came from the existing local registry cache; all Cargo commands used `--offline`. Development/test debug info and incremental artifacts were disabled to bound disk usage, without changing test selection or assertion behavior.

`cargo test --all-features --no-fail-fast`: **1029 passed, 14 failed, 0 ignored**, counting the final summary of each top-level runner (not double-counting lifecycle child-process summaries). All 14 failures are in `local_core` at AF_UNIX listener creation denied with EPERM by this cloud execution environment, before the affected integration assertions. This is NOT a full pass or native host acceptance.

`cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`, and `git diff --check` passed. Node dashboard tests: **13 passed**. Repository status-sync checker passed its **33 historical completed-item gates**; those gates are not a claim that the current 24-task plan is complete.

Final source includes the additional broken-pipe, migration-conflict/damaged-schema, refresh/pause/revocation, typed environment race/legacy replay, and bounded path-output tests. Independent focused reviews found and helped correct the second-pass applicability counter and output-boundedness issues; this is not exhaustive security certification.

## Native CI and remaining scope

Historical run `39383250045` belongs to base `4ac6cfc`, not this candidate: Ubuntu succeeded, macOS failed the now-corrected task-backup fixture, and Windows terminal status is tracked separately. Native platform, portable archive and real Codex synthetic-host checks must bind the new published exact head. Do not inherit old green checks or equate synthetic delivery with model understanding.

All changes have cloud-local commits and a recoverable bundle independently of remote publication. No merge, release, paid provider call, user credential use, user Mac operation, or deployment occurred. The project remains a technical MVP; authenticated human-origin import, automatic supported-host feedback and the remaining plan gaps are not marked complete by this slice.
