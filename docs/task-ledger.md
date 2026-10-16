# Workspace task ledger (P1, schema 8)

The MVP persists caller-reported requirements and resumable context. It does not execute a plan, import authenticated user approval, or create permissions. Source kinds and verification labels are untrusted caller metadata. A model can call itself `caller_reported`; these types reduce accidental misuse, not producer forgery. A same-scope event reference proves existence and scope, not the truth of its content. There is no trusted user-event importer in this slice.

## Operation contract

All operations take an exact `project/*` namespace and stable `task_id`. A registered core session obtains its actual project namespace from its registry grant, including an approved legacy namespace; the server rejects cross-project calls and revalidates the session on every call. The standalone local CLI is explicit database administration under the local OS-user boundary, not a remote scoped session. Never infer the selected task from the current directory.

| MCP tool | Shared application function | CLI operation |
| --- | --- | --- |
| `create_task` | `application::task::create_task` | `task create` |
| `revise_task` | `application::task::revise_task` | `task revise` |
| `get_task` | `application::task::get_task` | `task get` |
| `list_tasks` | `application::task::list_tasks` | `task list` |
| `record_task_checkpoint` | `application::task::record_task_checkpoint` | `task checkpoint` |
| `build_task_resume_context` | `application::task::build_task_resume_context` | `task context` |

CLI syntax is `agent_llm_mm task <operation> --json '<request JSON>'`, using the configured database. It requires a current database and never initializes or migrates implicitly. `get_task_snapshot(store, namespace, task_id)` is an additional read-only service for the dashboard; it returns current task plus latest checkpoint in one transaction, without optional recall or budget truncation. Existing query-based `build_task_context` is unchanged.

### Create

```json
{
  "namespace":"project/example",
  "task_id":"release-review",
  "request_id":"create-release-review-1",
  "title":"Review release readiness",
  "source":{"kind":"caller_reported","reference":null},
  "reason":"Initial task requirements",
  "requirements":[{
    "requirement_id":"no-deployment",
    "kind":"prohibition",
    "text":"Prepare the review without deploying to the user computer.",
    "state":"reported",
    "source":{"kind":"caller_reported","reference":null},
    "replaces":null
  }]
}
```

A successful mutation returns `{record, replayed, operation_id}`. `operation_id` is a hashed local durable receipt identifier, not the raw request key. Identical retries return the original record even after later revisions. Reuse of the same operation/namespace/request key with different parameters fails. Mutation and receipt commit atomically under `BEGIN IMMEDIATE`; this says nothing about external tool side effects.

### Requirements and revisions

`TaskRevision` has namespace, task ID, version, title, status, requirements, source, reason and recorded time. Versions are immutable; only the head pointer changes. Requirements have stable IDs and one of `goal`, `requirement`, `prohibition`, `completion_criterion`, `open_question`. Their state is `reported`, `candidate`, or `withdrawn`. Reported means the caller presents it as a current requirement; it is not independently authenticated approval.

`revise_task` requires `expected_version`, reason, typed source, and an array of explicit changes:

- `{"operation":"add","requirement":{...}}` keeps all existing entries and rejects duplicate IDs.
- `{"operation":"replace","requirement_id":"old","requirement":{"requirement_id":"new",...,"replaces":"old"}}` withdraws the old entry and adds a new stable ID; old revisions remain unchanged.
- `{"operation":"withdraw","requirement_id":"old"}` explicitly withdraws one entry.

Model/tool-labelled extraction may only add candidates. It cannot replace or withdraw reported requirements or change lifecycle. This is a structural contract; the caller still controls the source label. A candidate cannot replace a reported entry even inside a caller-reported revision. Promotion requires an explicit caller-reported replacement with a new ID.

Optional status changes are explicit revisions. Active and paused tasks may be revised. `completed` and `cancelled` are terminal; restarting requires a new task. A paused task can be explicitly revised to active. Lifecycle is separate from source, claim mode and verification. No lifecycle value grants execution authority. Limits: 128 accumulated requirement IDs, 4 KiB per requirement, 128 KiB per task payload; narrow task scope rather than silently deleting history.

### Checkpoints

