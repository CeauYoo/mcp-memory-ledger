# Security

MCP Memory Ledger is a local-first technical demo / MVP. There is no published security-support lifetime, response-time commitment or production certification. This document defines reporting and handling guidance, not a guarantee that every experimental surface is hardened.

## Report privately

Do not publish a suspected vulnerability, exploit details, credentials, private memories, databases or raw logs in a public issue or pull request.

First check the repository's **Security** tab for GitHub private vulnerability reporting and confirm that **Report a vulnerability** is available for the repository you intend to contact. This document does not establish that the feature is enabled. If it is unavailable, verify an appropriate private reporting channel with a repository maintainer before sharing sensitive details. A public request to identify a private channel should contain no vulnerability details or private data. No reporting email address or response deadline is assumed here.

A useful private report contains:

- The affected commit SHA or binary version, OS, and whether the path is standalone stdio, a workspace bridge, the local core or a dashboard.
- A minimal synthetic reproduction, expected and actual behavior, and the relevant trust boundary.
- The apparent impact and any containment already performed. State which evidence is observed and which is an inference.

Provide only the minimum necessary evidence through the verified channel. Replace personal paths and record contents with synthetic values. Do not send API keys, authorization headers, cookies, full configuration files, raw database files, user chat transcripts or unrestricted logs. Keep original evidence locally for later, explicitly agreed review.

## Boundaries to review

- Namespace filtering alone is not authentication or session authorization. Workspace requests must be constrained by a trusted registration and current server-side grants.
- Core workspace sessions must not gain global snapshot, decision, self-model or index-administration access through tool arguments. Core ingest must not indirectly invoke automatic model reflection.
- Local sockets, registry files and databases require appropriate local access controls. A root path or workspace ID is not a secret; this design does not claim to resist arbitrary malicious processes running as the same OS user.
- A loopback dashboard is not automatically authenticated. New core monitoring is disabled by default, and administrator access must be isolated from workspace capabilities. Existing standalone stdio and new-core monitor behavior must be reviewed separately.
- Provider payloads, private memory and diagnostic artifacts are sensitive. A redaction helper is not permission to collect or upload them.

The implementation and acceptance status of these boundaries is tracked in the [workspace matrix](docs/workspace-core-acceptance.md) and [current status](docs/project-status.md). Requirements are not evidence that a changed path has passed verification.

## If a secret or sensitive artifact was exposed

Stop further distribution and use a verified private channel. The credential owner should revoke or rotate an exposed credential through its issuer; deleting a message or Git commit is not sufficient. Record the affected artifact or commit identifiers without copying the secret. Removal of published artifacts or repository-history rewriting requires the relevant owner's approval and coordination; do not silently overwrite evidence or unrelated work.

Changes to supported versions, disclosure timing and remediation status must be based on actual review and recorded evidence. This repository does not promise a staffed response service or a fixed remediation timeline.
