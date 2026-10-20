# Semantic index and unsupported-preview integration — 2026-10-20

This local candidate extends the frozen follow-on slice `88d20c7` with explicit resumable semantic indexing and a fail-closed Windows/unknown context-preview correction. It is not yet published and does not include the separate owner-attested import or host-result feedback implementations.

## Verification

Package artifacts were cleaned before changing working copies; dependency cache reuse did not reuse another sibling's compiled package. The combined command selected library tests plus semantic_index, enhanced_recall, codex_context_adapter, deployment_path_preview, sqlite_lifecycle, task_resume_enhanced, workspace_adoption and database_backup: **229 passed, 0 failed** at the top-level runner scope. The earlier three-slice full-suite result is historical and not substituted for these new targets.

Strict all-target/all-feature Clippy initially found `items_after_test_module` in context CLI preview tests. The test module was moved to the end without changing production behavior; affected preview tests and final strict Clippy passed. Formatting and diff checks passed. Native platform and full combined regression remain separate pending gates.

## Capabilities and limits

The optional derived index traverses active claims/events beyond the previous bounded scan, persists source/profile-bound rebuild progress, and supports explicit start/resume/status/search/reset. Search is paginated and page-local ranked, not an automatic global top-k query. Existing enhanced recall and task modes are unchanged. Current and stale declaration metadata is returned with explicit uncertainty; index search only evaluates time intervals, not T20 environment matching. Future-valid omitted records require an explicit rebuild when they become valid. Derived checksums detect inconsistency, not authenticated producer identity.

Windows/unknown context preview no longer produces an installable placeholder command: it emits null hooks and a stable unsupported reason while retaining diagnostic argv. POSIX rendering is unchanged. Pinned Codex supports Windows hooks, but this preview's inputs cannot establish effective shell overrides or the host's captured COMSPEC; runtime rendering/checked writes remain unsupported, not falsely certified by lexical strings.

No deployment, user-machine access, merge, provider inference, or new remote publication occurred. Local commits and bundles remain independent of the pending upload confirmation.
