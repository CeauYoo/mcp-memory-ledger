# Local Product Alpha PRD

## 1. 产品定位

MCP Memory Ledger 的 Local Product Alpha 是 validated local MVP 之后的第一阶段产品化目标。`agent_llm_mm` 只是当前 Rust crate、binary 和配置兼容标识。

当前仓库的 MVP release gate 已经通过，项目可以表述为“已验证本地 MVP，进入正式产品化路线”。但 Local Product Alpha 仍需按本 PRD 和后续 product alpha gate 完成；在该 gate 通过前，不能声明 GA、生产级完整自治、远程团队服务或完整 self-governance agent。

自 `2026-07-10` 起，具体执行顺序以 [全新项目规划](../plans/README.md) 为准：先完成 Truth and Safety 与 Trustworthy Recall，再进入本 PRD 的真实 Local Alpha evidence gate。

Local Product Alpha 的目标是让一个真实本机用户能够安装、配置、运行、检查、备份并排查这个本地 MCP `stdio` memory 服务，同时继续保持 `run_reflection` 作为 identity / commitments 的唯一 durable write path。

## 2. 目标用户

- 本机 AI 客户端用户：使用 Codex 类本地客户端，希望接入一个可持久化的 MCP memory 服务。
- Agent 开发者：需要用稳定的本地环境验证 event、snapshot、decision、reflection 和 automatic self-revision MVP 的证据链。
- 项目维护者：需要明确配置、数据隔离、dashboard、provider、验证命令和故障排查边界，避免把 demo 能力误表述成正式自治产品。

## 3. 产品承诺

Local Product Alpha 承诺提供：

- persistent local memory：以 SQLite 持久化事件、claims、evidence、reflection audit 和 trigger ledger 等核心数据。
- controlled self-revision：automatic self-revision 只在已接线的本地 MCP runtime hooks 内 best-effort 运行，并且通过治理后仍复用 `run_reflection` 写入。
- local observability：本机只读 dashboard、`doctor` 预检和 demo artifacts 能帮助用户检查服务状态与 self-revision 证据链。
- explicit configuration：provider、database、dashboard 和后续 daemon 等能力通过本地配置显式控制。
- conservative product wording：准确区分已通过的 MVP gate、正在建设的 Local Product Alpha、后续 beta / remote / GA 阶段。

Local Product Alpha 不承诺生产级高可用、远程团队管理、无人值守完整自治或多租户隔离。

## 4. 范围内

- 一个清晰的本机安装 / 启动 / 预检路径，优先覆盖 macOS，并保留 Windows parity 检查口径。
- 可理解的本地配置入口，说明 provider、SQLite `database_url`、dashboard 和 daemon 默认边界。
- 正式数据、测试数据和 demo 数据的数据库隔离建议。
- 本机只读 dashboard 的启动与检查说明，默认不提供写操作。
- 至少一个真实 provider 的就绪路径说明，继续保留 mock / deterministic demo 的验证用途。
- `doctor` 输出能够解释 config、provider、dashboard、database 和 runtime hook 状态，且不泄露 secrets。
- self-revision demo package 继续作为当前 MVP 边界内的可重复证据链。
- Local Alpha exit gate 明确要求安装、配置、运行、检查、备份 / 恢复说明和验证命令通过。

## 5. 明确非目标

以下内容是 Local Product Alpha 的 explicit non-goals，不应在本阶段对外承诺：

- remote write admin：不提供远程写管理面；任何未来 remote write admin 都必须先具备 auth、authorization、audit、rollback 和测试。
- multi-tenancy：不声明多用户 / 多租户能力；namespace、database、dashboard、operation log 和 config 隔离通过测试前不做该承诺。
- production self-governance：不声明生产级完整 self-governance；当前仍是受限、保守、局部接线的 automatic self-revision MVP。
- all-entry auto-reflection：不把 automatic self-revision 扩展为所有 MCP entrypoint 或所有请求的统一自动反思。
- background autonomy：不默认启动后台 daemon、定时调度器或无人值守自治循环；后续 daemon 必须 disabled by default，并先以 observe-only gate 推进。
- durable write path replacement：不绕过或替换 `run_reflection` 作为 identity / commitments 的 durable write path。
- GA / production-ready claims：不声明 GA、生产级可用、完整安全边界或正式团队服务。

## 6. First-Run Contract

