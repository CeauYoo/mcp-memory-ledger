# MCP Memory Ledger

面向本地 AI Agent 的证据门控记忆与自我修订层。

语言：[English](../README.md) | 简体中文 | [日本語](README.ja.md)

MCP Memory Ledger 是一个本地优先的 Rust MCP `stdio` 记忆服务。它把交互、证据、claim、自我快照和自我修订审计记录到 SQLite 中，让 AI 客户端可以在可审计的边界内使用长期记忆，而不是只依赖一次性的 prompt 上下文。

当前项目适合作为本地 Agent 记忆、MCP 集成、SQLite 持久化和受治理自我修订的技术 MVP。它不是生产级自治 Agent 平台，远程团队模式、多租户、安装包发布、daemon 写能力和生产安全边界仍按路线图和门禁分阶段推进。

[正式化改进与主线同步计划](formalization-improvement-plan-2026-08-25.md)集中整理了当前产品、工程、安全、发布和 GitHub 主线差距。它是验收映射，不代表 Local Alpha 或 production-ready 已完成；当前 active plan 仍是唯一任务入口。

## 2026-10-09 可用性候选

新增 `recall_memory` / `build_task_context`（总计12工具）、事务内更正、可选request_id持久化重放、结构化Event反馈与schema v4显式迁移。支持北京/咖啡/记忆等短中文literal召回；context硬预算包含完整结果JSON metadata但不含JSON-RPC包装。详见[本地工作流与兼容边界](local-memory-usability.md)。当前仍为technical MVP，非正式Alpha发布；legacy direct/auto global self-model治理仍是experimental，namespace不是多用户认证边界。

## 核心能力

- **本地 MCP 记忆服务**：通过 `stdio` 暴露 `ingest_interaction`、`search_memory`、`get_memory`、`get_reflection_history`、`get_self_model_history`、`get_evidence_relation`、`supersede_memory`、`build_self_snapshot`、`decide_with_snapshot` 和 `run_reflection` 共 10 个工具。
- **按 scope 检索 Event / Claim / Episode / Reflection**：`search_memory` 要求显式 namespace。省略类型时默认返回有界、recent-first 的 Event；`record_type = Claim / Episode / Reflection` 分别返回 scoped Claim、按同 scope Event 投影的 Episode，以及只通过同 scope Claim 端点归属的 Reflection。record-only Reflection 不可见。additive `record_types` 可请求这四类 tagged record 的 scoped union。确定性读取路径不依赖 provider；mixed-scope Claim revision edge 整边隐藏。
- **按稳定 ID 查找**：`get_memory(namespace, id, record_type?)` 返回一条完整 Event、Claim、Episode 或 scoped Reflection。省略 `record_type` 保持 Event 语义。Episode / Reflection 把 `id` 当作 opaque exact persisted reference。missing、跨 namespace 与 record-only Reflection 返回 `record: null`，不扩大查询。
- **Claim 修订历史**：`get_reflection_history(namespace, claim_reference, limit?)` 从一条 exact scoped Claim 出发，按 newest-first 返回双向 revision chain。missing / cross-scope / mixed-scope 路径保持空或隐藏。
- **identity/commitment 修订审计**：`get_self_model_history(namespace, history_type, limit?)` 读取 claim-attributed reflection 上的 identity 或 commitment 补丁。这是审计轨迹，不是 versioned identity/commitment ledger。
- **证据关系运行时**：`get_evidence_relation` 把调用方 trigger window 与同 scope Event 做 intersect-only 收窄，并报告 selected / available-not-selected。不引入 ranking 或 widening。
- **受审计的 scoped supersede**：`supersede_memory` 通过既有 `run_reflection` 事务替换一条同 scope Claim。旧 Claim 保留为 `Superseded`；missing / cross-scope 输入 fail closed。这不是第二条 durable write path。
- **SQLite 持久化**：保存 event、claim、evidence、reflection audit、trigger ledger 和 operation log。
- **证据门控自我修订**：claim、identity、commitment 的修订必须经过明确证据和治理规则；`run_reflection` 仍是 identity / commitment / reflection 的唯一持久化写路径。
- **有界 scoped snapshot**：M0.2 显式路径接受 namespace、可选 evidence manifest 和 inclusive 时间窗；SQLite 做 owner/namespace 收窄与 recent-first 排序。
- **有界本地运维**：包含 operation-log 查询、backup / restore、脱敏诊断、显式 `init` / `migrate`，以及默认不写库的 `doctor --read-only`。`serve` 拒绝缺失或过期数据库，而不会隐式改库。
- **运行时与源码门禁**：启用无认证 dashboard 时只允许 localhost/loopback，tracing 固定写 stderr；Rust `1.95.0` 下的 Linux/macOS CI 执行格式、全特性 Clippy、full tests 与状态同步。
- **Provider 接入**：内置 `mock`、`openai-compatible` 和 OpenRouter 配置路径；provider 密钥只应放在本机私有配置或环境变量中。

