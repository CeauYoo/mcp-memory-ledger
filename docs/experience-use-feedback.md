# Task-bound experience observations (schema 9)

This technical-demo slice records caller-reported experience use. It adds no executor, model invocation, trusted producer identity, causal attribution, or permissions. The existing candidate library, explicit activation, revision and rollback remain authoritative. Candidate procedure steps remain inert knowledge.

## Interfaces

- MCP `record_experience_use`; CLI `experience-use record --json '<request>'`
- MCP `list_experience_uses`; CLI `experience-use list --json '<request>'`
- MCP `derive_experience_from_use`; CLI `experience-use derive --json '<request>'`

The shared application module is `application::experience_use`. Registered core sessions use their exact project grant, with scope-first validation and per-call session revalidation. Standalone CLI is explicit local database administration. Opening does not initialize or migrate implicitly.

### Immutable observation

```json
{
  "namespace":"project/example","request_id":"use-write-1","use_id":"use-1",
  "task_id":"release-review","task_version":1,"expected_task_version":1,
  "candidate_id":"cache-lesson","candidate_version":2,"expected_candidate_version":2,
  "decision":"adopted","reason":"Caller chose this suggestion for a staging check",
  "conditions":[{"condition":"Only staging, same cache implementation","assessment":"caller_reported_satisfied"}],
  "action_event_refs":["event:staging-attempt"],"outcome_event_refs":["event:staging-result"],
  "verification":"passed","evaluation_method":"Caller-reported local test",
  "limitations":["Single run; no independent verification or causal attribution"],
  "related_use_id":null
}
```

The task revision and candidate version are exact historical bindings. `expected_task_version` and `expected_candidate_version` independently compare current heads in the same `BEGIN IMMEDIATE` write transaction. New `proposed` or `adopted` observations require a current active task and current active candidate. `rejected` can describe a historical version. Both candidate source closure and action/outcome event ownership are checked in the exact namespace.

Decisions are `proposed`, `adopted`, `rejected`. Applicability assessments are `caller_reported_satisfied`, `caller_reported_not_satisfied`, `unknown`. Every report requires 1–16 explicit conditions, a reason, an evaluation method and 1–16 limitations. Unknown natural-language conditions are never machine-verified. A new adoption requires all supplied conditions to be caller-reported satisfied; this does not establish that the caller supplied every relevant condition or that their reports are true. Verification is `not_performed`, `passed`, `failed`, or `inconclusive`; any performed verification requires existing same-scope outcome references.

Limits: 32 references per event role, 1,024 bytes per condition, 4,096 bytes for reason/method, 2,048 bytes per limitation, 32,768 bytes for the complete request. Identical retries return the original receipt even after withdrawal; changed payload under the same request key fails. Use IDs are immutable and cannot be overwritten. Receipts and relational writes commit atomically.

Later results use a new `use_id` with `related_use_id` pointing to the earlier observation. They preserve the exact task/candidate binding and historical decision, but may add corrected outcome and verification reports after either head has changed. The expected heads must still match the current heads. A changed decision is a separate new observation, not a mutation of the old one; task/candidate identifiers correlate the history. Historical `passed` is never rewritten to `failed` because the candidate was withdrawn.

### History and invalidation

List request: `{"namespace":"project/example","task_id":"release-review","limit":20,"after_id":null}`. Limit is 1–100; paginate by the last returned use ID. Results are ordered by use ID, not by time. Each view contains the immutable `record`, `currently_reusable`, and explicit `invalidation_reasons`. Reusability is computed in a fresh read transaction from current task/candidate heads, candidate status/source closure, event sources, decision and reported applicability. It is only structural eligibility under caller reports, never independent effectiveness verification or permission to execute.

Candidate revision, rejection or rollback immediately makes old versions ineligible on subsequent reads. Historical rows stay available. Resume context rechecks selected candidate heads and warns about applicability uncertainty; it also scans at most 100 use records to flag prior invalidated adoptions, explicitly diagnosing a potentially incomplete scan. Paginate history for complete review. These optional reads are not one global atomic snapshot. Changes after the final read require a new context.

There is no persisted experience-use eligibility cache. Derived eligibility is recomputed, and fresh context omits inactive candidate heads. Derived pending candidates created from feedback are separate reviewable proposals, not cached endorsements of the source experience. No claim is made that already injected model context can be remotely erased; rebuild context and inspect the current versions before relying on it.

### Recoverable use → Episode → pending candidate composition

`derive_experience_from_use` takes `namespace`, `request_id`, `use_id`, `episode_id`, `candidate_id`, `episode_content` (existing `EpisodeContent`) and `candidate_content` (existing `ExperienceContent`). Request limit is 65,536 bytes. The new Episode's nonempty source references must be a subset of the selected use's actual action/outcome event links, checked in the exact scope. Candidate source IDs must contain exactly the new Episode ID.

1. Atomically validate provenance and reserve the hash of the complete derivation request in the existing receipt log.
2. Call existing `create_episode` with a deterministic child retry key.
3. Call existing `create_candidate` with another deterministic child retry key; the result is pending.

The returned `reservation_id`, `use_id`, `episode_id`, `candidate_id`, `stage`, optional `episode`/`candidate` write results, and `retryable_error` reflect actual completed steps. Stages are `reserved`, `episode_created`, and `pending_candidate_created`. This is recoverable composition, not an all-or-nothing multi-step transaction. Fix a blocker then retry the identical full request. A conflicting full payload is rejected even if a later stage previously failed. A returned error may require choosing a new request and IDs rather than blind retries. Replaying a completed candidate step returns its original pending receipt, not a statement of its present activation status; read the current candidate head separately.

Corrections can also use the existing Episode, revise-candidate and rollback APIs. Revision/rollback creates pending versions and requires separate explicit activation. All source descriptions, observations and inferred lessons remain caller supplied; this slice does not generate drafts with a model or run automatic learning.

## Lifecycle and verification

Migration 9 adds only `experience_uses` and `experience_use_events`, indexes and migration ledger entry. It does not rewrite schema 8 or old candidate/task payloads and receipt bytes. Canonical schema doctor, required/preserved table inventories and full SQLite backup cover both new tables. Scoped memory export explicitly excludes experience-use observations; it is not a full backup. Legacy adoption refuses task/use history until a source-closure contract supports it.

Added deterministic tests cover scope and CAS, applicability, receipt conflict/replay, withdrawal and immutable results, late outcome observations, context omission, recoverable pending derivation, source membership, receipt failure rollback, and schema-8 migration byte preservation. Interface tests cover strict fields, registered-session policy, tool schemas and CLI boundaries. Suggested verification after integration: `cargo test --test experience_use_feedback --test experience_workflow --test task_ledger --test sqlite_lifecycle --test mcp_session_policy --test mcp_stdio --test local_core`.

Status: T13's bounded local observation and traceability core is implemented. T12 and T15 have current-head revalidation/invalidation support but no semantic applicability engine or remote context erasure. T14 has an explicit recoverable caller-authored composition. T16 remains partial: evidence-bounded caller-authored pending proposals exist, but model draft generation and host automatic triggering are separate work. No real-model or paid-provider results are claimed by these tests.
