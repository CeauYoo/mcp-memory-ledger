# Threat Model: Local and Remote Surfaces

## Scope

This threat model covers the current local-first productization track and the
future remote/team boundary. It is a planning and review artifact, not evidence
that remote/team mode, multi-tenancy, or production self-governance is
implemented.

Current trusted product core:

- local MCP `stdio` integration
- local SQLite persistence
- governed `run_reflection` write path for identity, commitments, claims, and
  reflections
- local read-only dashboard
- local support bundle generation
- durable operation-log metadata
- observe-only daemon diagnostics

Future remote/team work must pass the mitigations in this document before any
remote write/admin capability is enabled.

## Assets

| Asset | Why it matters | Primary risks |
| --- | --- | --- |
| SQLite data | Stores events, claims, evidence, decisions, trigger ledger, reflection audit, and operation log data | Data loss, tampering, cross-namespace reads, accidental demo/formal data mixing |
| Provider credentials | Allow calls to configured model providers | Secret leakage, unauthorized model spend, impersonation of a configured operator |
| Reflection audit | Records governed self-revision decisions and supporting evidence | Audit tampering, missing rejection history, fabricated support for identity or commitment changes |
| Operation log | Explains runtime actions, statuses, namespaces, correlation IDs, and failures | Missing accountability, secret leakage through summaries, hidden write/admin abuse |
| Support bundles | Package diagnostic metadata for maintainers | Accidental export of secrets, private paths, SQLite files, provider payloads, or other tenants' data |

## Attacker Capabilities

### Local Attacker

A local attacker may have one or more of these capabilities:

- read files under the user's workspace, config directory, or generated target
  directories
- modify local TOML config or environment variables
- start the binary with a different `database_url` or provider configuration
- inspect generated support bundles
- access the local dashboard from the same machine
- replace demo or test SQLite files with crafted data
- run local commands as the same OS user

This model does not treat full OS compromise as solvable by the application, but
the product should still avoid making local compromise worse through plaintext
secret leakage, unsafe default binds, or accidental export of full databases.

### Remote Attacker

A remote attacker becomes relevant only if an HTTP surface is exposed outside
localhost or through a reverse proxy. They may attempt to:

- access unauthenticated dashboard routes
- replay or forge admin/API requests
- escalate from read-only routes to write/admin routes
- infer provider credentials or private prompts from diagnostics
- enumerate namespaces, tenants, or operation-log rows
- trigger expensive provider calls
- start, stop, or reconfigure daemon behavior
- upload, download, or poison support bundles
- exploit missing rate limits or overly broad CORS policy

Remote write/admin work must assume hostile network traffic, not just trusted
team members on a private LAN.

## Trust Boundaries

| Boundary | Current posture | Required future posture |
| --- | --- | --- |
| MCP client to local `stdio` server | Local process boundary; current product core | Keep local; do not turn MCP `stdio` into network transport without a separate design |
| Local config to runtime | TOML and env inputs influence provider, database, dashboard, and daemon settings | Validate shape, redact secrets in diagnostics, reject unsafe remote binds without auth |
| Runtime to SQLite | Local persistence for semantic memory and operational metadata | Enforce namespace/tenant constraints before team mode; preserve backup/restore boundaries |
| Runtime to provider | Outbound provider calls using configured credentials | Never expose provider credentials in doctor, dashboard, operation log, reflection audit, or support bundles |
| Dashboard HTTP to browser | Local read-only inspection in Local Alpha | Remote dashboard must start read-only and require auth, authorization, audit, redaction, and rate limits |
| Support bundle generator to filesystem | Local-only metadata packaging | Keep full SQLite, TOML secrets, provider payloads, and cross-tenant data excluded by default |
| Future admin/API HTTP to runtime | Not implemented | Block writes until auth, authorization, audit, rollback, and tests are complete |

## Write-Admin Abuse Paths

The following abuse paths must remain blocked until a write/admin gate exists:

- remote write to identity, commitments, claims, or reflections outside governed
  `run_reflection`
- remote write that calls `run_reflection` without authorization and durable
  audit context
- remote provider credential creation, update, readback, or deletion without
  secret handling and rollback design
- remote daemon start / stop / trigger mutation that changes local state or
  provider spend
- remote database backup, restore, migration, or export that can overwrite
  formal data or leak SQLite data
- remote support bundle generation that includes full databases, TOML secrets,
  provider payloads, or another namespace's data
- cross-namespace or cross-tenant operation-log reads through dashboard filters
- audit deletion, audit rewrite, or operation-log suppression by the same actor
  performing a write
- policy changes that allow background autonomy or production self-governance
  without a separate gate

## Required Mitigations Before Remote/Team Mode

Remote read-only dashboard requires:

- explicit remote enablement; disabled by default
- non-local bind rejected unless authentication is configured
- authorization even for read-only views, with at least a viewer role
- bounded route inventory and no write routes in the first stage
- redaction tests for provider credentials, `Authorization`, `Bearer`, tokens,
  local paths, and provider URL secrets
- rate limits or equivalent abuse controls for HTTP routes
- CORS policy that does not allow arbitrary browser origins by default
- durable audit entries for remote access attempts by default, including
  rejected attempts when safely observable; any exception must be documented
  with the reason the attempt cannot be safely observed without leaking
  sensitive data

Remote write/admin requires all read-only mitigations plus:

- role-based authorization per operation
- human-readable admin action audit with actor, route, namespace/tenant,
  correlation ID, request shape, decision, result, and failure reason
- rollback or compensating-action plan per write class
- tests for auth failure, authorization failure, audit persistence, rollback or
  failure note behavior, redaction, and unchanged error semantics
- explicit allowlist of write/admin routes
- forbidden route tests for capabilities not yet approved
- `run_reflection` retained for semantic memory writes unless superseded by a
  reviewed architecture decision

Team mode and multi-tenancy require:

- namespace-to-tenant mapping that is explicit and validated
- database isolation or tenant-aware shared schema decision
- tests proving operation-log, dashboard, reflection audit, support bundle, and
  backup/export reads cannot cross tenant boundaries
- provider credentials scoped per tenant or deployment boundary
- migration tests for tenant metadata
- backup and restore procedures that preserve tenant boundaries
- incident runbook for suspected cross-tenant exposure

## Local Alpha Security Position

Local Alpha may claim local-first diagnostics and local data-safety boundaries
only when the release gate has fresh evidence. It must not claim:

- remote/team mode is implemented
- multi-tenancy is implemented
- remote write admin is available
- production self-governance is available
- support bundles are a remote upload channel
- dashboard exposure through a public reverse proxy is safe

The safe default remains local-only operation, separate SQLite databases for
formal/test/demo data, support bundles without secrets or full databases, and
semantic writes governed by `run_reflection`.


## Local owner-channel attestation boundary

`task-owner-attest` is a distinct explicit local CLI, absent MCP, bridge routes, source enums and read-only dashboard/admin capabilities. It preserves operator-supplied exact changes/evidence but does not authenticate a human, verify evidence authorship, or isolate hostile processes under the same OS account. Same-user shell invocation is possible. Imported requirements remain knowledge only; they never grant execution authority. Server-owned event/provenance and shared transactional guards prevent ordinary task clients from withdrawing/relabeling imports or using lifecycle deactivation to bypass live imports. Configured-host signed imports are future work, not implemented by this local MVP slice.
