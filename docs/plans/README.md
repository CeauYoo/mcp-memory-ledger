# 项目计划索引

本目录只保留一个当前执行入口。历史计划可以用于追溯，但不得覆盖当前代码事实、项目状态或 active plan。

## 当前计划

- [2026-07-10-product-replan.md](2026-07-10-product-replan.md)
  - 状态：`active / M0 complete / M1 active / M1.0.1, M1.0.2, M1.0.3, M1.1.1, M1.1.2, M1.1.3, M1.1.4, M1.1.5, M1.1.6, M1.2.1, M1.2.2, M1.2.3, M1.2.4, M1.2.5, M1.2.6 and M1.2.7 complete`
  - 目标：把现有 technical MVP 收束为可信、可检索、可审计、可恢复的本地 MCP Memory Ledger
  - 当前里程碑：`M1 Trustworthy Recall`
  - 当前执行状态：schema7 有界全局 self-model 版本/diff/显式补偿已完成本地验证，见[阶段结果](2026-10-10-schema7-results.md)；schema6、in-crate 模块拆分、上下文/诊断与 caller budget 的有限实现已完成；M1.3.0 与 M2.0.1 已完成。剩余真实用户客户端/fresh-machine/正式发布与真实模型效果门见[最终核对](2026-10-09-original-plan-final-reconciliation.md)，不是重复实现队列

## 权威层级

| 文档 | 唯一职责 |
| --- | --- |
| [`docs/origin-and-principles.md`](../origin-and-principles.md) | 灵感起点、不可丢失的架构原则和新能力准入条件 |
| [`docs/positioning.md`](../positioning.md) | 公共名称、目标用户和对外口径 |
| [`docs/project-status.md`](../project-status.md) | 当前分支真实实现状态 |
| [`docs/roadmap.md`](../roadmap.md) | Now / Next / Later 产品路线 |
| [当前 active plan](2026-07-10-product-replan.md) | 任务顺序、依赖、证据门和停止条件 |
| [`docs/product/follow-up-reality-gates.md`](../product/follow-up-reality-gates.md) | 已实现、部分实现、未验证和阻断证据 |

出现冲突时，先以当前代码和测试证据为准，再按上表顺序修正文档。测试文件存在不等于测试已经在当前分支通过；只有当前运行记录可以作为 fresh evidence。

## 已被取代的计划

2026-03 至 2026-06 的 specs、execution plans、P1/P2/P3、productization、阶段进度和发布记录已移至本地分支 `codex/archive/pre-mainline-reset-2026-07-10`。完整路径和读取方式见 [`docs/archive.md`](../archive.md)。

归档文件中的已完成项、未完成框和阶段名称只描述当时语境，不再拥有执行权。后续任务必须从当前 active plan 领取，并把结果写回当前状态与 reality gate。

## 更新规则

1. 开始实现前，只把当前 active milestone 的一个最小切片标为进行中。
2. 代码、测试与对应文档在同一任务更新。
3. 先记录验证命令和结果，再改变完成状态。
4. 遇到证据不符时停止扩张，不自动进入下一个里程碑。
5. 新的大方向先记录决策，不另建第二套并行总路线。