## 适合场景

- 给本地 AI 客户端接入 MCP memory
- 研究 Agent 如何基于证据更新长期记忆
- 验证 self snapshot、reflection、commitment gate 的最小闭环
- 作为 Rust + SQLite + MCP `stdio` 项目的工程参考
- 生成本地诊断材料，定位 provider、配置、dashboard 或 operation log 问题

## 快速开始

macOS：

```zsh
./scripts/agent-llm-mm.sh bootstrap-local
./scripts/agent-llm-mm.sh init
./scripts/agent-llm-mm.sh doctor --read-only
./scripts/agent-llm-mm.sh serve
```

Windows：

```powershell
pwsh -File .\scripts\agent-llm-mm.ps1 bootstrap-local
pwsh -File .\scripts\agent-llm-mm.ps1 init
pwsh -File .\scripts\agent-llm-mm.ps1 doctor --read-only
pwsh -File .\scripts\agent-llm-mm.ps1 serve
```

`bootstrap-local` 会从 dev 示例生成本机配置模板，不覆盖已有文件，不生成 secret，也不会启动服务。

平台和接入文档：

- [macOS 开发与接入指南](development-macos.md)
- [Windows 开发与接入指南](development-windows.md)
- [本机 MCP 接入说明](local-mcp-integration-2026-03-26.md)

## 演示

运行可重复的 self-revision demo：

```zsh
./scripts/run-self-revision-demo.sh
```

该 demo 会启动本地 deterministic `openai-compatible` stub provider，通过真实 MCP `stdio` 服务跑 canonical scenario，并把报告写入 `target/reports/self-revision-demo/...`。

## 本地诊断

生成脱敏支持包：

```zsh
./scripts/generate-support-bundle.sh target/support-bundles/manual-check
```

支持包只包含脱敏 JSON 摘要，不复制完整 SQLite 数据库、raw TOML、provider payload 或原始 `.log` 文件。需要导出日志片段或聚焦某次 MCP tool call 时，请显式传入 `--log-file` 或 `--correlation-id`。

汇总本地 Alpha 门禁状态：

```zsh
./scripts/local-alpha-evidence-summary.sh \
  --evidence-root . \
  --output-json target/reports/local-alpha/evidence-summary.json \
  --output-md target/reports/local-alpha/evidence-summary.md
```

该命令只读取已有本地证据并输出汇总，不生成缺失证据，也不自动认证本地 Alpha。

## 当前边界

已实现：

- MCP `stdio` 主链路
- SQLite 持久化和 owner / namespace 约束
- `run_reflection` 审计式 claim 替换
- 最小 identity / commitment 修订
- 基于 trigger ledger 的自动自我修订 MVP
- 只读 dashboard、doctor、本地支持包和本地门禁汇总脚本
- 显式 SQLite schema version / migration ledger、事务迁移、写入前 backup 与 restore rehearsal
- dashboard loopback 强制边界、stderr-only tracing、固定 Rust `1.95.0` 的 Linux/macOS source CI
- M1.0.1–M1.0.3 scope / data-integrity 前置门
- M1.1.1–M1.1.6 scoped Event / Claim / Episode / Reflection search、evidence-relation runtime 与跨类型 union
- M1.2.1–M1.2.5 Event / Claim / Episode / Reflection lookup 与 Claim reflection history
- M1.2.6 scoped identity/commitment revision audit（`get_self_model_history`）；不是 versioned ledger
- M1.2.7 scoped Claim audited supersede（`supersede_memory`）；复用 `run_reflection`，默认不 hard delete

