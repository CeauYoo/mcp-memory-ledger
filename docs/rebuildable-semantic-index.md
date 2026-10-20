# Explicit rebuildable semantic index (T18, partial MVP)

Format v2 of this optional derived index traverses **all active claims, events, persisted Episodes, and current active experience candidates in an allowed namespace**, in resumable bounded batches. Unlike `cache-rebuild`, it actually generates and persists document vectors. It is not an ANN service, an automatic background job, or a global top-k retrieval route. Existing enhanced/hybrid recall and task context still use their bounded candidate window; they are not silently switched to this index. The host-only [unified task mode](unified-task-selection.md) explicitly uses bounded index traversal and a single optional selection budget.

## Host permission and explicit commands

Use the existing `[local_embedding]` configuration: `enabled = true`, `local_only_declared = true`, an exact namespace allowlist, loopback endpoint, and pinned model/digest/dimensions/prefixes. No separate index permission list or provider override exists. CLI JSON cannot enable embedding, expand the allowlist, select another provider, or change a profile. Loopback alone does not prove the model service stays local; the operator remains responsible for that declaration. See [local embeddings](local-embeddings.md).

With that host configuration and an already initialized current database:

```sh
agent_llm_mm retrieval index-start --json '{"namespace":"project/example"}'
agent_llm_mm retrieval index-status --json '{"namespace":"project/example"}'
agent_llm_mm retrieval index-resume --json '{"namespace":"project/example","generation":"<returned generation>","batch_size":4}'
agent_llm_mm retrieval index-search --json '{"namespace":"project/example","generation":"<returned generation>","query":"coffee preferences","page_size":32}'
```

- `index-start`: explicitly installs the optional derived schema if absent and replaces this namespace's rebuild. No inference. Other namespace rows remain intact. Restarting replaces the previous usable run; until completion, searches fail explicitly rather than using partial coverage.
- `index-resume`: explicitly authorizes one document inference batch of **1–4 sources**, default 4. Repeat with the returned generation until `complete=true`. A source consumes the cursor even if it cannot be indexed; `examined`, `indexed`, and `omitted` expose that difference. Failure or cancellation does not advance the batch cursor or partially commit its vectors. A completed, non-stale resume is a no-op without inference. Format v2 records the earliest future-valid boundary of omitted sources and marks the run stale when that boundary is reached, even without a source write.
- `index-status`: no inference, schema installation, or repair. A stale run reports `stale=true` and `stale_rebuild_required`; missing/corrupt state is an error.
- `index-search`: explicitly authorizes query embedding and reads **1–64 vectors**, default 32. Repeat with the exact returned `next_cursor` object, same query and generation. A different query, generation, profile or query-vector digest is rejected. `hits` are sorted by cosine score **within the page only**, not a global top-k. Walk all pages and merge scores for full-index ranking. Do not stop after a good-looking first page and call it exhaustive.
- `index-reset`: explicitly removes all known semantic-index extension objects and all namespaces' vectors/runs, with no inference or authoritative row changes. Use it to recover a partial/corrupt known installation, then start/resume again. Unknown/conflicting extension objects are not deleted automatically; reset fails and rolls back if they prevent validation. Such conflicts require explicit operator repair.

The library API is `adapters::sqlite::semantic_index::SemanticIndex::{new,start,resume,status,search_page}` and `adapters::sqlite::reset_semantic_index`. Production CLI builds the existing Ollama adapter; tests inject deterministic synthetic ports with the exact host-config profile. The vector cache is independent and is not required or populated by these commands.

## Coverage, scope and stale data

The cursor is a stable per-kind rowid keyset, not an OFFSET or a fixed 64-record prefix. Dedicated scope-first indexes support bounded seeks without per-page full-partition sorts. Namespace determines the canonical owner. Both source traversal and current-record hydration enforce owner and namespace before materializing payloads. Oversized provenance is omitted, never truncated into apparently complete evidence.

A completed zero-omission format-v2 build reports `all_active_claims_events_persisted_episodes_current_active_experiences_at_source_generation`. Coverage includes active claims, events, persisted Episodes, and the exact current versions of active experience candidates. It remains narrower than “all memory”: reflections, tasks, inactive candidates, and historical candidate versions are not directly indexed. A build with any oversized or temporally inapplicable source reports `completed_with_omissions`. Applicability conditions/environment remain declarations, not proven facts or permission. Time applicability is checked before document transmission and on every search read. A not-yet-valid source omitted during rebuild requires a later rebuild to become indexed; this does not happen automatically. Time-based read omissions are also reported per page. `complete` records completion of that rebuild, not continuous temporal completeness: advancing the clock does not change the source generation or refill omitted rows. The checksummed `next_valid_from` boundary makes the run stale when an omitted future-valid source becomes eligible, requiring explicit rebuild even without a source write. The page includes `applicability_policy`, and every hit includes `applicability` plus the available `declared_applicability`. A matching declaration is labeled `declared_conditions_and_environment_not_evaluated`, never “satisfied”; a mismatched digest is explicitly unknown. The index-search API has no `reported_environment`, and does not run T20's enhanced-recall environment filters. After T20 integration its typed environment declarations are returned as metadata only. Existing build counters describe indexed sources, while per-page omissions describe additional read-time exclusions; neither proves current applicability or permission.

