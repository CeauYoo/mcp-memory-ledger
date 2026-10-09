# Bounded task context: linked Episodes and observable diagnostics

`build_task_context` remains an offline, literal, scope-required read. It returns
whole primary Claim/Event records and their original scoped provenance first.
It then tries complete linked rich Episode snapshots and finally an optional
diagnostic block. It does not change sources,
activate experience candidates, infer semantic contradictions, or execute a task.

## Exact shared budget and compact labels

`max_bytes` limits the compact serialized result's UTF-8 bytes, including every
metadata field, optional caller-budget receipt, Episode, diagnostic and the
`serialized_bytes` field itself. It excludes the MCP/JSON-RPC transport envelope.
There is no token estimate. Oversized records are omitted whole; later smaller
records can still fit. Primary records are never removed to fit Episodes or
diagnostics. Omission metadata is reserved before packing. Insufficient metadata
space is an explicit invalid-parameters error.

The versioned context-only labels preserve the existing raw-recall contracts:

| Context label | Meaning |
| --- | --- |
| `json_utf8_v1` | Compact JSON UTF-8 byte budget, including all result metadata |
| `balanced_v2` | Balanced Claim/Event quotas; Claim receives the odd slot; redistribute unused quota; interleave selected types; within each type order by matched-term count, known recording time descending, then ID ascending |
| `active_unverified_v1` | Active Claim, not independently verified |
| `historical_event_v1` | Historical Event, not a current conclusion |
| `recorded_desc_v1` | Recording time descending after matched-term count; record type identifies the timestamp source |
| `recorded_unknown_v1` | Claim recording time is unknown; no creation time or recency is assumed |

Raw `recall_memory` keeps its existing verbose labels and does not run the
additional status or Episode inspections. Null `index_warning`, absent optional
sections and empty Episode lists are omitted from the context JSON. Full primary
record fields, values and provenance are unchanged. Compact labels recover some
space for explicit omissions; fixed evaluator budgets and gold identities are
not changed to accommodate metadata.

## Linked rich Episode snapshots (D09)

Only Event IDs and Claim evidence Event references from **actually packed primary
records** can seed `rich_episodes`. References are collected in deterministic primary/provenance order, stopping
at 65 distinct references (64 plus one sentinel), then that bounded window is
sorted; at most 64 are inspected. Each source performs an ordered reverse-edge lookup of
at most nine same-scope Episode IDs (eight plus a sentinel). Episode namespace and
current source Event owner/namespace are checked before each lookup limit. The
bounded union is sorted and deduplicated; at most eight candidates are hydrated.
A sentinel or union overflow is reported, rather than implying exhaustive recall.

Each rich snapshot is returned complete: objective, actions, observations,
outcome, lesson, limitations and source references. Before exposure, the reader
checks payload size, shape/content limits, Episode identity, namespace and
recording time against the row; requires exact equality between payload source
IDs and authoritative source edges; and rechecks every original source Event's
current scope. At most 65 edges are read per candidate; over-limit, missing,
foreign, Unknown-owner, malformed or mismatched dependencies omit the **entire**
snapshot. No source list is silently rewritten to make an unsafe snapshot fit.
The inspection uses a read transaction and does not repair damaged data.

Existing legacy Episode references remain intact on primary-record provenance.
This context extension does not hydrate those unbounded legacy membership graphs
or change the preexisting legacy browse API.

`omissions.episodes` reports:

- `byte_budget`: complete retrieved rich snapshots that did not fit
- `candidate_limit`: a lower bound of one when rich candidate inspection has more
  results; this is **not** an exact rich total
- `unavailable`: bounded candidates missing or rejected by validation
- `source_limit`: a lower bound of one when a distinct source sentinel exceeds
  the 64-source inspection cap; not an exact count of all uninspected sources
- `rich_lookup_supported`: whether this store implements the optional rich reader

There is no semantic Episode search: an unlinked Episode or one reachable only
from a primary record omitted by the budget is not inspected. Goal/outcome/lesson
text is stored caller-authored experience, not independently verified truth.

## Observable status diagnostics (D10)

`diagnostics.contract = observable_v1`. If the entire diagnostic section cannot
fit after primary and Episode records, it is omitted and
`omissions.diagnostics = byte_budget`. Absence must not be read as zero conflicts,
zero history, known recording times or verified facts.

The store inspects separate deterministic Claim-ID-ordered windows for Disputed
and Superseded Claims. Owner, namespace and persisted status are applied **before**
each window limit. At most 256 rows plus one completeness sentinel are read per
status. The same ASCII-insensitive literal substring OR contract is applied to
subject/predicate/object. The active-only derived FTS index is not used for these
historical statuses.

Each status reports `sampled_claim_count`, `sampled_match_count`,
`scope_scan_complete`, at most eight original canonical `claim:` references, and
`references_truncated`. The match count is a **sampled lower bound** unless
`scope_scan_complete` is true. Zero with an incomplete scan does not establish
absence. Reference truncation concerns matching rows in the inspected window;
`scope_scan_complete` separately describes uninspected rows. These are stored
status observations, not a discovered contradiction or an inferred expiry.

The remaining diagnostics describe **only actually returned primary records**:

- active Claims remain independently unverified, regardless of mode or age
- unknown Claim recording times and empty evidence-reference lists are factual
  counts, with at most eight sorted references each
- no returned records and query terms lacking a returned literal match describe
  this retrieval/packing result, not missing knowledge in the world or ledger
- exact equal subject/predicate strings with differing object strings form a
  possible differing-value group; predicates may be multi-valued, so this is
  explicitly **not** an inferred contradiction. The full group count is over at
  most 100 returned records, with at most four groups/eight references each shown

`expiry = not_evaluated_no_expiry_contract` is deliberate: recorded/observed time
is not a validity interval. No age threshold, expiry date or stale-by-age claim is
invented. Superseded status is available separately as persisted history.

## Cost, consistency and limits

The row windows, hydrated snapshots, source fan-out, group comparisons and output
are bounded. SQLite may still need to filter source rows, and arbitrarily long
legacy Claim text costs time to inspect; these contracts are not a hard CPU-time
or SQLite VM-instruction quota. Reads use existing indexes and never add a schema
or repair-on-read path. Primary retrieval/hydration, rich Episode inspection and
status inspection are separate read snapshots; no globally atomic cross-section
snapshot is promised during concurrent writes. Every hydration independently
rechecks its scope. Optional store methods return unsupported rather than
fabricating empty scans.

## Verification

`tests/context_diagnostics.rs` covers scope-before-limit, foreign decoys, honest
sample bounds, reference limits, unsupported missing matches, multi-valued
warnings only among returned records, old/unknown times without invented expiry,
read-only status inspection, whole rich Episode packing, exact JSON
boundaries, fan-out/deduplication, and corrupt/moved/foreign dependency rejection.
Existing indexed-recall and fixed 1,800-byte evaluator fixtures are retained.
