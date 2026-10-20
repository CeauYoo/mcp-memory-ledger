# Unified task selection (T12, opt-in local implementation)

The host may set `[retrieval] task_context_mode = "unified"` alongside its existing, explicitly enabled local-only embedding policy. Literal remains the default. Enhanced and hybrid modes retain their previous behavior. Request JSON cannot select a mode, provider, endpoint, model or namespace allowlist.

Unified mode uses the same explicit semantic index, now format v2. Build it with the documented `retrieval index-start` and `index-resume` commands. A missing, incomplete, stale or incompatible index does not trigger a rebuild or document embedding during task resume. The result retains mandatory task data and reports the optional retrieval limitation. The query is embedded once per selection, and that transient vector is reused across internal pages. Explicit rebuilds can reuse the existing embedding cache when host configuration enables it. There is no separate vector authority store.

## Source and authority boundary

One ranked set contains:

- Current active claims and events from the existing authoritative projection.
- Persisted Episodes from `experience_episodes`, not legacy `episode_events` aggregates.
- Only each experience candidate's current active version. Historical active versions, pending revisions, rejected and superseded candidates are never current knowledge.

References use `claim:`, `event:`, `experience_episode:` and `experience_candidate:`. Candidate version, content digest, source generation and score appear in `optional_selection`. Memory payloads occur once in `related_memory`, Episodes once in `episodes`, and experiences once in `experiences`. Procedure steps remain inert knowledge. Selection does not activate candidates, adopt experience, verify an artifact, authorize execution, or prove that the host/model received or followed anything.

Every source is exact-namespace scoped. Event/claim roots and Episode source events require their canonical namespace-derived owner. SQL preflight checks types, UTF-8 byte lengths, cumulative transitive provenance payload sizes and edge counts before materializing payloads. Normalized provenance edges must agree with the JSON payload, and SQL identity/status/version/timestamps must agree with the decoded record. Invalid data fails closed; oversized sources are omitted whole, not truncated.

Applicability declarations now accept the two new reference kinds in the existing declaration table. They remain bound to the exact content digest; an old declaration does not establish conditions for a revised experience. Caller-reported task `reported_environment` is optional and defaults to empty. Exact typed mismatches exclude a source; absent facts remain unknown. Free-text conditions and limitations are retained, not automatically proven. These facts are unauthenticated and cannot modify host access or embedding policy.

## Budgets and consistency

Reported task requirements are selected first and never ranked against optional knowledge. The optional query combines the title and whole reported requirements up to 4096 UTF-8 bytes; bounded query construction never truncates the mandatory output. The remaining JSON budget is shared across source kinds, not split into independent memory/experience quotas.

The selector examines four source-stratified windows of up to 64 vectors each (256 total), retains at most 256 KiB of candidate JSON between windows, and selects at most the requested limit (task resume currently eight). Each source hydration is at most 32 KiB with at most 64 provenance edges. Temporary page/vector allocations remain separately bounded by the existing index page/dimension limits. Whole optional records, including their selection/applicability metadata, either fit or are omitted. Smaller later-ranked sources can backfill oversized choices.

Partial traversal is labeled `bounded_partial_index_scan`; unexamined records are not called irrelevant. Each kind has its own reference-ordered prefix window and `kind_coverage` reports its examined count and unexamined suffix. The existing unique `(namespace,generation,reference)` index supplies exact namespace/generation equality plus binary reference range seeks, without `LIKE` or temporary sorting. Large fact/event populations therefore cannot consume the Episode/experience read allowance; ranking and output byte limits still decide which examined records are selected. A relevant record beyond its own kind’s first 64 references may still be missed. Global ranking is asserted only when traversal is exhausted without retained-candidate byte omissions. `not_selected` reports bounded aggregate reasons for source availability/applicability, environment mismatch, candidate retention, result limit and output byte budget. Out-of-scope records are never enumerated. Index build omissions remain separate from read-time selection omissions.

The existing 100–5000 ms host deadline covers optional work. A fraction is reserved for final revalidation. Timeouts preserve mandatory context. Selected candidates receive bounded current-version/content/status rechecks, source generation is rechecked, and task/checkpoint revisions are rechecked. A changed/unavailable optional source snapshot drops the optional bundle; a changed task/checkpoint preserves the existing fail-closed resumability behavior. Mandatory and optional reads are not advertised as one atomic database snapshot.

## Index format and compatibility

