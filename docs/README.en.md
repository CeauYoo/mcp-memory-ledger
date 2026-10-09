# MCP Memory Ledger

A local-first Rust + SQLite MCP `stdio` memory service. It records evidence-linked memories, retrieves scoped records, corrects Claims with retained history, and manages inert versioned experience candidates.

The [Chinese repository README](../README.md) is the single maintained front door. Use the [documentation map](document-map.md), [quickstart](quickstart.md), [client configuration](local-mcp-integration-2026-03-26.md), [runnable workflow](runnable-memory-workflow.md), and [tool index](tool-reference.md).

Current runtime: 31 MCP tools, schema 6, rebuildable FTS5 with literal short-CJK fallback, bounded complete-JSON context, explicit migrations and backup/recovery. Runtime binary name: `agent_llm_mm`.

This remains a technical MVP. Lexical retrieval is not semantic search; bytes are not model tokens; namespaces, caller budgets and feedback labels are not authentication. Experience activation changes recall eligibility, never action authority. Export is interchange, not backup. Legacy global self-governance is experimental.

[Current status and evidence](project-status.md) distinguish exact-commit CI from actual user-client/fresh-machine installation, real-model efficacy and human release approval. [The active plan](plans/2026-07-10-product-replan.md) remains the only execution queue; dated reports are historical evidence.
