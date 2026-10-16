# MCP Memory Ledger

## 动作检查候选切片（T09–T11）

新增原生 `codex-actions` 的 deny-only 参数检查、独立操作者策略与配置/撤回预览，详见[动作检查合同](docs/codex-action-checks.md)。默认只检查匹配的 apply_patch；普通任务修订自动读取当前 active 版本，policy epoch 不随任务版本提升。shell、hosted tools、stdin、宿主故障和同 OS 用户篡改不受全面保护。已通过云端定向测试与严格 Clippy；真实宿主动作检查仍待验收，没有安装用户配置。

## 当前开发切片

按[唯一实施计划](docs/plans/2026-10-16-task-continuity-and-host-integration.md)推进任务要求、进度续接与客户端接入。本轮新增[任务账本](docs/task-ledger.md)、[Codex 上下文适配器](docs/codex-task-context-adapter.md)及原监控界面的只读任务摘要；新增能力的验证状态见[当前状态](docs/project-status.md)。源码更新不等于已部署，真实宿主注入与模型采用须分别取得证据。

上一实现基线 `7448b02` 的[三平台 CI](https://github.com/yooyui/mcp-memory-ledger/actions/runs/38860968271)已通过，含 Windows 新核心及候选包、复杂历史接入和限定范围安全修复。本轮新增代码不能继承旧提交的绿灯。带日期的开发过程保留在[状态记录](docs/project-status.md)及[此前 README](https://github.com/CeauYoo/mcp-memory-ledger/blob/9656fe8fd6463c64688b72da74c22c41b16b8212/README.md)。

让本地 AI 客户端记住有来源的信息，找回证据，并在纠错后保留历史。

这是 Rust + SQLite 实现的 MCP `stdio` 记忆服务，当前定位是 **local-first technical MVP**。可以离线验证记录、检索、反馈纠错和备份恢复；尚未完成正式 Local Alpha、fresh-machine 安装和真实模型效果验收。

[文档导航](docs/document-map.md) · [快速开始](docs/quickstart.md) · [MCP 接入](docs/local-mcp-integration-2026-03-26.md) · [当前状态](docs/project-status.md) · [English](docs/README.en.md) · [日本語](docs/README.ja.md)

## 能做什么

- **记录经验采用反馈**：`record_experience_use` / `list_experience_uses` 与 `experience-use record|list|derive --json` 记录精确任务/经验版本、适用条件、采用/拒绝理由及结果来源；调用方报告不等于真实模型采用或独立验证。追加观察保留历史，当前可复用状态在查询时重新判断；显式 `derive_experience_from_use` 可分阶段恢复写入 Episode 与待审候选，不自动激活。已通过云端定向回归，三平台与真实宿主结果单独记录。

- **保存任务要求与续接状态**：任务修订、稳定要求 ID、候选隔离、checkpoint 与完整性诊断；内容是有来源的调用方报告，不授予执行权限。
- **接入 Codex 上下文**：原生适配器提供显式任务选择、配置预览和生命周期 JSON；输出与真实宿主接受分开记录，支持范围见适配器合同。

- **记录与追溯**：持久化 Event、Claim、丰富 Episode、Reflection 及证据关系；区分服务端记录时间和调用方观察时间，未知历史保持未知。
- **按范围找回记忆**：显式 namespace 的浏览、稳定 ID 查询、修订历史，以及 FTS5 / 短中文精确字面召回。
- **准备任务上下文**：优先装入完整 Claim / Event，再加入有界 Episode 和可选状态诊断；整个结果按紧凑 JSON UTF-8 字节计量，不截断单条证据。
- **反馈与纠错**：保存反馈候选，验证明确的结构化证据合同，原子提交更正；保留旧 Claim、来源和可重试回执。
- **积累可审核经验**：语义/流程候选支持查看、拒绝、版本修订、回滚和显式激活。激活只改变可召回状态，不执行流程或授予权限。
- **可追溯本地打包**：从指定 Git 版本构建原生 portable archive，校验后隔离解包运行；这不是已批准的正式发行包。详见[本地包合同](docs/portable-packages.md)；平台脚本与实际验证范围见对应开发指南。
- **版本化全局 self-model**：identity/commitment 追加版本、同范围可见 diff、expected-version 冲突检测和显式组件补偿回滚；仍受既有 Claim、证据与全局写边界约束。详见[合同](docs/self-model-versions.md)。
- **本地数据运维**：显式初始化/迁移、默认只读诊断、索引检查/重建、备份恢复，以及有界只读 scoped export。

当前 standalone `serve` 注册 **41 个 MCP 工具**；核心工作空间会话开放其中 **34 个**，其余 7 个全局入口拒绝。新增 6 个任务工具和 3 个经验采用工具沿用逐调用范围与撤销检查，验证状态以本提交结果为准。工具用途见[工具与合同索引](docs/tool-reference.md)，模式差异见[本机核心合同](docs/local-workspace-core.md)。二进制/crate 兼容名称仍是 `agent_llm_mm`，脚本和配置前缀仍是 `agent-llm-mm`。

## 从这里开始

1. [快速开始](docs/quickstart.md)：固定 Rust 工具链、构建、显式 mock 配置、新库初始化和只读检查。
2. [最小到完整工作流](docs/runnable-memory-workflow.md)：记录 → 重连 → 检索 → 查看证据 → 纠错 → 回看历史，并运行隔离的离线示例。
3. [连接 MCP 客户端](docs/local-mcp-integration-2026-03-26.md)：使用绝对二进制/配置/数据库路径，以 `stdio` 启动。
4. [数据库运维](docs/database-operations.md)：旧库升级、备份、恢复到新路径、故障诊断和派生索引修复。

平台脚本与开发细节单独见 [macOS](docs/development-macos.md) / [Windows](docs/development-windows.md)。首次使用建议保留 mock 和隔离测试库；常规 deterministic read 与新反馈/经验工具不需要模型，旧 ingest 自动反思钩子在满足触发条件时可能使用已配置 provider。

## 可选模型协议

默认 `mock` 不访问模型。显式配置可选择原有 `openai-compatible` / `openrouter` Chat Completions，或新增 `openai-responses`（OpenAI 原生 Responses）/ `anthropic`（Claude 原生 Messages）；旧配置不需要迁移。原生路径覆盖纯文本、非流式 decision 与 self-revision proposal，保留本地治理与持久化边界。见[配置示例](docs/quickstart.md#5-可选原生模型配置)和[协议合同](docs/provider-contract.md)。

新增适配器的本地 fixture 不等于 live provider 认证；本轮未运行付费 API。无 streaming、工具调用、vision、多轮托管会话或 provider 路由。OpenAI 仍支持 Chat Completions；Responses 是可选原生路径，不是强制废弃旧接口。

## 必须知道的边界

- 检索是**字面匹配**：FTS5 和短 CJK fallback 不是 embedding、中文分词或语义理解；同义改写可能找不到。
- `max_bytes` 是完整结果的 JSON 字节上限，不是模型 token 上限，也不包含 MCP / JSON-RPC 外层包装。
- namespace 是数据范围，不是多用户认证；反馈来源标签不证明真实性，可选 caller budget 不是服务端配额。
- legacy 全局 self-model / decision 路径仍是实验性能力；不得当作完整可信策略执行器或统一隔离边界。
- export 是交换格式，不是可恢复备份或自动脱敏；保留全部历史与成功写回执，不自动删除。
- [Memory-chan Live Desk](docs/dashboard-live-desk.md) 支持中英日、真实导航/筛选、进程缓冲与持久历史、断线反馈和只读健康检查；统计范围与最近 100 条窗口明确。
- 多 MCP 共用 dashboard 端口时，非必需监听会在会话存活期间等待并接管已释放端口；进程缓冲仍各自独立，详见 [Live Desk 生命周期](docs/dashboard-live-desk.md#多个-mcp-会话共用端口)。
- 旧 standalone dashboard 仅限 loopback 且无认证。新核心默认关闭监控 HTTP，管理员能力独立隔离验证，不复用旧无认证入口；没有生产级远程/团队服务、后台写入自治或已发布安装包。

## 验证到了哪里

实现基线 `9ba0050` 的[精确提交 CI](https://github.com/yooyui/mcp-memory-ledger/actions/runs/37930021202) 已通过：Linux/macOS 各 633 项 Rust 测试，Windows 247 项 native 测试；三平台均通过 9 项 Python 测试及实际二进制离线工作流。Windows native 覆盖不等于全部 shell wrapper parity。

固定离线检索/更正代理指标为 8/10，保留两个语义改写未命中；不能据此声称真实模型任务收益或 token 节省。容量测量包含速度与存储代价，不承诺生产 SLA。最新文档提交是否绿色，以它自己的 checks 为准。

[验证方法与复现](docs/testing-guide-2026-03-24.md) · [当前事实与剩余门禁](docs/project-status.md) · [70 项最终核对](docs/plans/2026-10-09-original-plan-final-reconciliation.md)

## 参与开发

阅读 [CONTRIBUTING](CONTRIBUTING.md)、[项目起点与原则](docs/origin-and-principles.md) 和[路线图](docs/roadmap.md)。报告问题见 [SUPPORT](SUPPORT.md)，安全问题先核实 [SECURITY](SECURITY.md) 所述私密渠道；[协作准则](CODE_OF_CONDUCT.md)仍是拟定稿，[CHANGELOG](CHANGELOG.md)不构成发布批准。唯一执行队列是 [active plan](docs/plans/README.md)；历史计划、阶段证据和发布草案不能替代它。

See [experience-use contract](docs/experience-use-feedback.md) for schema 9, caller-report boundaries, read-time invalidation, recoverable pending derivation, and explicit partial status of T12/T14–T16.
