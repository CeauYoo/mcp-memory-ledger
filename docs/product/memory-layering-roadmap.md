# Memory Layering Roadmap

## Purpose

This roadmap defines the future multi-layer memory direction for
`agent_llm_mm`. It is a product capability roadmap, not an implementation status
claim.

Local Alpha remains a local MCP memory service plus governed self-revision. It
is not a complete multi-layer cognitive architecture, not a production
self-governing system, and not a remote/team memory product.

Historical projection slice: the episode summary read model can project
objective, outcome, `lesson`, and linked evidence ids together. This is
projection-only local metadata over existing episode events. It does not add a
durable episode table, schema migration, procedural memory, slow variables,
semantic memory, lifecycle policy, or durable self-model write path.

## 2026-10-09 bounded runtime continuation

The [original-plan continuation](../memory-feedback-experience.md) adds schema-v5 richer immutable Episodes and caller-authored semantic/procedural candidates, source-relation tables, versioned reject/revise/rollback, explicit activation and bounded active-only recall. This implements bounded runtime subsets of Phases 1–3, not complete cognitive memory or every phase exit. The old snapshot projection remains legacy-snapshot-only; its absent procedural label is not an inventory of the separate v5 tools. No candidate activation changes identity, commitments, action permissions or executes a procedure.

Foundation decision: explicit migration/backup/rehearsal/restore and atomic receipts are implemented and tested. For this additive append-only slice, lifecycle retention is deliberately retain-all: no pruning/compaction/delete API is introduced, whole-database SQLite backup/restore remains the supported recovery unit, and all history/receipts stay together. Schema6 adds a bounded read-only scoped interchange export with complete source-reference closure (see [export contract](../scoped-export.md)); it is not a restore backup. Privacy-aware retention/deletion and automatic compaction remain conditional on explicit policy and source/reference/replay consistency contracts. This bounded choice permits the user's requested experience-candidate work without pretending the broader lifecycle roadmap is complete.

## Product Wording Boundary

Use conservative stage wording until the matching gates pass:

| Stage | Allowed Wording | Blocked Wording |
| --- | --- | --- |
| MVP | validated local MVP entering productization | formal product, production-ready, complete autonomy |
| Local Alpha | local MCP memory plus governed self-revision | complete multi-layer cognitive architecture |
| Beta | controlled beta after lifecycle, migration, and support gates | GA, production self-governance |
| Remote/team | remote or team capability only after auth, audit, isolation, and backup gates | multi-tenant production service without isolation tests |
| GA | GA only after security, migration, lifecycle, support, and recovery gates pass | GA by roadmap intent alone |

## Prerequisites

Broader multi-layer runtime implementation remains gated on stable, tested
foundation contracts. The bounded v5 slice uses the explicit retain-all and
whole-database recovery decision above; it does not waive the remaining gates:

- evidence semantics stable: evidence ids, evidence queries, projections, and
  redaction rules have stable meanings.
- reflection policy stable: `run_reflection` governance, rejection,
  suppression, cooldown, and diagnostics are stable enough to support richer
  memory writes.
- schema migration policy tested: local SQLite migrations, rollback
  expectations, and compatibility checks are tested against prior local
  databases.
- data lifecycle/backup gates in place: backup, restore, export, retention, and
  deletion rules exist before new memory layers increase data value and risk.
- product wording remains staged: docs keep MVP, Local Alpha, Beta,
  remote/team, and GA claims separate.

## Layer Definitions

| Layer | Future Role | Local Alpha Status | First Gate |
| --- | --- | --- | --- |
| Working memory | Short-lived task context, active goals, temporary constraints, and current evidence handles | Not implemented as a distinct layer | Define expiry, visibility, and no-durable-commit rules |
| Episodic memory | Structured records of tasks, outcomes, lessons, and linked evidence | Legacy read-only projection plus separate v5 immutable durable Episodes; v5 records are not included in the legacy snapshot inventory | Complete broader snapshot integration and lifecycle contracts |
| Semantic memory | Stable distilled concepts, project facts, domain rules, and cross-episode summaries | v5 caller-authored, evidence-linked, versioned candidates with explicit activation and current-active recall; no automatic extraction or truth proof | Prove evidence-backed extraction and contradiction handling |
| Procedural memory | Reusable workflows, policies, checklists, and operational playbooks | v5 inspectable, rejectable, versioned candidates and rollback; steps remain inert text | Any execution needs its own authority and safety contract |
| Slow variables | Long-horizon preferences, calibrated thresholds, trust levels, and policy weights | Not implemented | Define governance, review cadence, and bounded update paths |
| Self-model layering | Explicit model of agent capabilities, limits, commitments, and known failure modes | Current self-revision diagnostics are partial evidence only; durable self-model writes remain blocked outside `run_reflection` | Define read-only projection before any durable self-model writes |

