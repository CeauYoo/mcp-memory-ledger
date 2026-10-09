# Original planning document: final implementation reconciliation

The historical [70-row matrix](2026-10-09-original-plan-traceability.md) remains an immutable assessment of e14a59e. This addendum follows every ID through the implemented schema5 feedback/experience slice, schema6 temporal/scope/export slice and final bounded context/engineering work. It does not turn examples or conditional research choices into mandatory product features.

## All 70 material IDs

| Original IDs | Current bounded status and evidence |
|---|---|
| F01–F08 | Preserved Rust/SQLite/MCP architecture, explicit lifecycle and existing revision path; deterministic reads, local MVP boundary and single active plan. |
| F09 | Current contract/status/testing/platform docs reconciled with each stage; historical manifests stay tied to historical source. |
| B01–B09, B11 | Transactional scope/state/version validation, single-successor Claim-ID CAS, exact-content feedback token, atomic audit/receipts and replay; distinct durable Reflection origin/effect scopes. `correction_atomicity`, `feedback_candidates`, `reflection_scope_history`, `schema6_migration`. |
| B10 | Conditional future full-audit product contract. Required minimal transactional audit is implemented; arbitrary direct-import auditing and tamper-proof retention are not claimed. |
| C01–C08 | Bounded structured feedback and object-only candidate support validation, inspect/reject/commit/replay/recall. Producer labels are caller assertions, not authentication or truth certification. `feedback_provenance`, `feedback_candidates`, executable evaluator. |
| C09 | Implemented; final exact-head CI pending: opt-in cooperative caller budgets and explicit stop reasons, preserving existing automatic-reflection policy. No durable task controller or cross-client quota claim. |
| D01–D07, D11 | Separate lexical recall, rebuildable FTS with exact fallback, scoped retrieval, bounded type allocation/ranking, full provenance and exact compact JSON byte budget. `indexed_text_recall`, `text_memory`, fixed evaluator. |
| D08 | Conditional optional semantic retrieval. Unsupported paraphrase fixtures remain visible; no remote embedding dependency is silently added. |
| D09–D10 | Implemented; final exact-head CI pending: source-linked rich Episode evidence in context, bounded observable state diagnostics, selection/omission reasons. Expiry is unassessed without an effective-validity contract; differing values are possible multi-valued conflicts, not inferred contradictions. |
| S01–S06 | Nullable historical times, distinct observed/recorded times, indexed normalized ordering, durable Reflection/Episode/candidate relations, reverse indexes and rebuildable derived FTS. `temporal_metadata`, `sqlite_temporal_store`, `schema6_migration`, `sqlite_lifecycle`. |
| O01–O05 | Reproducible 10k/100k multi-scope/revision-chain measurements; actual HTTP concurrency; latency/bytes/DB size/RSS snapshots; separate reservation-acquisition primitive; query-plan-driven indexes. Explicit pool/busy defaults and preserved journal mode are conservative measured decisions. |
| O06 | Native exact-head CI is separately verified. Actual user-client/fresh-machine installation, formal packaging and human release approval remain distinct external evidence gates. |
| M01–M03 | Implemented; final exact-head CI pending: targeted private in-crate SQLite read/write/mapping, MCP runtime/diagnostic/transport and automatic-reflection candidate/evidence/policy/commit modules. No framework/crate rewrite. |
| M04–M05 | Release-tools feature preserved; behavioral invalid-config/retry tests supplement textual contracts. |
| A01–A02 | Fixed synthetic diagnostic task manifest, independent query variants, wrong/stale/scope/provenance/budget metrics. Ten tasks plus fifty variants are not sixty independent tasks or broad quality certification. |
| A03–A05 | Offline A/B/C and single-component ablation framework implemented. Real same-model task success, causal efficacy, actual tokens and costs remain unmeasured. Deterministic answer selection is a proxy. |
| A06 | Explicitly gated external efficacy experiment: chosen model/version, rubric, provider/data authorization and budget required. Connection certification is not efficacy evidence. |
| E01–E05 | Durable rich Episodes and source-linked semantic/procedural candidates, inspect/reject/revise/activate/rollback, immutable histories, scoped recall, migration/backup/recovery and bounded read-only export. Retain-all is the conservative lifecycle policy. Activation changes recall eligibility only. |
| R01–R02 | Correctly conditional under the original document: autonomy/RL/multi-agent competition and remote/team services require separate actual needs and authority. |

## Why the remaining boundaries are legitimate

- Original L69 gives expected_version as an example. Immutable Claim identity plus single-successor state CAS and exact-content/status tokens fulfill the selected version-conflict contract; a universal low-level monotonic mutation counter/ABA guarantee is not silently promised.
- Original L75 explicitly makes complete audit a future conditional requirement. Current mandatory successful writes keep minimal audit and receipt atomically; diagnostic logs remain separate.
- Original L91 asks to test Chinese short terms and identifiers, not to install a tokenizer regardless of evidence. Literal CJK/identifier/punctuation and negative paraphrase cases are preserved.
- Original L108 calls for gradual relationalization where constraints/queries need it. Durable queried source relations are normalized; every nested feedback JSON field need not become a table.
- Original L123/L125 makes WAL, pool tuning and pagination measurement-driven choices. Preserving journal mode avoids an unnecessary migration/recovery-policy change. WAL checkpoint qualification becomes mandatory if WAL is later enabled.
- Original L220/L267 requires inspectable/versioned candidates, not automatic extraction or procedure execution. Caller-authored source-linked proposals remain inert until explicit recall activation, with no permission changes.
- Original L255 preserves lifecycle gates. Retain-all histories/receipts, safe backup/restore and selective interchange export satisfy this additive slice. Destructive retention/compaction requires an explicit policy and authorization.
- Original L269–275 requires actual model evidence for efficacy conclusions. Offline mechanism tests cannot close that evidence gap or infer token savings from bytes.