`record_task_checkpoint` requires namespace, task ID, request ID, checkpoint ID, `expected_version`, `expected_checkpoint_sequence` (zero before the first checkpoint), steps, blockers, next_steps, artifacts and source. Optional environment and repository_revision are reported identifiers. Sequence and task version are checked in the same write transaction. Checkpoints append; they do not overwrite one another or revive terminal tasks.

Each step contains `step_id`, text, existing Claim `mode` (`Draft`, `Acted`, `Observed`, `Said`, `Inferred`), existing feedback `verification` (`not_performed`, `passed`, `failed`, `inconclusive`), and `evidence_refs`. Plans remain Draft, attempts can remain Acted, observations use Observed. A reported verification other than not_performed requires Observed and nonempty existing same-scope event references. This is structural evidence linkage, not independent verification. Artifacts contain a URI and optional reported digest. The service does not open arbitrary paths or assert that artifacts still exist or match.

## Resume context and budget

Request: `{"namespace":"project/example","task_id":"release-review","max_bytes":16384}`. The accepted compact-JSON UTF-8 budget is 1024–262144 bytes, excluding the transport envelope. `serialized_bytes` is measured, not a token estimate.

- All current reported requirements are loaded by exact task ID into `mandatory_requirements`, independently of text recall. `mandatory_requirement_count` retains the original count even if the whole block cannot fit. Candidates are inspected through get/list rather than silently becoming mandatory.
- Task and latest checkpoint are loaded in one read transaction (`mandatory_snapshot_consistent=true`). Checkpoint carries its own `task_version` and sequence. A checkpoint from an earlier task version is labelled stale and `can_resume=false`.
- Optional memory uses the existing scoped lexical context service; optional active experience recall preserves exact candidate versions and source metadata. These reads do not share the task snapshot. `snapshot_consistent=false` and diagnostics state this, even when a later task re-read sees the same head. `task_version_after_selection` reports the actual re-read version. A changed task or checkpoint makes the response incomplete and non-resumable. Optional candidates are suggestions, not instructions or automatically adopted experience.
- Optional blocks are dropped before required data. If the checkpoint cannot fit, or the entire mandatory block cannot fit, `complete=false`, `can_resume=false`, and explicit diagnostics require a larger budget or narrower task. Hard constraints are never selectively omitted while claiming completeness. Extremely small metadata budgets produce an explicit error.
- Paused/completed/cancelled tasks and all-candidate tasks have `can_resume=false`. `can_resume=true` only means this task context is eligible for continuation under the host's existing permissions, never that any specific action is authorized.
- `artifact_verification=not_performed` and `verification_basis=caller_reported_not_independently_verified` are always explicit. Resume must recheck artifact existence/digests and environment-sensitive results before relying on prior completion.
- A fresh context ID is only a build identifier. It does not prove a global consistent snapshot, host delivery, model understanding or successful application.

## Persistence, migration and recovery

Explicit schema migration 8 adds task_heads, task_revisions, task_requirements, task_checkpoints and task_checkpoint_sources. Requirements and checkpoint source links are relational; bounded payloads preserve typed content. Lifecycle doctor verifies the canonical schema, preserved row counts and foreign keys. A schema-7 database is rejected by ordinary opening until explicitly migrated; migration uses the existing backup and restore-rehearsal workflow. Full database backup includes task data and durable receipts.

The scoped memory export format remains limited to its existing record types and explicitly excludes tasks/checkpoints. It is not a complete task backup. Legacy namespace adoption currently rejects databases containing any task history because its source-closure contract has not been extended; task histories are not silently copied, rewritten or ignored. Direct database modification by the same OS user remains outside the adapter's trust boundary.

## Verification

`cargo test --test task_ledger --test mcp_session_policy --test sqlite_lifecycle --test schema7_migration` exercises retention, immutable history, explicit replacement, candidate separation, stale CAS, receipt replay/conflict, workspace isolation, inactive/all-candidate refusal, context overflow, checkpoint sequence and stale version, plan/result separation, source existence, context snapshot honesty, explicit v7 migration, concurrent CAS, receipt-write failure rollback, cross-scope evidence rejection, complete backup/restore and CLI argument boundaries. Legacy adoption refusal and live bridge session-revocation coverage are in their respective suites. These are deterministic local tests; they are not evidence that a real host has injected or understood a context, and they do not count as 30 independent user-experience scenarios. Host transport and dashboard tests are separate.
