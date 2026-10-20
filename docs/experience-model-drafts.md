# Evidence-bounded experience model drafts

`generate_experience_draft` is an explicit, opt-in technical-demo/MVP operation. Its request contains `namespace`, `request_id`, `use_id`, `episode_id`, `candidate_id`, and `allow_model_generation: true`. Omitted or false consent is rejected before reservation or provider use. No automatic trigger is installed.

The model receives only the selected immutable experience-use observation and its exact action/outcome Events in the same project namespace. Relational edges are checked before reservation. Input is bounded to 128 KiB; oversized evidence fails closed rather than being silently truncated. Provider responses must be a single strict JSON object of at most 64 KiB with `episode_content` and `candidate_content`. Unknown fields, invented evidence references, an unrelated source episode, invalid content and malformed JSON are rejected. Evidence is untrusted input, not instructions. Recorded verification is caller-reported; a model draft is not independent verification or proof of causality.

The shared typed ModelPort operation defaults to unsupported. Chat Completions, Responses and Anthropic use one prompt/parser contract. All shared HTTP provider envelopes are bounded to 16 MiB using both advertised content length and actual body chunks, with existing timeout, redaction and no-retry behavior preserved. These size limits are separate from token budgets.

## Durable attempt semantics

An immutable operation-log reservation is committed before the model call. It includes the bounded selected input and a hash of the complete caller request. Concurrent or repeated identical keys never cause a second model call; changed payloads with the same key are rejected. A separate immutable terminal receipt records either a saved validated proposal, a known failure, or rejection. Raw provider errors are not persisted. No schema upgrade is required. The new draft receipt loader and model-input path enforce byte bounds; inherited Episode/candidate derivation receipt readers are unchanged and are not guaranteed bounded against arbitrarily corrupted database rows. Database tampering is outside this normal-write contract.

- `incomplete_or_in_flight`: reservation exists but no terminal result. The original invocation may still be running, or may have stopped during or after a provider call. The system cannot safely distinguish these. It never automatically retries the model for this key.
- `known_failed`: the local invocation returned a provider/transport error or unsupported-operation error; repeating the key returns this terminal outcome. A timeout does not prove the remote provider failed to process or bill the request. This is a known local failure, never authorization to retry automatically.
- `rejected`: the proposal or provider-parsed output was invalid; repeating the key does not call again.
- `saved_proposal`: validated content is durable, and downstream derivation has not finished.
- `pending_candidate_created`: existing Episode and candidate composition completed. The new candidate is pending, with separate explicit activation required.

Saved-proposal replay calls only the existing deterministic derivation composition, never the model. Its Episode and candidate writes are individually idempotent; the composition is recoverable, not one atomic transaction. An unrelated ID collision can block derivation and requires caller resolution. A new request key authorizes a new attempt, including a possible new provider charge; an incomplete old attempt must first be investigated by the caller.

Generation exposes no tools, execution rights, permission changes, automatic activation or identity/self-policy updates. It does not claim semantic grounding beyond bounded source membership. Host automatic feedback and paid/live-model quality validation remain separate work.

## Offline verification

Run `cargo test --test experience_draft`, provider prompt/transport unit tests, `cargo fmt --check`, and strict Clippy. Synthetic fixtures cover explicit consent, exact scope, pending-only derivation, single-call replay/concurrency, terminal failures and redaction, crash-ambiguous reservations, saved-proposal recovery, oversized evidence and tampered edges. No real keys or paid model calls are needed. Source-specific test outcomes belong in the current verification record, not inferred from test presence.