Format v2 adds persisted Episodes/current active experiences, scope indexes and invalidation triggers for all five experience/provenance tables. The authoritative schema version is unchanged by this feature; this must coexist with independently introduced authoritative migrations.

A canonical format-v1 extension is recognized as a valid disposable old extension, but `index-start` refuses to overwrite it. Explicitly run `index-reset`, then rebuild. There is no lazy migration, deletion of authoritative rows, or reuse of old cursors. Known partial/corrupt extension teardown retains the prior conservative recovery rules; unknown dependents still block removal.

A source omitted because it is not yet valid records its earliest `valid_from` boundary in checksummed run state. When that boundary is reached, the run becomes stale even without a database write, requiring explicit rebuild. The original assessment's boundary is retained, avoiding a clock-crossing race between eligibility assessment and boundary bookkeeping. Expiry is rechecked at read/selection time.

## Offline validation scope

New tests: `selection_source_reads` and `unified_selection`. Existing semantic-index, enhanced-recall, task-resume and experience suites are also regression targets. All embeddings in the new tests are synthetic; no model service, credentials, user computer, deployment or publication is involved. Isolated passing tests are not a claim of combined schema-11/owner-import integration, native Windows execution or live-provider quality.

### Isolated validation record, 2026-10-20

On the isolated `ee45804` base plus this slice, 115 library tests and 148 tests across the 12 targeted integration suites above passed (263 total). Strict Clippy passed for the library and `unified_selection`, `selection_source_reads`, `semantic_index`, `task_resume_enhanced`, `enhanced_recall`, and `task_ledger`; formatting/diff checks and the 33-item status-sync gate passed. All 202 relative links across the changed entry/contract/testing documents resolved. Builds used a dedicated target with debug information and incremental compilation disabled. This record is not combined schema-11/owner-import acceptance, full cross-platform CI, or live-provider certification.

Independent review follow-up: final applicability time validation now occurs after all asynchronous rechecks, including the task's last source-generation read. A near-future `valid_until` regression covers expiry during callback/final assembly with no database write. New experience provenance plans no longer sort before their 65-row preflight limit; the module-local plan regression checks the exact preflight/materialization query shapes for absence of full scans and temporary sorting. Independent read-only SQLite probing observed the same 2,332 VM steps for the transitive preflight with 100 and 10,000 normalized event edges. Neither this observation nor the regression is a general performance guarantee for arbitrary SQLite environments.

After these review fixes, the same isolated command passed 116 library tests and 149 targeted integration tests (265 total). The additional tests are the no-sort source-plan unit regression and final-assembly expiry regression; the earlier 263-test count belongs to the preceding checkpoint. Combined owner-import/schema-11 validation is still a separate parent integration gate.

The reviewed follow-up also passed the same strict focused Clippy target set, formatting and diff checks, and the 33-item status-sync gate. No additional live or cross-platform claim is implied.

## Final assembly freshness (2026-10-20 integration)

The task/checkpoint recheck runs after every other awaited optional/source read, including when the optional byte budget is skipped. An owner requirement or checkpoint arriving during those reads makes the retained earlier mandatory snapshot non-resumable instead of silently mixing versions. Applicability-time validation follows that last await. This is an optimistic point-in-time check, not a cross-store atomic snapshot or a guarantee against changes after return. The execution host must still enforce its own current permissions. Regression tests include late owner import during source-generation validation and during experience-use reads with no optional budget, plus owner-provenance retention under optional failure/timeout.

Source-balanced follow-up: public ID-pagination remains unchanged; the internal kind helper emits `has_more` without a misleading ID cursor. No derived or authoritative schema/version change is needed. Regression includes 305 irrelevant claims and 305 irrelevant events ahead of a matching Episode/current active experience, plus 1,000 malformed foreign-generation vectors. Exact-query-plan verification asserts the canonical three-column unique index, both equality keys, bounded reference range and absence of full scan/temp sort.

The source-balanced follow-up passed the exact index-plan unit test and 34 targeted integration tests (`semantic_index` 19, `task_resume_enhanced` 5, `unified_selection` 10), plus strict focused Clippy, formatting/diff checks and the 33-item status-sync gate. In a separate in-memory probe of the exact SQL, increasing foreign-generation rows from 1,000 to 100,000 left claim-window work at 1,719 VM steps and Episode-window work at 57 steps; this verifies the generation-key seek in that SQLite environment, not a universal performance guarantee. No source/derived schema version change, real model call, task-assembly modification or combined-acceptance claim is included in this follow-up.
