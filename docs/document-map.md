# 文档导航

[统一 README](../README.md)是项目首页。本页按读者目标组织阅读路径，避免把阶段日志当成当前教程。

## 我想先用起来

1. [快速开始](quickstart.md)：固定工具链、源码构建、显式 mock 配置、初始化与只读诊断。
2. [可运行工作流](runnable-memory-workflow.md)：可复制 MCP 参数和离线端到端脚本。
3. [MCP 客户端接入](local-mcp-integration-2026-03-26.md)：绝对 binary/config/database 路径、stdio 和排错。
4. [数据库操作](database-operations.md)：init/migrate/doctor、备份/恢复与索引维护。

平台细节：[macOS](development-macos.md) / [Windows](development-windows.md)。语言摘要：[中文兼容入口](README.zh-CN.md) / [English](README.en.md) / [日本語](README.ja.md)。

## 我想知道每个工具的合同

从[31 工具索引](tool-reference.md)开始，再按需读专页：

| 主题 | 权威合同 |
| --- | --- |
| 反馈纠错、FTS/短中文、丰富 Episode/经验版本 | [Feedback & experience](memory-feedback-experience.md) |
| 完整 JSON byte cap、来源 Episode、诊断的缺失/省略 | [Task context](context-diagnostics.md) |
| caller-owned 检索/反思/重试预算 | [Operation budget](caller-operation-budget.md) |
| schema6 时间、未知历史、旧 receipt/fingerprint | [Temporal metadata](temporal-metadata.md) |
| Reflection 来源与影响范围、安全 targetless history | [Scope/history](reflection-scope-history.md) |
| 只读有界交换与依赖完整性 | [Scoped export](scoped-export.md) |
| 模型配置和兼容边界 | [Provider contract](provider-contract.md) |

字段细节由运行时 `tools/list`、[DTO](../src/interfaces/mcp/dto.rs)和相应 application 类型决定。中文索引负责解释，专页负责完整合同；避免复制几套不同参数表。

## 我想判断项目是否可发布

- [当前实现状态](project-status.md)：已实现 / 部分 / 未实现及精确提交证据。
- [评估方法](evaluation-methodology.md)：固定 fixture、A/B/C/消融、容量观测和不能推出的结论。
- [测试指南](testing-guide-2026-03-24.md)：fast/core/full、离线工作流、CI/平台范围。
- [最终 70 项核对](plans/2026-10-09-original-plan-final-reconciliation.md)：原方案有限实现收口与外部证据门。
- [Reality gates](product/follow-up-reality-gates.md)、[Local Alpha gate](product/release-gate-local-alpha.md)、[Release readiness](release-readiness.md)、[release engineering](product/release-engineering.md)。

通过代码测试不等于真实用户客户端、fresh-machine、真实模型收益或人工 release approval。[正式化映射](formalization-improvement-plan-2026-08-25.md)列差距，不创建第二份任务队列。

## 我想贡献或理解设计

- [项目起点与原则](origin-and-principles.md) → [路线图](roadmap.md) → [唯一 active plan](plans/2026-07-10-product-replan.md)。
- [贡献指南](../CONTRIBUTING.md)、[工具链政策](toolchain-policy.md)、[私有模块边界](implementation-module-boundaries.md)。
- [项目定位](positioning.md)、[FAQ](faq.md)、项目说明 [中文](project-overview.zh-CN.md) / [English](project-overview.en.md) / [日本語](project-overview.ja.md)。
- 数据与权限：[data lifecycle](product/data-lifecycle.md)、[data safety](product/data-safety-local-alpha.md)、[threat model](security/threat-model-local-and-remote.md)。
- 辅助能力：[support bundle](product/support-bundle-local-alpha.md)、[correlation ID](product/correlation-id-contract.md)、[daemon observe-only](product/daemon-observe-only-gate.md)、[self-revision demo](self-revision-demo-guide-2026-04-24.md)。
- 条件性方向：[memory layering](product/memory-layering-roadmap.md)、[remote/team boundary](product/remote-team-mode-boundary.md)、[rmcp compatibility spike](spikes/rmcp-compatibility-2026-07-14.md)。这些不是已实现承诺。

## 历史与证据权威

- [带日期的实现历史](project-status-history-2026-10-09.md)：保留旧 schema/工具数/测试数/分支状态，不能作为当前状态。
- [早期 v4 可用性基线](local-memory-usability.md)、[原 70 项 baseline](plans/2026-10-09-original-plan-traceability.md)、[v5 增量](plans/2026-10-09-continuation-results.md)、[schema6 阶段](plans/2026-10-09-schema6-results.md)。
- [计划索引](plans/README.md)与[历史归档](archive.md)保存出处；历史 checkbox 没有当前执行权。

冲突时以实际代码、测试和数据库 readback 为准，并修正文档。README 是入口，状态页是当前事实，路线图是顺序，active plan 是执行队列，reality gates 是完成约束。
