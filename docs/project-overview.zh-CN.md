# MCP Memory Ledger 项目说明

## 简述

MCP Memory Ledger 是一个 Rust 编写的本机 MCP `stdio` memory demo，用于验证 AI 客户端里的长期记忆 / 自我快照 / 反思修订最小闭环。当前版本以 SQLite 为持久化基础，更适合作为技术 demo、集成验证仓库和研究型原型，而不是完整产品。

兼容说明：当前 Rust crate、二进制、脚本、配置样例和部分历史文档仍使用技术标识 `agent_llm_mm` / `agent-llm-mm`。对外项目名统一使用 MCP Memory Ledger。

## 当前范围

当前能力与 31 工具合同统一见[当前状态](project-status.md)和[工具索引](tool-reference.md)。已包含 schema6、反馈纠错、字面召回/上下文与版本化经验候选；仍是 technical MVP，真实客户端/fresh-machine、真实模型收益与人工发布门未关闭。

## 适合的使用方式

- 本机 AI 客户端接入实验
- self-agent memory 相关概念验证
- Rust + MCP + SQLite 的最小工程骨架参考

## 文档约束

- 每次处理完一个任务后，如果该任务影响了行为、能力边界、接入方式、配置、验证命令或协作规则，必须同步更新对应文档。
- 不应把文档更新留到最后统一处理；代码与文档应尽量在同一轮任务内一起收口。
- [正式化改进计划](formalization-improvement-plan-2026-08-25.md)只负责差距与验收映射，不是第二份执行队列。当前授权开发分支为 `CeauYoo/dev_work_dots`，通过上游 `dev-work` 草稿 PR 集成；任何 merge/release 均需单独明确批准。

## 当前验证状态

当前精确提交证据见[实现状态](project-status.md)，早期阶段见[带日期的历史记录](project-status-history-2026-10-09.md)。

## 致谢

本仓库在开发、讨论和文档整理过程中明确使用了 OpenAI Codex 作为协作式开发工具。感谢 OpenAI 提供相关工具与研究生态，使这种以讨论驱动、迭代收口的开发方式成为可能。
