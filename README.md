# MCP Memory Ledger

本地整合验证：1117 项 Rust 测试通过，14 项因云端 AF_UNIX 限制失败；Node 13 / Python 87 通过，严格 Clippy 通过。这不是原生全绿，详见[整合验证](docs/final-local-integration-validation-2026-10-20.md)。

让本地 AI 客户端记住有来源的信息，找回证据，并在纠错后保留历史。

这是 Rust + SQLite 实现的 MCP `stdio` 记忆服务，当前定位是 **local-first technical MVP**。可以离线验证记录、检索、反馈纠错和备份恢复；尚未完成正式 Local Alpha、fresh-machine 安装和真实模型效果验收。

[文档导航](docs/document-map.md) · [快速开始](docs/quickstart.md) · [MCP 接入](docs/local-mcp-integration-2026-03-26.md) · [当前状态](docs/project-status.md) · [English](docs/README.en.md) · [日本語](docs/README.ja.md)

## 当前本地候选（2026-10-20，未发布）

已发布源码基线仍为 `4ac6cfc`。以下是其后的本地候选能力，不是完整 24 项计划完成声明、原生验收或部署批准。原 UI、中英日文与既有权限边界保留；没有安装、部署或修改用户机器。

- **Schema 11 本地所有者声明**：显式 `task-owner-attest` 保留精确要求、证据和逐要求来源；普通任务修改不能删除仍受保护的要求。须显式备份/排练迁移。这不是认证人类身份、同用户进程隔离或执行权限；签名宿主导入未实现。[合同](docs/local-owner-task-attestation.md)
- **显式上下文刷新与回执**：controller refresh 重新核验任务、范围、状态与产物；适配器 Schema 2 记录精确输出摘要，旧状态须显式迁移。输出回执不是宿主接受或模型采用证明，没有后台推送。[合同](docs/codex-task-context-adapter.md)
- **可选 v2 语义索引与统一选择**：显式构建/恢复/重置同域 claims、events、持久 Episodes 与当前活动经验索引；宿主 opt-in `unified` 模式先保留强制任务要求，再按共享预算选择知识。按来源种类报告有界覆盖与省略，不承诺全库 top-k，不自动重建；默认字面及既有增强模式不变。[索引](docs/rebuildable-semantic-index.md) · [统一选择](docs/unified-task-selection.md)
- **类型化环境报告**：`reported_environment` 仅是调用方未认证事实；类型和值不符可排除来源，缺失保持 unknown，不能更改宿主策略、模型或范围白名单。[增强召回](docs/enhanced-recall.md)
- **T14 最小结果 outbox 候选**：`codex-results` 只处理显式关联的历史 use，保存有界输出元数据和持久分阶段投递/重试；不是成功、因果收益或执行授权。原生宿主结果接受仍待验收。[合同](docs/codex-result-feedback.md)
- **部署预览**：只做目标平台路径词法检查，不证明同步归属或可部署；不安装。Windows hook 预览与 checked writes 仍 unsupported，不输出可安装占位命令。[路径预览](docs/deployment-path-preview.md) · [Windows 边界](docs/windows-hook-preview-assessment-2026-10-20.md)

已发布基线中的[显式模型草稿](docs/experience-model-drafts.md)、[增强召回](docs/enhanced-recall.md)和[本地 embedding / 派生缓存](docs/local-embeddings.md)继续保留；模型能力默认关闭或使用 mock，启用须遵守对应合同。

## 能做什么

- **记录经验采用反馈**：`record_experience_use` / `list_experience_uses` 与 `experience-use record|list|derive --json` 记录精确任务/经验版本、适用条件、采用/拒绝理由及结果来源；调用方报告不等于真实模型采用或独立验证。追加观察保留历史，当前可复用状态在查询时重新判断；显式 `derive_experience_from_use` 可分阶段恢复写入 Episode 与待审候选，不自动激活。既有与新增路径按版本分别验证，见当前状态。

- **保存任务要求与续接状态**：任务修订、稳定要求 ID、候选隔离、checkpoint 与完整性诊断；内容是有来源的调用方报告，不授予执行权限。
- **接入 Codex 上下文**：原生适配器提供显式任务选择、配置预览和生命周期 JSON；输出与真实宿主接受分开记录，支持范围见适配器合同。

- **记录与追溯**：持久化 Event、Claim、丰富 Episode、Reflection 及证据关系；区分服务端记录时间和调用方观察时间，未知历史保持未知。
- **按范围找回记忆**：保留 FTS5 / 短中文字面召回，另有增强入口、显式适用性声明和可选本地向量；原始查询及省略诊断可追溯。
- **准备任务上下文**：优先装入完整 Claim / Event，再加入有界 Episode 和可选状态诊断；整个结果按紧凑 JSON UTF-8 字节计量，不截断单条证据。
- **反馈与纠错**：保存反馈候选，验证明确的结构化证据合同，原子提交更正；保留旧 Claim、来源和可重试回执。
- **积累可审核经验**：语义/流程候选支持查看、拒绝、版本修订、回滚和显式激活。激活只改变可召回状态，不执行流程或授予权限。
- **可追溯本地打包**：从指定 Git 版本构建原生 portable archive，校验后隔离解包运行；这不是已批准的正式发行包。详见[本地包合同](docs/portable-packages.md)；平台脚本与实际验证范围见对应开发指南。
- **版本化全局 self-model**：identity/commitment 追加版本、同范围可见 diff、expected-version 冲突检测和显式组件补偿回滚；仍受既有 Claim、证据与全局写边界约束。详见[合同](docs/self-model-versions.md)。
- **本地数据运维**：显式初始化/迁移、默认只读诊断、索引检查/重建、备份恢复，以及有界只读 scoped export。

