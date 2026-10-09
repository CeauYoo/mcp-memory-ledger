# Evidence-bound feedback, indexed recall, and experience candidates

This local-first technical MVP implements a bounded runtime subset of the A–E continuation of the supplied planning discussion. It is not a production release or autonomous controller. [Original-plan baseline matrix](plans/2026-10-09-original-plan-traceability.md) preserves every material requirement; this page describes the additive runtime slice, not completion of every research/product gate.

Schema6 subsequently adds [temporal metadata](temporal-metadata.md), [independent Reflection scopes](reflection-scope-history.md), and [read-only scoped export](scoped-export.md). Its [separate results](plans/2026-10-09-schema6-results.md) supersede the schema5 temporal/scope boundaries; historical evaluation remains tied to its own source manifest.

## Start and migrate safely

Build `agent_llm_mm`, configure the SQLite path, then use explicit `init` for a new database or `migrate` for a supported older database. Schema **v5** introduced durable feedback candidates, richer Episodes, versioned experience candidates and derived retrieval objects. Serving never migrates implicitly. Migration keeps backup, restore rehearsal, writer reservation, canonical structure/FK/count readback and commit atomicity. Restore a backup to a new path, inspect it, and switch deliberately. Never overwrite a live database in place.

`doctor --read-only` now also deeply inspects the retrieval projection and postings without repairing them. Durable schema damage remains fail-closed. Missing/broken derived search objects do not invalidate ledger facts: recall reports a degraded literal strategy; explicitly call `rebuild_retrieval_index` to restore the index. This read fallback does not promise healthy writes: source triggers can reject ingestion or correction while their derived targets are broken. `inspect_retrieval_index` performs the same read-only deep check. Both index tools operate on the whole local database, without a namespace parameter. Rebuild recreates derived objects transactionally and never rewrites Events, Claims, identity or commitments.

Runtime SQLite explicitly retains the previously-used SQLx maximum of ten pool connections and five-second busy timeout. It preserves the existing journal mode rather than silently enabling WAL. The implementation does not claim WAL/checkpoint/recovery qualification or production throughput. Explicit runtime defaults and measurement rationale are recorded in the evaluation report; migration writer admission remains fail-fast with bounded reader-lock wait after reservation.

## C: external feedback to a recalled correction

The new feedback-candidate, experience and recall tools are provider-free. Legacy `ingest_interaction` retains its best-effort automatic-reflection hook, which can invoke the configured model when triggered; use the explicit mock configuration for a fully offline end-to-end demonstration. Use a concrete namespace such as `project/demo` throughout the scoped workflow.

1. `ingest_interaction`: record the original Event and Claim. Save returned raw IDs; use `claim:<id>` and `event:<id>` canonical references in later calls.
2. `get_feedback_target_version(namespace, target_claim_reference)`: obtain `target_version`, `status`, and `current_object`.
3. Record a new Event with `feedback`. Set `observed_target` to that exact canonical Claim reference, `observed_version` to the returned fingerprint, `expected` to the old object, and `actual` to the proposed replacement object. Include producer, verification method, result and limitations truthfully.
4. `propose_feedback_candidate`: provide namespace, target reference, `expected_target_version`, `replacement_object`, `evidence_event_ids`, summary and request_id. This only persists a proposal.
5. `validate_feedback_candidate`: provide namespace, candidate_id and a new request_id. Inspect state and validation reasons.
6. `commit_feedback_candidate`: provide namespace, candidate_id and a new request_id. It revalidates inside the existing `run_reflection` transaction, then atomically writes the old status, replacement Claim, evidence links, Reflection, committed candidate state and durable receipt.
7. `recall_memory` / `build_task_context`: retrieve the current replacement and source references. `get_memory` and `get_reflection_history` retain the superseded conclusion and correction lineage.

`get_feedback_candidate` inspects state/reasons/result IDs. `reject_feedback_candidate` records a reason and ends the candidate. No-new-evidence suppression returns an identical existing proposal or rejects a different proposal against the same target version unless a new evidence Event is added. Rejected candidates stay rejected.

