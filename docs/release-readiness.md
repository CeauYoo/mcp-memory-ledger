# 下一候选版本准备评估

## 结论

更新：2026-10-09。当前能力与精确提交证据统一见[实现状态](project-status.md)，本页只评估发布边界。仓库已经公开。当前分支下一候选版本仍只能按以下口径评估：

- 技术 demo
- research-oriented MVP
- local MCP integration prototype

下一次 tag / candidate 前的执行口径见 [Release Gate](release-gate.md)。该 gate 只覆盖本机 Rust MCP `stdio` technical demo / MVP 的最低核验，不代表 Local Alpha、production autonomy、remote administration、multi-tenant deployment 或 background daemon readiness。

[正式化改进与主线同步计划](formalization-improvement-plan-2026-08-25.md)集中记录从当前 technical MVP 到可交付 Local Product Alpha 仍缺少的产品、数据、安全、发布、平台与 GitHub 治理条件。它不替代 release gate，也不把计划项升级为完成证据。

产品化阶段的发布工程规则见 [Release Engineering](product/release-engineering.md)。第一阶段 release artifact 采用 source-only tag 或同等保守的源码形态；不要把它描述为安装包、托管服务、Beta、GA 或 production-ready 交付。

不建议定位为：

- 生产级产品
- 完整 self-agent memory system
- 已接入真实模型的成熟决策引擎

## 公开仓库口径

当前公开仓库应继续维持以下口径：

- 它明确承认这是一个 MVP / demo
- 它没有把最小 provider 集成包装成完整产品能力
- 它保留了原始设计讨论和实现复核资料
- 它说明了本仓库是在 OpenAI Codex 的协作式开发与讨论流程中逐步形成的

这类表述对公开仓库是有帮助的，因为它能让读者同时看到：

- 当前成果
- 当前边界
- 开发方法

## 为什么当前 source-only snapshot 可以继续公开

### 1. 工程闭环已经存在

当前仓库已经打通：

- 事件写入
- 命题持久化
- 自我快照构建
- 最小决策门控
- 反思修订

### 2. 本机接入路径已经存在

当前已有：

- `doctor`
- `serve`
- 本机 MCP `stdio` 服务入口
- Codex MCP 配置样例

### 3. 自动化验证已经存在

以下是早期门禁建立记录；当前数量与精确提交 CI 以状态页为准：

- `2026-07-10` 已建立 `fast` / `core` / `full` 三级验证；发布前使用 `./scripts/test-tier.sh full`
- `./scripts/status-sync-check.sh` 通过，active plan 与 reality gate 一致且根目录 SQLite fixture 未回流
- `cargo clippy --all-targets --all-features -- -D warnings` 通过，当前静态质量基线无 warning
- `doctor` 默认是 no-write inspection；只有显式 `init`、`migrate` 或 `doctor --allow-bootstrap` 可以改变 SQLite，`serve` 只接受 current database
- `cargo test --features release-tools --test non_mvp_product_tracks -v` 覆盖 release evidence index、provider certification preflight 和 packaging preflight 的本地只读 / preflight 边界；richer memory semantics projection 保留在默认 `product_completion_read_models` 核心回归中

### 4. 当前边界已经能被文档清楚说明

这是能否上 GitHub 的关键点。当前仓库虽然仍是 MVP，但它的边界并不是模糊的：

- `openai-compatible` provider 已接入
- provider 配置走本地 TOML 文件
- 丰富 Episode 与语义/流程候选已有有界运行时；它们不执行步骤或授予权限
- 默认数据库作用域已明确为“本机用户共享默认库，隔离靠显式 `database_url`”

只要这些边界在 README 和说明文档里写清楚，这个仓库就适合发布为协作型 demo。

## 当前不适合过度承诺的点

### 1. `decide_with_snapshot`

