# MCP Memory Ledger 路线图

本切片云端全量 929 项通过、14 项 AF_UNIX 环境阻断，无其他失败；精确提交 CI 待运行。详细范围见[第二切片验证记录](task-experience-validation-2026-10-16.md)。

## 当前推进范围（2026-10-16）

首批任务续接后，动作检查及 Schema 9 经验反馈/P5 只读展示已合入源码，已通过云端定向回归，最终全量已记录，下一精确提交 CI 待核验。`20d4037` 的 Ubuntu 已通过；macOS / Windows artifact fixture 路径规范化测试修补已合入，原生复验尚未闭合，详见[当前状态](project-status.md)。T16 模型草稿、P4 检索与真实宿主探针正在推进，不提前记为完成。

## T09–T11 已整合范围

受限 patch 检查、故障降级、配置预览与撤回候选已合入源码；见 [动作检查合同](codex-action-checks.md)。默认最新任务版本重读，不因普通修订或 checkpoint 要求重批 policy；仅显式 pinned 模式固定任务版本。定向回归已通过，真实宿主证据待完成。实际配置安装、通用 shell 强制和完整安全边界不在已完成范围。

## P3 / P5 当前有界实现

已合入任务版本绑定的经验采用/结果记录、当前资格重查、失效原因及调用方显式 use → Episode → pending 候选组合；复用既有经验版本/激活体系，不建立第二套经验库。原 owner-only 界面增量展示三语言历史报告与结果来源，明确区别当前资格。见[经验反馈合同](experience-use-feedback.md)和[网页范围](dashboard-live-desk.md)。

T13 记录核心已实现；T12/T15 为当前版本和来源重查支持，T14 为显式可恢复组合，均未宣称完整自然语言适用性或自动反馈闭环。T16 现有调用方候选与在研模型草稿分开记账；P4 及真实宿主注入继续按唯一计划推进。整合测试、真实浏览器和用户部署各自验收，不继承首批结果。

## 任务续接首批源码实现

Schema 8 增加任务要求修订、候选隔离、checkpoint 和有界续接上下文；6 个新任务 MCP 工具复用范围与撤销策略。原界面提供 owner-only 只读任务摘要，旧无凭据 observer 不暴露任务内容。原生 Codex 适配器提供显式项目默认任务、生命周期上下文输出和配置预览。

- [任务合同与迁移](task-ledger.md)
- [Codex 适配与真实证据边界](codex-task-context-adapter.md)
- [网页任务摘要](dashboard-live-desk.md)

本轮云端 Rust 878 项通过，14 项 IPC 环境阻断；严格 Clippy、Node 11 项及 Python 127 项通过。详细范围见[首批验证记录](task-continuity-validation-2026-10-16.md)；后续 CI 状态见本页顶部。

首批与后续整合版本分别记账。调用方报告不等于认证用户输入或独立结果验证；hook 输出不等于宿主接受。真实 Codex 会话启动受云端沙箱限制，自动注入尚未验收；Windows 核心仍已支持，但新 hook 的 Windows shell 配置预览未验证。旧 schema 须显式迁移；scoped export 不包含任务数据，完整数据库备份包含。

动作检查与经验反馈的已实现范围见上文，P4 检索增强及未闭合验收继续按唯一计划推进。实际用户机器配置和部署仍另行确认。


## 当前执行入口与已完成基线（2026-10-16）

已批准按[完整唯一实施计划](plans/2026-10-16-task-continuity-and-host-integration.md)分阶段推进。P0 与 T01–T08 首批后已继续整合动作检查及 P3/P5 切片，当前处于集成审查与回归；真实宿主注入等未闭合项仍单列，批准不等于全部完成。

