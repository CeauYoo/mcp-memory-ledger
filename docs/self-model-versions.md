# Versioned global identity and commitments

Schema 7 adds one append-only sequence for the global identity/commitment pair.
This is a bounded, experimental single-user local contract. Namespace attribution
is not authentication or tenant authorization; existing legacy global reflection
write authority is unchanged. `run_reflection` remains the only production durable
global write path. No provider proposal, feedback candidate or daemon gains rollback
or global mutation authority.

## Versions and effective time

Version `0` captures the initialized projections, with a known initialization time,
or captures an old database's current projections during explicit migration. A
migration baseline has `effective_at: null`: the historical time when those values
became valid is unknown. Existing audit patches are not replayed into invented
history. Both baselines record when capture occurred.

Every accepted reflection explicitly writing identity and/or commitments appends
one version, including accepted no-op writes. Claim-only correction and ordinary
record-only reflection append none. Versions contain internal complete snapshots,
written-component flags and source-version pointers. A written component points to
its new version; an inherited component retains its earlier source. Identity
order and duplicates are significant. Commitment snapshots use actual persisted
projection order rather than an assumed caller-side normalization.

New versions have server `recorded_at` and equal `effective_at`; they become
observable only after the transaction commits. This is a recording-time boundary,
not a promise about historical validity, exact commit-wall-clock time, backdating
or future scheduling. Integer version order is authoritative.

`identity_claims` and `commitments` remain current projections. Global writes and
version reads verify their equality with the ledger head, failing closed on drift.
The low-level `IdentityStore::save_identity` remains explicitly unversioned for
compatibility and test setup. It is not a production mutation route; using it after
initialization can cause drift. Serve and retrieval-index repair never rebaseline.

## Governed writes and retries

Existing target-bearing `run_reflection` accepts optional
`expected_self_model_version`. It rejects a stale value and rejects a guard on a
request with no global write. Legacy calls may omit it. A new optional `request_id`
provides durable replay; reuse with altered typed payload conflicts. Receipt lookup
precedes Claim-state and version checks. Omitted new input/result fields preserve
old serialized receipt payloads; old result receipts replay without a version key.
Successful new global results include only the version number in
`self_model_version`, never the internal snapshot.

The receipt namespace is explicit `origin_namespace` when supplied, otherwise the
legacy `self` receipt namespace. The key is scoped by operation and that namespace;
it does not grant any permission. A retry should preserve its complete typed input.

## Explicit component-selective compensation

Example shape, using actual existing IDs and the current version from an opted-in
version read:

```json
{
  "reflection": {"summary": "Restore the reviewed identity wording."},
  "supersede_claim_id": "actual-current-claim-id",
  "origin_namespace": "project/demo",
  "replacement_evidence_event_ids": ["event:actual-new-review-evidence"],
  "request_id": "restore-reviewed-identity-1",
  "expected_self_model_version": 4,
  "self_model_rollback": {
    "target_version": 2,
    "components": ["identity"],
    "confirm": true
  }
}
```

Rollback requires all of the following in the same writer transaction:

- Explicit caller opt-in `confirm: true`, durable request key, expected current version and a
  nonempty duplicate-free selection of `identity` and/or `commitments`.
- Existing nonsuperseded Claim anchor, explicit matching origin namespace and
  newly supplied nonempty valid same-known-scope evidence. Evidence is not
  implicitly borrowed from the historical version. The Event need not be newly
  created or unused; its content remains a caller assertion, not an independently
  verified observation. `confirm: true` is not proof of human approval or
  authentication. No legacy cross-world evidence
  exception applies.
- No ordinary identity/commitment patch in the same request.
- A target preceding the current version, excluding baselines, which explicitly
  wrote every selected component. An inherited component is not a rollback target.
- Both the historical source and current component source have verified exact
  same-scope provenance and existing nonempty same-scope durable evidence.
  Unknown, mixed, foreign-source and unverifiable components are rejected.

Only selected components are restored. Current unselected components are retained,
as is the baseline `forbid:write_identity_core_directly` commitment restriction.
The compensation appends a new version; its selected components point to the new
rollback reflection and its supplied evidence. `rollback_target_version` records
the earlier source. No history is removed, counters reset or external actions undone.

The Claim effect remains the ordinary `run_reflection` effect: without a
replacement the anchor becomes disputed; with a valid replacement it is superseded
and the replacement is evidence-linked. Rollback is not a targetless or no-Claim
mutation exception. A successful keyed retry returns its original result even
when the first call changed that Claim's state.

## Scoped versions and diffs

`get_self_model_versions` requires explicit namespace,
`allow_global_version_metadata: true`, bounded `limit` (1–100, default20), and
optional exclusive `before_version`. This is a record-count bound, not a byte or
compute cap; legacy components can be large and full internal snapshots add storage
cost. The opt-in acknowledges that the current
global counter and version gaps reveal that other global writes occurred. This
single-user metadata contract is not tenant isolation.

The read uses one consistent database snapshot for head verification and page
construction. Only written components with verified same-scope source reflection
and valid durable evidence appear. Baselines, unknown/mixed sources, inherited
aggregate components and foreign payloads are excluded. The result does not expose
internal aggregate snapshots through this tool or existing history/export/recall/
context tools.

Diffs preserve ordered duplicate identity semantics. Before/removed values are
shown only when the previous component source is also permitted in the requested
scope; otherwise only change/count metadata and a redaction explanation are
returned. No hash of low-entropy removed content is exposed. Source references,
timestamps and compensation metadata remain inspectable for permitted writes.

## Atomicity, schema and recovery

A global write runs in the existing `BEGIN IMMEDIATE` transaction: receipt replay,
Claim/evidence validation, head/projection and expected-version checks, rollback
resolution, value validation and baseline preservation, Claim/global mutations,
Reflection and normalized provenance, version insertion, optional trigger ledger,
Claim CAS, receipt and one commit. Failure at any step rolls back every mutation.
Transaction ports without version support fail closed rather than silently skipping
version writes.

The ledger has contiguous version/predecessor checks, reflection and source/target
foreign keys, component/kind constraints and append-only update/delete guards.
Canonical structural inspection includes those guards. These prevent accidental
rewrites; they are not tamper-proof protection against an administrator modifying
the database/schema directly.

Explicit `init` or `migrate` alone seeds version0. Migration uses the existing
backup anchor, restore rehearsal, writer reservation, row-preservation, foreign-key
and canonical-structure gates. Restore into a new path, then inspect and verify
versions before switching clients. Full database backups preserve this ledger;
scoped export remains interchange and does not include aggregate version snapshots.
Retain-all remains unchanged; no destructive retention or compaction is introduced.

## Binary rollback is a separate recovery action

A schema6 binary cannot open a schema7 database. Self-model compensation does not
downgrade schema or reverse external effects. To return to an older binary, restore
the pre-migration backup into a new path, verify read-only with its matching old
binary and manually switch configuration. Later writes are not magically present
in that older backup. Never perform a destructive in-place downgrade or claim a
lossless reversal.
