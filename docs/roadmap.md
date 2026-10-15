# MCP Memory Ledger 路线图

## 2026-10-15 本机核心与工作空间方向

用户已批准在本地主线补齐单核心服务、轻量 stdio 桥、稳定 workspace 绑定、服务端范围授权、个人记忆显式共享与统一监控。当前 **M1.W1 进行中，新核心/桥待验收、未部署**；具体顺序只由[唯一 active plan](plans/2026-07-10-product-replan.md)管理，[验收矩阵](workspace-core-acceptance.md)记录证据与未测边界。

旧版 d9e33d3 的正式 Mac 已有直接 MCP、网页及一个真实自然语言召回/更正样本；不能再笼统说真实客户端完全未测，也不能把它们当作新架构通过。当前 Codex roots 实测67条断言通过，但未提供自动跟随项目的依据：连接固定绑定，移动会话不会自动重绑。新核心首期为 macOS/Linux Unix sockets；Windows 保持现有 stdio，新核心暂不支持。正式部署与现有配置保持不变。

早期 app-server、体验工具及模型效果边界仍见[2026-10-15验收范围](acceptance-2026-10-15.md)。fresh-machine、真实模型效果/费用与发布门保持开放。

## 2026-10-13 验收工具更新

三协议验收器已覆盖 Chat、Responses、Anthropic；新增请求/输入/输出限制与失败用量记录。回环 fixture 与真实调用分层，未知价格不声明美元硬上限；真实模型与用户客户端仍待验收。候选包增加更正重放、跨 namespace 直接查询/历史隔离及恢复后的历史/证据比对。当前步骤和证据口径见[本轮验收记录](acceptance-2026-10-13.md)。


状态：`active summary` · 更新：2026-10-15

目标是可信、可检索、可审计、可恢复的本地记忆。当前仍为 technical MVP；[状态页](project-status.md)列实际能力，[唯一 active plan](plans/2026-07-10-product-replan.md)列任务、依赖、证据门与停止条件。本页不是第二份任务队列。

## 已完成：M0 与原方案的有限实现

M0 的 scoped snapshot、治理原子性、显式数据库生命周期、loopback/stderr 边界和固定工具链已实现。随后完成 scoped 四类记录读取、可靠纠错/重试、反馈候选、FTS5 字面召回、丰富 Episode/经验版本、schema6 时间与 Reflection 范围、readonly export、任务上下文/预算及内部模块拆分。

完整实现清单见[当前状态](project-status.md)；原 A–E 讨论的 70 项要求见[最终核对](plans/2026-10-09-original-plan-final-reconciliation.md)。这组有限代码交付已完成，不能继续称为“下一步待实现”，也不能据此将 M1/M2 的真实用户与发布门自动打勾。

原生 OpenAI Responses 与 Anthropic Messages 的有界文本适配已加入，旧 Chat Completions/OpenRouter 保留，见[协议合同](provider-contract.md)。这是本地配置/协议/stdio fixture 的实现切片，不关闭 M1 用户客户端或 M2 live provider 门；streaming、tools、vision、托管会话与广泛 provider 扩张仍不在本轮范围。

## Now：M1.W1 本机核心与可信工作空间

按 active plan 的 W1→W2→W3→W4 依赖推进，不重做已完成的账本/检索/更正基础：

- 先完成稳定 registry、固定 `--workspace-id`/`--workspace-root` 或一次全局 `--registry` 配置的启动cwd精确绑定与逐调用授权刷新；个人读写显式授予，撤销也约束旧连接和回放。
- Unix核心统一持库与observer，多个stdio桥连接；普通workspace隐藏/拒绝七个全局工具，core ingest关闭自动反思，未知工具默认拒绝。
- 再补全客户端聚合、成功召回历史、管理员能力隔离与恢复验证；核心默认关闭监控HTTP，显式管理员CLI与内存登录只作隔离合成验证，真实核心网页端到端仍待验收。
- 以隔离候选、独立审查和精确提交CI收口，再分别验证实际客户端协议和自然语言体验。正式切换、全局配置及开机服务另行确认。

现有端口接管补丁只处理旧架构局部生命周期问题，不能替代上述架构。M1.3.1原冻结故事及新workspace授权故事须逐项对齐；少量实际样本不自动关闭全部M1退出门。

## Next：M2 Local Product Alpha

源代码绑定 portable build/解包验证与实际 Windows wrapper 行为测试已有实现和历史精确提交证据；无需重复领取。新核心/桥须扩展候选验证，并区分 macOS/Linux 新路径与 Windows legacy stdio。当前进度以[状态页](project-status.md)、[本地包合同](portable-packages.md)及[本轮验收矩阵](workspace-core-acceptance.md)为准。

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

Streamable HTTP、远程/团队认证授权与多租户、remote dashboard、write-capable autonomous daemon、队列重试、experimental tasks、多 Agent 调度和自治流程仍需独立需求、威胁模型与权限。本轮已批准的本机workspace授权和手动启动核心不属于这些后置方向，也不授权安装service manager。经验候选 activation 不构成执行授权。

主线保持 `M0 → M1（含本次W1–W4）→ M2 → M3 → 条件性 M4`；已批准的有限实现前移及仍开放外部门以active plan为准。正式化差距见[验收映射](formalization-improvement-plan-2026-08-25.md)，不是扩展开发范围的默认许可。

## 历史与依据

- [项目起点与主线原则](origin-and-principles.md)
- [Reality gates](product/follow-up-reality-gates.md)
- [阶段历史](project-status-history-2026-10-09.md) / [计划索引](plans/README.md) / [历史归档](archive.md)
