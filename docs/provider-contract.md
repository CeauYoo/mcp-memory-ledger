# Provider Readiness Checklist

## 0. Scope

- This repository remains a local Rust MCP `stdio` technical demo / MVP, not a production provider gateway.
- This checklist records the implemented protocol boundary and readiness requirements for future providers; documentation alone does not establish live readiness.
- A new provider should match the minimum observable behavior already expected from `mock`, `openai-compatible`, `openrouter`, `openai-responses`, and `anthropic`: deterministic config loading, bounded network behavior, explicit parse errors, redacted diagnostics, and preserved MCP `stdio` behavior.
- Items are marked `existing`, `partial`, or `gap` against the current test suite. For a new provider, every applicable item must either be `existing` or gain a provider-specific regression in the same change. `partial` and `gap` items are blockers for broadening provider support until the missing regressions are added or an explicit documented exception is accepted for that provider.

## 1. Provider Checklist

### Current Provider Matrix

The provider matrix is a read-only contract exposed by config and `doctor`.
It is not a provider router and does not make future providers usable.

| Provider | Support state | Configurable | Adapter status | Missing implementation before support |
| --- | --- | --- | --- | --- |
| `mock` | supported | yes | built-in deterministic mock | none |
| `openai-compatible` | supported | yes | OpenAI-compatible chat completions | none |
| `openrouter` | supported | yes | OpenRouter chat completions via OpenAI-compatible transport | none |
| `openai-responses` | supported | yes | native OpenAI Responses, text/non-streaming | live evidence remains separate |
| `anthropic` | supported | yes | native Anthropic Messages, text/non-streaming | live evidence remains separate |
| `azure-openai` | planned-only | no | not implemented | config parser, doctor diagnostics, model adapter, error handling, redaction, MCP stdio tests |
| `local` | planned-only | no | not implemented | config parser, doctor diagnostics, model adapter, error handling, redaction, MCP stdio tests |

`doctor` reports this matrix for diagnostics, including which supported
provider is currently selected and the missing implementation checklist for
planned-only rows. Planned-only providers must remain non-configurable until a
real adapter, validation contract, redaction behavior, error handling, and
provider-specific MCP `stdio` regressions are added in the same change.

| Item | Required behavior | Current verification status | Current signals |
| --- | --- | --- | --- |
| Config validation behavior | Config loading must preserve provider selection, provider-specific fields, config-file precedence, and required-field validation. Missing required provider fields must fail before normal runtime use. | existing | `tests/provider_config.rs` covers default `mock`, explicit `openai-compatible` and `openrouter` config loading, example TOML parsing, `AGENT_LLM_MM_CONFIG`, `AGENT_LLM_MM_DATABASE_URL`, and missing required field failures through `doctor`. |
| Provider matrix diagnostics | `doctor` must report the current provider matrix as a read-only contract and must not mark future providers as supported or configurable. Planned-only rows must expose the missing config parser, doctor diagnostics, model adapter, error handling, redaction, and MCP stdio test prerequisites. | existing | `tests/provider_config.rs::provider_matrix_lists_supported_and_future_providers_as_contract_only`, `tests/provider_config.rs::future_providers_are_rejected_by_config_parser_until_implemented`, and `tests/provider_config.rs::doctor_reports_provider_matrix_without_marking_future_providers_supported`. |
| `doctor` redaction behavior | `doctor` may expose provider, base URL origin shape, model, provider matrix, status, and runtime readiness, but must not expose API keys, URL userinfo, path content, query values, or equivalent secrets. | existing | `tests/provider_config.rs::doctor_report_does_not_contain_api_key_in_serialized_output` and `tests/provider_config.rs::doctor_reports_openrouter_provider_without_exposing_api_key` assert provider secrets are absent from serialized reports. |
| Timeout handling | Provider network calls must use bounded timeout configuration and surface timeout failures as provider errors, not hangs or silent fallback. | existing | `tests/openai_compatible_model.rs::openai_compatible_model_surfaces_timeout_as_error` confirms that a 200ms timeout against a non-responding server surfaces as a provider error. |
| Non-success HTTP status behavior | Non-2xx provider responses must return observable provider errors. | existing | `tests/openai_compatible_model.rs::openai_compatible_model_surfaces_non_success_status`. |
| Malformed JSON behavior | Malformed or schema-incompatible model responses must fail without panic and without fabricating a valid decision or self-revision proposal. | existing | `tests/openai_compatible_model.rs::openai_compatible_model_fails_gracefully_on_malformed_json_response` confirms that completely invalid JSON is surfaced as a provider error without panic. |
| Decision action parsing | Chat Completions uses the first assistant message; native protocols extract completed text before the shared action parser. Blank text is rejected. | existing | `openai_compatible_model_parses_first_assistant_message_into_action` and `openai_compatible_model_rejects_empty_action`. |
| Self-revision proposal parsing | `should_reflect`, `rationale`, `machine_patch`, and defaulted patch fields must parse consistently, including fenced JSON. | existing | `openai_compatible_model_parses_self_revision_proposal_from_assistant_message`, `openai_compatible_model_defaults_missing_machine_patch_in_self_revision_proposal`, and `openai_compatible_model_accepts_fenced_json_self_revision_proposal`. |
| Evidence policy parsing | `proposed_evidence_event_ids`, `proposed_evidence_query`, and `confidence` must parse into the structured self-revision proposal contract. | existing | `openai_compatible_model_parses_self_revision_evidence_policy`. |
| MCP `stdio` provider path | Config-selected provider behavior must flow through the real MCP `stdio` path without corrupting protocol output. | existing | `tests/mcp_stdio.rs::decide_with_snapshot_over_stdio_uses_openai_compatible_provider_from_config_file`, `tests/mcp_stdio.rs::decide_with_snapshot_over_stdio_uses_openrouter_provider_from_config_file`, and `tests/mcp_stdio.rs::ingest_interaction_auto_reflection_uses_openrouter_provider_from_config_file`. |