Every insert/update/delete in claims, events, evidence links, episode links, reflections, recall applicability, persisted Episodes and their source links, or experience candidate roots, versions and source links changes a derived source-generation nonce. This is **conservatively global**: even an unrelated namespace mutation makes existing runs stale. All such stale builds/searches must restart. There is no maximum-rowid/count shortcut that misses edits or deletions. A mutation during inference causes the compare-and-swap commit to fail; no stale vectors/cursor advancement are published. Generation checks also reject pre-reset workers and old search cursors. Rapidly changing ledgers can require retries; there is no claim of snapshot rebuilding across concurrent writes.

Keys bind namespace, rebuild generation, current content/provenance SHA-256, index-format version, and SHA-256 of the complete embedding profile (including model digest, dimensions and both prefixes). Hydrated authoritative content is rehashed before returning any hit. Deletions, inactive claims or candidates, candidate-version changes, owner/namespace changes, mismatched content and applicability changes cannot be treated as current indexed evidence. Vectors are ranking hints only, never execution authority.

## Derived schema and recovery boundary

This is an **explicit optional schema extension**, like the existing explicitly installed vector cache, not a mandatory authority migration. The authoritative `user_version` remains unchanged by this extension because no authoritative table shape, row, or required startup object changes. Only `index-start` transactionally installs these optional tables, triggers and performance indexes. Existing databases without any semantic objects remain valid. No read/serve/startup automatically installs the extension.

Canonical validation includes every optional table, trigger and index; partial, modified and foreign extension objects fail closed. The lifecycle inspector validates the optional extension when present, and workspace adoption recognizes only its canonical triggers/tables. A global teardown reserves a lifecycle write transaction, disables foreign-key cascades for schema removal, and validates the remaining authoritative database before committing; failures roll back. An unknown extension is not authority to delete it. Teardown preflights attached unknown triggers/indexes too, because dropping a table would otherwise implicitly remove them.

Source triggers write only the disposable generation row. Vector INSERT/DELETE triggers maintain an O(1) count checked against the rebuild's expected count; vector UPDATE invalidates the run. Persisted cursor, counters, profile/source identifiers and completion state are bound by a control checksum, so a damaged cursor cannot silently skip records. A completion marker is additionally checked against a bounded next-source seek. Vector type, dimensions, byte length and checksum are checked before use; metadata/DDL reads and vector materialization have allocation bounds. The format is corruption-detecting, not an authentication scheme against an attacker who can rewrite the entire SQLite database and all checksums.

Full database backup/restore preserves the optional index and in-progress cursor. Scoped ledger exports omit derived vectors/profile state; a destination must rebuild. Teardown/start never changes authoritative memories or applicability declarations.

## Limits and verification

A rebuild page materializes at most four authoritative records, each at most 32 KiB including provenance; document prefixes and transport remain under the existing adapter's 256 KiB request budget. A search page materializes at most 64 vectors (8192 dimensions maximum, at most 2 MiB) and 64 bounded authoritative records, plus at most 64 bounded applicability declarations (16 KiB each). Query text is at most 4096 bytes. Resume/search have an overall timeout of the configured provider timeout plus five seconds. Data-page keyset seeks are bounded; installation/index creation and explicit teardown are corpus-sized maintenance operations, not bounded inference batches. Index disk storage grows with corpus size; there is no misleading 4096-row cache eviction or claim of bounded total disk storage.

Offline regression suite:

```sh
cargo test --test semantic_index
cargo test --test enhanced_recall --test embedding_cache --test workspace_adoption --test scoped_ledger_export --test database_backup
```

These tests use only synthetic vectors, source records and databases. They cover records beyond the old 64-source window, reopen/resume, paginated search, SQL seek plans, stale commits, profile changes, namespace permissions, applicability/oversize omissions, corrupt vectors/state/schema, explicit reset, CLI dispatch without inference, adoption, export, full backup and restore. They do not certify a real Ollama installation, model quality, or any paid/provider operation.

### Validation provenance

Validation is performed from the isolated semantic-index working copy with package artifacts rebuilt after cleaning `agent_llm_mm` from the shared target. Dependency artifacts are reused, but no sibling working copy's library/test outputs are treated as evidence. A subsequent combined integration build may supersede these isolated results. No real model or provider was called during implementation or tests.

2026-10-20 index-and-integrity slice result: `semantic_index` 18/18 passed; relevant baseline targets (`enhanced_recall`, `embedding_cache`, `workspace_adoption`, `scoped_ledger_export`, `database_backup`) 52/52 passed; `cargo clippy --lib --test semantic_index -- -D warnings`, formatting and diff checks passed. The independent focused review's cursor-integrity and unknown-dependent teardown findings were fixed and regression-tested. This is not a claim that the entire cross-platform suite or a live embedding provider was certified.

The subsequent applicability-output annotation follow-up passed all 19 semantic-index tests and the same strict focused Clippy, formatting and diff checks. Its new regression verifies unknown, current-digest and stale-digest declarations without claiming environment satisfaction. The 52-targeted-regression baseline above belongs to the preceding index-and-integrity slice; a combined T20 integration run remains separate evidence.

Format-v2 note: canonical v1 extensions require explicit `index-reset` followed by rebuild; reads never silently upgrade them. The unified-selection slice adds five experience/provenance trigger families and typed source hydration. Historical v1 validation counts above remain historical evidence, not the v2 acceptance record.

Unified task selection uses a separate internal source-balanced read: at most 64 reference-ordered vectors for each of claim/event/persisted-Episode/current-active-experience, with one prepared query. It reuses the existing `(namespace,generation,reference)` unique index and does not change public ID-pagination or schema. Per-kind remaining coverage is explicit; large fact populations no longer hide entire later source kinds, but within-kind suffixes can remain unexamined.
