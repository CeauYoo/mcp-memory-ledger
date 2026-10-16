# Native Codex host verification

The Ubuntu CI step uses official Codex **0.159.2**, pinned to the published full-package SHA256. It preserves the bundled sandbox resources. [Release and asset digests](https://github.com/openai/codex/releases/expanded_assets/rust-v0.159.2).

`scripts/probe-codex-native-host.py` creates a fresh private HOME, project, registry, and SQLite database. The ledger uses its mock provider. Only the generated native adapter hook is reviewed by exact command, source, definition hash, and binary hash, then trusted through the same public `config/batchWrite` operation used by the [official TUI](https://github.com/openai/codex/blob/rust-v0.159.2/codex-rs/tui/src/hooks_rpc.rs). No real user configuration or project trust is changed.

Codex receives two synthetic turns using a fixed Responses/SSE fixture bound only to `127.0.0.1`, with no credentials, external model, inference, or charge. Startup must place the task requirement in both the completed hook's context entry and the fixture request. After closing the host and revising the ledger task, public thread resume and a second turn must expose the new requirement in both observations. No private transcript is read; persistence is accessed only by public thread ID. This demonstrates host injection for the tested runs, not model understanding, tool authorization, or fork/compact coverage. Adapter receipts are not promoted by this test.

```sh
python3 scripts/probe-codex-native-host.py \
  --codex "$CODEX_BIN" --ledger "$LEDGER_BIN" \
  --output "$NEW_SUMMARY_PATH" --allow-synthetic-local-turn
```

Paths must be absolute and the summary path must not exist. The script emits only booleans, counts, failure categories, and the ledger binary digest. Raw provider requests and RPC events are examined in memory, never included in the uploaded summary. Do not upload temporary HOME files. The child environment is allowlisted and does not inherit credentials. No sandbox-disable or project-trust exceptions are permitted.

Exit 0 requires all four startup/resume request/event checks. An environment failure exits 2 and must not be relabeled as successful injection. Cloud development execution was blocked at local-core socket creation (`environment_permission_denied`, zero fixture requests); exact hook trust was separately verified by an isolated marker-only probe. The full-chain script is an integration candidate until the Ubuntu CI run succeeds. Public schema validation and Python syntax checks alone do not close that gap.
