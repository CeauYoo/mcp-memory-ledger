# Caller-owned operation budget (C09)

Status: implemented as an optional, cooperative MCP contract on `recall_memory`,
`build_task_context`, and `run_reflection`. The repository remains a technical demo /
MVP. This is not a durable quota, an autonomous agent loop, or a new source of write
authority.

## Scope and compatibility

- Omit `caller_budget`, or supply `null`, to retain the existing tool behavior and
  response shape. Existing per-call limits, evidence checks and automatic-reflection
  cooldown/suppression rules are unchanged.
- `recall_memory` and `build_task_context` each count as one retrieval attempt. Context
  assembly's internal recall and bounded diagnostics do not count as separate attempts.
- `run_reflection` counts as one reflection attempt. An explicit retry additionally
  counts against `retries`.
- No budget is persisted, inferred from other requests, attached to a session, or shared
  between clients. The caller owns, carries forward and, if necessary, synchronizes the
  counts. Resetting or omitting counts can bypass this cooperative guard; it is not a
  security or billing boundary.
- These are the only three opt-in MCP routes. `search_memory`, feedback/experience
  candidate tools, direct application calls and automatic-reflection paths do not gain
  global quota enforcement. No new MCP tool, model call, background work, database schema,
  credential, remote authority or fabricated client session is introduced.

## Request

Example `build_task_context` arguments:

```json
{
  "namespace": "project/example",
  "query": "rollback evidence",
  "limit": 10,
  "max_bytes": 4000,
  "caller_budget": {
    "limits": {"retrievals": 3, "reflections": 1, "retries": 1},
    "used": {"retrievals": 0, "reflections": 0, "retries": 0},
    "is_retry": false,
    "evidence": "unknown"
  }
}
```

Both `limits` and `used` are required; all three counts are required in each object.
Counts are integers from 0 through 1000, checked at runtime as well as described in the
tool schema. Zero disables the relevant operation. `used` may exceed a newly lowered
limit, in which case its remaining allowance is zero. Negative, fractional, string,
oversized and missing counts fail closed. Unknown fields inside the budget or count
objects are rejected to catch misspelled controls.

`is_retry` defaults to `false`. `evidence` defaults to `unknown` and accepts:

- `unknown`: no caller assertion about new or sufficient evidence; ordinary validation
  still applies, including on the first call.
- `new`: the caller reports new evidence. This does not authenticate it or bypass scope,
  reference, transaction, or evidence-sufficiency validation.
- `unchanged`: stop a requested reflection with `no_new_evidence`.
- `insufficient`: stop a requested reflection with `insufficient_evidence`.

Both `unchanged` and `insufficient` still permit retrieval when its count permits it:
a fresh task may reread the same facts, and missing evidence may require another search.
A previously rejected reflection may be explicitly retried when the caller supplies an
appropriate evidence signal and remaining counts. Existing automatic-reflection retry
and suppression behavior remains independent and unchanged.

## Deterministic admission and stop reasons

The server reads and validates the raw optional budget envelope before decoding unrelated
tool arguments. Once the envelope is valid, it admits or stops exactly one requested
tool attempt. The first applicable stop reason wins, in this order:

1. `no_new_evidence`: reflection plus explicit `unchanged` evidence.
2. `insufficient_evidence`: reflection plus explicit `insufficient` evidence.
3. `retry_budget_exhausted`: `is_retry` and used retries at or above the retry limit.
4. `retrieval_budget_exhausted` or `reflection_budget_exhausted`: the requested
   operation's used count is at or above its limit.

A stopped call returns JSON-RPC error `-32602` with the structured decision under
`error.data.caller_budget`. It does not decode unrelated tool arguments, invoke the
operation, write core records or append an operation-log entry. `next_used` is unchanged.
For example:

```json
{
  "code": -32602,
  "message": "caller operation stopped: reflection_budget_exhausted",
  "data": {
    "caller_budget": {
      "contract": "caller_operation_budget_v1",
      "operation": "reflection",
      "allowed": false,
      "stop_reason": "reflection_budget_exhausted",
      "next_used": {"retrievals": 2, "reflections": 1, "retries": 0},
      "remaining": {"retrievals": 1, "reflections": 0, "retries": 1}
    }
  }
}
```

On admission, exactly one operation count and, if applicable, one retry count increment.
The receipt uses the same shape with `allowed: true` and `stop_reason: null`:

- A successful result adds `caller_budget` to `result.structuredContent`.
- Any subsequent parameter decoding, semantic conversion or operation failure retains
  its ordinary error and adds the admitted receipt to `error.data.caller_budget`.
  Counts measure admitted attempts, not successful reads/writes. Ordinary failure
  diagnostics may still be written after admission.
- An invalid budget envelope itself cannot be admitted and returns a validation error
  without a receipt. No budget counter is silently invented or reset.
- With no receipt available after a transport disconnect, the caller must account for
  the uncertain attempt conservatively. A receipt is not an exactly-once execution
  guarantee, a replay token, or proof that an operation committed.

Copy `next_used` into `used` for the next explicitly chosen tool call. `remaining` is
calculated after this attempt, with each value floored at zero. A successful last allowed
attempt can return zero remaining; a subsequent attempt then stops. There is no implicit
next call or recommendation to continue simply because allowance remains.

## Context byte cap and business idempotency

For `build_task_context`, the optional receipt participates in primary result packing
before whole records are selected. Its bytes are included in `serialized_bytes` and
the hard `max_bytes` cap on the compact UTF-8 JSON result. It is never appended after
measurement or dropped to make room. If the complete mandatory metadata and receipt
cannot fit, the tool fails with an admitted-attempt receipt in the error. Error envelopes
are not successful context results and are not governed by the context-result cap.

Caller counters are transport policy metadata, not durable business payload. In
particular, conversion to `ReflectionInput` omits them, so changing counts cannot alter
its serialized business request or write-receipt hash. Opting in grants no new evidence,
scope, identity-update, commitment-update, activation or write authority.

## Verification

```bash
cargo test --test caller_operation_budget
cargo test --test mcp_stdio caller_budget::
```

The domain/DTO tests cover all stop reasons, deterministic precedence, zero/lowered/full
limits, overflow boundaries, strict shapes, first-call defaults, retry accounting and
business-payload isolation. Real local MCP subprocess tests verify tool discovery,
omission/null compatibility, unchanged-evidence retrieval, early stops with malformed
unrelated fields and no ledger/diagnostic writes, invalid envelopes, admitted failures,
successful evidence-backed reflection, and exact UTF-8 caps with receipts. These tests
are local integration evidence, not external-client acceptance or product certification.
