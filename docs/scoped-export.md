# Scoped ledger interchange export

`export_memory` is an inspectable JSON interchange for one explicit namespace.
It is read-only and remains part of the local technical demo / MVP. It does not
implement import, replay, compaction, deletion, retention, or a restore protocol.
Use the verified whole-database backup/restore workflow for recovery.

## Request and bounds

```json
{
  "namespace": "project/example",
  "max_records": 1000,
  "max_relations": 4000,
  "max_bytes": 1048576
}
```

The namespace is required. Its canonical owner is derived; callers cannot supply
an owner, a global wildcard, a filesystem path, or an output destination. Default
limits are 1,000 records, 4,000 relations, and 1 MiB. Supported limits are 1–10,000
records, 1–100,000 relations, and 1 KiB–16 MiB. A record limit counts each event,
claim, reflection, persisted experience episode, experience head, and historical
experience version. Relationship limits include structural evidence and source
references, legacy episode membership, current-version links, and history links.

The result's `used_bytes` is the exact UTF-8 length of the entire compact JSON
result, including metadata, arrays, counts, escaping, and `used_bytes` itself.
A budget failure returns an error, never a partial graph or a truncation cursor.
Metadata is included in the budget, so an empty scope still needs space for its
interchange envelope. Preflights also conservatively count scoped source rows,
relations, and stored byte lengths before fetching their payloads. Source data
that will later be quarantined can therefore consume the bounded source budget;
a failure is not a claim that an exported graph of that size exists. All original
evidence edges of scoped Claims are preflighted, including unsafe endpoints;
foreign Event payloads are never fetched for that check.

## Included facts and closure

- Canonically owned Events and Claims in the exact namespace, including known
  recording timestamps and separately preserved caller observation timestamps.
  Unknown historical timestamps remain unknown. Recording timestamps are parsed and
  emitted as canonical UTC DateTime values; original RFC3339 lexical offsets are
  not a byte-for-byte preservation contract. Caller observation strings retain
  their original representation.
- Claims and all of their original evidence links only when every source Event
  is exported in the same scope. Any foreign, unknown-owner, missing, or otherwise
  quarantined source omits the whole Claim; it is never emitted with shortened
  provenance. A genuinely source-less Claim remains distinguishable and is retained
  without evidence links.
- Legacy episode membership only when the entire membership group belongs to
  the requested scope and every source Event is retained.
- Reflection summary, provenance, explicit origin, and affected-scope metadata
  only for known attribution whose origin and every effect fit the requested
  scope. Every evidence/claim endpoint must be exported, and normalized evidence
  must agree with the durable provenance. Unknown or mixed attribution is omitted.
- Persisted experience Episodes with complete, validated same-scope Event source
  closure, and experience heads plus all historical versions with complete Episode
  source and previous/rollback-version closure. A corrupt/unsafe historical source
  quarantines the entire candidate history instead of exporting a misleading fragment.

Events with unsafe feedback evidence or canonical `claim:`/`event:` targets are
quarantined with their associated version labels/hashes. Claim evidence and Event
feedback targets/evidence are closed together until no unsafe dependency remains,
including dependent Claims, Events, episode membership groups, Reflections, and
experience that can no longer close their source graph. Mixed-scope edges are
not returned as unresolved references. The omission policy is explicit and does
not reveal the identifiers or counts of unrelated records. History preserves the
status at the historical version; a formerly active version is not present-day
execution authority.

## Exclusions and boundaries

Global identity/commitment state and patch payloads, operation logs, durable retry
receipts, operation IDs/hashes, trigger keys, feedback candidates, derived search
indexes, configuration, and credentials are not read into the export. There is
no new global read or mutation authority. Scoped caller-authored content is
preserved; this is not a secret-redaction service. Users should review the JSON
before sharing it outside their local environment.

All preflights and fetches run in one SQLite read transaction. Export performs no
schema/index migration, repair, receipt append, or operation-log write. Ordering
is deterministic by identifiers and numeric version; unchanged durable data
produces identical compact JSON. There is deliberately no wall-clock export
stamp that would make otherwise identical exports differ.

`domain::ledger_export::validate_export` checks an already decoded format-v1
interchange's envelope, exact byte count, owner/scope agreement, duplicate records,
reference closure within the decoded graph, complete version histories, and
record/relation caps. Only the exporter can compare this graph with the original
ledger and establish that no original source edge was omitted; standalone
validation cannot detect an edge deleted from an otherwise self-consistent file.
Validation does not import it, authenticate its contents, or establish truth or
permission.

## Verification

```bash
cargo test --test scoped_ledger_export
```

The suite exercises scope/mixed-relation privacy, reflection provenance, exact
full-response budgets, stable repeated exports, oversized source-row failure,
complete experience history and source closure, structural validation, concurrent
WAL snapshot consistency, and absence of database mutation on read-only export.

Schema7 `self_model_versions` internal aggregate snapshots and component-source
history are not part of this scoped export. Use the separately opted-in
[get_self_model_versions contract](self-model-versions.md) for permitted written
components and source-safe diffs, and a full database backup for recoverable ledger
history. A new global version does not broaden export visibility.