部分实现：

- M0.2 只对显式 scoped snapshot 收口；省略 `namespace` 的 legacy 调用仍保持 unscoped 兼容，完整面向用户的 recall contract 仍未完成
- `decide_with_snapshot` 仍围绕动作字符串协议，不是完整决策引擎；允许结果标记为 `experimental_non_authoritative`
- episode 目前主要是轻量 scope 投影，不是完整自传式记忆模型
- 运行时读取已覆盖四类 search/lookup、union、Claim history、self-model audit 与 evidence-relation；`supersede_memory` 是 `run_reflection` 的 scoped Claim 纠错门面，不是第二条 durable write path
- leftover `Owner::Unknown` 行对 schema 仍合法，但对 namespace-derived scoped read 不可见
- provider live evidence 只证明配置和连通性，不证明模型质量、SLA 或生产可用性
- 本地 Alpha 门禁仍依赖真实 fresh-machine、Windows parity 和人工 release decision 等外部证据
- `rmcp` 仍固定为 `0.5.0`；对官方 `2.2.0` 的隔离 spike 因一个 handler error contract 回归而判定本里程碑不直接升级

未实现：

- 完整 memory layering
- versioned identity/commitment ledger、record-only Reflection history，以及 Event / Episode / Reflection 纠错
- M1.3.0 current-schema structural readback 与 M1.3.1 真实客户端退出门
- richer evidence ranking / weighting
- 生产级 remote / team / multi-tenant 能力
- daemon 写能力和后台自治运行
- 安装包、service manager、auto-updater 和发布认证流程

完整实现状态见 [当前实现状态](project-status.md)、[路线图](roadmap.md) 和 [当前 active plan](plans/2026-07-10-product-replan.md)。下一领取顺序是 `M1.3.0 Current-Schema Structural Readback Gate`。

## 文档

### 理解项目

1. [项目起点与主线原则](origin-and-principles.md)
2. [项目定位](positioning.md)
3. [当前实现状态](project-status.md)
4. [Now / Next / Later 路线图](roadmap.md)
5. [正式化改进与主线同步计划](formalization-improvement-plan-2026-08-25.md)
6. [当前 active plan](plans/2026-07-10-product-replan.md)

当前 active plan 是唯一任务入口；历史计划只用于追溯，不再定义当前工作。

### 开发与验证

- [macOS 开发说明](development-macos.md)
- [Windows 开发说明](development-windows.md)
- [本机 MCP 接入](local-mcp-integration-2026-03-26.md)
- [测试指南](testing-guide-2026-03-24.md)

### 参考与历史

- [文档总览](document-map.md)
- [历史归档](archive.md)

## 验证

测试分为 `fast`、`core`、`full` 三级；发布证据、打包和 provider certification
工具由非默认 `release-tools` feature 承载。

常用本地检查：

```zsh
./scripts/test-tier.sh fast
./scripts/test-tier.sh core
./scripts/status-sync-check.sh
./scripts/agent-llm-mm.sh doctor
git diff --check
```

涉及 provider、dashboard、发布证据或支持包的改动，请按 [测试指南](testing-guide-2026-03-24.md) 跑对应分层验证。

## 命名兼容

对外项目名是 MCP Memory Ledger。当前 Rust crate、binary、脚本、配置样例和部分历史文档仍使用 `agent_llm_mm` / `agent-llm-mm` 作为技术标识；这属于兼容保留，不代表项目名仍是旧名称。

## 致谢

本仓库在开发、复核和文档整理过程中使用了 OpenAI Codex 作为协作式开发工具。感谢 OpenAI 提供相关工具与研究生态，使这种以讨论驱动、迭代收口的开发方式成为可能。

## 许可证

本项目采用 Apache License 2.0，详见 [LICENSE](../LICENSE) 与 [NOTICE](../NOTICE)。

Copyright 2026 yooyui
