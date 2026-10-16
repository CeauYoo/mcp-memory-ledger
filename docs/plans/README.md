# 项目计划索引

本目录只保留一个当前执行入口。历史计划可以用于追溯，但不得覆盖当前代码事实、项目状态或 active plan。

## 当前计划

- [2026-10-16-task-continuity-and-host-integration.md](2026-10-16-task-continuity-and-host-integration.md)
  - 状态：`active / implementation approved`；用户已批准按方案实施
  - 当前切片：P0 与 T01–T08 最小闭环，持久任务要求、checkpoint、任务上下文、Codex 接入和送达证据
  - 后续：T09–T11 受支持动作检查；T12–T16 经验采用反馈；P4 检索在接口稳定后并行；P5 随功能交付
  - 基线：7448b02 三平台 CI 与候选包已过；workspace 生命周期、配置输出、原生恢复、有界复杂历史、Windows core 和限定安全审查不再重复列为待实现
  - 真实宿主注入、fresh-machine、真实模型效果、用户机器安装与正式发布保持各自独立证据门

## 权威层级

| 文档 | 唯一职责 |
| --- | --- |
| [`docs/origin-and-principles.md`](../origin-and-principles.md) | 灵感起点、不可丢失的架构原则和新能力准入条件 |
| [`docs/positioning.md`](../positioning.md) | 公共名称、目标用户和对外口径 |
| [`docs/project-status.md`](../project-status.md) | 当前分支真实实现状态 |
| [`docs/roadmap.md`](../roadmap.md) | Now / Next / Later 产品路线 |
| [当前 active plan](2026-10-16-task-continuity-and-host-integration.md) | 任务顺序、依赖、证据门和停止条件 |
| [`docs/product/follow-up-reality-gates.md`](../product/follow-up-reality-gates.md) | 已实现、部分实现、未验证和阻断证据 |

出现冲突时，先以当前代码和测试证据为准，再按上表顺序修正文档。测试文件存在不等于测试已经在当前分支通过；只有当前运行记录可以作为 fresh evidence。

## 已被取代的计划

- [2026-07-10-product-replan.md](2026-07-10-product-replan.md)：历史路线与阶段证据，2026-10-16 起不再是 active plan。

2026-03 至 2026-06 的 specs、execution plans、P1/P2/P3、productization、阶段进度和发布记录已移至本地分支 `codex/archive/pre-mainline-reset-2026-07-10`。完整路径和读取方式见 [`docs/archive.md`](../archive.md)。

归档文件中的已完成项、未完成框和阶段名称只描述当时语境，不再拥有执行权。后续任务必须从当前 active plan 领取，并把结果写回当前状态与 reality gate。

## 更新规则

1. 开始实现前，只把当前 active milestone 的一个最小切片标为进行中。
2. 代码、测试与对应文档在同一任务更新。
3. 先记录验证命令和结果，再改变完成状态。
4. 遇到证据不符时停止扩张，不自动进入下一个里程碑。
5. 新的大方向先记录决策，不另建第二套并行总路线。
