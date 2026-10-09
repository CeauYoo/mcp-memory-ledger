# Synthetic local observations, 2026-10-09

These files are measured evidence from one shared Linux development environment, not production benchmarks, real model experiments, or release approval. Exploratory runs overlapped development/test activity. The final pair waited for local compilation and tests to finish, but no exclusive host CPU or random execution-order control was imposed. Read [the protocol and limitations](../evaluation-methodology.md).

- `exact-e14a59e-capacity.json` and `exact-baseline-build-manifest.json`: subsequent independently rebuilt exact `e14a59e77554891d5bfb03a88390713f07193102` source archive, tree `5067bbd118e134235cdde9c4d39ecbbb3b8c1423`, Rust 1.95.0 dev/default-features binary `da522b8a50961dd7fa7dba016343f800c20607b93cf3b6af3350d9208bc1fead`. This exact baseline is distinct from the unverified old smoke binary.
- `offline-evaluation-final.json`, `task-metrics-final.json`, `query-scenarios-final.json`: frozen final binary `af3f3ecd62a36655101048f20111b22c4eaa9e68639027c8ffcfa9129f42d44a`; full source-content/toolchain manifest is `final-indexed-build-manifest.json`. Sixty A/B/C and ablation rows over ten tasks, plus fifty separately counted raw query scenarios, are deterministic/proxy evidence only. Two unsupported paraphrase task misses are retained. Missing and limited evidence fail closed; semantic/procedural rollback produces a new pending v5 while preserving prior active history. Zero scope or provenance failures were observed. No model/token-cost evidence.
- `literal-baseline-observed.json`: preserved prior binary SHA-256 `7bda1e5b3fba880eb8eec1f9e96593df01c460b3fdda55074838de17be8a452c`. Its exact source-commit provenance is **not verified here**. Do not relabel this as a controlled `e14a59e` or `11f9bd55` commit comparison. The binary existed in a prior installed-binary smoke artifact.
- `indexed-regression-observed.json`: intermediate working-tree debug binary `3b88a7142499fb7ea6992b2b3f00fa3f69a17b422abd03b647ef41b17b6f59aa`, retained because measurement found a severe query-planner regression. It is not the final implementation.
- `indexed-fixed-observed.json`: immutable debug binary `a49d645c5c85ef861a02e793635ddbc1b5c5d73fb1153596c686995ee83bceed`, after MATERIALIZED candidate CTE and FTS-first CROSS JOIN. This pre-publication build includes schema/index/retrieval changes; later runtime/validation changes do not retroactively change its checksum.

The benchmark default command uses twenty measured repetitions after two warmups for each query at both 10,000 and 100,000 event-plus-Claim rows. Both sizes' deterministic event/Claim payload hashes agree across all three binaries. Each binary initializes a fresh temporary database. These are warm-cache observations from the same host and debug profile; generated schema, selection policy and response metadata differ. The old smoke binary remains unverified. The subsequently rebuilt exact baseline resolves that source attribution for the final pair; shared-host load and sequential execution still limit causal interpretation.

## Final paired capacity observation

After local compilation/test workers became quiet, the final indexed binary was measured first, then the independently rebuilt exact e14a59e baseline. Both used the same host, Rust 1.95.0 dev/default-feature profile, deterministic event/Claim data hashes, two warmups and twenty repetitions. These sequential observations are not randomized trials; the shared host and external load remain uncontrolled. Lower or higher numbers do not establish a universal causal speedup or SLA. The pre-publication indexed source is identified by the 118-file content manifest rather than falsely attributed to its base commit.

| Rows | Workload | Exact e14 p50 / p95 ms | Final indexed p50 / p95 ms |
| ---: | --- | ---: | ---: |
| 10000 | selective_ascii | 6.59 / 9.24 | 11.03 / 15.01 |
| 10000 | common_short_cjk | 38.41 / 41.98 | 16.41 / 18.87 |
| 10000 | revision_chain | 39.09 / 44.57 | 16.92 / 18.89 |
| 10000 | absent | 2.82 / 3.47 | 1.20 / 1.54 |
| 100000 | selective_ascii | 44.77 / 46.94 | 11.44 / 13.82 |
| 100000 | common_short_cjk | 267.05 / 297.04 | 32.22 / 33.70 |
| 100000 | revision_chain | 252.36 / 278.31 | 28.10 / 31.28 |
| 100000 | absent | 25.74 / 31.50 | 1.26 / 1.93 |

Raw final pair: `final-indexed-capacity.json` and `exact-e14a59e-quiet-capacity.json`. Source/build manifests are adjacent. Final before-read database sizes are 6,664,192 / 66,678,784 bytes (10k / 100k), compared with exact baseline 2,486,272 / 24,522,752 bytes. Index storage is an explicit tradeoff. Internal lock telemetry and peak memory remain unmeasured; RSS snapshots and end-to-end reservation probes are in the JSON.

`runtime-sqlite-query-plan.json` captures a passing one-test run of the actual Rust query builder with SQLx SQLite 3.46.0. Both Claim and Event plans materialize the candidate CTE, execute the MATCH virtual-table path, perform integer-primary-key document lookup, and use the partial fallback index. This is stronger runtime-plan evidence than the separately labeled Python primitives, while remaining a small query-plan test rather than a capacity measurement.

Full JSON includes all samples' summary distributions, byte sizes, RSS snapshots, query-plan primitives, mixed workload and writer-reservation results. Internal lock waiting and peak RSS are not instrumented. Python EXPLAIN primitives may use a different SQLite version than SQLx; exact bundled-runtime plan evidence is retained separately when captured. No latency threshold or success/token-cost claim is inferred.

## Complete synthetic workflow artifacts

The three `offline-final-*.json.gz` files retain full compact JSON task rows (including contexts), MCP request/response transcript, and inert adapter requests. They are ordinary gzip, readable with Python `gzip.open(path, "rt", encoding="utf-8")` followed by `json.load`. `offline-full-artifacts.json` records compressed/uncompressed SHA-256 and sizes. No credentials, user data, paid model calls, or model outputs are included. The uncompressed metric summaries are provided for easy review.