基线 `7448b02a7de8d5dd32c32a149fdd024f798bbf0f` 的 [CI 38860968271](https://github.com/yooyui/mcp-memory-ledger/actions/runs/38860968271) 三平台全部成功：Ubuntu/macOS 各 848 项 Rust，Windows 514 项 configured Rust；三平台精确来源候选包通过，Windows named-pipe core 的 42 次 MCP 工具调用、隔离、确认写后重启/重放、备份恢复与清理通过。Ubuntu 真实 Chrome 检查通过。

workspace 生命周期、只读配置生成、原生 DB-only 恢复、有界 reflection/experience 历史接入及 Windows 核心均已实现并获得上述范围验证；[独立审查](independent-security-review-2026-10-16.md)及修复已完成限定范围。它们不再是待实现项目。仍非全面安全认证、真实用户安装、任意在途崩溃保证或正式发布；真实宿主注入与新增功能须获得新证据，用户机器安装另行确认。

### 以下为历史阶段记录

下文带日期的“待验证/未提交/Windows 未支持”仅保留当时事实，由上面精确基线结论更新；旧阶段路线不再拥有执行优先权，当前任务只由唯一实施计划管理。


## 2026-10-16 Windows 本机核心 candidate

新增 Windows 本机命名管道 `core` + stdio `bridge` 实现，与 Unix 共用绑定、逐调用撤销、资源限额和关闭排空逻辑。新建 IPC/状态文件使用当前用户私有 DACL；已有文件/目录须当前用户所有，访问授权只接受当前用户、SYSTEM 和 Administrators。它不隔离同用户任意进程或特权管理员，也不是远端多租户服务。`serve` 兼容入口保留。

当前为待精确提交 Windows 原生 CI 验证的 candidate，不能把 Linux 编译或此前 legacy Windows CI 当作本轮通过。验证包括真实桥多会话、范围拒绝、重连/恢复，以及解包二进制烟测；无用户电脑操作、生产 ACL 修改、凭据生成、服务安装或部署。入口与限制见 [Windows 指南](development-windows.md)。


旧项目接入扩展为可归属 reflection/experience 全版本/feedback/trigger 历史的双向来源闭包；不再仅支持基础事实。unknown/mixed/缺失来源、未知 provenance schema 与全局 self-model 后续版本继续开放，不自动迁移、清洗或部署。见[范围](workspace-legacy-adoption.md)。

## 2026-10-16 Now：先补齐可用功能

当前执行切片为 workspace 解绑/停用/启用、只读检查与客户端 TOML 生成、原生 backup/restore 到新路径，代码已接入但未提交、待最终验证。见[状态](project-status.md)、[生命周期](local-workspace-core.md)、[配置输出](workspace-client-config.md)和[恢复手册](database-operations.md)。

允许将 M2 中这些有界实用功能前移，不以 M1 真实客户端验收尚缺为实现阻塞；保留数据、不自动部署、不扩入远程/自治。此前 `f8eb9ed` 精确 CI 已绿，本轮有界独立源码审查已完成（见[报告](independent-security-review-2026-10-16.md)）；新核心真实客户端自然语言、fresh-machine 与发布门仍开放。后文 W1–W4 是依赖和验收范围，不是要求先重复文档/测试阶段才能继续功能的队列。

## 2026-10-15 本机核心与工作空间方向

用户已批准在本地主线补齐单核心服务、轻量 stdio 桥、稳定 workspace 绑定、服务端范围授权、个人记忆显式共享与统一监控。当前 **M1.W1 进行中，新核心/桥待验收、未部署**；具体顺序只由[唯一 active plan](plans/README.md)管理，[验收矩阵](workspace-core-acceptance.md)记录证据与未测边界。

旧版 d9e33d3 的正式 Mac 已有直接 MCP、网页及一个真实自然语言召回/更正样本；不能再笼统说真实客户端完全未测，也不能把它们当作新架构通过。当前 Codex roots 实测67条断言通过，但未提供自动跟随项目的依据：连接固定绑定，移动会话不会自动重绑。新核心首期为 macOS/Linux Unix sockets；Windows 保持现有 stdio，新核心暂不支持。正式部署与现有配置保持不变。

早期 app-server、体验工具及模型效果边界仍见[2026-10-15验收范围](acceptance-2026-10-15.md)。fresh-machine、真实模型效果/费用与发布门保持开放。

## 2026-10-13 验收工具更新

三协议验收器已覆盖 Chat、Responses、Anthropic；新增请求/输入/输出限制与失败用量记录。回环 fixture 与真实调用分层，未知价格不声明美元硬上限；真实模型与用户客户端仍待验收。候选包增加更正重放、跨 namespace 直接查询/历史隔离及恢复后的历史/证据比对。当前步骤和证据口径见[本轮验收记录](acceptance-2026-10-13.md)。


状态：`active summary` · 更新：2026-10-15

目标是可信、可检索、可审计、可恢复的本地记忆。当前仍为 technical MVP；[状态页](project-status.md)列实际能力，[唯一 active plan](plans/README.md)列任务、依赖、证据门与停止条件。本页不是第二份任务队列。

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

M2 的正式退出仍依赖 M1 退出门；有界实现可按顶部当前切片前移：

- 已具备可追溯 binary archive/checksum 与 configured-host CI 的无 Rust 解包闭环；仍须真实 fresh-machine 10 分钟内验收；
- Windows x86_64 / PowerShell 7.6.6 已有实际 wrapper 与包验证证据；持续验证新 head，并明确其他 OS/架构与用户客户端支持范围；
- 原生 backup / restore-to-new-path 已进入当前实现切片；read-only verify 与 manual switch 保持显式步骤，跨平台和真实恢复证据继续单列；
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