- application 会在 provider 调用前用当前服务端 commitment store 覆盖调用方 snapshot commitments，并同时 gate requested action 与 provider-selected action
- 其余 snapshot 字段仍由 caller 提供，且没有 server-created snapshot handle、完整 provenance binding 或 policy arbitration，因此不能把 envelope 解释为完整可信策略批准
- 已可走 `openai-compatible` 或 OpenRouter provider；配置示例和本地 stub 不是 live evidence，显式 `--live` runner 只生成 bounded live preflight evidence，不是 provider 质量、SLA 或 gateway 认证
- 返回契约仍是最小动作字符串；允许结果显式标记为 `experimental_non_authoritative` 和 `server_commitment_gate_only`

因此不应把它写成“完整 AI 决策引擎”。

### 2. memory 语义仍然是 MVP

当前已完成四类 scoped search/lookup、跨类型 union、Claim-linked reflection history、scoped identity/commitment audit、evidence-relation runtime 与 scoped Claim supersede 首片，但仍没有完整实现：

- 真实用户 MCP 客户端 recall/correction 与 fresh-machine 发布证据（structural readback 已实现）
- 完整 versioned identity/commitment ledger 与 rollback（安全归属 record-only Reflection history 已实现）
- Event/Episode/Reflection 通用纠错与破坏性生命周期政策
- 流程自动执行/授权；当前 procedural candidate 只是可审核、可版本化、可召回文本

### 3. 数据隔离策略已有最小可发布结论

SQLite 落盘已经可用，默认路径语义也已收口为“本机用户共享默认库”。剩余注意点不在于语义不清，而在于正式数据、测试数据和实验数据仍应通过显式 `database_url` 主动隔离。

## 建议的发布口径

如果你要在 GitHub 上写一句简短介绍，建议使用这种口径：

“A Rust-based local MCP `stdio` memory demo for AI clients. It validates a minimal loop around interaction ingestion, self-snapshot construction, gated decisions, and reflection, backed by SQLite persistence.”

## 发布前建议检查项

### 最低必做

- 按 [Release Gate](release-gate.md) 跑完整发布 gate（minimum gate、self-revision 证据 gate、dashboard gate），并单独记录 sandbox-only failure 与代码失败的区别
- 按 [Release Engineering](product/release-engineering.md) 记录 release evidence directory、version naming、changelog、compatibility matrix、soak 证据和 deprecation 状态
- 若候选变更涉及 runtime、persistence、dashboard、daemon、provider 或 MCP 行为，运行 `./scripts/release-soak-local.sh <candidate-name> [config_path]` 生成本机候选证据；该脚本不生成真实 fresh-machine、Windows runner、remote/team、安装包、tag 或发布认证证据
- 若候选变更涉及 release evidence、provider certification 或 packaging，运行 `./scripts/release-evidence-index.sh <candidate-name>`、`./scripts/provider-certification-check.sh [config_path]`、`./scripts/packaging-preflight-check.sh <candidate-name>` 和 `cargo test --features release-tools --test non_mvp_product_tracks -v`；如果已有候选 archive，再运行 `./scripts/packaging-archive-evidence.sh <candidate-name>` 生成 checksum manifest 后重跑 packaging preflight；这些 preflight 只读取本地 evidence/config shape，不认证 live provider、不创建安装包、不上传文件；stub/simulated provider evidence、provider evidence 占位文件、URL path secret、零字节或部分 packaging archive、缺失 checksum manifest 或 manifest mismatch 都不能被当作通过证据
- 确认 README、状态文档、路线图、三语说明都已更新
- 确认接入命令与验证命令可以直接复制使用
- 确认对“已实现 / 部分实现 / 未实现”的边界没有过度承诺
- 确认 remote write/admin claims blocked：公开说明不得暗示远程写管理、生产级自治、多租户、Beta、GA 或 production-ready 已实现

### 如果准备公开仓库

- 已补齐 `Apache-2.0` `LICENSE` 与 `NOTICE`
- 确认 `CONTRIBUTING.md` 与 README 链接可直接使用
- 确认公开说明里不会误导读者认为它已经是完整产品

### 如果只是团队私有协作

- 可以先不补开源协作文档
- 重点保证 README、文档索引和验证说明足够清楚
