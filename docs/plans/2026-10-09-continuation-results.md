# Original-plan continuation: v5 historical delta and subsequent closure

The matrix below records the v5 delivery. The next schema-v6 slice now implements its previously deferred temporal fields/keys, independent Reflection scopes and authoritative evidence relations, safe targetless history and readonly bounded export. See [temporal](../temporal-metadata.md), [scope](../reflection-scope-history.md), and [export](../scoped-export.md). M01–M03 module separation and D10 status diagnostics are actively being completed rather than left as arbitrary deferrals. Final source-specific verification is tracked in each stage and the draft PR.

The [70-item e14a59e baseline matrix](2026-10-09-original-plan-traceability.md) is intentionally immutable historical evidence. This addendum maps the continuation back to those IDs. It does not turn all original planning suggestions into completed product claims.

## Implementation delta

| Baseline IDs | Current disposition | Concrete evidence / limitation |
|---|---|---|
| F01–F08, B01–B03, B05–B09, C01–C04, D01–D02, D05, D07, D11, M04–M05 | Existing contracts preserved and extended | Original ledger/reflection/receipt/scope tests remain; schema5 adds explicit migration, new layer readback and rebuildable derived index. |
| F09 | Implemented for this slice | README/status/roadmap, platform guides, testing guide, layered roadmap and workflow/evaluation docs updated together; historical sections are explicitly scoped. |
| B04 | Implemented bounded current-content version contract | `claim_version` SHA-256 plus commit-time transaction check; experience expected_version is monotonic per candidate. Claim fingerprint is not a universal mutation counter and does not detect low-level ABA overwrites. |
| B10 | Partial, unchanged audit product boundary | Mandatory atomic minimal receipts on all new writes. No promise of tamper-proof retention, arbitrary importer coverage or automatic receipt pruning. Retain-all is the current policy. |
| B11, S02 | Partial / independent Reflection effect-scope migration deferred | New candidate paths cannot alter global identity/commitments. Legacy self-model governance and Claim-attributed Reflection reads remain separate; no unsafe historical scope backfill is invented. |
| C05–C08 | Implemented for explicit object-only evidence contract | Persisted proposal/inspection/rejection, exact target/version/expected/actual/source/scope checks, model/inconclusive/missing denial, nonempty-limitations manual-review block, atomic guarded `run_reflection`, recall/history. This is report alignment, not factual truth or source authentication. |
| C09 | Partial | No-new-evidence candidate suppression, query/response limits and existing reflection guards. General per-task model/tool-turn cost controller is not added. |
| D03, S05 | Implemented | Derived FTS5 trigram projection, transactional triggers, deep read-only inspector, explicit atomic rebuild, missing/corrupt fallback and VACUUM/restore tests. |
| D04 | Implemented literal contract; broader linguistic quality remains partial | Short/long Chinese, ASCII, identifiers, punctuation, non-ASCII case, NUL legacy content, mixed scope and fixed query variants tested. No segmentation, spelling or semantic inference claim. |
| D06 | Implemented bounded policy | Independent Claim/Event quotas, interleaving, matched-term score, Event recency and deterministic ID; unknown Claim time remains unknown. |
| D08 | Deferred by original evidence/cost gate | Fixed paraphrase misses are retained as evidence. They motivate future optional semantic retrieval; they do not authorize a model/provider dependency or paid run. |
| D09–D10 | Partial | Exact-budget working context explains matches/validity/time/omissions, active-only Claims and historical Events. Rich Episodes/experience have separate bounded read/recall APIs; unified Episode context packing, conflict discovery and expiry contracts remain open. |
| S01, S06 | Deferred compatible data-model migration | New Claim timestamps and normalized legacy sorting need explicit event-time/recording-time semantics and unknown-history behavior. Current events use true RFC3339 ordering; source dates are not rewritten to migration time. No demonstrated need to risk broad timestamp migration in this slice. |
| S03 | Partial, improved | New Episode→Event and versioned candidate→Episode relations have FKs and reverse indexes. Legacy Reflection and feedback JSON relations remain until an actual queried constraint needs safe backfill. |
| S04 | Implemented relevant query-plan indexes; time-column work separate | Scope/status indexes, reverse event→claim/episode indexes, FTS materialized candidate path and exact bundled-runtime plan regression. No claim of indexing every future query. |
| O01–O02 | Implemented reproducible synthetic capacity harness | 10k/100k ledger rows,20 namespaces,long revision chain,p50/p95,response bytes,DB size,RSS snapshots and bounded-lock request observations; raw reports retain source/build/SQLite caveats. RSS is not peak, observed request delay is not internal lock telemetry. |
| O03 | Partial | Concurrent real MCP write/recall/dashboard-like browse workload; no browser dashboard rendering/performance certification. |
| O04–O05 | Partial, explicit conservative settings | Measurement exposed and fixed a real repeated-FTS-probe plan regression. Existing10-connection/5s defaults made explicit, journal mode preserved. WAL/checkpoint qualification and cursor paging remain measurement-driven future choices. |
| O06 | Partial | New suites and offline workflow added to Linux/macOS/Windows CI. Final exact-head results required separately. Full Windows wrappers, fresh-machine, binary packaging and human Alpha approval remain open. |
| M01–M03 | Partial, incremental modularization | New read/index/experience/feedback code lives in focused modules; existing huge store/server/automatic-reflection files are not fully rewritten. Wide mechanical refactor deferred to avoid coupling unrelated churn with data migration. |
| A01–A02 | Implemented fixed offline contract diagnostics; representative quality partial | Ten independent synthetic tasks plus50 labeled query variants; errors/rejection/stale answer/scope/provenance/bytes separately reported. Variants are not50 independent tasks or broad user-quality proof. |
| A03–A05 | Offline A/B/C and ablation framework implemented; real-model outcome evidence deferred | Same fixed inputs, no memory / recall / feedback+correction, single-component ablations, future same-model record schema. Actual model success/token usage/cost remains null, not inferred from bytes or deterministic selector outputs. |
| A06 | Deferred external experiment, not deferred framework | Provider authorization/model version/budget/rubric required before live efficacy runs. Connection certification is not causal task-improvement evidence. |
| E01 | Implemented bounded durable episode snapshot | Objective/actions/observations/outcome/lesson/limitations plus scoped source Events, inspection/listing,restart and populated backup/restore. Episode content is caller-authored; correction appends another episode, not in-place rewrite. |
| E02–E04 | Implemented inspectable inert candidates | Semantic/procedural kinds, source Episodes, explicit reject/activate, append-only version history, stale-version conflict, revise/rollback to pending, active-only exact-budget recall. Synthesis is explicit caller input, not autonomous model extraction; activation never grants authority or executes steps. |
| E05 | Implemented migration/recovery for bounded retain-all slice; general lifecycle partial | v4→v5, canonical/FK/count readback, restoration of populated version history and retry receipts; whole-db backup is recovery/export unit. Selective export, retention/deletion and compaction need their own contracts and are not implied. |
| R01–R02 | Deferred as the original plan required | No autonomous controller,RL,multi-agent competition,remote/team database or new hosted dependency. |

