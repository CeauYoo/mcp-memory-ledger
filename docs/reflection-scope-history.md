# Reflection scope and history boundaries

Reflection origin and affected scopes are persisted independently. They describe audit provenance; they never grant authority to mutate Claims, identity, commitments, or other namespaces.

- `scope.status` is `verified`, `legacy_unambiguous`, or `unknown`.
- `scope.origin_scopes` identifies the verified source scope. Mixed or unverifiable sources remain unknown.
- `scope.affected_scopes` identifies mutated Claim scopes and includes `self` for existing global identity/commitment changes. Record-only reflections have no effects.
- Existing Claim-anchored reads retain their endpoint-scope checks. An affected scope is never an alternate route to a Reflection.
- Targetless full records require an explicit query scope, one verified/unambiguous matching origin, nonempty durable evidence wholly inside that scope, and all effects inside that scope. A project-origin global identity update stays hidden from both project and self targetless history.

## New evidence-backed record-only input

`run_reflection` accepts omitted `supersede_claim_id` only with explicit `origin_namespace`. This path requires at least one existing same-scope evidence Event and rejects replacement Claims and all identity/commitment updates. Existing target-bearing input remains supported.

```json
{
  "reflection": {"summary": "The test failure came from stale fixture data."},
  "origin_namespace": "project/demo",
  "replacement_evidence_event_ids": ["event:fixture-failure"]
}
```

Source validation, scope and evidence relation insertion, Reflection recording, optional existing mutations, and successful audit/write receipt all run in the same transaction. Failure in any relation write rolls back the entire operation.

## Migration and durable provenance

Explicit migration backfills historical scope only when existing source endpoints and evidence establish exactly one known owner/namespace. Targetless records with missing, malformed, mixed-owner, mixed-project, or absent evidence stay unknown and hidden. No scope is inferred merely from a global identity/commitment effect.

`reflection_evidence` is an authoritative FK-backed ledger relation with preserved evidence order and a reverse Event lookup index. It is not disposable or rebuilt as part of text-index recovery. Once `evidence_normalized` is true, reads use the relation even if legacy JSON differs or the relation is empty. Unnormalizable historical JSON is retained for compatibility rather than inventing links.

The previous internal global-governance behavior is unchanged. Schema6 metadata itself introduced no global rollback or mutation permission. Schema7 adds a separate bounded [version/compensation contract](self-model-versions.md), preserving these existing scoped history boundaries. Its read tool exposes permitted written components only, never inherited aggregate snapshots.
