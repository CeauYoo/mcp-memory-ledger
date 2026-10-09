# MCP Memory Ledger Project Overview

## Summary

MCP Memory Ledger is a Rust-based local MCP `stdio` memory demo that validates a minimal loop for long-term memory, self-snapshot construction, and reflection. The current version uses SQLite for persistence and is best described as a technical demo, integration prototype, or research-oriented MVP rather than a complete product.

Compatibility note: the current Rust crate, binary, scripts, configuration examples, and some historical docs still use the technical identifier `agent_llm_mm` / `agent-llm-mm`. Use MCP Memory Ledger as the public project name.

## Current Scope

Current capabilities and all 31 tools are maintained in [current status](project-status.md) and the [tool index](tool-reference.md). Schema6, feedback correction, literal recall/context and versioned experience candidates are implemented. Actual user-client/fresh-machine evidence, real-model efficacy and human release approval remain open.

## Best Fit

- Local AI client integration experiments
- Self-agent memory demos and technical validation
- A minimal Rust + MCP + SQLite reference implementation

## Documentation Discipline

- After finishing each task, update the corresponding documentation whenever that task changes behavior, capability boundaries, integration flow, configuration, verification commands, or collaboration rules.
- Do not defer documentation updates until the end of a larger batch of work; code and docs should be closed out together whenever possible.
- The [formalization improvement plan](formalization-improvement-plan-2026-08-25.md) is a gap and acceptance map, not a second execution queue. The authorized development branch is `CeauYoo/dev_work_dots`, with a draft PR to upstream `dev-work`; merging or releasing requires separate explicit approval.

## Verification Status

[Current implementation evidence (2026-10-09)](project-status.md). Earlier stages remain in the [dated history](project-status-history-2026-10-09.md).

## Acknowledgement

This repository was developed, discussed, and documented with active support from OpenAI Codex as a collaborative development tool. Thanks to OpenAI for the tooling and research ecosystem that made this workflow possible.