The v1 support contract is deliberately mechanical and narrow: preserve subject, predicate, mode and scope; change only the object; require every listed observation to have exact target/version/expected/actual alignment, same canonical owner/namespace, caller_reported or tool_reported source, and definitive passed/failed verification. Each feedback `evidence_refs` entry must also appear in the candidate evidence manifest and satisfy the same checks. Model assertions, missing feedback, inconclusive/not_performed checks, mismatches and absent evidence cannot commit. Nonempty free-text limitations produce `limitations_require_review` and block this v1 automatic validation/commit path: the service cannot mechanically determine whether arbitrary natural-language restrictions permit the proposed generalization. Unsupported natural-language support remains rejected rather than guessed.

These checks establish **reported evidence-contract alignment**, not producer authentication or general semantic truth. A caller can lie in a source label; the service does not attest a tool execution. Limitations remain inspectable evidence text; they are not silently treated as satisfied. An empty limitations list is a caller report that no restrictions are supplied, not authenticated proof that none exist. Existing general `supersede_memory` remains an explicit user-directed correction API with structural evidence checks; it is not retroactively described as externally verified.

### Version and retry semantics

- Claim target version is SHA-256 of current typed Claim content and status, not a monotonic mutation counter. A changed target is rejected at commit. A historical A→B→A overwrite through low-level legacy APIs is not detected merely by content hashing.
- Claim corrections are still single-successor state-CAS transactions: active/disputed may transition, superseded is terminal; there is no correction branch from the same old Claim.
- New feedback/experience writes require request_id, scoped to namespace and operation. Retry the same key with the same typed content after a lost response. Changed content under the same key fails; committed retries retain original result IDs.
- Successful write receipts are mandatory transactional audit, not disposable diagnostics. Retain them for replay correctness. This does not establish tamper-proof audit retention or permission control over direct database writers.

## D: indexed task-related memory

`recall_memory` keeps the explicit namespace/query/limit interface. FTS5 trigram provides candidates for terms of at least three Unicode characters; shorter terms use an exact scoped fallback. Every result is rechecked against original ledger text and scope. ASCII matching is case-insensitive, CJK and non-ASCII case remain literal. Quotes, percent signs, underscores and code punctuation are not query operators. Up to eight distinct whitespace terms are ORed; query max512 UTF-8 bytes, limit1..100. This is substring retrieval, not Chinese word segmentation, stemming, typo tolerance or semantic embeddings.

The derived projection has its own stable integer IDs, so arbitrary text ledger IDs and SQLite VACUUM do not invalidate it. Transactional triggers track Event/Claim mutations; only active Claims enter the Claim projection. Explicit rebuild reconstructs source projection and postings from durable facts. Embedded-NUL legacy source text has a documented exact fallback rather than being assumed portable across SQLite tokenizers.

Selection splits the candidate budget between Claims and Events; Claims receive an odd extra slot and unused slots are filled from the other type. Within each type, matched-term count sorts first, then known recording time, then stable ID. Schema6 new Claims have recording time; historical unknown times remain null and sort last. Caller observation time is distinct and does not control recency. Interleaving avoids a Claim-only prefix consuming the context budget. Standalone `search_memory` browse ordering is preserved; schema6 mixed-type queries now order per-type candidates consistently before LIMIT.

Responses explain matched terms, active-Claim versus historical-Event status, time basis, type-selection policy, unavailable candidates and degraded-index warnings. An active Claim is not independently verified; an old Event is not a current authoritative conclusion. Disputed/superseded Claims are excluded, not silently described as current. Task context additionally exposes bounded observable status diagnostics and source-linked rich Episodes; arbitrary semantic conflict discovery and effective-expiry semantics are not inferred. See [context contract](context-diagnostics.md).

`build_task_context` measures the complete compact result JSON, including metadata, explanations and its own byte-count field. Oversized records are omitted intact; later smaller records may fit. Byte cap excludes JSON-RPC/MCP envelope and is not a tokenizer budget. Concurrent retrieval is not a transactionally pinned snapshot: changed/deleted full records are counted as unavailable.

Runtime recall cheaply checks index object definitions and falls back on missing objects/SQL errors. It does not stream all postings to certify arbitrary out-of-band tampering on each query; deep read-only inspection detects projection/posting inconsistencies. Run inspection after untrusted direct database manipulation and rebuild if needed.

## E: richer Episodes and inert knowledge/procedure candidates