当前 standalone `serve` 注册 **43 个 MCP 工具**；核心工作空间会话开放其中 **36 个**，其余 7 个全局入口拒绝。任务、经验采用、显式草稿与增强召回沿用逐调用范围与撤销检查，验证状态以本提交结果为准。工具用途见[工具与合同索引](docs/tool-reference.md)，模式差异见[本机核心合同](docs/local-workspace-core.md)。二进制/crate 兼容名称仍是 `agent_llm_mm`，脚本和配置前缀仍是 `agent-llm-mm`。

## 从这里开始

1. [快速开始](docs/quickstart.md)：固定 Rust 工具链、构建、显式 mock 配置、新库初始化和只读检查。
2. [最小到完整工作流](docs/runnable-memory-workflow.md)：记录 → 重连 → 检索 → 查看证据 → 纠错 → 回看历史，并运行隔离的离线示例。
3. [连接 MCP 客户端](docs/local-mcp-integration-2026-03-26.md)：使用绝对二进制/配置/数据库路径，以 `stdio` 启动。
4. [数据库运维](docs/database-operations.md)：旧库升级、备份、恢复到新路径、故障诊断和派生索引修复。

平台脚本与开发细节单独见 [macOS](docs/development-macos.md) / [Windows](docs/development-windows.md)。首次使用建议保留 mock 和隔离测试库；常规 deterministic read 与调用方反馈/经验工具不需要模型；显式模型草稿需要 opt-in，可选本地 embedding 按配置运行，旧 ingest 自动反思钩子在满足触发条件时可能使用已配置 provider。

## 可选模型协议

默认 `mock` 不访问模型。显式配置可选择原有 `openai-compatible` / `openrouter` Chat Completions，或新增 `openai-responses`（OpenAI 原生 Responses）/ `anthropic`（Claude 原生 Messages）；旧配置不需要迁移。原生路径覆盖纯文本、非流式 decision、self-revision proposal 与显式 experience draft，保留本地治理与持久化边界。见[配置示例](docs/quickstart.md#5-可选原生模型配置)和[协议合同](docs/provider-contract.md)。

新增适配器的本地 fixture 不等于 live provider 认证；本轮未运行付费 API。无 streaming、工具调用、vision、多轮托管会话或 provider 路由。OpenAI 仍支持 Chat Completions；Responses 是可选原生路径，不是强制废弃旧接口。

## 必须知道的边界

- 普通 `recall_memory` 保留字面合同；独立增强入口提供中文分词、同域别名与 RRF，可选有界本地向量召回。均不证明语义正确，适用条件未自动求值。
- `max_bytes` 是完整结果的 JSON 字节上限，不是模型 token 上限，也不包含 MCP / JSON-RPC 外层包装。
- namespace 是数据范围，不是多用户认证；反馈来源标签不证明真实性，可选 caller budget 不是服务端配额。
- legacy 全局 self-model / decision 路径仍是实验性能力；不得当作完整可信策略执行器或统一隔离边界。
- export 是交换格式，不是可恢复备份或自动脱敏；保留全部历史与成功写回执，不自动删除。
- [Memory-chan Live Desk](docs/dashboard-live-desk.md) 支持中英日、真实导航/筛选、进程缓冲与持久历史、断线反馈和只读健康检查；统计范围与最近 100 条窗口明确。
- 多 MCP 共用 dashboard 端口时，非必需监听会在会话存活期间等待并接管已释放端口；进程缓冲仍各自独立，详见 [Live Desk 生命周期](docs/dashboard-live-desk.md#多个-mcp-会话共用端口)。
- 旧 standalone dashboard 仅限 loopback 且无认证。新核心默认关闭监控 HTTP，管理员能力独立隔离验证，不复用旧无认证入口；没有生产级远程/团队服务、后台写入自治或已发布安装包。

## 验证到了哪里

已发布源码仍为 `4ac6cfc`；本地候选不是已发布版本，也不继承旧提交 CI。最终本地整合结果仍在核验，精确新提交的 Ubuntu / macOS / Windows 原生 CI 尚待完成；云端 AF_UNIX 限制、真实宿主接受、fresh-machine 安装和真实模型收益分别记账。24 项计划尚未全部完成。

历史测试数字和合成场景仅属于各自记录版本，不是当前候选的全量通过证明。固定离线检索/更正代理指标 8/10 也不代表真实模型任务收益、token 节省或生产 SLA。

[验证方法与复现](docs/testing-guide-2026-03-24.md) · [当前事实与剩余门禁](docs/project-status.md) · [恢复版本记录](docs/restored-feature-validation-2026-10-19.md) · [后续切片记录](docs/follow-on-validation-2026-10-20.md) · [历史 70 项核对](docs/plans/2026-10-09-original-plan-final-reconciliation.md)

## 参与开发

阅读 [CONTRIBUTING](CONTRIBUTING.md)、[项目起点与原则](docs/origin-and-principles.md) 和[路线图](docs/roadmap.md)。报告问题见 [SUPPORT](SUPPORT.md)，安全问题先核实 [SECURITY](SECURITY.md) 所述私密渠道；[协作准则](CODE_OF_CONDUCT.md)仍是拟定稿，[CHANGELOG](CHANGELOG.md)不构成发布批准。唯一执行队列是 [active plan](docs/plans/README.md)；历史计划、阶段证据和发布草案不能替代它。
