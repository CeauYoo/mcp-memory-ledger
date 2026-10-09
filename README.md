# MCP Memory Ledger

让本地 AI 客户端记住有来源的信息，找回证据，并在纠错后保留历史。

这是 Rust + SQLite 实现的 MCP `stdio` 记忆服务，当前定位是 **local-first technical MVP**。可以离线验证记录、检索、反馈纠错和备份恢复；尚未完成正式 Local Alpha、fresh-machine 安装和真实模型效果验收。

[文档导航](docs/document-map.md) · [快速开始](docs/quickstart.md) · [MCP 接入](docs/local-mcp-integration-2026-03-26.md) · [当前状态](docs/project-status.md) · [English](docs/README.en.md) · [日本語](docs/README.ja.md)

## 能做什么

- **记录与追溯**：持久化 Event、Claim、丰富 Episode、Reflection 及证据关系；区分服务端记录时间和调用方观察时间，未知历史保持未知。
- **按范围找回记忆**：显式 namespace 的浏览、稳定 ID 查询、修订历史，以及 FTS5 / 短中文精确字面召回。
- **准备任务上下文**：优先装入完整 Claim / Event，再加入有界 Episode 和可选状态诊断；整个结果按紧凑 JSON UTF-8 字节计量，不截断单条证据。
- **反馈与纠错**：保存反馈候选，验证明确的结构化证据合同，原子提交更正；保留旧 Claim、来源和可重试回执。
- **积累可审核经验**：语义/流程候选支持查看、拒绝、版本修订、回滚和显式激活。激活只改变可召回状态，不执行流程或授予权限。
- **本地数据运维**：显式初始化/迁移、默认只读诊断、索引检查/重建、备份恢复，以及有界只读 scoped export。

当前注册 **31 个 MCP 工具**，按用途见[工具与合同索引](docs/tool-reference.md)。二进制/crate 兼容名称仍是 `agent_llm_mm`，脚本和配置前缀仍是 `agent-llm-mm`。

## 从这里开始

1. [快速开始](docs/quickstart.md)：固定 Rust 工具链、构建、显式 mock 配置、新库初始化和只读检查。
2. [最小到完整工作流](docs/runnable-memory-workflow.md)：记录 → 重连 → 检索 → 查看证据 → 纠错 → 回看历史，并运行隔离的离线示例。
3. [连接 MCP 客户端](docs/local-mcp-integration-2026-03-26.md)：使用绝对二进制/配置/数据库路径，以 `stdio` 启动。
4. [数据库运维](docs/database-operations.md)：旧库升级、备份、恢复到新路径、故障诊断和派生索引修复。

平台脚本与开发细节单独见 [macOS](docs/development-macos.md) / [Windows](docs/development-windows.md)。首次使用建议保留 mock 和隔离测试库；常规 deterministic read 与新反馈/经验工具不需要模型，旧 ingest 自动反思钩子在满足触发条件时可能使用已配置 provider。

## 必须知道的边界

- 检索是**字面匹配**：FTS5 和短 CJK fallback 不是 embedding、中文分词或语义理解；同义改写可能找不到。
- `max_bytes` 是完整结果的 JSON 字节上限，不是模型 token 上限，也不包含 MCP / JSON-RPC 外层包装。
- namespace 是数据范围，不是多用户认证；反馈来源标签不证明真实性，可选 caller budget 不是服务端配额。
- legacy 全局 self-model / decision 路径仍是实验性能力；不得当作完整可信策略执行器或统一隔离边界。
- export 是交换格式，不是可恢复备份或自动脱敏；保留全部历史与成功写回执，不自动删除。
- dashboard 仅限 loopback 且无认证；没有生产级远程/团队服务、后台写入自治或已发布安装包。

## 验证到了哪里

实现基线 `9ba0050` 的[精确提交 CI](https://github.com/yooyui/mcp-memory-ledger/actions/runs/37930021202) 已通过：Linux/macOS 各 633 项 Rust 测试，Windows 247 项 native 测试；三平台均通过 9 项 Python 测试及实际二进制离线工作流。Windows native 覆盖不等于全部 shell wrapper parity。

固定离线检索/更正代理指标为 8/10，保留两个语义改写未命中；不能据此声称真实模型任务收益或 token 节省。容量测量包含速度与存储代价，不承诺生产 SLA。最新文档提交是否绿色，以它自己的 checks 为准。

[验证方法与复现](docs/testing-guide-2026-03-24.md) · [当前事实与剩余门禁](docs/project-status.md) · [70 项最终核对](docs/plans/2026-10-09-original-plan-final-reconciliation.md)

## 参与开发

阅读 [CONTRIBUTING](CONTRIBUTING.md)、[项目起点与原则](docs/origin-and-principles.md) 和[路线图](docs/roadmap.md)。唯一执行队列是 [active plan](docs/plans/2026-07-10-product-replan.md)；历史计划、阶段证据和发布草案不能替代它。
