# Changelog

This project remains a local-first technical demo / MVP. This file records changes; it is not a release approval or a support guarantee. Unreleased work has not replaced an installed binary merely because it appears here.

## Unreleased

### Documentation

- Add support, security-reporting and proposed collaboration guidance, plus bug, feature and pull-request templates.
- Record the local core / workspace implementation queue and separate protocol, natural-language, platform and deployment evidence in the [workspace acceptance matrix](docs/workspace-core-acceptance.md).

### Candidate packaging

- Include workspace setup and both explicit/global bridge examples in exact-source portable archives.
- Run the bundled no-Rust local-core verifier for Unix candidate archives; retain synthetic evidence and fail the candidate on verification errors. Windows keeps standalone stdio verification and reports the Unix core as unsupported.
- Document fixed-cwd client limits and the absence of automatic legacy namespace adoption; do not relabel or migrate existing user history implicitly.

### In progress — integration and acceptance pending

- A Unix local core with lightweight stdio bridges, stable workspace registration, per-call authorization refresh and explicit personal-memory grants.
- Restricted workspace tools, disabled automatic reflection in core ingest, shared observation and a separately gated administrator monitor.
- Dashboard listener lifecycle fixes and recovery coverage. A port-takeover patch alone does not complete the shared-core architecture.

These entries describe work under integration, not a claim that all tests or acceptance gates have passed. The new core initially targets macOS/Linux; Windows retains the existing standalone stdio path. Installed deployment, fresh-machine acceptance, live-model evaluation and release approval remain separate. See the [active plan](docs/plans/2026-07-10-product-replan.md) for scope and the [current status](docs/project-status.md) for evidence.
