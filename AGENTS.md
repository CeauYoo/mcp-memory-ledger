# Project Agent Instructions

## Platform Handling

- Detect the current system and use the matching platform workflow by default.
- On macOS, prefer:
  - `docs/development-macos.md`
  - `scripts/agent-llm-mm.sh`
- On Windows, prefer:
  - `docs/development-windows.md`
  - `scripts/agent-llm-mm.ps1`
- Do not mix platform-specific commands or examples into the main entry docs unless the task explicitly requires cross-platform restructuring.

## Synthetic Test Fixture Paths

- Canonicalize synthetic temporary roots before using production backup, registry or confinement helpers. macOS temporary paths may use the `/var` alias.
- Use `support::sqlite_url::sqlite_file_url` for fixture SQLite URLs instead of hand-built POSIX strings; preserve Windows path handling.
- Fix fixtures rather than weakening production symlink, permission or confinement checks. Cross-platform native CI remains required for native acceptance.

## Documentation Update Rule

- After every completed task, update the corresponding documentation in the same task if the change affects behavior, capability boundaries, integration flow, configuration, startup flow, verification commands, examples, or collaboration rules.
- Do not leave doc updates for a later cleanup pass when the required target docs are already clear.

## Documentation Mapping

- If behavior, tool contract, or capability boundary changes:
  - update `README.md`
  - update `docs/project-status.md`
  - update `docs/roadmap.md` if the roadmap or next-step framing changes

- If platform-specific setup, startup, or local integration changes:
  - update the active platform doc:
    - macOS: `docs/development-macos.md`
    - Windows: `docs/development-windows.md`
  - update `docs/local-mcp-integration-2026-03-26.md` if Codex / MCP registration flow changes

- If verification commands, smoke tests, or expected test baselines change:
  - update `docs/testing-guide-2026-03-24.md`
  - update the active platform doc if the command flow differs by platform

- If config shape, config loading, env vars, or example startup config changes:
  - update `examples/agent-llm-mm.example.toml`
  - update `examples/codex-mcp-config.toml`
  - update the active platform doc if user setup steps changed

- If contribution workflow or collaboration rules change:
  - update `CONTRIBUTING.md`
  - update the project overview docs if the rule is part of the repository’s standing expectations:
    - `docs/project-overview.zh-CN.md`
    - `docs/project-overview.en.md`
    - `docs/project-overview.ja.md`

## Scope Discipline

- Keep this repository described as a technical demo / MVP unless the user explicitly asks to reposition it.
- Keep “implemented / partial / unimplemented” language explicit and conservative.