## Verification and stopping condition

Schema6 is separately published at `726d3b62c40fb6f999651541bceb215af2eeb113`; its [stage report](2026-10-09-schema6-results.md) records 613 local Rust tests and independent synthetic workflows. The final context/refactor/budget source passed 551 default and 633 all-feature Rust tests (zero failures, ignored or filtered), fmt, strict all-target/all-feature Clippy, status-sync and diff checks. The final default binary SHA-256 is `08a340512396d28ebd7f6ebd6c4d13055c0fa299d97d50a3ea811e48442a2d7c`. Unchanged evaluator fixtures, executable workflows and source-consistent capacity observations are rerun against that binary; exact-published-head CI is still a separate pending gate.

After those finite final-stage capabilities and gates, the audit found no further unconditional original implementation requirement requiring another broad development phase. Keep external efficacy, user-environment validation, release decisions and optional research/policy choices visible instead of marking them complete or expanding the project indefinitely.


Independent review compared all 281 extracted original functions and original SQL/error literals, then probed budget admission/accounting and context byte/scope boundaries. It found and resolved unbounded rich candidate materialization, new legacy hydration, source-reference collection and admission-after-validation issues. The final real v5→6 regression also preserves historical receipt/candidate/feedback/fingerprint bytes through migration and demonstrates replay plus candidate commit without duplicates. Final-source aggregate tests cover the fixes; earlier independent probe binaries are not substituted for final verification.

## Final local workflow and capacity observations

The frozen default binary above passed the unchanged evaluator, installed-copy restart/backup/restore smoke, temporal/scope/export/ordering smoke and nine Python fixture tests. Fixed recall and corrected-answer proxies remain 8/10 overall (eight literal tasks pass; two unsupported paraphrases remain misses). All fifty separately counted raw query scenarios match their existing expectations, with zero scope/provenance assertion failures. These are deterministic mechanism results, not model task-success measurements.

The 141-file source digest is `59d5301c0d4e72eaf8b6d88372859b381bd56df8edeb11ba8ed760e2b68ad0fd`: SHA-256 of sorted compact UTF-8 JSON mapping each file under `src/**` plus `Cargo.toml`, `Cargo.lock` and `rust-toolchain.toml` to its SHA-256, with no newline. Rebuild the corresponding committed source using the recorded Rust1.95 default dev profile. The local manifest and raw runtime outputs are deliberately not published; repository scripts and synthetic fixtures regenerate them. Historical published schema6 evidence remains unchanged.

Preserve costs as well as passing metrics. Against the same-session immutable schema6 binary `3045e57b…`, final mean fixed-context bytes increased for B (1057.5→1339.9), corrected C (1133.1→1298.4) and the API-max budget ablation (2541.3→3361.8). Additional state/omission metadata and optional content consume bytes; this is not token savings.

Separate 20-repeat synthetic context timings (two warmups, limit20, 16,384-byte cap):

| Dataset | Query | Schema6 context p50 ms | Final context p50 ms |
|---|---|---:|---:|
| 100k Event+Claim rows | selective ASCII | 14.36 | 17.06 |
| 100k Event+Claim rows | short CJK | 47.69 | 47.63 |
| 100k Event+Claim rows | revision chain | 43.92 | 49.98 |

A 10k revision context observation was slower/noisier: p50/p95 31.43/34.77→46.82/76.86 ms. The designated paired results are retained, without a post-hoc latency pass threshold. Primary counts remained 2/20/20 for these three 16KB queries. Dataset hashes, exact byte accounting and scoped-response assertions passed. Sequential shared-host observations do not establish causal speedup/regression or production capacity.

The new direct Python SQLite reservation probe acquired then rolled back at both 10k/100k sizes, approximately53.6–53.8 ms acquisition elapsed during approximately50 ms controlled holds. This includes busy-handler scheduling/statement overhead and is distinct from full MCP request latency or Rust internal lock telemetry. No source data is changed by the primitive.

The independent final-binary boundary probe passed rich Episode linkage through selected Claim evidence, foreign/missing-source rejection, tiny/ample diagnostics, zero-budget stops and admitted-error receipts. A1000-source Claim retained all1000 provenance references in a28,017-byte context; at1800 bytes the entire Claim was omitted, leaving a1293-byte result. Nothing was truncated. In another1800-byte case, Claim+Episode fit without a caller receipt (1721 bytes); with the receipt, the Claim stayed and the whole Episode was omitted (1467 bytes). Source-limit reports a lower-bound sentinel1, not an exact omitted-source total. An initial local probe mistakenly expected936; correcting that expectation required no runtime change.

Fixed evaluator packing also remains explicit: B and minus-correction each rose from8 to10 total primary records (two Chinese cases regain a supporting Event), corrected C stayed at8 Claims, and the API-max ablation stayed at24 primary records. There were no primary losses in this comparison. Diagnostics fit only2/10 fixed-budget rows and all10 API-max rows. Optional metadata can be omitted and must not be interpreted as evidence of no conflict. Final source/harness/binary hashes were rechecked unchanged; all raw probe/measurement files remain local.
