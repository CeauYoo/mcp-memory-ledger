# Ledger temporal metadata

This is an additive local-demo/MVP data contract, not a temporal truth model.

## Recording time and observation time

- `recorded_at` is the ledger creation time assigned by the application's `Clock`.
  An ingested Event and its derived Claims share one recording timestamp. A
  replacement Claim gets the recording time of its Reflection transaction.
- `observed_at` is optional caller provenance on `ingest_interaction`. It is not
  an effective date, validity interval, or permission to backdate a ledger write.
  It is copied to that Event and its derived Claims without substituting for
  `recorded_at`.
- Omitted observation time remains unknown. Replacement Claims do not infer an
  observation timestamp from their predecessor, supporting evidence, or clock.
- Historical Claims whose recording time was never stored remain unknown. Neither
  migration time, evidence time, IDs, nor row order establishes their creation time.
  Low-level `StoredClaim::new` therefore still constructs unknown metadata;
  application write paths attach known recording times explicitly.

`observed_at` is a top-level optional string. Accepted values use strict RFC3339
syntax, a four-digit year, an explicit `Z` or numeric offset, and at most nine
fractional-second digits. The maximum is 35 ASCII bytes. Excess fractional
precision is rejected rather than silently truncated. The exact accepted string,
including its offset and fraction, is retained. Past and future observation times
are permitted; the service does not independently verify the caller's clock.

Example input fragment:

```json
{
  "observed_at": "2026-10-09T15:45:00.123456789+05:45",
  "request_id": "observation-42",
  "event": {
    "owner": "World",
    "namespace": "project/example",
    "kind": "Observation",
    "summary": "The configuration was observed."
  },
  "claim_drafts": [],
  "episode_reference": null
}
```

The service assigns `recorded_at`; callers cannot set it through this input.

## Reads and ordering

Source records returned through `get_memory`, `search_memory`, and recall expose
nullable Claim `recorded_at` and Event/Claim `observed_at`. Unknown values are JSON
`null`, not a synthetic epoch or the time at which the read was performed.
Existing Event and Reflection `recorded_at` output remains a canonical UTC
RFC3339 timestamp with preserved nanosecond precision. This output normalization
does not rewrite historical timestamp text in SQLite.

Mixed source searches use known Claim recording times alongside Event, Episode,
and Reflection recording times. Unknown Claim times sort after known times;
recording time is descending, then record type and descending canonical ID break
ties. The SQLite union paths use this same ordering before each per-type limit,
so a low-ID or earlier-inserted row cannot hide the actual highest-ranked record.
Standalone source-query tie order remains backward compatible. Lexical recall
uses recording recency after term count only when the Claim time is known, retaining an explicit
unknown-time explanation otherwise. Observation time does not affect recency.
Claim recorded-time range filters remain unsupported; the existing Event range
filters retain their inclusive bounds.

The SQLite schema retains raw timestamp text and stores derived UTC seconds and
nanoseconds. Its indexed sort key is an opaque fixed-width encoding of biased
UTC seconds and the nanosecond component, not a formatted calendar timestamp.
This preserves ordering across offsets, including extreme RFC3339 offsets, and
matches Chrono's representation of leap seconds (where the nanosecond component
can be at least one billion). Lowercase RFC3339 separators are supported. The
encoding is an internal derived index, not a public timestamp format.

Existing valid Event/Reflection text is not rewritten. Unparseable historical
timestamps are not replaced with migration time; absent derived keys cannot
satisfy a bounded recorded-time filter. Such malformed records may still fail
the typed source read rather than inventing a timestamp.

## Compatibility and replay

- Existing StoredClaim and StoredEvent constructors still work. Older serialized
  values that omit new metadata deserialize with unknown observation/Claim times.
- An ingest request with omitted or null `observed_at` uses the exact original
  four-element payload hash. Existing durable receipts therefore remain replayable.
  Explicit observation metadata is included in a version-tagged hash; reusing a
  request ID with different observation metadata is rejected.
- Replaying a successful request returns the original result without updating the
  Event, Claim, or recorded timestamp. The write receipt result shape is unchanged.
- `claim-version:v1` hashes exactly the original `claim_id`, Claim content, and
  status fields. Additive temporal metadata is excluded so a schema migration
  cannot invalidate persisted feedback candidates. Content/status changes still
  invalidate the target fingerprint.

## Verification

Run the focused contract suite:

```sh
cargo test --test temporal_metadata
cargo test --test sqlite_temporal_store
```

It covers application-clock recording versus caller observation, nanosecond and
offset preservation, bounded strict input, unknown source values, constructor and
JSON compatibility, exact v1 fingerprints, old receipt replay, immutable keyed
retries, changed observation rejection, and mixed known/unknown ordering. The
SQLite adapter suite additionally checks nanosecond/offset normalization,
including leap seconds and extreme offsets, recall
recency, immutable low-level metadata, query plans, and per-type union prelimit
ordering, including equal timestamps and unknown Claims. Migration, backfill,
backup/restore, and Reflection replacement paths also need their lifecycle and
Reflection suites; these focused tests alone are not release or deployment
qualification.