`record_episode` persists `{namespace, episode_id, request_id, content}`. Content includes title, objective, actions, observations, outcome, lesson, limitations and source_event_refs. At least one observation and existing same-scope source Event are required. `get_episode_detail` and `list_episode_details` read these records. They coexist with historical Episode membership projections; legacy `get_memory(record_type=Episode)` is not silently redefined.

`propose_experience_candidate` accepts `{namespace, candidate_id, request_id, content}`. Content includes kind (`semantic` or `procedural`), title, statement, steps, limitations and source_episode_ids. Semantic content has no steps; procedures require at least one step. The candidate starts pending and contains caller-authored synthesis linked to evidence. No model generation or universal truth proof is implied.

- `get_experience_candidate`: current version by default; optional version reads immutable history. A historical version may retain its old active status; only an active current head is eligible for recall.
- `list_experience_candidates`: bounded scoped current heads.
- `set_experience_candidate_status`: explicit state change with expected_version and request_id. Pending can become active/rejected/superseded; active can become rejected/superseded; rejected can become superseded. Every change appends a new version.
- `revise_experience_candidate`: append supplied content as a new pending version; content need not differ from the previous version.
- `rollback_experience_candidate`: copy an earlier version into a new pending version; history is preserved. Reactivation is a separate explicit action.
- `recall_experience_candidates`: active current versions only, same namespace, Unicode-lowercased literal AND query terms, identifier order, limit1..100, max_bytes512..65536. The query permits 1..16 distinct whitespace terms and at most 1024 UTF-8 bytes; this is a separate contract from Event/Claim recall. `used_bytes` is exact compact result JSON size, not just candidate array size, and excludes the MCP/JSON-RPC envelope.

Rejected/superseded history cannot be reactivated directly. Stale expected_version fails rather than overwriting a competing write. Source relations are durable foreign-key tables and revalidated in the transaction. Canonical namespace-derived owner excludes legacy Unknown evidence from new experience records.

Activation only makes candidate knowledge eligible for recall. Procedure steps remain text: **no execution, permissions, identity, commitments, daemon writes or global policies are changed**. This service cannot authorize the connecting Agent to perform those steps. Conflicting candidate content can remain separate and visible; no automatic “winner” rewrites facts.

Fields are bounded: title256 bytes, ordinary text4096 bytes, payload64KiB, source list64, read list100. All examples use synthetic local data. New Episode summaries are immutable snapshots; corrections create a new Episode rather than replacing the old report.

## Reproduce and interpret evidence

```sh
cargo build --bin agent_llm_mm
python3 scripts/evaluate-memory-loop.py --help
python3 scripts/benchmark-memory-capacity.py --help
cargo test --test feedback_candidates --test experience_workflow --test indexed_text_recall --test sqlite_lifecycle
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
./scripts/test-tier.sh full
./scripts/status-sync-check.sh
```

Use the harness command options documented in [evaluation methodology](evaluation-methodology.md). It records a complete runnable feedback→correction→recall and Episode→candidate→activation→recall→revision/rejection/rollback journey. Fixed offline A/B/C and ablations are deterministic mechanism proxies. Fifty query variants over ten independent tasks are not fifty independently collected real tasks. Token usage/cost and real same-model outcome improvement remain not measured unless a separately authorized model run supplies actual records.

Performance reports include environment, build, raw timings, query plans and limitations. A shared-VM 10k/100k measurement is not a production capacity promise. Actual final commit and platform checks must be verified separately; configured CI is not evidence of a passing run.

## External evidence and conditional future choices

Schema6 supplies new Claim recording timestamps, normalized derived time keys and independent durable Reflection origin/effect/evidence relations. Historical unknowns remain unknown, and ambiguous scope remains quarantined. Feedback JSON is still bounded source data rather than speculatively normalizing every nested field. Existing `run_reflection` global governance remains experimental. Namespace is data partitioning, not hostile multi-user authentication.

The original store/MCP/auto-reflection module separation is implemented in [private in-crate modules](implementation-module-boundaries.md); it preserves the existing architecture and release-tools boundary. Destructive retention/compaction policy, fresh-machine packages, formal Alpha approval, broad task-quality certification and same-model causal efficacy remain open. Embeddings require demonstrated semantic misses and an explicit cost/privacy choice; remote/team services, RL and an autonomous controller remain outside this plan's finite implementation.
