# Codex result feedback candidate (T14)

This is a technical-demo candidate, not installed automatic protection or verified native-host delivery. It observes metadata and writes knowledge only. It grants no execution permissions, verifies no success, infers no causal benefit from experience use, and never activates a candidate. Original `codex-actions` receipts remain unchanged with outcome `unknown`.

## Pinned public contract

Codex **0.159.2** has a real `PostToolUse` event, rather than an invented callback:

- [Official generated input schema](https://github.com/openai/codex/blob/rust-v0.159.2/codex-rs/hooks/schema/generated/post-tool-use.command.input.schema.json)
- [Official input serialization](https://github.com/openai/codex/blob/rust-v0.159.2/codex-rs/hooks/src/events/post_tool_use.rs#L144-L166)
- [Core dispatch](https://github.com/openai/codex/blob/rust-v0.159.2/codex-rs/core/src/hook_runtime.rs#L297-L327)
- [Shell integration assertions](https://github.com/openai/codex/blob/rust-v0.159.2/codex-rs/core/tests/suite/hooks.rs#L5453-L5462), [delayed unified-exec assertions](https://github.com/openai/codex/blob/rust-v0.159.2/codex-rs/core/tests/suite/hooks.rs#L5802-L5819)

Input includes session/turn/tool IDs, cwd, tool input, and tool-specific JSON output. There is no universal successful-execution field. Shell results may be plain text. A nonzero shell result may still trigger the event. The collector does not parse success-like text or grant permissions from `permission_mode`. `transcript_path`, model and agent fields are accepted only for schema compatibility; no transcript is read.

Hosted tools and specialized bypass paths are not complete coverage. Deferred unified-exec output can be reported under the original tool ID. This implementation additionally requires the original turn ID; a cross-turn mismatch stays unassociated rather than guessing. No host replay guarantee is assumed. Hooks cannot undo side effects and may fail to run. Source/schema inspection and synthetic direct-input tests do not prove actual hook delivery.

## Explicit selection and candidate-only configuration

First register the workspace and bind the session/task with `codex-context`. This adapter never selects a task by cwd or adopts the newest recalled experience. The selected use must already exist and be currently reusable. Its immutable task/candidate revisions and historical decision are captured before execution. A context session watermark rejects a newly linked task older than a previously consumed task revision. Use lookup is capped at 100 pages of 50 records and an eight-second between-call deadline; a bridge call itself is capped at four seconds.

Prepare an absolute path in an existing private, non-synchronized directory (Unix 0700/current owner, file 0600; Windows private ACL checks):

```sh
agent_llm_mm codex-results init --outbox /ABS/private/result-outbox.sqlite
agent_llm_mm codex-results preview \
  --outbox /ABS/private/result-outbox.sqlite --state /ABS/private/context.sqlite \
  --registry /ABS/private/registry.json --socket /ABS/private/core.sock \
  --use-id EXPLICIT_EXISTING_USE
```

Preview prints JSON only. It does not write, merge, install, trust, or modify configuration. It generates an explicit-use `PreToolUse` link handler and `PostToolUse` capture handler, both **apply_patch-only**, with 30-second timeouts and POSIX shell quoting. Windows command preview is deliberately unsupported. Changing the use requires reviewing a new candidate; this is intentionally not automatic experience selection. Other existing hooks must be preserved during any separately authorized deployment.

`link` takes public `PreToolUse` JSON on stdin and requires `--use-id`; `capture` takes public `PostToolUse` JSON and omits that flag. Both use the four absolute-path flags above. They always print `{}` and generic incomplete-operation diagnostics on failure. This is passive collection, not an action gate: missing links do not stop execution. A successful hook process alone is not proof of completed core feedback; inspect the outbox.

## Durable capture and delivery

The adapter matches session, turn, tool name/ID, workspace binding, and canonical argument digest. Links cannot be reassigned. An unlinked result, fork, mismatched arguments, or changed duplicate output is rejected rather than attached to the latest use. Existing action-check receipts are neither read as proof of execution nor changed into success receipts.

Raw stdin is limited to 2 MiB; duplicate object keys at every level, depth over 32, and oversized per-container entry counts are rejected. Only strict ASCII metadata identifiers, configured binding metadata, digests/counts/timestamps, and fixed generated descriptions survive. Raw tool input/output, commands, patches, excerpts, transcript paths, arbitrary summaries, and remote error text are not persisted or printed. The outbox is bounded to 10,000 records and 16 KiB per record; it never evicts deduplication evidence to make space.

SHA-256 input/output digests permit conflict detection without storing originals. They are **not** authentication or anonymization: guesses of low-entropy secret content can be tested against a digest. Keep the private outbox private. Same-OS-user tampering is not prevented; labels such as `tool_reported` do not authenticate a host. The normalized evidence hash, tool/action IDs and historical revisions are included in the fixed core Event summary.

Capture is committed before delivery. A fresh scoped, revocation-aware core bridge handles every read/write; no direct core-database writes or unscoped auto-reflection occur. Stages are:

1. Immutable action/use link.
2. Minimized captured result.
3. Exact event request durably prepared, then Event ID recorded after confirmation.
4. Exact related-use request durably prepared, then full returned observation checked before completion.

The Event has no claim drafts and `ToolReported`/`NotPerformed` feedback. The related-use observation preserves historical task/candidate versions and decision, references the result Event, and sets applicability to unknown. Neither success nor adoption is newly inferred. Normal read-time invalidation still applies.

Both request payloads and idempotency keys are durable before sending. Same requests survive a crash or uncertain core timeout. Stored payloads must exactly match the schema-version-one minimized serializer; incompatible future template changes must fail pending records closed or provide explicit migration, never silently rewrite a pending request under its original key. Current-head CAS expectations are distinct from historical revisions, and observed head floors cannot decrease across attempts.

```sh
agent_llm_mm codex-results inspect --outbox /ABS/private/result-outbox.sqlite \
  --session-id SESSION --tool-use-id TOOL_ID
agent_llm_mm codex-results retry --outbox /ABS/private/result-outbox.sqlite \
  --state /ABS/private/context.sqlite --registry /ABS/private/registry.json \
  --socket /ABS/private/core.sock --session-id SESSION --tool-use-id TOOL_ID
```

`retry` replays uncertain writes exactly. Confirmed Event/use rejections remain stopped; an operator may explicitly add `--retry-rejected true` after resolving the cause. Only a confirmed rejected use permits fresh CAS expectations and a new bounded attempt key; an uncertain response does not. Maximum use retry transitions: 32. Event request identity stays constant. Partial event-only progress is reported rather than called atomic completion. Capacity maintenance, revocation, conflicting records, and incompatible migrations require explicit handling; there is no automatic deletion or grant repair.

Deriving an Episode/pending candidate remains the existing explicit `derive_experience_from_use` operation. This adapter never invokes derivation, reflection, a model, candidate activation, or permission-policy updates.

## Verification and limits

Run module tests and direct-input real-core smoke as documented in the testing guide. They cover bounded parsing, privacy canaries, immutable links, conflicts, stage replay, uncertain versus rejected writes, historical versions, head floors, scoped revocation, and actual bridge transport. They do **not** invoke Codex and do not establish native `PostToolUse` acceptance.

A new pinned-package synthetic native-host result probe remains pending. The pre-existing startup/resume context probe does not cover this adapter. No actual hooks, user-machine settings, credentials, or paid provider were changed by this implementation.

Local acceptance record (2026-10-20): module tests run separately from the integration smoke. The real-core smoke cannot reach seeding in this cloud sandbox: Unix socket creation returns `Operation not permitted`. A same-command reviewed escalation failed in environment startup before the command launched. No denied filesystem was accessed or altered, and no alternate transport or permission workaround was attempted. The CI step is prepared for later authorized publication; it has not run for this candidate. Native-host result acceptance is still pending.

Retry classification is conservative: only the reviewed local core's JSON-RPC `InvalidParams` validation/CAS response is a confirmed write rejection. Generic internal errors or MCP `isError` results can follow a durable write and therefore remain uncertain with the original request/key. Error text is never parsed for authorization or persisted.

Validated locally on Linux for this candidate: 27 `codex_results` module tests, 8 existing experience-use feedback tests, 21 scoped-session policy tests, strict `cargo clippy --lib --tests -- -D warnings`, formatting/whitespace checks, Python smoke syntax, and a non-installing CLI preview/failure-redaction check. These do not establish macOS/Windows execution, live scoped-core delivery, or native-host acceptance. The prepared two-case real-core smoke remains environment-blocked before seed data is created.
