# In-crate implementation boundaries

The service remains one Rust crate with the existing domain/application/ports/adapters architecture. This extraction addresses original-plan M01–M03 without changing the public API, MCP tool registration, SQLite schema, dependency graph, or `release-tools` feature boundary.

## SQLite store

`src/adapters/sqlite/store.rs` owns `SqliteStore`, its pool, bootstrap entrypoint, and private module declarations. Its lifecycle-helper re-exports retain the existing adapter-only visibility.

- `store/reads.rs`: scoped record queries, standalone and union ordering, history visibility, provenance loaders, and shared identity/commitment reads.
- `store/ports.rs`: Event/Claim/Episode/Reflection/Trigger/Identity/Commitment port implementations. Their mutations delegate to transaction write helpers; existing port-specific reference queries stay beside the corresponding implementation.
- `store/transactions.rs`: ingest/reflection transaction runners, poisoned-transaction guards, write receipts, compare-and-set updates, and shared ledger row mutations.
- `store/rows.rs`: typed row decoding, enum/JSON codecs, timestamp parsing, and opaque seconds/nanoseconds sort-key generation.
- `store/schema_compat.rs`: legacy table compatibility and baseline seed helpers used by the explicit migration lifecycle.
- `store/operation_log.rs`: operation-log append and filtered diagnostic reads.

These are ordinary private Rust modules, not textual `include!` fragments. Cross-module helpers use explicit imports and restricted visibility. SQL text, bind order, limits, scope predicates, ordering/tie-breakers, `BEGIN IMMEDIATE`, commit/rollback paths, and receipt serialization are preserved by the extraction.

## MCP transport and runtime

`src/interfaces/mcp/dto.rs` continues to own parameter DTOs and domain conversions. `server.rs` retains stdio/dashboard/daemon startup, public runtime constants, the `Server`, and the single `rmcp` tool-router implementation.

- `server/runtime.rs`: concrete model/store dependencies, runtime initialization, and application-port delegation, including the four union-read forwards.
- `server/diagnostics.rs`: best-effort durable diagnostic logging, dashboard diagnostics, safe error classification, and application-to-MCP error conversion.
- `server/transport.rs`: JSON parameter decoding, correlation IDs, and structured MCP results.

The refactor keeps handler call order, error codes, logging behavior, tool descriptions, and generated input schemas unchanged. Separate capability additions may evolve individual DTOs or handlers; those changes require their own behavioral coverage.

## Automatic reflection

`src/application/auto_reflect_if_needed.rs` retains the public input/result types and the explicit `execute` pipeline.

- `auto_reflect_if_needed/candidate.rs`: freeze the trigger window, narrow it to the requested scope, determine whether to consider a trigger, and assemble the revision snapshot.
- `auto_reflect_if_needed/evidence.rs`: normalize raw/canonical Event references and constrain model-selected evidence to that frozen window.
- `auto_reflect_if_needed/policy.rs`: cooldown/unchanged-evidence suppression and deterministic identity/commitment patch governance.
- `auto_reflect_if_needed/commit.rs`: record rejected/suppressed trigger outcomes and route approved revisions through the existing `run_reflection` durable write path.

Stage types and helpers remain private to this application module. The model proposes; the existing deterministic policy and transaction path still control commitment. No second durable self-model write path is introduced.

## Regression evidence

The extraction reuses behavioral coverage rather than asserting only that source strings exist:

- `sqlite_store`, `sqlite_lifecycle`, `sqlite_temporal_store`: reads, ordering, transaction failures, migration, and temporal contracts
- `reflection_scope_history`, `scoped_ledger_export`: scope visibility and export/import boundaries
- `schema6_migration`, `correction_atomicity`: legacy receipt/fingerprint compatibility, replay, and atomic corrections
- `application_use_cases`: automatic-reflection trigger, evidence, suppression, policy, and commit paths
- `operation_log`, `mcp_stdio`, `daemon_config`: diagnostic outcomes, real MCP calls/tool schemas, and startup boundaries

Run the normal repository gates after integration:

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo test --all-features
```

The local extraction audit also compared the 281 pre-existing function bodies, allowing only visibility/module-path qualification and formatting changes, and verified that all existing Rust string literals were unchanged. This is a movement check, not a substitute for the behavior suites above.