## Evidence and stopping condition

The finite implementation is complete only when focused and full tests, formatting, all-feature Clippy, status-sync, real stdio workflow, explicit migration/recovery, controlled synthetic reports and independent review agree, and the exact published head reaches terminal CI. These are separate from formal product release and real-model outcome gates.

A planner regression was preserved instead of hidden: the first indexed design repeatedly probed FTS and was much slower. A materialized candidate set / FTS-first plan barrier fixed that path, with a test against the same query builder and SQLite library used by runtime. Reports show the storage/latency tradeoff, including cases where a small selective scan can still be faster. No universal speedup claim is made.

Independent review also found that recorded free-text feedback limitations were not gating commit. The v1 validator now blocks any nonempty limitation for review; it does not silently discard restrictions, parse them heuristically or certify arbitrary tool reports true.

See [runtime contracts](../memory-feedback-experience.md), [evaluation method](../evaluation-methodology.md), and [published evidence](../evaluation-evidence/README.md). Exact-SHA platform results are recorded in the draft PR; a future green run cannot be inferred merely from workflow configuration.

## Local verification record

最终冻结源码已通过 570 项完整 all-feature 测试（0 failed/ignored）、fmt、all-target/all-feature Clippy `-D warnings`、status-sync32 门和 diff-check。新增旧版本迁移、只读 doctor 损坏索引提示、恢复及 capability scope 断言包含在这次完整测试中。安装副本重启/更正/备份恢复 smoke、真实 MCP60 行 A/B/C/消融、50 查询变体及经验生命周期另行通过。详见 [本地验证记录](../evaluation-evidence/local-verification.json)；GitHub 平台验证必须绑定发布后的 exact head，不能用本地结果替代。