A Local Alpha user can clone or unpack the repository, create one local config from the safe dev example, run explicit `init`, verify with `doctor --read-only`, and start `serve` without editing source files. Old databases use explicit `migrate`; `serve` never bootstraps them implicitly. Users must be able to validate lifecycle status, provider shape, database path, dashboard status, daemon status, and runtime hooks before starting the MCP service.

`bootstrap-local` is a local config helper, not an installer or production bootstrapper. It must not create secrets, overwrite an existing config, run `doctor`, start `serve`, enable daemon behavior, or imply that Local Alpha, Beta, remote/team mode, multi-tenancy, GA, or production-ready status is complete.

Wrapper scripts resolve relative `bootstrap-local` targets from the repository root, matching the existing wrapper behavior of entering the project root before handling modes. Platform docs should recommend absolute target paths when the script is invoked from another working directory.

## 7. 用户工作流

### 7.1 首次本机启动

1. 用户阅读 `README.md` 和对应平台开发文档。
2. 用户运行 `bootstrap-local` 生成本机配置，或手工复制一个 example profile，显式选择 provider 和 SQLite `database_url`。
3. 用户运行 `./scripts/agent-llm-mm.sh doctor` 检查配置、provider、database、dashboard 和 runtime hook 状态。
4. 用户运行 `./scripts/agent-llm-mm.sh serve`，把服务作为本机 MCP `stdio` 子进程接入 AI 客户端。

### 7.2 验证 self-revision MVP 证据链

1. 用户运行 `./scripts/run-self-revision-demo.sh`。
2. 用户查看 `target/reports/self-revision-demo/latest/` 下的 `doctor.json`、snapshot before / after、decision before / after、timeline、SQLite summary 和 report。
3. 用户确认 demo 只证明当前 MVP 边界，不代表 daemon、remote admin 或完整自治已经实现。

### 7.3 本机观测

1. 用户仅在需要时通过配置启用 `[dashboard]`。
2. dashboard 绑定本机地址并保持 read-only。
3. 用户通过 dashboard 检查 MCP tool 调用、runtime operation 和 auto-reflection 事件，不通过 dashboard 执行写操作。

### 7.4 数据安全

1. 用户为正式数据、手工测试数据和 demo 数据配置不同 SQLite 文件。
2. 用户在升级、迁移或实验前备份 SQLite 数据库。
3. 用户恢复时优先恢复到新路径，再切换 `database_url` 验证。

### 7.5 故障排查

1. 用户先运行 `doctor` 判断 provider、database、dashboard 和 runtime hook 状态。
2. 用户复核平台文档和 release / product gate 文档中的验证命令。
3. 维护者使用 demo artifacts、operation evidence 和 git diff 来判断是否是配置问题、provider 问题、数据问题或实现回归。

## 8. Alpha Exit Gate

Local Product Alpha 只有在以下条件全部满足后，才可以从“validated local MVP entering productization”升级为“local product alpha”：

- fresh machine setup 可以只按文档完成本机安装、配置、`doctor` 和 `serve`。
- macOS 主路径可用，Windows 文档至少明确 parity 状态和差异。
- `doctor` 能解释 config、provider、dashboard、daemon、database 和 runtime hooks 状态，并保持 secret redaction。
- 正式数据、测试数据和 demo 数据的 `database_url` 隔离规则已写清楚。
- SQLite backup / restore / export runbook 已存在，并建议先恢复到新路径。
- 本机 dashboard 默认只读，不提供 remote write admin。
- daemon 如果出现，默认 disabled；任何自动行为先通过 observe-only gate。
- self-revision demo artifacts 可从当前安装路径重新生成，并能证明 before / after decision shift。
- 产品文档没有声明 remote write admin、multi-tenancy、production self-governance、all-entry auto-reflection、GA 或 production-ready。

## 9. 验收命令

Task A1 的文档验收命令：

```bash
rg -n 'Local Product Alpha|non-goals|remote write|multi-tenancy|self-governance' docs/product/prd-local-alpha.md README.md docs/document-map.md
git diff --check
```

Local Product Alpha 后续完整 gate 应在 `docs/product/release-gate-local-alpha.md` 中单独定义；在该文档落地前，本 PRD 不替代 release gate。

## 10. 文档入口

- 仓库首页：[`README.md`](../../README.md)
- 项目起点：[`docs/origin-and-principles.md`](../origin-and-principles.md)
- 文档总览：[`docs/document-map.md`](../document-map.md)
- 当前实现状态：[`docs/project-status.md`](../project-status.md)
- 当前路线图：[`docs/roadmap.md`](../roadmap.md)
- 唯一 active plan：[`docs/plans/README.md`](../plans/README.md)
