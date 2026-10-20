# Owner provenance and semantic index compatibility — 2026-10-20

This locally committed candidate combines explicit schema11 local-owner attestation with the previously integrated semantic index, typed environment matching, context refresh/output identities and noninstallable unsupported-platform previews. Publication remains queued pending confirmation; no remote branch was changed by this integration.

## Verification scope

Combined targeted checks passed **282 top-level tests** across the library, owner_attestation, semantic_index, task_ledger, codex_context_adapter, enhanced_recall, task_resume_enhanced, sqlite_lifecycle, schema7_migration, workspace_adoption, database_backup and mcp_session_policy targets. A new cross-slice compatibility test also passed, for **283 selected tests**. Strict all-target/all-feature Clippy, formatting and diff checks passed after final changes.

The additional test starts with a complete optional index, models a schema10 source database, performs the explicit schema11 migration, imports an exact protected requirement, performs real production backup/restore, resets only the derived index, and verifies preserved provenance and ordinary-mutation refusal. All data and inference are synthetic. It does not establish native runtime or human-authorship evidence.

Synthetic owner/index fixtures now use canonical temporary roots and the production SQLite URL encoder. This prevents the known macOS `/var` alias failure pattern and avoids assuming POSIX URL formatting on Windows; production path confinement is unchanged. Windows CI now explicitly selects owner_attestation, owner_semantic_compatibility, semantic_index and deployment_path_preview targets. Those newly configured native runs have NOT executed locally and remain pending publication/CI.

## Boundaries

Owner-channel attestation is not authenticated human input, hostile-same-user isolation, or execution permission. Semantic indexing is still the v1 claims/events-only, explicit CLI implementation in this candidate; unified typed v2 selection and automatic host-result feedback are separate ongoing slices. The earlier 1029/14 three-slice full regression is historical; a new complete-suite/native result must bind the final combined source rather than inherit that result.
