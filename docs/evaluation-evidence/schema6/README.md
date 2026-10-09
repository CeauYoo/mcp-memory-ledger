# Schema 6 synthetic local evidence, 2026-10-09

These are provider-free observations from one shared Linux development session. They are technical MVP diagnostics, not real-model results, production capacity, a latency gate, or release approval. Historical schema 5 files in the parent directory are unchanged. See [methodology and limitations](../../evaluation-methodology.md).

## Frozen build and reproducibility

- Schema 6 binary SHA-256: `3045e57b2f1490fea5be62f2acf716f307f0841279298f40a094c81f15001a91`.
- Exact 126-file `src/**`, Cargo and toolchain content manifest SHA-256: `266b959c46130b8798b56a26968de29a31f26fc92f3a39c37281956c4c815d9a`. `build-manifest.json` records individual hashes, the encoding of the manifest digest, Rust/Cargo versions and base commit. This was a working-tree build, so its base commit alone does not identify the evaluated source.
- Both compared binaries use Rust 1.95.0 dev/debug, default features. The schema 6 binary was copied after the implementation lead's final default-feature build. The paired schema 5 binary is the previously frozen `af3f3ecd...`, checked against its existing source/build manifest, and rerun in this session. It was not rebuilt during this stage; its original [schema 5 build manifest](../final-indexed-build-manifest.json) records the earlier build provenance.
- Final functional runs started after local compilation/tests were quiet. Timing order was schema 6 default, schema 5 default, then a separate schema 6 `--dashboard-http` run. There was no exclusive host or randomized order. Results are sequential observations, not a controlled causal speedup claim.
- Each capacity query has two discarded warmups and twenty measured repetitions. Default mixed MCP workload has ten rounds. Actual HTTP workload has two discarded warmup rounds and twenty measured rounds per endpoint. All three capacity runs have matching event/Claim seed hashes at each size.
- Fixed ten-task and fifty-query selections, k, gold identities and the 1,800-byte packing budget are unchanged. The two unsupported paraphrases remain in aggregate results. `build-manifest.json` records the harness/fixture hashes; reproduction commands appear below. No paid/network model was invoked.

## Functional results and the packing tradeoff

The fixed evaluation completed sixty task/variant rows and fifty separately counted raw query scenarios. B recall and corrected C recall remain 8/10 overall (8/8 literal, 0/2 unsupported paraphrases). The corrected exact-answer proxy is 8/10; corrected stale-conclusion-use proxy is zero. Raw-query positives remain 30/30 and all twenty negative cases return no false-positive records. Missing-evidence and limited-evidence corrections fail closed; semantic/procedural activation, revision, rejection and rollback lifecycle checks pass. No scope leak or missing-provenance assertion failed. Model task success, token usage and token cost remain unmeasured/null.

Preserve the cost as well as the passing gold metric: at the same 1,800-byte budget, seven literal B rows and their seven minus-correction counterparts now pack one record instead of schema 5's two. Their relevant Claim remains, but the supporting Event no longer fits with temporal metadata. Lower average context bytes in these rows do not demonstrate better compression or token savings. `functional-comparison.json` retains per-task old/new recall, answer proxies, packed-record counts and byte counts. No budget or gold was adjusted to restore the dropped records.

The local installed-binary simulation passes restart, durable correction replay, scoped recall, exact UTF-8 budget accounting, SQLite backup/restore integrity and restored read-only doctor checks. This is not fresh-machine evidence. Eight pure Python fixture tests pass, including the opt-in HTTP contract checks.

The extended temporal/export smoke passes observed-versus-recorded timestamps, immutable replay, targetless scoped history, rejection of targetless global updates, read-only bounded export and actual multi-type union ordering before LIMIT. Its first attempt contained a single-type request incorrectly labeled as a multi-type union. The initial failure and diagnostic remain local and are not published. Correcting that fixture to request Event + Claim made the smoke pass on the same binary; no runtime change was needed.

## Default-only capacity pair

p50 / p95 milliseconds, local MCP request timing including retrieval, provenance and serialization:

| Event + Claim rows | Query | Schema 5 p50 / p95 | Schema 6 p50 / p95 |
| ---: | --- | ---: | ---: |
| 10000 | selective_ascii | 10.79 / 18.24 | 9.67 / 10.89 |
| 10000 | common_short_cjk | 17.36 / 20.81 | 15.61 / 19.89 |
| 10000 | revision_chain | 20.66 / 41.47 | 18.71 / 44.20 |
| 10000 | absent | 1.49 / 1.89 | 1.37 / 1.63 |
| 100000 | selective_ascii | 10.95 / 13.29 | 10.25 / 11.12 |
| 100000 | common_short_cjk | 39.67 / 46.01 | 32.87 / 45.05 |
| 100000 | revision_chain | 30.89 / 35.23 | 34.13 / 51.10 |
| 100000 | absent | 1.40 / 1.90 | 1.46 / 1.84 |

At 100k, the schema 6 revision-chain query is slower in this observed pair: 34.13 / 51.10 ms versus 30.89 / 35.23 ms. This result is retained, without a post-hoc pass threshold or claim that noisy sequential observations establish a regression cause. The separately timed schema 6 HTTP-option run's preceding default query phase measured 27.99 / 34.25 ms for the same query, illustrating within-session variation; it does not replace the designated paired result.

