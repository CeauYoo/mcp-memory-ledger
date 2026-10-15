# Support

MCP Memory Ledger is a technical demo / MVP. Repository discussions and issues are best-effort collaboration, not a support contract or response-time guarantee. For suspected vulnerabilities, use the private-channel guidance in [SECURITY.md](SECURITY.md), not a public bug report.

## Start with the smallest reproduction

Read the [quick start](docs/quickstart.md), [MCP integration guide](docs/local-mcp-integration-2026-03-26.md) and your [macOS](docs/development-macos.md) or [Windows](docs/development-windows.md) guide. Record the actual binary version or commit SHA; a source checkout's HEAD does not identify an already running binary.

Use the [bug form](.github/ISSUE_TEMPLATE/bug_report.yml) for an observed problem or the [feature form](.github/ISSUE_TEMPLATE/feature_request.yml) for a proposal. Include:

- OS and architecture, client name/version, binary version or SHA, and known schema version.
- The entry path: standalone stdio, workspace bridge/local core, dashboard or a CLI operation.
- Minimal steps with synthetic input, expected/actual results, and an error class or a short manually sanitized excerpt.
- Which verification level was exercised and any untested boundary.

Do not read, collect or paste credentials to diagnose an issue. Do not upload a raw SQLite database, configuration file, private-memory contents, user chat transcript or unrestricted log. Even a generated support bundle needs local review before sharing; it is not an automatic upload channel. Avoid screenshots containing real history. Keep original evidence local and share a synthetic reproduction whenever possible.

## Label the evidence accurately

| Evidence | What it can establish | What it does not establish |
| --- | --- | --- |
| Offline mock / protocol fixture | Deterministic contract behavior without a real model | Provider connectivity, model quality or natural-language tool selection |
| Direct MCP protocol call | Behavior of the named binary and client protocol path | Whether a model chooses the right tool in conversation |
| Real natural-language client session | The observed tool choices and result in that sample | Reliable behavior for all prompts or a controlled model-quality result |
| Local/CI package run without Rust in PATH | Candidate archive and isolated runtime behavior | A genuinely fresh machine or a supported release |
| Fresh-machine run | The recorded machine, platform and first-run procedure | Other platforms, long-term reliability or release approval |
| Authorized live-model run | The recorded provider/model request and its measured result | Unmeasured quality, cost, safety or a software-enforced dollar cap when prices are unknown |

Real provider tests require separate data and spending authorization. Do not run paid requests just to fill a bug report. Leave unavailable token/cost information unknown; never supply credentials in the issue.

## Operational limits

Use a separate synthetic database for reproductions. Start with read-only diagnosis; do not use initialization, migration, deletion, a provider change or a service restart to hide an unknown failure. Back up before an approved migration, restore to a new path, and verify before a user-approved switch. See [database operations](docs/database-operations.md).

The new Unix core/bridge is under integration and acceptance; it is not yet a verified replacement for the existing deployment. Its initial target is macOS/Linux, while Windows retains existing standalone stdio support. Workspace binding is fixed for a connection; moving a client conversation to another directory does not prove that its MCP binding changed. Report the mode and binding behavior without posting a private registry. See the [acceptance matrix](docs/workspace-core-acceptance.md).

Feature requests should describe a concrete memory workflow and an observable acceptance condition. Remote/team hosting, autonomous execution, destructive retention and live-model spending do not become approved because an issue was opened.