## Native protocol contract

Provider selection is explicit; a model name does not select or switch the wire
protocol. Existing Chat Completions/OpenRouter configuration remains compatible.
OpenAI recommends Responses for new projects and continues to support Chat
Completions; this change does not deprecate the latter. See the official
[OpenAI migration guide](https://developers.openai.com/api/docs/guides/migrate-to-responses)
and [Anthropic Messages reference](https://platform.claude.com/docs/en/api/messages/create).

| Provider | TOML section | Configured base URL + appended endpoint | Authentication | Request shape |
| --- | --- | --- | --- | --- |
| `openai-compatible` | `[model.openai_compatible]` | configured base + `/chat/completions` | Bearer | existing chat messages |
| `openrouter` | `[model.openrouter]` | configured base + `/chat/completions` | Bearer | existing chat messages |
| `openai-responses` | `[model.openai_responses]` | `https://api.openai.com/v1` + `/responses` | Bearer | `instructions`, `input`, `store: false`, `max_output_tokens` |
| `anthropic` | `[model.anthropic]` | `https://api.anthropic.com/v1` + `/messages` | `x-api-key`, `anthropic-version: 2023-06-01` | `system`, `messages`, `max_tokens` |

Set `base_url` explicitly (the official roots above are examples, not implicit defaults).
Native sections accept `base_url`, `api_key` or `api_key_env`, `model`,
`timeout_ms`, `max_tokens` (default `2048`), and optional `temperature`.
Temperature is omitted by default rather than forcing a value unsupported by
some models. Explicit values must be finite, 0–2 for Responses and 0–1 for Anthropic,
and still need to be supported by your selected model. `max_tokens` maps to Responses `max_output_tokens` and Anthropic
`max_tokens`; it bounds model output, not the ledger's context-byte budget.
Choose a model available to your own account that supports the selected text
protocol. Base URLs are API roots, not full endpoint URLs. All HTTP adapters now reject
URL userinfo, query strings and fragments; supply credentials through the API-key
field/environment variable. Redirecting gateways must be configured at their final API root. Configure secrets in
local environment variables or an ignored private TOML; never commit a key.

Shared pure prompt construction and decision/self-revision parsing are separate
from each protocol's request/response mapping. A shared HTTP side-effect layer
owns authenticated POST, one configured request timeout, disabled redirects,
status checks, and safe error classification. There are no automatic retries,
provider failover, or silent mock fallback. API keys are redacted in Debug; malformed TOML diagnostics report location without source excerpts;
errors must not include raw response bodies, credentials, or secret-bearing URLs.

Native extraction accepts completed assistant text, including multiple text
blocks in order, and then applies the same domain parser. Provider reasoning /
thinking blocks are not included in action or proposal text. Refusals, incomplete
or truncated generations, unexpected native statuses, tool output, empty text,
and malformed envelopes fail closed rather than being committed as decisions or
self-revisions. Reflection still goes through the existing evidence/governance
checks and `run_reflection`; an adapter cannot create a second write path.

Scope is non-streaming text only: no tools/tool execution, vision, hosted
conversation state, provider-side memory, or native structured-output guarantee.
Responses explicitly sends `store: false`; that request flag is not a general
promise about the provider's data-retention policy. Local HTTP fixtures verify
wire contracts and both decision/reflection paths without paid API calls. They
do not establish endpoint reachability, model quality, live certification, or a
release gate. The existing live certification runner rejects native providers;
preflight keeps `live_certified = false` for both, including when evidence files
are supplied. Extending that runner requires separate work and authorization.

The detailed legacy test-name lists below remain a Chat Completions compatibility
baseline. Native protocol and integration tests supplement them, rather than
replacing them; see the [testing guide](testing-guide-2026-03-24.md).

## 2. Structured Decision Protocol

`decide_with_snapshot` uses a versioned response envelope while preserving the
older minimal action field:

| Field | Status | Meaning |
| --- | --- | --- |
| `protocol_version = 2` | implemented | Additive decision envelope version. |
| `decision_id` | implemented | Local opaque identifier derived from the requested action for traceability; it is not a global scheduler or execution id. |
| `requested_action` | implemented | The action requested by the caller. |
| `selected_action` | implemented | The model-selected action when the commitment gate passes; `null` when blocked. |
| `decision.action` | implemented compatibility alias | Backward-compatible minimal action string retained for existing callers. |
| `blocked` | implemented | `true` when the commitment gate blocks before the model call. |
| `status` | implemented | `blocked` or `model_decision`. |
| `reason` | implemented | Bounded reason string such as `commitment_gate_blocked_action`; `null` for model decisions. |
| `gate` / `policy_checks` | implemented | Current commitment-gate metadata. This is not a full policy arbitration engine. |
| `confidence` | implemented as bounded local metadata | Currently `bounded-local-metadata` for model decisions; not provider-native scoring. |
| `non_claims` | implemented | Machine-readable boundaries that the decision is not full planning, policy arbitration, provider-native structured JSON, or rich confidence scoring. |
| provider diagnostics class | partial | Provider transport/status/parse failures surface as MCP errors and safe operation-log diagnostics; the decision envelope does not yet carry a separate provider diagnostics object. |

Compatibility and safety requirements:

- blocked decisions must return `blocked = true`, `decision = null`,
  `selected_action = null`, and must not call the provider
- non-blocked decisions must keep the legacy `decision.action` field alongside
  `selected_action`
- malformed provider HTTP JSON must fail without panic and without fabricating a
  valid decision
- serialized decision responses and diagnostics must not expose provider API
  keys, URL userinfo, path content, query secrets, or raw provider error
  payloads
- the envelope does not execute tools, schedule work, or create identity /
  commitment / reflection writes

Current verification:

```zsh
cargo test --test decision_flow -v
cargo test --test mcp_stdio decide_with_snapshot_over_stdio_uses_openai_compatible_provider_from_config_file -v
cargo test --test mcp_stdio baseline_commitment_blocks_forbidden_identity_write_over_stdio -v
cargo test --test openai_compatible_model -v
```

## 3. Current Coverage Map

`tests/provider_config.rs`:

- `default_config_uses_mock_provider_when_no_config_file_is_present`
- `load_from_path_reads_openai_compatible_provider_from_toml_file`
- `load_from_path_reads_openrouter_provider_from_toml_file`
- `load_prefers_config_path_from_environment`
- `load_prefers_database_url_env_over_default_config_file`
- `dev_example_config_parses_without_real_secrets`
- `prod_local_example_config_parses_with_local_dashboard_and_disabled_daemon`
- `openrouter_example_config_parses_without_live_looking_secret`
- `generic_example_config_parses_and_keeps_daemon_disabled`
- `provider_matrix_lists_supported_and_future_providers_as_contract_only`
- `future_providers_are_rejected_by_config_parser_until_implemented`
- `doctor_fails_when_openai_provider_config_is_missing_api_key`
- `doctor_reports_openrouter_provider_without_exposing_api_key`
- `doctor_fails_when_openrouter_provider_config_is_missing_model`
- `doctor_report_does_not_contain_api_key_in_serialized_output`
- `doctor_reports_provider_matrix_without_marking_future_providers_supported`

`tests/openai_compatible_model.rs`:

- `openai_compatible_model_parses_first_assistant_message_into_action`
- `openai_compatible_model_rejects_empty_action`
- `openai_compatible_model_surfaces_non_success_status`
- `openai_compatible_model_parses_self_revision_proposal_from_assistant_message`
- `openai_compatible_model_defaults_missing_machine_patch_in_self_revision_proposal`
- `openai_compatible_model_accepts_fenced_json_self_revision_proposal`
- `openai_compatible_model_parses_self_revision_evidence_policy`
- `openai_compatible_model_fails_gracefully_on_malformed_json_response`
- `openai_compatible_model_surfaces_timeout_as_error`

`tests/mcp_stdio.rs`:

- `decide_with_snapshot_over_stdio_uses_openai_compatible_provider_from_config_file`
- `decide_with_snapshot_over_stdio_uses_openrouter_provider_from_config_file`
- `ingest_interaction_auto_reflection_uses_openrouter_provider_from_config_file`

## 4. Required Validation Commands

Run these before treating a new provider as ready for the MVP track:

```zsh
cargo test --test provider_config -v
cargo test --test openai_compatible_model -v
cargo test --test native_model_protocols -v
cargo test --test mcp_stdio -v
cargo test --test mcp_stdio decide_with_snapshot_over_stdio_uses_openai_compatible_provider_from_config_file -v
cargo test --test mcp_stdio decide_with_snapshot_over_stdio_uses_openrouter_provider_from_config_file -v
cargo test --test mcp_stdio ingest_interaction_auto_reflection_uses_openrouter_provider_from_config_file -v
cargo test --test support_bundle support_bundle_reports_openrouter_config_shape_without_provider_secrets -v
```

If the new provider adds provider-specific parsing or transport behavior, add provider-specific regressions alongside these commands rather than weakening the shared contract.

## 5. Provider Expansion Requirements

- Keep the existing `doctor` redaction, timeout, malformed JSON, non-success
  status, provider matrix, and MCP `stdio` provider-path regressions passing.
- Add provider-specific config, redaction, timeout, non-success, malformed
  response, and MCP `stdio` path regressions for the new adapter in the same
  change that marks it supported.
- Keep `run_reflection` as the only durable self-revision write path; provider
  work must not introduce a side-channel durable write path.

## 6. Non-Goals

- This checklist does not add Azure OpenAI, local gateway, or any other future provider.
- The provider matrix does not make `azure-openai`, `local`, or any other future provider configurable or runnable.
- OpenRouter support is limited to the OpenAI-compatible chat completions transport.
  Config examples or local stub verification are not live evidence; users must run
  the explicit live runner before the provider preflight can count live evidence
  or set the narrow `live_certified = true` flag.
- Provider certification preflight remains local and read-only. A live evidence
  file is only present when provider, `status = passed`, expected
  `evidence_kind`, `mode = live`, non-empty `generated_at`,
  `local_only = false`, `endpoint_reached = true`,
  `redaction_reviewed = true`, `request_outcome = passed`, and successful
  command evidence with `name = provider-live-certification`,
  `command = scripts/provider-live-certification-run.sh --live` or
  `command = ./scripts/provider-live-certification-run.sh --live`, and explicit
  `exit_code = 0` all match the expected slot. Stub/simulated provider
  evidence, placeholder JSON, thin self-labeled live JSON, wrong-provider files,
  failed files, malformed JSON, unsupported command evidence, or files missing
  those live metadata/provenance fields must stay invalid. Complete live
  evidence still must not set `live_certified = true` unless config preflight
  also passes.
- Provider certification summary may report redacted provider config shape such
  as base URL host shape, timeout, and credential-configured boolean, but model
  ids are serialized only as `<redacted-model>`.
- `scripts/provider-live-certification-run.sh --live` is a bounded evidence
  generator for the preflight above. It may call the configured provider only
  to record endpoint reachability, a decision probe, a self-revision parse
  probe, error-handling provenance, and redaction-review provenance. It must
  not serialize API keys, URL userinfo, URL path content, query values, model
  ids, request bodies, response bodies, or provider-native payloads. A passing
  run may satisfy the bounded preflight evidence slots, but it is not provider
  quality certification, SLA or gateway certification, Local Alpha approval,
  Beta, GA, production-ready, release approval, production readiness, or proof
  that model decisions/self-revisions are useful.
- This checklist does not turn the project into a remote provider service or production credential manager.
- This checklist does not change the MCP tool contract, dashboard boundary, or automatic self-revision runtime hooks.