Before-read database size increases from schema 5's 6,664,192 / 66,678,784 bytes to schema 6's 7,827,456 / 76,701,696 bytes at 10k / 100k, respectively. Temporal/scope metadata and derived storage have a measurable size cost. Default writer-reservation probes committed at both sizes. Raw reports retain mixed-workload distributions, response bytes, Linux RSS snapshots and Python SQLite primitive plans. RSS is not peak memory, reservation latency is not internal lock telemetry, and Python plans do not establish the bundled Rust SQLite execution plan.

## Actual loopback HTTP concurrency (O03)

`capacity-http-schema6.json` is a separate opt-in run of `benchmark-memory-capacity.py --dashboard-http`. A per-round three-worker barrier starts ingestion, recall and HTTP browsing together. The HTTP worker reads all three endpoints sequentially. All requests are direct `127.0.0.1` GETs, with mock provider, browser opening disabled, SSE disabled, no proxies and no redirect following. No user database is accessed.

The event and operation-log endpoints include `namespace=project%2Fcapacity-00&limit=20`. p50 / p95 milliseconds include a new loopback connection and reading the JSON response:

| Event + Claim rows | Endpoint | HTTP p50 / p95 |
| ---: | --- | ---: |
| 10000 | `/api/summary` | 1.13 / 1.51 |
| 10000 | `/api/events` | 1.31 / 2.10 |
| 10000 | `/api/operation-log` | 5.34 / 6.01 |
| 100000 | `/api/summary` | 1.21 / 1.79 |
| 100000 | `/api/events` | 1.08 / 1.73 |
| 100000 | `/api/operation-log` | 20.71 / 23.43 |

All 132 observed HTTP requests (including warmups, excluding the extra isolated read-only checks) returned 200. Scoped responses contain zero foreign-scope rows. Once writers/recalls completed, isolated GET-only operation-log counts stayed 76 → 76 at both sizes. Raw HTTP samples retain method, exact endpoint, status, bytes, response SHA-256 and timings; final JSON responses are retained for inspection.

This closes a bounded actual dashboard read-concurrency observation, not browser rendering, SSE, authentication, production capacity or sustained-load coverage. Summary/events use this process's in-memory recorder; operation-log uses shared durable SQLite history. **10k/100k counts events plus Claims, not operation-log rows**; the history endpoint is reading logs generated by this bounded workload. Extra writes, process and HTTP work make this a different phase: do not pool it with the default-only pair or use its final database bytes interchangeably.

## Local verification and reproduction

Final local verification passed the fixed evaluator, installed-binary restart/backup smoke, extended temporal/scope/export smoke and eight pure Python tests. The implementation lead separately recorded 613 passing Rust tests and gate statuses in `local-gates.json`. The frozen source and binary identity are unchanged by this evidence-publication subset.

Use the [evaluation methodology](../../evaluation-methodology.md) and repository's synthetic fixtures to reproduce the results. Select or rebuild the exact manifested source first; a current checkout may contain later changes. Each command needs a new, empty output directory. From the repository root, with the intended binary built at `target/debug/agent_llm_mm`:

```sh
python3 -m unittest discover -s tests/fixtures/memory-evaluation -p 'test_*.py' -v
python3 scripts/evaluate-memory-loop.py --binary target/debug/agent_llm_mm --output target/reports/schema6-reproduction/offline
python3 scripts/local-memory-smoke.py --binary target/debug/agent_llm_mm --output target/reports/schema6-reproduction/local
python3 scripts/temporal-scope-export-smoke.py --binary target/debug/agent_llm_mm --output target/reports/schema6-reproduction/temporal
python3 scripts/benchmark-memory-capacity.py --binary target/debug/agent_llm_mm --output target/reports/schema6-reproduction/capacity --sizes 10000 100000 --repeats 20 --label exact-manifested-build
python3 scripts/benchmark-memory-capacity.py --binary target/debug/agent_llm_mm --output target/reports/schema6-reproduction/http --sizes 10000 100000 --repeats 20 --label exact-manifested-build-http --dashboard-http
```

These provider-free scripts regenerate their own synthetic data, raw outputs and traces locally. They do not require published raw artifacts. The benchmark's databases are disposable temporary files; local smoke outputs remain under the specified output directory. Paired schema 5 capacity reproduction requires its exact separately manifested binary rather than relabeling a current binary.

## Published artifact inventory

- `build-manifest.json`: exact schema 6 source, binary, toolchain and harness/input hashes.
- `functional-comparison.json`, `query-scenarios.json`: reviewed task-level comparison and fixed raw-query metrics, including misses and packing changes.
- `capacity-schema6.json`, `capacity-schema5-paired.json`, `capacity-http-schema6.json`: reviewed default capacity pair and distinct actual-HTTP observation.
- `local-smoke.json`, `local-doctor-restored.json`: reviewed installed-binary simulation and restored-database checks.
- `local-gates.json`: implementation lead's local Rust verification statuses and log hashes.
- `artifact-manifest.json`: SHA-256 and byte counts for the other currently published files in this directory, including this README.

Raw MCP traces, full context/adapter artifacts, compressed logs, detailed temporal exports/diagnostics and additional evaluator reports remain local under `target/reports/schema6-unpublished/`; they are not published or replaced with decoded copies. The scripts and fixed synthetic fixtures reproduce these outputs. This deliberately limited published subset does not provide the complete raw workflow transcript.

All measured payloads are synthetic. Binary executables and databases remain local build artifacts. Historical schema 5 reports are unchanged.