## Phased Direction

### Phase 0: Contract Stabilization

Goal: keep Local Alpha honest while preparing the data contracts.

- Stabilize evidence semantics and projection names.
- Keep `run_reflection` as the durable identity and commitment write path.
- Keep automatic behavior local-first, opt-in where applicable, and governed.
- Finish schema migration, backup, restore, and retention gates before adding
  new durable memory tables.

Exit: current memory, evidence, reflection, and lifecycle contracts can be
tested without relying on demo-only assumptions.

### Phase 1: Richer Episodic Semantics

Goal: turn task history into structured episodes without introducing a full
semantic or self-model layer.

The first historical local-safe slice was the read-only lesson projection in
the existing episode summary read model. It allows objective, outcome,
`lesson`, and linked evidence ids to be inspected together without creating a
new durable write path. The separate v5 runtime now persists richer immutable
records; it does not silently redefine this projection or complete every phase gate.

Full-phase episode expectations:

- `goal`: what the task attempted to accomplish.
- `outcome`: what actually happened, including success, partial success,
  refusal, or failure.
- `lesson`: a bounded, reusable takeaway that does not exceed the evidence.
- `linked_evidence_ids`: evidence ids that support the episode.
- `snapshot_projection_test`: a deterministic test proving the episode appears
  in the expected snapshot projection without corrupting existing projections.

Acceptance expectations for a complete durable episode layer:

- An episode can be written and read without changing identity or commitments.
- Episode summaries preserve linked evidence ids rather than copying raw
  evidence bodies into every projection.
- The snapshot projection test proves compatibility with the existing local MCP
  memory flow.
- The feature remains local-only and does not imply semantic memory,
  procedural memory, slow variables, or self-model writes are implemented.

Historical projection-only slice exit: the projection is test-covered and
read-only; that slice added no durable table or migration. The separate v5 slice
adds explicit migration, durable richer Episodes and dedicated detail/list reads.

Full phase exit: richer episode records are test-covered, migration-covered, and
projected read-only before any later layer consumes them.

### Phase 2: Semantic Memory Candidate

Goal: introduce evidence-backed, stable distilled facts only after richer
episodes are reliable.

- Extract semantic candidates from multiple episodes.
- Require source episode and evidence links.
- Track contradictions and supersession instead of overwriting facts silently.
- Keep semantic memory read-only in projections until governance rules exist.

Exit: semantic candidates can be generated, inspected, rejected, and migrated
without replacing episodic records.

### Phase 3: Procedural Memory Candidate

Goal: make reusable workflows explicit without silently changing runtime
behavior.

- Model procedures as versioned, inspectable records.
- Link procedures to evidence and human-facing docs where applicable.
- Require explicit activation before a procedure affects runtime behavior.
- Track rollback and deprecation metadata.

Exit: procedures are searchable and auditable, but runtime use remains gated.

### Phase 4: Slow Variables and Policy Calibration

Goal: represent long-horizon preferences and policy weights with strict
governance.

- Define allowed slow-variable names, types, bounds, and review cadence.
- Require evidence links and policy approval for updates.
- Keep updates reversible and visible in audit projections.
- Prevent one task from causing broad unreviewed preference drift.

Exit: slow variables are durable only when bounded, reviewed, and recoverable.

### Phase 5: Self-Model Layering

Goal: expose a layered self-model only after evidence, reflection, episodes,
semantic memory, procedural memory, and slow variables have stable gates.

- Start with read-only self-model projections.
- Separate capabilities, limitations, commitments, and known failure modes.
- Preserve `run_reflection` as the durable commitment path unless a later
  architecture decision replaces it with migration and rollback support.
- Block claims of production self-governance until long-run, security,
  lifecycle, and recovery gates pass.

Exit: the self-model is inspectable, evidence-linked, and governed before any
broader autonomy claim is made.

## Non-Goals

- This roadmap does not implement multi-layer memory.
- This roadmap does not replace `run_reflection`.
- This roadmap does not add remote/team memory service behavior.
- This roadmap does not claim Beta, GA, production self-governance, or complete
  autonomous agent behavior.
- This roadmap does not allow Local Alpha wording to imply a complete
  multi-layer cognitive architecture.
- This roadmap does not treat the read-only `lesson` projection as procedural
  memory, semantic memory, slow variables, durable self-model writes, migration
  coverage, backup/retention policy, or complete lifecycle governance.

## Verification Hooks for Future Work

Future implementation specs should include at least:

- migration tests from existing local SQLite databases.
- snapshot projection tests for each new layer.
- evidence-link integrity tests.
- redaction and support-bundle tests.
- backup/restore compatibility checks.
- product wording checks that block overstated Local Alpha, Beta, remote/team,
  or GA claims.
