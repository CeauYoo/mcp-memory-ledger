# MCP Memory Ledger 路线图

状态：`active summary` · 更新：2026-10-10

目标是可信、可检索、可审计、可恢复的本地记忆。当前仍为 technical MVP；[状态页](project-status.md)列实际能力，[唯一 active plan](plans/2026-07-10-product-replan.md)列任务、依赖、证据门与停止条件。本页不是第二份任务队列。

## 已完成：M0 与原方案的有限实现

M0 的 scoped snapshot、治理原子性、显式数据库生命周期、loopback/stderr 边界和固定工具链已实现。随后完成 scoped 四类记录读取、可靠纠错/重试、反馈候选、FTS5 字面召回、丰富 Episode/经验版本、schema6 时间与 Reflection 范围、readonly export、任务上下文/预算及内部模块拆分。

完整实现清单见[当前状态](project-status.md)；原 A–E 讨论的 70 项要求见[最终核对](plans/2026-10-09-original-plan-final-reconciliation.md)。这组有限代码交付已完成，不能继续称为“下一步待实现”，也不能据此将 M1/M2 的真实用户与发布门自动打勾。

## Now：关闭 M1 真实用户闭环证据

用实际 MCP 客户端验证：记录 → 重连 → scoped 检索 → 查看证据 → 更正 → 回看历史，并让两个干扰 namespace 不混入结果。

- 复用[可执行离线工作流](runnable-memory-workflow.md)，不要重新复制一套合同。
- 精确提交 CI 已有实现基线证据；新 head 仍必须单独核验。
- 客户端、OS 和数据库路径须明确记录；mock 协议 smoke 不代替真实用户客户端验收。

## Next：M2 Local Product Alpha

本轮先实现不依赖外部账号的源代码绑定 portable build/解包验证和 Windows wrapper 行为测试；它们提供 M2 的实现基础，不能自动关闭 M1 真实客户端门。当前进度以[状态页](project-status.md)和[本地包合同](portable-packages.md)为准。

在 M1 退出门基础上补齐：

- 已具备可追溯 binary archive/checksum 与 configured-host CI 的无 Rust 解包闭环；仍须真实 fresh-machine 10 分钟内验收；
- Windows x86_64 / PowerShell 7.6.6 已有实际 wrapper 与包验证证据；持续验证新 head，并明确其他 OS/架构与用户客户端支持范围；
- backup → restore-to-new-path → read-only verify → manual switch；
- 一个真实 provider 的连通/解析证据（需明确数据与费用授权）；
- 人工 release decision、rollback note 和公开文档审阅。

数据生命周期已实现不等于发布证据已齐。不得把 source build、安装模拟或 provider certification 当作正式 Alpha。

## Later：M3 检索质量与生命周期

最小字面检索、FTS5、固定双语 fixture、10k/100k 观测和有界 export 已前移完成。剩余质量工作由测量驱动：

- 扩充独立任务集，并做真实同模型对照、token/成本与因果效果实验；
- 根据证据评估 relation ranking、semantic retrieval 和查询策略；
- 有界 identity/commitment 追加版本、来源安全 diff、记录边界 effective time 与显式补偿 rollback 已实现，见[合同](self-model-versions.md)；更广泛生命周期政策另行决定；
- 只有先确定数据政策和授权，才研究 retention、tombstone、compaction；目前 retain-all。

计划质量门仍为 recall@5 ≥ 0.80、provenance coverage = 100%、namespace leakage = 0；固定 10 项离线 proxy 达到部分数字不代表完整 M3 通过。

## 不排期：M4 Remote / Autonomy

Streamable HTTP、认证/授权/多租户、remote dashboard、write-capable daemon、队列重试、experimental tasks、多 Agent 调度和自治流程仍需独立需求、威胁模型与权限。经验候选 activation 不构成执行授权。

顺序保持 `M0 → M1 → M2 → M3 → 条件性 M4`。正式化差距见[验收映射](formalization-improvement-plan-2026-08-25.md)，不是扩展开发范围的默认许可。

## 历史与依据

- [项目起点与主线原则](origin-and-principles.md)
- [Reality gates](product/follow-up-reality-gates.md)
- [阶段历史](project-status-history-2026-10-09.md) / [计划索引](plans/README.md) / [历史归档](archive.md)
