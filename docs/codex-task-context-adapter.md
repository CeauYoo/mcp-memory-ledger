# Codex task context adapter (partial, opt-in)

Status: native Rust adapter implemented for the public Codex lifecycle JSON contract inspected on 2026-10-16. This is a technical-demo/MVP feature. No user machine deployment or global configuration edit is performed. It does not authorize or intercept tool execution (T09 onward is outside this batch).

## Protocol and evidence

The adapter reads only `session_id`, `cwd`, `hook_event_name`, and `source`. It never reads transcript files or imports the user-prompt field as requirements. Recognized events are SessionStart (`startup`, `resume`, `clear`, `compact`, `fork`) and UserPromptSubmit. Unknown events degrade to manual context rather than guessing a protocol. Host payloads do not authenticate their producer or include a trustworthy host version.

[Official hooks documentation](https://developers.openai.com/codex/hooks/) defines the public JSON and configuration; accessed 2026-10-16. The installed `codex-cli 0.159.2` public app-server schema describes `additionalContextLimit: 0` as disabling spilling. The preview uses that setting only alongside the adapter's strict byte cap. Default capacity is 8,192 bytes, absolute ceiling 32,768 bytes. The adapter reserves space for metadata and JSON escaping when requesting core context, then measures both the additional context and complete escaped output again. It never truncates mandatory requirements. An insufficient budget produces a warning, not a partially complete-looking task context.

Capability evidence must remain separated:

- Documentation/schema support: checked for the contract above.
- Real host configuration parsing: isolated 0.159.2 `initialize` and `hooks/list` succeeded; handler was listed with `additionalContextLimit: 0`, no configuration errors, and `trustStatus: untrusted`.
- Real lifecycle execution/injection: **blocked, not passed**. `thread/start` failed before any model request: `fs sandbox helper failed ... error building bubblewrap command: app-server socket directory must be a user-owned directory with mode 0700`. No trust bypass or sandbox relaxation was attempted. Startup, resume, fork, and compact injection remain unverified.
- User-machine installation: not performed.

The `capabilities` subcommand only probes the installed CLI's public version with bounded output/time. That executable might differ from the invoking host. It never labels version discovery as working injection. Future validation must use a naturally supported isolated environment and independently trusted hook definitions. A hook-completed `context` entry can demonstrate that one host run accepted context; it does not establish model adoption or prove future sessions.

## One global hook and explicit task choice

Set `STATE_DIR`, `REGISTRY_FILE`, `CORE_SOCKET`, and `PROJECT_ROOT` to operator-selected absolute paths. `STATE_DIR` must be a private non-synchronized directory; the registry, core socket, and existing core data must also follow the approved local/non-synchronized placement. No machine-specific path is supplied here. The core must already be running, with the workspace explicitly registered. Use a private, non-synchronized directory chosen by the operator for adapter state. Use canonical directory paths without symlink ancestors. The directory must already exist and be private (0700 on Unix; the existing Windows private-directory checks apply).

```
agent_llm_mm codex-context init --state "$STATE_DIR/state.sqlite"
agent_llm_mm codex-context select-default --state "$STATE_DIR/state.sqlite" --registry "$REGISTRY_FILE" --socket "$CORE_SOCKET" --root "$PROJECT_ROOT" --workspace-id WORKSPACE_ID --task-id TASK_ID
agent_llm_mm codex-context preview --state "$STATE_DIR/state.sqlite" --registry "$REGISTRY_FILE" --socket "$CORE_SOCKET"
```

`preview` writes only JSON to stdout and never edits host config. Its `hooks_json` value is the prospective hooks file, not the whole report. Review its commands before separately installing/trusting them through the host. The POSIX command uses quoted absolute paths. Windows receives the native argument vector and an explicitly unsupported automatic-shell-preview marker; a Windows hook command renderer is not yet verified. This limitation concerns the new hook configuration only, not existing Windows core, bridge, or task CLI support.

The selected project default is an explicit operator choice, never a task inferred from cwd. A new startup session can bind to that choice after fetching fresh, complete, active task context. A changed default affects only future startups. Existing session bindings always win; paused/cancelled tasks do not silently switch to another task. Multiple matching default roots are ambiguous and require explicit selection.

An unknown resume/fork/clear session requires a direct binding, preventing old/forked work from being silently assigned a different default:

```
agent_llm_mm codex-context bind --state "$STATE_DIR/state.sqlite" --registry "$REGISTRY_FILE" --socket "$CORE_SOCKET" --root "$PROJECT_ROOT" --workspace-id WORKSPACE_ID --task-id TASK_ID --session-id HOST_SESSION_ID
agent_llm_mm codex-context inspect --state "$STATE_DIR/state.sqlite" --session-id HOST_SESSION_ID
```

A different task cannot overwrite an existing session binding; use a new host session. The registry's actual namespace is respected, including adopted legacy project namespaces. Canonical cwd must be inside the explicitly bound root. The registry is revalidated and the core bridge performs its existing per-call scope/revocation checks.

## Freshness, failures, and receipts

Each supported lifecycle event fetches the current bounded context through the existing native MCP bridge; there is no direct ledger database access and no dependency on MCP already being connected in the host. No historical context is replayed before unrelated tool calls. The adapter does not skip a new lifecycle based on unverified prior stdout. A monotonic task-version floor rejects regressed replies. Already-bound sessions raise this floor even for newly observed paused/incomplete revisions; these observations remain `core_built` and never count as output. A later core rollback therefore cannot resurrect an older active revision. Receipt digests identify the core-built context before separate artifact observations. Mandatory task/checkpoint snapshot consistency is required; optional recall may truthfully report a separate snapshot. Stale checkpoint/version diagnostics, inactive tasks, incomplete requirements, missing bindings, revoked registrations, unsupported input, core unavailability, timeout, and over-budget envelopes degrade to a short warning. This is not a fail-closed tool gate. Changes during a running turn have no proactive push yet; the next supported lifecycle refreshes them.

Receipts in the adapter database contain IDs, version, digest, event/source and time, not prompt/transcript/full context. `core_built` is recorded before stdout; `output_written` requires successful write and flush. A write failure leaves the built receipt and never appends a second JSON output. If receipt persistence fails after stdout, stderr states that the receipt is unconfirmed. A core request timeout is not blindly retried.

`host_delivered`, `model_claimed`, and `result_verified` are distinct protocol levels. There is intentionally no general setter or ordinary CLI/API to promote a receipt into those levels. The database constrains writable stages to built/output. The current bridge cannot provide trusted host acceptance, so status reports delivery unverified and adoption/effect unknown. This is a same-OS-user cooperative boundary; someone who can directly modify the database can change local data. The adapter does not assert authenticated authorization.

The independent adapter database has explicit schema version 1 and explicit initialization; hook runs never initialize/migrate it. It is **not** included in the main ledger's backup/export. Preserve it separately when moving sessions. Initialization never overwrites a file. Receipts currently have no automatic retention policy; operators should monitor the file size. No secrets should be placed in task context.

## Artifact revalidation

The adapter adds separate `adapter_artifact_observations` without changing the core checkpoint or its historical verification state. Only explicit local paths or local file URIs inside the canonical bound workspace are considered. Traversal, symlink paths and external URLs are not followed. Up to 32 artifacts, 8 MiB per file and 32 MiB total are read; missing files report failed, and unknown digests/unsupported/oversized references report not performed. Content matches require an explicit `sha256:` digest. Existence without a supported digest is not content verification. Unix opens use target-native `O_NOFOLLOW | O_NONBLOCK`, and the opened handle must be a regular file, so a final-component symlink/FIFO substitution is rejected without waiting for a writer. Windows retains the declared cooperative reparse-point/handle limitations rather than claiming adversarial same-user race resistance. Metadata is checked around bounded reads; results are point-in-time observations, not durable completion or authorization. This is not a defense against a hostile same-user process racing filesystem changes.

## Verification

Run `cargo test --test codex_context_adapter` and the task-ledger/core integration tests through the normal serialized build workflow. These tests are synthetic and do not establish host delivery. The opt-in `scripts/probe-codex-context-host.py` checks public schema/config parsing in an isolated temporary home, does not trust hooks automatically, does not request turns or model calls, and records exact host errors. Actual injection acceptance remains blocked until a real host produces the required evidence.

### Future real-host acceptance procedure (design only)

The two current gaps are independent: the environment blocked thread creation, and the synthetic hook remains **untrusted**. Even if thread creation succeeds elsewhere, an untrusted/skipped hook is not an injection pass. The supplied probe deliberately never grants trust and therefore cannot by itself complete injection acceptance.

For a later authorized run on a supported machine, retain the exact synthetic home/config produced by the probe, and use only a fixed, officially installed Codex version. Through the CLI's documented `/hooks` interface:

1. Launch that CLI with `HOME` and `CODEX_HOME` pointing to the retained synthetic directories, without importing any user credentials, MCP configuration, or transcript/history. Keep the test-only offline provider and no-model-request restriction.
2. Open `/hooks`, select the handler from that synthetic `hooks.json`, inspect the complete generated command (only the fixed synthetic context marker), and approve/trust **that exact definition** through the native review UI. Do not trust unrelated hooks, alter global/user-real settings, write trust hashes manually, or use a trust-bypass flag. Editing the command invalidates its reviewed hash and requires review again.
3. Using the same synthetic home, call public `initialize`, `initialized`, `hooks/list`, and `thread/start` via `codex app-server --stdio`. Confirm the selected definition's `trustStatus` is `trusted`, with the expected command/hash and zero config errors. Persist no approval beyond this isolated test home.
4. Require an actual public `hook/completed` notification with `status: completed`, `eventName: sessionStart`, and an `entries` item with `kind: context` containing the exact synthetic marker. Record the host version, event, session ID and evidence. A config-list entry, successful bridge stdout, or a normal thread-start response is insufficient.
5. Separately exercise resume/fork with the public thread APIs and verify their actual lifecycle sources. Compaction generally needs a populated turn/history; do not manufacture a success without the requisite public, no-inference test path. If it requires a model call, stop at that missing authorization/evidence rather than sending one. These results remain per-event/per-run, not evidence of model understanding or tool-action enforcement.

This procedure is not currently automated by the probe. A headless CI job must not silently bypass review merely to obtain green status; use the approved native review flow for the exact synthetic definition or report the trust-dependent acceptance as not performed. Delete the synthetic test environment through the operator's normal cleanup after retaining non-sensitive evidence; it does not change the real user's configuration.
