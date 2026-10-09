## 2026-10-09 原方案续建（进行中）

用户明确要求按原始讨论继续规划与实现，完成“基本可用性候选”不是本轮停止条件。[完整原文要求追踪矩阵](2026-10-09-original-plan-traceability.md)冻结于 e14a59e，逐项区分已有实现与缺口；它是本执行队列的验收依据，不是第二套产品架构。

本轮有限交付顺序为：A 固定反馈/状态合同和离线 A/B/C + 消融样本；B 延续原子审计与幂等，并增加内容指纹 stale-version 校验；C 外部反馈→候选→确定性支持合同→原子提交→召回；D SQLite FTS 可重建派生索引、短中文精确 fallback、类型预算/解释及 10k/100k 测量；E 持久化有界 Episode 与可检查、拒绝、版本化、回滚的语义/流程候选。新层必须随 schema 显式迁移、恢复测试和同提交文档交付。候选激活仅影响记忆可见性，不改变行为权限。

依赖调整：已完成的生命周期和有界本地安装闭环允许提前实现这组本地检索/经验运行时切片，以验证原方案的反馈闭环；M2 的真实 fresh-machine、正式打包、完整 Windows shell-wrapper 门，以及 M3 的广泛真实模型收益门仍保持开放。不得将这次前移解释成正式 Local Alpha/生产认证。原路线中的数据模型进一步规范化、大模块重构等要求必须在追踪矩阵保持 partial，不能因本批次范围有限而消失。

验证分两层：无外部模型调用的固定离线代理指标、10k/100k 容量与事务行为可在本轮运行；同模型同任务的真实成功率/token 成本仅提供记录协议，未获模型服务与费用授权前不调用或声称通过。embedding、远程团队平台、强化学习、自治控制器仍按原文研究边界后移。

---

# MCP Memory Ledger 全新项目规划

状态：`active / M0 complete / M1 active / M1.0.1, M1.0.2, M1.0.3, M1.1.1, M1.1.2, M1.1.3, M1.1.4, M1.1.5, M1.1.6, M1.2.1, M1.2.2, M1.2.3, M1.2.4, M1.2.5, M1.2.6 and M1.2.7 complete`
规划日期：`2026-07-10`
规划输入基线：`dev-work@6fcbb5f`
主线整理基线：`1f7390d`
写入边界：当前按 active milestone 领取独立最小切片；完成单个切片不代表完整里程碑、Local Alpha 或发布 gate 已通过。

## 1. 总体决策

这次规划不是推倒重写，而是重新排序。

MCP Memory Ledger 已经拥有可运行的 Rust + SQLite + MCP `stdio` 核心、受治理的 reflection 写路径和大量测试资产。当前问题不是“功能太少”，而是产品价值、运行时事实和发布工程之间失衡：写入与治理工具已经很多，但可信的按 scope 读取、检索、解释、纠错和恢复仍不完整。

新的主线是：

> 先把项目做成一个可信、可检索、可审计、可恢复的本地 Agent Memory Ledger，再讨论自治、远程和团队能力。

因此做出以下决策：

1. 公共名称继续使用 **MCP Memory Ledger**；`agent_llm_mm` / `agent-llm-mm` 只保留为实现兼容标识。
2. 第一目标用户是单机开发者和本地 AI 客户端用户，不是团队管理员或自主 Agent 平台运营者。
3. 第一产品闭环是“记录 → 检索 → 查看证据 → 纠错/撤销 → 备份恢复”，不是“自动决策 → 后台自治”。
4. MCP `stdio` 保持主传输。远程 HTTP、认证和团队模式不进入近期承诺。
5. `decide_with_snapshot`、automatic self-revision、daemon 和多层记忆投影在可信读取闭环完成前统一按实验能力管理。
6. 发布证据工具只服务于产品能力，不再作为独立主线继续扩张。
7. 所有里程碑按“全局建模、单点验证、最小执行、证据通过后扩张”推进。

## 2. 事实基线

### 2.1 已实现

- 单一 Rust crate，正式 CLI 为 `serve` 与 `doctor`。
- MCP `stdio` 运行时暴露 12 个工具（新增 `recall_memory`、`build_task_context`）：`ingest_interaction`、`search_memory`、`get_memory`、`get_reflection_history`、`get_self_model_history`、`get_evidence_relation`、`supersede_memory`、`build_self_snapshot`、`decide_with_snapshot`、`run_reflection`。其中 `search_memory` 已覆盖显式 scope 的 Event、Claim、Episode 与 Reflection provenance 首片，`get_memory` 已覆盖 Event、Claim、Episode 与 scoped Reflection 首片，`get_reflection_history` 已覆盖 exact scoped Claim revision chain 首片，`get_self_model_history` 已覆盖 scoped identity/commitment revision audit 首片，`get_evidence_relation` 已覆盖 scoped evidence-relation runtime 首片，`supersede_memory` 已覆盖 scoped Claim audited supersede 首片。
- SQLite 持久化 events、claims、evidence links、episode events、reflections、trigger ledger、identity、commitments 和 operation log。
- ingest 与 reflection 具备事务边界；`run_reflection` 是当前 identity / commitments 的唯一 durable write path。
- mock、OpenAI-compatible 和 OpenRouter provider 路径存在。
- 本地 dashboard、support bundle、backup / restore、release preflight 和多类状态报告存在。
- 测试已分为 `fast` / `core` / `full` 三级；发布证据、打包与 provider certification 工具由非默认 `release-tools` feature 承载。

### 2.2 2026-07-10 审计阻断事实与当前状态

| 编号 | 当前事实 | 风险 | 规划结论 |
| --- | --- | --- | --- |
| F-01 | M0.2 已为显式 snapshot 建立 namespace / manifest / time-window scope 与稳定排序；省略 namespace 的 legacy 调用仍保持 unscoped 兼容 | legacy 路径仍可能读取过宽，完整 recall contract 尚未建立 | M0.2 限定退出门已通过；剩余边界继续保持公开 |
| F-02 | M0.3 已用服务端 commitments 覆盖 caller commitments、复检 requested / provider-selected action，并把允许结果标为 experimental non-authoritative；其他 snapshot 字段仍由 caller 提供 | 尚无完整 trusted snapshot handle 或结构化 policy arbitration | M0.3 限定退出门已通过；剩余能力进入后续独立切片 |
| F-03 | M0.3 已用 claim → evidence → episode distinct join 替代全局数量推断，并覆盖 governance transaction failure atomicity | 现有 join 仍不是完整 provenance graph，事务证据也不是 crash recovery | M0.3 限定退出门已通过；完整 provenance / recovery 继续保持公开边界 |
| F-04 | M0.4 已拆分显式 init / migrate / bootstrap permission；默认 doctor 只读，serve current-only | remote backup / scheduled backup / production DR 仍不属于本地 SQLite 合同 | M0.4 已收口；后续 schema 变更继续复用 ledger / backup / rehearsal / transaction / readback 门 |
| F-05 | M0.4 已为 legacy rebuild 建立 schema version、migration ledger、备份/恢复演练与显式事务 | 本地 SQLite 合同已收口；remote/scheduled/production DR 仍不存在 | 后续 schema 变更必须复用同一迁移与恢复门禁 |
| F-06 | M1.1.1 / M1.1.2 / M1.1.3 / M1.1.4 / M1.1.5 / M1.1.6 已提供 scoped Event / Claim / Episode / Reflection search、scoped evidence-relation runtime 与稳定跨类型 union；M1.2.1 / M1.2.2 / M1.2.4 / M1.2.5 增加 Event/Claim/Episode/Reflection stable-ID lookup；M1.2.3 / M1.2.6 增加 Claim-linked reflection history 与 scoped identity/commitment revision audit 首片；M1.2.7 增加 scoped Claim audited supersede 首片；M1.0.1–M1.0.3 前置门已通过 | 四类 search、union、四类 lookup、Claim revision chain、self-model audit history、evidence-relation runtime 与 scoped supersede 可用，但 versioned identity/commitment ledger 仍缺；record-only Reflection 仍因无 scope 不可读 | 继续按 M1 建设 Read Model v2；M1.3.0 与 M2.0.1 已完成；继续有界可用性候选验证 |
| F-07 | Dashboard 仍无认证，但启用时已拒绝非 loopback host | 本地只读口径已有强制边界；remote dashboard 仍未授权 | M0.5 已收口；保持 loopback-only，认证与 remote 另走独立 gate |
| F-08 | Linux/macOS CI 与 CLI stderr tracing 已建立；真实二进制包尚未建立 | source gate 已持续化，artifact delivery 仍不完整 | M0.5 已收口；M2 补真实包 |
| F-09 | evidence / episode / memory layer projection 主要停留在定义和测试调用 | 测试存在被误读为运行时产品能力 | 未接入前标记 partial / experimental |
| F-10 | 文档有多套并行计划、旧测试总数与命名漂移 | 下一步来源不唯一，状态容易高估 | 本计划成为唯一 active plan |

### 2.3 外部协议校准

- MCP 官方仍把 `stdio` 与 Streamable HTTP 都定义为标准传输，并建议客户端在可行时支持 `stdio`。本项目当前不需要为了“跟上协议”而提前进入远程服务。[MCP Transports](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports)
- MCP Resources 适合暴露由应用控制的上下文数据。M1 可以在稳定查询契约之后评估只读 `memory://` resources，但不应绕过同一 scope 和 provenance 规则。[MCP Server Features](https://modelcontextprotocol.io/specification/2025-06-18/server/index)
- 当前仓库固定 `rmcp = 0.5`；2026-07-14 的隔离 spike 已对照官方 `2.2.0`，确认直接升级仍有 router warning 与 handler error-contract 回归，因此本里程碑 no-go。迁移清单与测试矩阵见 [`rmcp` compatibility spike](../spikes/rmcp-compatibility-2026-07-14.md)，不得顺手扩入 remote/tasks/OAuth。
- MCP Tasks 仍带实验性质，不是当前本地记忆闭环的前置依赖。[MCP Tasks](https://modelcontextprotocol.io/specification/2025-11-25/basic/utilities/tasks)

## 3. 主用户与核心任务

### 3.1 主用户

第一阶段只服务一个明确角色：

> 在本机使用 Codex 类或其他 MCP 客户端，希望 Agent 跨会话保留项目事实、偏好、承诺和更正记录的开发者。

### 3.2 Jobs To Be Done

1. 当客户端会话结束或重启后，我仍能取回之前明确记录的项目事实。
2. 当一条记忆被返回时，我能看到它来自哪个 event、属于哪个 namespace、何时记录、是否已被替换。
3. 当记忆错误或过期时，我能更正、supersede 或撤销，而不是直接丢失历史。
4. 当我切换项目或用户 scope 时，其他 scope 的数据不会混入结果。
5. 当数据库升级或机器迁移时，我能先备份、验证恢复，再切换正式路径。
6. 当 provider 不可用时，确定性的本地记录和检索仍然可用。

### 3.3 产品承诺

Local Alpha 只承诺：

- local-first、single-user、MCP `stdio`；
- scoped persistent memory；
- deterministic recall with provenance；
- auditable correction / supersession；
- explicit init / migrate / backup / restore；
- loopback-only local diagnostics；
- 保守、可验证的产品措辞。

### 3.4 明确非目标

- 远程写管理、团队模式和多租户；
- 完整自治 Agent 或无人值守后台自我修订；
- 完整决策引擎、policy arbitration 或 action executor；
- 所有入口自动反思；
- provider 质量、SLA 或模型能力认证；
- service manager、auto-updater、云同步和生产灾备；
- 把只读 projection、preflight 或文档状态当作已交付产品能力。

## 4. 目标架构

### 4.1 模块职责

| 层 | 目标职责 | 不允许的越界 |
| --- | --- | --- |
| Ledger Core | Event、Claim、Evidence、Episode、Reflection、Identity、Commitment 的不变量 | 不依赖 MCP、HTTP 或 provider |
| Read Model | 按 `MemoryScope` 查询、排序、过滤、provenance join、历史读取 | 不在应用层全库读取后再截断 |
| Governance | conflict、supersession、manual reflection、bounded auto-reflection | 不绕过 evidence validation 或 durable write path |
| Persistence | SQLite schema、versioned migration、index、transaction、retention | 不在 doctor 中隐式迁移正式库 |
| MCP Interface | 兼容旧工具，新增稳定的 memory read contract | 不信任调用方自带 policy / snapshot |
| Local Ops | init、doctor read-only、migrate、backup、restore、support bundle | 不自动发布、不复制原始凭据或数据库 |
| Labs | decision、daemon、memory layering、experimental tasks | 不进入 Local Alpha 默认能力或完成口径 |

### 4.2 核心对象

M0 先冻结以下概念，不立即重做所有表：

- `MemoryScope`：owner + namespace + optional project/root identity。
- `MemoryRecord`：可检索的 event / claim 统一读取视图，保留原始 ID 和类型。
- `Provenance`：claim → evidence event → episode / reflection 的可读取关系。
- `SnapshotHandle`：服务端创建、带 scope / version / evidence manifest 的可信 snapshot 引用。
- `RevisionVersion`：identity / commitment 的版本、effective time、diff 和 rollback target。
- `MigrationVersion`：明确的 schema version、执行记录和恢复状态。

### 4.3 兼容策略

- 保留原有 4 个 MCP 工具，通过 additive `search_memory` / `get_memory` / `get_reflection_history` 和字段扩展读取合同；`search_memory` 省略 `record_type` 时仍保持 Event 语义，不静默改变旧 caller 的成功/失败语义。
- `decide_with_snapshot` 在可信 snapshot handle 完成前明确标为 experimental；如果无法兼容修正，则先新增 v2 工具而不是直接破坏旧 schema。
- 数据迁移必须先支持旧数据库 dry-run、备份和 rollback rehearsal。
- `agent_llm_mm` 技术标识的完整重命名不与产品功能改造捆绑。

## 5. Now / Next / Later

| 阶段 | 目标 | 预计节奏 | 扩张条件 |
| --- | --- | --- | --- |
| **Complete — M0 Truth and Safety Reset** | 修复可信性边界、迁移和诊断语义；建立 CI | 已于 2026-07-14 收口 | M0 所有证据门已通过 |
| **Active — M1 Trustworthy Recall** | 做出真正可用的 scoped read / search / provenance / correction 闭环 | 2–4 周参考节奏 | 真实本地客户端完成核心用户旅程 |
| **Next — M2 Local Product Alpha** | 真实二进制包、fresh-machine、Windows 边界、备份恢复与人工 release decision | 2–3 周参考节奏 | Alpha exit gate 全部有 fresh evidence |
| **Later — M3 Retrieval Quality and Lifecycle** | ranking、retention、versioned revision、runtime projections | 3–5 周参考节奏 | M1/M2 稳定，先冻结评测集 |
| **Not scheduled — M4 Remote and Autonomy** | remote/team、write-capable daemon、full autonomy | 不排期 | 有真实需求且安全前置门全部满足 |

预计节奏只用于排序，不是发布日期承诺。

### 2026-10-09 有界可用性候选执行批次

最新已核验上游基线为 `dev-work@c318caf`，已包含 stdio bootstrap 回归修复；9 月 5 日讨论稿只作为设计输入，不作为已实现或已验证证据。本批次在现有 M1 → M2 主线内收束一个 **local-first technical MVP 可用性候选**，不宣告 Local Alpha 发布。

依赖调整：将 M2.0.1 生命周期排他性与 M1.3.0 一起作为所有 schema 变更的前置门；把 M3 中最小中英文文本召回和有界任务上下文前移为 M1 用户闭环所需的实用性切片。FTS/embedding/ranking 调优、50 场景全面评测仍留在 M3。

| 批次阶段 | 有界交付 | 验收与停止条件 |
| --- | --- | --- |
| 0 基线与范围 | 重拉上游、保留既有修复、更新唯一队列 | 新 head 与差异已核验；独立开发分支、单一草稿 PR；不合并或部署 |
| 1 数据库前置门 | M1.3.0 结构读回 + M2.0.1 安全生命周期 | 同版本弱化约束 fail closed；只读检查不改库；并发 init/migrate 不破坏他人数据；迁移失败保留原库与恢复锚点 |
| 2 可靠更正与重试 | 事务内 scope/evidence/state 校验、单后继更正、持久化幂等及原子写入回执 | active/disputed 可更正，terminal 拒绝；相同 key/payload 返回原结果，不同 payload 冲突；失败无部分 claim/evidence/audit；直接 reflection 同样受约束 |
| 3 外部反馈 provenance | 有界结构化观察，保留来源、目标、预期/实际、验证方法与限制 | 原始观察不可改写；caller/tool/model 声明可区分但不视为认证；跨 scope 证据拒绝；不增加执行权限 |
| 4 本地召回与任务上下文 | additive 文本 recall 与独立有界 context，不改变历史 browse | 北京/咖啡/记忆及英文、字面符号可召回；active claim 不被事件淹没；固定离线小样本 recall@5 ≥ 0.80、provenance 100%、scope 泄漏 0；完整 JSON UTF-8 字节数不超过请求预算 |
| 5 可重现交付候选 | init → MCP 写入 → 重启 → 检索 → evidence → 更正 → history → backup/restore | 实际进程 stdio transcript 与读回证据、full/fmt/Clippy/status-sync、精确远端提交与 CI；平台实测和 fresh-machine 尚未完成项继续明确标注 |

每阶段先 focused 回归，再相应全量门禁；代码、能力边界和测试文档同阶段更新。每个通过阶段发布到 `CeauYoo:dev_work_dots` 并更新指向上游 `dev-work` 的同一草稿 PR。不得用历史 CI、文档或模拟环境替代当前检查，不调用付费/远程模型作默认验收。

本批次结束条件是上述有界本地工作流与声明的平台证据可复现、候选可 review；正式二进制跨平台发布、真实 fresh-machine ≤10 分钟、live-provider 认证、人工 release decision 未取得证据时保持开放。强制 embeddings、云服务、远程/团队、自治控制器、广泛重构和完整 procedural memory 不在本批次范围内。

## 6. M0 — Truth and Safety Reset

M0 已收口，当前只能从 M1 领取一个独立最小切片。新 provider、远程、daemon 写能力或正式发布工作仍未因 M0 完成而获得授权。

### M0.1 统一事实入口和仓库卫生

- [x] **M0.1 Mainline and repository hygiene**

- 已完成（2026-07-10）：把 status-sync 的 active plan 指针从旧 P1/P2/P3 文件迁到本计划。
- 已完成（2026-07-10）：确认测试不依赖后，移除被跟踪的根目录 SQLite fixture，并增加精确忽略规则。
- 已完成（2026-07-10）：让 `status-sync-check` 阻断根目录 SQLite fixture 回流。
- 已完成（2026-07-10）：旧计划、原始日志和阶段记录保存在本地历史分支，不作为当前任务源。

证据门：tracked worktree 不含意外数据库；文档只有一个 active plan；status sync 读取本计划并阻断 fixture 回流。

本机安全后续不属于仓库完成状态：私有配置文件保持 ignored / untracked；其中的非占位 provider credential 应由用户单独轮换。若后续要增加 tracked-file secret scanner，应作为独立安全切片设计，避免扫描或输出本机私有配置内容。

### M0.1.1 测试与工具链减重

- [x] **M0.1.1 Test and toolchain slimming**

- 已完成（2026-07-10）：增加 `scripts/test-tier.sh`，固定 `fast` / `core` / `full` 三级入口。
- 已完成（2026-07-10）：把 Local Alpha evidence、release decision、product readiness、provider certification 和 packaging 模块、二进制与测试移入非默认 `release-tools` feature；资产仍由 `full` 层完整覆盖。
- 已完成（2026-07-10）：把 richer memory semantics projection 的核心用例从发布测试迁回默认 `product_completion_read_models` 回归。
- 已完成（2026-07-10）：关闭 13 个无内部单元测试的 bin test harness；真实二进制 E2E 仍由集成测试覆盖。
- 已完成（2026-07-10）：把 `status-sync-check` 收敛为纯 active plan / reality gate 检查，同时保留 wrapper 的根目录 SQLite fixture 门；没有完成态 checkbox 时 fail closed，不再通过编译整套测试来核对精确数量。
- 已完成（2026-07-10）：移除 README、状态文档和测试指南中的精确测试总数 / suite count 复制口径。

证据门：默认 `cargo check`、`fast`、`core`、`status-sync` 通过；`full` 与 all-feature Clippy 证明归档到非默认 feature 的测试和工具仍可用。

### M0.2 Scoped Snapshot v2

首个最小切片（2026-07-11）已实现：`build_self_snapshot` 增加 additive `namespace`，由 server 推导 owner 并生成 `MemoryScope`，显式 scope 的 claims / event references / episode references 在 query port / SQLite 层过滤；`MemoryScope` 反序列化只接受完整匹配 scope 或全空 legacy 状态；省略 namespace 保留 legacy unscoped 兼容行为。该切片落地时 M0.2 仍保持开放，直到后续 time / manifest、ID、完整排序与 auto-reflection 边界一起满足。

第二个最小切片（2026-07-11）已实现：`build_self_snapshot` 增加 additive `evidence_manifest`；显式 manifest（包括空数组）必须同时显式提供 `namespace`，最多 256 项，并在 DTO 中保序线性去重；DTO、application 与 store 都不能进入 legacy unscoped 兼容路径或绕过数量上限，且 snapshot 参数在 optional auto-reflection 之前完成校验。裸 event ID 与 `event:<id>` 在 snapshot 输入中统一为 canonical reference，SQLite 以 manifest event IDs 与 server-owned owner/namespace 同时过滤。越 scope ID、显式空 manifest 与空交集均不会扩大查询；tool operation log 使用 snapshot namespace。该切片当时未覆盖 time window、其他入口的全局 ID 统一、完整排序和 scoped auto-reflection；其中 time window / 排序由下一段的第三个切片补齐。

第三个最小切片（2026-07-11）已实现：`build_self_snapshot` 增加 additive `recorded_after` / `recorded_before` 闭区间；任一时间边界都要求显式 `namespace`，RFC3339 输入归一到 UTC，倒置窗口会在 optional auto-reflection 前被 DTO / application 拒绝。evidence 在 SQLite 中按 owner/namespace ∩ manifest（如有）∩ time window 查询；SQL 会把项目 canonical timestamp 与旧库常见的 `Z` / offset RFC3339 文本归一成固定宽度 UTC 排序键，并保留 9 位小数秒，再以 `rowid DESC` 收口稳定顺序。episode 先在相同 scope + window 中选择每个 episode 最新的合格 `(recorded_at, event rowid)`，再以该元组和 `episode_reference` 形成全序。显式窗口的空交集保持为空；claims 因无 recorded timestamp 仍只按 scope 收窄。legacy unbounded snapshot 仍兼容，但 SQLite snapshot 读取已改为 recent-first。该切片落地时，scoped auto-reflection 仍是 M0.2 的开放项。

第四个最小切片（2026-07-12）已实现：automatic reflection 不再借用 legacy unscoped snapshot。触发 namespace 先派生为完整 `MemoryScope`，候选 event ID 固定为 explicit manifest，并读取该 scope ∩ manifest 的记录时间形成闭区间；同一 scope/window 也传入 episode read。没有合格候选时该路径保持 not-triggered，不会扩张到历史全量。显式 MCP `build_self_snapshot` 的兼容调用不变；M0.2 仍不包含 snapshot 外的全局 ID 统一。

第五个最小切片（2026-07-12）已实现：active reflection runtime 的 MCP `replacement_evidence_event_ids`、application explicit/query 合并和 auto-reflection model proposal/governed evidence 都接受裸 event ID 与 `event:<id>`。它们经 `EventReference` 解析，拒绝空白/空 ID/重复前缀，并以底层 raw ID 保序去重；SQLite 查询、事件存在性校验、evidence link 与明确的 audit/diagnostic `*_event_ids` 继续使用 raw ID，未做表迁移。此切片只覆盖 active runtime；episode projection、离线 demo/support bundle 和其他 repository-wide ID 表面仍保持 partial inventory。

第六个最小切片（2026-07-13）已实现：只读 evidence relation 的 `trigger_window_event_ids` / `selected_evidence_event_ids` 与 episode summary 的 `episode_event_ids` / `linked_evidence_ids` 都接受裸 event ID 与 `event:<id>`。它们先经 `EventReference` 解析为 raw ID 并保序去重，随后执行 subset/no-widening、count 与 window-rank；空白、空 ID、重复前缀 fail closed。字段继续按既有 `*_event_ids` / `event_id` 契约输出 raw IDs，不增加持久化、MCP tool 或 runtime read path。离线 demo、support bundle 和其他 repository-wide ID 表面仍未修改。

第七个最小切片（2026-07-13）已实现：盘点 offline self-revision demo runner、内置/独立 demo stub、tests 与生成 artifacts 后，确认 runner 没有 caller-provided event-ID 输入；snapshot `evidence` 已是 canonical reference；stub 只返回空 `proposed_evidence_event_ids` 与受控 query；SQLite summary 的 `supporting_evidence_event_ids` 是明确 raw 兼容字段。唯一需收口的外部边界是 `timeline.json` baseline event，现经 `EventReference` fail-closed 解析并输出 canonical `event_reference`，不再把 raw MCP `event_id` 直接作为 artifact 引用。未改 SQLite schema、active runtime/projection、发布证据或 provider 调用；support bundle 只记录为后续 inventory。

收口结论（2026-07-14）：snapshot scope、snapshot query 的 scope/manifest/time 交集、snapshot 内 canonical event reference、SQLite recent-first 稳定排序、scoped auto-reflection snapshot、active reflection runtime event-ID 等价性、read-only evidence/episode projection event-ID 等价性，以及 offline demo artifact event reference 均已有实现与回归证据，M0.2 限定退出门完成。repository-wide event-ID 统一仍为 `partial`，不再作为这个 scoped-snapshot 里程碑的无限扩张条件。

- [x] **M0.2 Scoped Snapshot v2**

- 已完成：snapshot 输入使用服务端派生的 `MemoryScope`。
- 已完成：store/query port 层按 namespace、owner、时间和显式 evidence manifest 取交集，空交集不扩大。
- 已完成：M0.2 涉及的 active reflection runtime、只读 evidence/episode projections 与 offline demo artifact 接受并规范化裸 event ID / `event:<id>`；明确 raw-ID 兼容字段保持原契约。
- 已完成：recent-first 排序在 SQL 层完成，并使用 `(recorded_at, rowid/id)` 稳定收口。
- 已完成：auto-reflection snapshot 仅使用当前 trigger window 和允许的 scope 关系。

证据门：至少覆盖 self / world / 两个 project / 两个 user namespace；跨 scope 注入为 0；相同输入产生相同顺序。

### M0.3 Governance Correctness

首个最小切片（2026-07-14）已实现：`decide_with_snapshot` 在任何 gate 或 provider 调用前读取当前 `CommitmentStore`，以服务端 commitment descriptions 覆盖 caller snapshot commitments；caller 删除或注入 commitment 都不能改变服务端 policy context。requested action 先 gate，provider-selected action 返回后再经过同一 gate；selected action 被拒绝时结果为 `blocked`、`decision = null`，并以 `commitment_gate_blocked_selected_action` 和 `bounded-local-policy-rejected` 暴露有界解释。现有 input schema 与 `ModelDecision { action }` provider contract 不变；identity / claims / evidence / episodes 仍是 caller-provided，M0.3 整体保持开放。

第二个最小切片（2026-07-14）已实现：identity auto-reflection 不再用 `min(global episode count, supporting claim count)` 推断跨 episode 支持。application 将与 proposed identity value 匹配的 active claim IDs 交给只读 `EpisodeStore` provenance port；SQLite 通过现有 `evidence_links` 与 `episode_events` 做 distinct join，只返回真实可达的 supporting episodes。无关 episode、无 evidence link 的 supporting claim 与空 claim 集不计数；非 SQLite store 对非空 provenance 查询默认 fail closed。未增加表、迁移、MCP schema 或新 write path。

第三个最小切片（2026-07-14）已完成证据收口：生产写路径不需调整；`auto_reflect_if_needed` 的 validation rejection 在 transaction 前落 rejected audit，`run_reflection` 将 identity、commitments、claim/evidence、reflection 与 handled trigger ledger 放入同一 reflection transaction。新增组合 identity + commitment commit-failure 注入、handled-ledger append failure 全状态断言，以及真实 SQLite duplicate-ledger failure 回归，证明失败事务不留下部分更新，事务外最终只有 rejected trigger audit。此结论不扩张为 crash recovery 或 distributed transaction 保证。

第四个最小切片（2026-07-14）已实现：v2 response envelope 在不改变 input schema、legacy `blocked` / `decision` / `status` 字段或 `ModelDecision { action }` provider contract 的前提下，新增 `decision_authority` 与 `policy_scope`。blocked 路径标记为 `not_applicable_blocked`；允许的 action-string 结果明确标记为 `experimental_non_authoritative`，policy scope 固定为 `server_commitment_gate_only`。因此 `gate.blocked = false` 只表示当前服务端 commitment literal gate 未阻断，不能再被解释成完整 policy passed；MCP tool description、operation summary、non-claims 与协议文档使用相同边界。

收口结论（2026-07-14）：服务端 commitment 绑定与 requested/selected dual gate、真实 claim → evidence → episode 支持、local transaction failure atomicity，以及 action-string decision authority 边界均已有实现与回归证据，M0.3 的限定退出门完成。`decide_with_snapshot` 仍是 experimental technical-MVP path；caller-provided identity / claims / evidence / episodes、缺少 server-created snapshot handle、完整 provenance graph、structured action validation 与 policy arbitration 均继续保持开放边界。

- [x] **M0.3 Governance Correctness**

- [x] **M0.3.1 Trusted decision commitments and dual gate**
- [x] **M0.3.2 Claim evidence episode provenance**
- [x] **M0.3.3 Governance failure atomicity**
- [x] **M0.3.4 Experimental non-authoritative decision result**

- 已完成：`decide_with_snapshot` 使用服务端 commitments 作为当前 policy context。
- 已完成：requested action 与 provider-selected action 都经过同一 commitment gate。
- 已完成：不可结构化验证的 action-string 结果返回 `experimental_non_authoritative`，并将 policy scope 限定为 `server_commitment_gate_only`，不标记完整 policy passed。
- 已完成：用 claim → evidence → episode 的真实 distinct join 计算跨 episode 支持。
- 已完成：治理失败只产生 rejected audit，不留下部分 identity / commitment / reflection 更新。

证据门：伪造 caller snapshot 不能移除 baseline commitment；provider 返回受禁 action 必须被阻断；无关 episode 不计入支持数；允许的 provider action-string 必须显式返回 experimental / non-authoritative authority 与 bounded policy scope。

### M0.4 Explicit Database Lifecycle

收口结论（2026-07-14）：CLI 已拆分 `init`、`migrate`、默认只读 `doctor`、显式 `doctor --allow-bootstrap`；`serve` 只打开 current database。SQLite 使用 `PRAGMA user_version` 与 `schema_migrations` ledger，旧库 migration 在原库写入前创建 backup anchor 并对备份副本完成 restore rehearsal，随后在单一事务内执行 rebuild、default seed、row-count preservation、`foreign_key_check` 和 ledger readback。missing / old / read-only 数据库的只读检查不创建、迁移或 seed；实际 release soak 已把 init / doctor / product smoke / support bundle 绑定到 candidate-isolated database，并明确拒绝把未确认正式库路径作为 soak 写目标。此结论不扩张为 remote backup、scheduled backup、cloud sync 或 production disaster recovery。

- [x] **M0.4 Explicit Database Lifecycle**

- 已完成：拆分 `init`、`migrate`、`doctor --read-only` 和显式 `doctor --allow-bootstrap`。
- 已完成：`doctor --read-only` 对不存在数据库、旧 schema 和不可写路径只报告，不 create / migrate / seed。
- 已完成：建立 schema version 与 migration ledger。
- 已完成：legacy rebuild 在事务和备份锚点下执行；失败后原库可恢复。
- 已完成：每次迁移执行 `foreign_key_check`、表/行数 readback 和恢复演练。
- 已完成：release soak 强制使用隔离数据库，不接受未确认的正式库路径。

证据门：doctor 前后数据库 checksum / schema / row count 不变；故障注入后恢复成功；旧库 roundtrip 无数据丢失。

### M0.5 Runtime Boundary and CI

- [x] **M0.5 Runtime Boundary and CI**

- 已完成（2026-07-14）：无认证阶段启用 dashboard 时拒绝非 loopback host；disabled 配置不会启动暴露面。
- 已完成（2026-07-14）：CLI 在参数解析后记录 command-level tracing，所有 tracing 固定写入 stderr；真实 JSON stdout 与 MCP `stdio` 不混入日志。
- 已完成（2026-07-10）：修复测试辅助代码的 `clippy::collapsible_if`，恢复当前 toolchain 的零 warning 基线。
- 已完成（2026-07-14）：增加 GitHub Actions，Linux / macOS 均执行 format、all-feature Clippy、full test tier 与 status sync；Windows 保持在 M2 完整 parity gate。
- 已完成（2026-07-14）：`rust-toolchain.toml` 固定 Rust `1.95.0`，`Cargo.toml` 声明 `rust-version = "1.95"`；当前支持下限只承诺该已验证版本。
- 已完成（2026-07-14）：对 `rmcp 0.5.0` → `2.2.0` 完成隔离兼容 spike；机械修复后 compile 通过，但 `stdio` 回归为 47/48 且有 router warning，因此结论为 M0.5 不直接升级，后续只允许独立迁移切片。

证据门：CI workflow 在 clean clone 执行固定门禁；本机同命令通过；stdout 只有 MCP/命令 JSON；dashboard loopback 规则有正反测试；SDK spike 有 no-go 结论与独立测试矩阵。

## 7. M1 — Trustworthy Recall

### M1.0 Scope and Data-Integrity Gates

2026-08-09 的只读复核确认：M1.1.3 新增的 Episode 查询自身已使用 scope-first projection，可以保持 completed；但现有写侧和其他读侧仍有三项 scope / data-integrity 缺口。2026-08-13 已完成 M1.0.1、M1.0.2 与 M1.0.3。后续可按原冻结顺序领取 M1.1.4 之后的 Reflection、Evidence Relation、Stable Union、lookup/history 或 correction feature slice。

- [x] **M1.0.1 Scoped Identity Evidence-to-Episode Gate**

- 已完成（2026-08-13）：`list_episode_references_supporting_claims` 现在接收完整 `MemoryScope`；legacy unscoped 在 port 默认实现、SQLite 与 application 调用点 fail closed。
- 已完成（2026-08-13）：SQLite 在 Episode 分组/计数前同时 JOIN `claims` 与 `events`，按 server-derived owner + namespace 限制两端；恶意跨 namespace evidence link 不再把外 scope Episode 计入 identity revision。
- 该项复用现有 schema，不新增 migration / index / MCP tool。它不修复 mixed-scope Claim revision-edge redaction（M1.0.2）或 Unknown owner 写读可达性（M1.0.3），也不构成完整 provenance graph。

- [x] **M1.0.2 Mixed-Scope Claim Revision Edge Redaction**

- 已完成（2026-08-13）：Claim search/get 复用同一 `load_claim_revision_links` 路径；任一端越 scope 时整边隐藏，不保留 Reflection ID、另一端 Claim ID、计数或存在性标志。
- 已完成（2026-08-13）：source（replacement 看 superseded）与 superseded-by（superseded 看 replacement）两个方向都有 SQLite 与 MCP `search_memory` / `get_memory` 负向回归。同 scope revision edge 与 history 整边隐藏策略保持不变。
- 该项没有 schema migration / 新 MCP tool。它不冻结 Unknown owner 写读可达性（M1.0.3），也不代表完整 revision graph。
- [x] **M1.0.3 Owner-Namespace Read-Write Reachability Contract**

- 已完成（2026-08-13）：新写入只接受 `MemoryScope::for_namespace` 派生的 owner + namespace 对（`self`/`world`/`user/*`/`project/*`）。MCP DTO 与 Claim `validate` 拒绝 `Owner::Unknown`；scoped read 继续精确匹配 owner + namespace，不用 unscoped fallback 或 `OR owner = unknown`。
- 已完成（2026-08-13）：只读 `doctor` / `inspect_database` 输出 `unknown_owner_inventory`（events/claims 计数），`rewrite_performed = false` 且 `rewrite_requires_separate_approval = true`。schema 仍允许 legacy Unknown 行，不自动迁移或改写。
- 该项没有 schema migration 或新 MCP tool。它不把 Unknown 行变成 scoped-readable，也不批准后续 rewrite。

三项代码任务彼此可独立，但执行优先级固定为治理判断完整性 → 跨 scope metadata 泄漏 → 写读可达性。它们是后续 M1 feature 的前置门，不把本地 namespace 选择器表述成远程 tenant authorization。

### M1.1 Read Model v2

- [x] **M1.1.1 Scoped Event Recall Read Model**

- 已完成（2026-07-15）：新增独立 `MemoryReadStore` / `search_memory` application path 和 additive MCP 工具；强制显式 namespace，由服务端推导 owner，在 SQLite 中先按 owner + namespace 收窄，再应用 exact event reference、kind、inclusive time window、recent-first 稳定顺序和 `1..=100` limit。
- 已完成（2026-07-15）：event 返回 canonical ID、recorded_at、owner、namespace、kind、summary，以及 persisted `evidence_links` / `episode_events` 派生的 claim IDs 与 episode references；查询不调用 provider，不写 semantic memory tables，只保留有界 operation-log metadata。
- 该完成项只代表 event recall 首片，不代表完整 M1.1、跨 record-type search、文本检索、`get_memory`、reflection history、supersession/correction 或 M1 退出门完成。

- [x] **M1.1.2 Scoped Claim Provenance Read**

- 已完成（2026-07-15）：`search_memory` 增加 additive `record_type = Claim`；省略 `record_type` 仍默认 Event。Claim 查询要求显式 namespace，支持 canonical/raw `claim_reference`、`claim_status`、`mode` 与 `1..=100` limit；省略 `claim_status` 时 DTO 默认 `Active`。
- 已完成（2026-07-15）：SQLite 在所有 claim filter / limit 之前先按 server-derived owner + namespace 收窄；结果返回 canonical `claim:<id>`、subject/predicate/object、mode、status、canonical evidence event references、episode references，以及直接 source/superseded reflection links。查询只读、provider-free，跨 scope exact claim reference 返回空结果。
- Claim 当前没有 stored `recorded_at`，因此 Claim 查询拒绝 event reference、event kind 与时间过滤。Claim lookup 由独立 M1.2.2 切片完成；该项仍不代表 episode/reflection record union、完整 revision history、correction contract 或 M1 退出门完成。

- [x] **M1.1.3 Scoped Episode Provenance Read**

- 已完成（2026-08-09）：`search_memory` 增加显式 `record_type = Episode` 与 optional exact `episode_reference`。Episode reference 继续按现有持久化字符串精确匹配并原样返回；本片不引入 `episode:` canonical/raw 等价改写，避免未经 inventory/migration 就合并历史值。
- 已完成（2026-08-09）：SQLite 先经 `episode_events -> events` 按 server-derived owner + namespace 收窄，再做 exact filter、分组、稳定 recent-first 排序与 `1..=100` limit。同一 reference 跨 namespace 时只返回请求 scope 的 membership；结果包含由同 scope 最新 Event 派生的 `recorded_at`、canonical recent-first Event references 与 canonical same-scope Claim references。路径只读、provider-free，未新增 schema/index，也未开放 Episode `get_memory`。

- [x] **M1.1.4 Scoped Reflection Provenance Read**

- 已完成（2026-08-13）：`search_memory` 增加显式 `record_type = Reflection` 与 optional exact `reflection_reference`。Reflection 没有自己的 owner/namespace 列；本片冻结 attribution：必须存在同 scope 的 superseded Claim，且 replacement 要么为空要么也在同一 scope。record-only Reflection 没有 Claim anchor，不能推断 namespace，因此不进入 scoped search。
- 已完成（2026-08-13）：SQLite 在 filter / 排序 / `1..=100` limit 前按上述 Claim 端点收窄；mixed-scope edge 整条隐藏；exact missing/cross-scope/record-only reference 返回空。结果返回 persisted reflection ID、recorded_at、请求 scope 的 owner/namespace、summary，以及同 scope Claim / evidence references。路径只读、provider-free，未开放 Reflection `get_memory`，也未把 record-only 行纳入 history。
- [x] **M1.1.5 Scoped Evidence Relation Runtime Read**

- 已完成（2026-08-13）：新增第 8 个 MCP 工具 `get_evidence_relation(namespace, trigger_window_event_ids, selected_evidence_event_ids?, selection_basis?)`。caller 提供的 trigger window 先按 raw/`event:<id>` 解析并保序去重，再与请求 owner+namespace 做 intersect-only 收窄；missing 与 cross-scope trigger ID 从窗口中省略，不扩大查询。
- 已完成（2026-08-13）：复用既有只读 `build_evidence_relation_report`：selected 必须是 scoped window 的子集，越窗 fail closed；结果返回 canonical `event:<id>`、window_rank、selected / available-not-selected、binary weight 与 rejection reason。路径只读、provider-free，operation metadata 仅含 `report_type`、`trigger_window_size`、`selected_count`、`result_count`。
- 该项没有 schema migration / index，也不引入 ranking、widening、stable union 或 correction。
- [x] **M1.1.6 Stable Cross-Type Record Union**

- 已完成（2026-08-13）：`search_memory` 增加 additive `record_types`。省略 `record_type` / `record_types` 仍保持 Event；两者同时出现、空数组或重复类型 fail closed。单类型路径与既有 JSON / SQL 顺序不变。
- 已完成（2026-08-13）：两个及以上类型组成 scoped union：分别按同一 scope 与 `1..=100` limit 读取，再按 `recorded_at DESC`（Claim 无 timestamp 排在最后）、type rank（Event / Episode / Reflection / Claim）、id DESC 稳定收口并截断。union 拒绝所有类型专属 filter；Claim 在 union 中仍默认 Active。既有 tagged record JSON 不变。Episode lookup 由后续 M1.2.4 单独完成；Reflection `get_memory` 仍开放。
- 该项没有 schema migration / index，也不开放 Episode/Reflection lookup、identity/commitment history 或 correction。

- 已完成四类 scoped search、evidence-relation runtime、跨类型 union、Episode / Reflection lookup、identity/commitment revision audit 与 audited supersede 首片；结构读回与生命周期排他性已完成；按 2026-10-09 有界批次继续。
- 查询结果保留 ID、namespace、owner、recorded_at、status、mode、provenance 和 supersession 状态。
- SQLite 增加经过 explain/benchmark 证明需要的索引；不先假设 FTS 或向量数据库。

### M1.2 MCP Memory Contract

- [x] **M1.2.1 Scoped Event Lookup**

- 已完成（2026-07-15）：新增 additive `get_memory(namespace, id)`，按 stable event reference 复用同一显式 scope read service；命中时返回与 `search_memory` 相同的完整 event/provenance record，未命中或 ID 属于其他 scope 时成功返回 `record: null`，不回退全库查询。
- 该完成项只代表 Event lookup；Claim lookup 由独立 M1.2.2 切片完成，episode/reflection lookup、通用 record union、history 与 correction contract 仍未完成。

- [x] **M1.2.2 Scoped Claim Lookup**

- 已完成（2026-07-15）：`get_memory(namespace, id, record_type?)` 在显式 `record_type = Claim` 时接受 canonical/raw Claim ID，复用同一显式 scope read service 返回与 Claim search 相同的 status/mode/provenance record。省略类型继续保持 Event 语义，连 raw Event ID 也不因 `claim:` 字面前缀被重判型。
- 已完成（2026-07-15）：精确 Claim lookup 不套用 search 的默认 Active 过滤，因此 Active、Disputed、Superseded 都可按 ID 读取；missing 与跨 scope Claim ID 返回 `record: null`，不进行 unscoped existence probe。该项不代表 episode/reflection lookup、完整 history、correction contract 或 M1 退出门完成。

- [x] **M1.2.3 Scoped Claim Reflection History**

- 已完成（2026-07-15）：新增第 7 个 MCP 工具 `get_reflection_history(namespace, claim_reference, limit?)`。它接受 canonical/raw Claim ID，以 exact scoped Claim 为锚点递归遍历 superseded/replacement 双向关系，按 `recorded_at DESC, reflection rowid DESC` 返回 newest-first revision chain；limit 默认 20、范围 `1..=100`，并返回 `has_more`。
- 已完成（2026-07-15）：missing/cross-scope Claim 返回空历史；mixed-scope revision edge 任一端越界时整条 edge 隐藏；supporting evidence 只保留请求 scope 内 canonical `event:<id>` references。路径只读、provider-free，operation metadata 仅记录 `history_type`、`result_count`、`has_more`。
- 该切片没有 schema migration 或新 index，SQLite 当前保留 technical-MVP 表扫描性能边界。它不覆盖 identity/commitment history、record-only reflection、episode/reflection `get_memory`、correction contract 或 M1 退出门。

- [x] **M1.2.4 Scoped Episode Lookup**

- 已完成（2026-08-13）：`get_memory(namespace, id, record_type?)` 在显式 `record_type = Episode` 时把 `id` 当作 opaque exact persisted Episode reference，复用同一 scoped search service 并以 `limit = 1` 返回与 Episode search 相同的 provenance record。
- 已完成（2026-08-13）：省略 `record_type` 仍保持 Event 语义；空白或首尾空白 reference fail closed；missing / 大小写不同 / 跨 scope ID 返回 `record: null`，不推断 `episode:` canonical/raw 等价。该项不代表 Reflection lookup、record-only history、correction 或 M1 退出门完成。
- [x] **M1.2.5 Scoped Reflection Lookup and Record-only History**

- 已完成（2026-08-13）：`get_memory(record_type = Reflection)` 把 `id` 当作 opaque exact persisted reflection ID，并以 `limit = 1` 复用同一 scoped search。归属规则与 M1.1.4 相同：必须存在同 scope superseded Claim，replacement 为空或同 scope。
- 已完成（2026-08-13）：missing、cross-scope 与 record-only Reflection 都返回 `record: null`，三者不可区分。record-only 行没有 Claim anchor，因此不能进入 scoped lookup 或 history；本片不新增第二条 history 工具。
- [x] **M1.2.6 Identity and Commitment History**

- 已完成（2026-08-13）：新增第 9 个 MCP 工具 `get_self_model_history(namespace, history_type, limit?)`。`history_type` 为 Identity 或 Commitment；limit 默认 20、范围 `1..=100`，并返回 `has_more`。
- 已完成（2026-08-13）：历史来自现有 reflection 审计列 `requested_identity_update` / `requested_commitment_updates`，归属规则与 Reflection search 相同。record-only 更新没有 Claim 锚点，保持不可见。该首片没有 schema migration / 新写路径 / rollback，也不把 `identity_claims` 或 `commitments` 现态表变成版本账本。
- [x] **M1.2.7 Audited Supersede Contract**

- 已完成（2026-08-13）：新增第 10 个 MCP 工具 `supersede_memory(namespace, claim_reference, replacement_claim, replacement_evidence_event_ids, summary)`。它要求显式 namespace、exact Claim 与至少一条同 scope evidence，replacement 必须留在同一 namespace。
- 已完成（2026-08-13）：写入只调用既有 `run_reflection::execute`，把旧 Claim 标为 `Superseded` 并追加 reflection 审计；默认不 hard delete。missing / cross-scope Claim 或 evidence fail closed。该首片不开放 identity/commitment 更新、evidence query narrowing、Event/Episode/Reflection 纠错或第二条 durable write path。

第一批建议接口：

- `search_memory`：按 scope、文本/结构过滤、时间范围、类型和 limit 检索；additive `record_types` 已提供四类 scoped union 首片。
- `get_memory`：按稳定 ID 返回完整记录与 provenance；已覆盖 Event / Claim / Episode / scoped Reflection。
- `get_reflection_history`：已完成 exact scoped Claim revision chain 首片；record-only Reflection 因无 scope 保持不可见。
- `get_self_model_history`：已完成 scoped identity/commitment revision audit 首片；不是 versioned ledger 或 rollback。
- `get_evidence_relation`：已完成 scoped trigger-window ∩ selected-subset runtime 首片；不引入 ranking 或 widening。
- `supersede_memory`：已完成 scoped Claim audited supersede 首片；复用 `run_reflection`，默认不 hard delete。

现有 `ingest_interaction` 保持兼容。MCP Resources 只在上述查询契约稳定后评估，且必须复用同一 read service。

### M1.2.8–M1.2.11 有界可用性补全

- [x] **M1.2.8 Transactional Corrections and Durable Replay** — 事务内 scope/evidence/state CAS，operation+namespace+request_id 作用域的 hash/result 原子回执；相同typed payload重放，不同payload冲突；replay不重复auto-reflection。scoped supersede不修改全局identity/commitments。legacy direct/auto全局self-model治理保持experimental边界。
- [x] **M1.2.9 Structured Feedback Provenance** — 有界caller/tool/model来源声明、producer、target/version、expected/actual、verification与limitations；同scope证据事务内验证；schema v4显式迁移。来源标签不是认证或真实性证明。
- [x] **M1.2.10 Offline Bilingual Literal Recall** — additive recall_memory针对active Claim与Event的scope-first literal OR substring查询；短CJK可用、ASCII大小写折叠、Claim优先、完整provenance；现有search_memory历史浏览不变。小型固定离线fixture，不声称全面M3检索质量或大库性能。
- [x] **M1.2.11 Byte-Bounded Task Context** — build_task_context保留完整record/provenance，完整compact结果JSON UTF-8硬字节预算，包含metadata/自身size；不计JSON-RPC包装、不等于token上限；过大record跳过并返回omission说明。

行为、兼容变化、验证命令和保留的legacy风险详见[可用性合同](../local-memory-usability.md)。四片共享事务、读模型与MCP接口，在同一集成提交验证发布；不代表正式Local Alpha gate完成。

### M1.3 用户闭环

2026-10-09：M1.3.0 / M2.0.1 已实现。只读检查在一致 SQLite read transaction 中对照 canonical schema 的 DDL、columns、FK 与 indexes；结构弱化返回 `schema_structure_invalid`。init 使用原子 `create_new`，失败不删除库/sidecar；migrate 在 backup/rehearsal/write 全程持有 SQLite write reservation。异常 init 文件保留供诊断，需要人工确认后处理，不自动重试覆盖。

- [x] **M1.3.0 Current-Schema Structural Readback Gate** — 在任何 schema-changing M1 slice 和真实客户端退出门前，使用 `table_info`、`foreign_key_list`、index/DDL fingerprint 等验证关键结构；同版本但约束被削弱的数据库不得报告 `current`。无 schema/index 的纯只读 slice 不由此单独阻塞。
- [ ] **M1.3.1 Real Client Recall and Correction Closure** — 仅在 scope/data-integrity、read/history/correction 与 current-schema structural readback 合同完成后执行真实 MCP 客户端故事，并保存 transcript、SQLite readback 与零 scope 泄漏证据。

标准验收故事：

1. 在 `project/a` 写入 10 条记忆，在 `project/b` 写入 5 条干扰记忆。
2. 断开并重新连接 MCP 客户端。
3. 在 `project/a` 检索，结果只包含当前 scope。
4. 查看任一命中的 event / claim / episode provenance。
5. supersede 一条错误 claim，默认结果只返回新版本，历史接口仍可回看旧版本。
6. provider 离线时重复上述 deterministic read path，结果仍可用。

M1 退出门：该故事由真实本地客户端完成，并保存 MCP transcript、SQLite readback 和无跨 scope 泄漏证据。

## 8. M2 — Local Product Alpha

- [x] **M2.0.1 Exclusive Init-and-Migration Lifecycle Gate** — init 清理只处理本次操作确定创建的文件；init/migrate 需排他执行或明确拒绝并发，moving-target 检测不能只比较版本与行数。若任何 M1 任务提前引入 migration，则相应排他性门禁同步前移。
- 建立版本化 macOS binary archive 和 checksum；Local Alpha 不要求 installer、service manager 或 auto-updater。
- quick start 不要求用户本机安装 Rust toolchain。
- `init → doctor --read-only → serve → remember/search/inspect/correct` 在 fresh macOS 环境可按文档完成。
- Windows runner 执行真实 runtime parity；未通过前明确写“不支持/未验证”，不做模糊 parity 声明。
- backup → restore-to-new-path → read-only verify → manual switch 全链路通过。
- 至少一个 real provider 路径只证明配置、连通与解析；deterministic memory read 不依赖 provider。
- support bundle 继续保持本地、脱敏、无原始 SQLite / TOML / provider payload。
- 人工 release decision 记录 reviewer、open gates、rollback note 和最终结论。

M2 退出指标：

- fresh-machine 首次完成核心用户闭环不超过 10 分钟；
- 不需要源码编辑或 `cargo run`；
- namespace leakage = 0；
- backup / restore roundtrip = 100%；
- 公开文档不存在 Local Alpha / Beta / GA 越级声明；
- release artifact 可解包、checksum 匹配、版本可追溯。

## 9. M3 — Retrieval Quality and Lifecycle

只有 M1/M2 通过后才进入：

- 冻结至少 50 个 scoped recall 场景，再比较 structured query、FTS5、recency、relation weight 等方案。
- 初始质量目标为 recall@5 ≥ 0.80、provenance coverage = 100%、namespace leakage = 0；若基线差异较大，先记录基线再批准目标调整。
- 增加 tombstone、retention、compaction、export 和恢复验证，不直接 hard delete 审计历史。
- identity / commitment 引入版本、diff、effective time 和 rollback target。
- evidence relation、episode summary 和 memory layer projection 要么接入正式 read path，要么删除或明确留在 labs。
- automatic self-revision 只在可靠 trigger、scope、idempotency、rollback 和人工可解释性通过后扩张。

## 10. M4 — 暂不排期

以下方向不进入当前执行队列：

- Streamable HTTP remote server；
- OAuth / authz / rate limit / tenant isolation；
- remote read-only dashboard 与 remote write admin；
- write-capable daemon、queue、retry、dead letter 和持续自治；
- MCP experimental tasks；
- multi-agent orchestration；
- full procedural memory / self-model autonomy。

进入条件：M2 稳定、至少 3 个真实用户完成两周 dogfood、无数据丢失或 scope 泄漏、需求证据明确，并在实现前完成独立 threat model 与 rollback runbook。

## 11. 验证策略

### 11.1 验证阶梯

每个里程碑按以下顺序扩张：

1. 只读现状确认；
2. 一个失败测试证明风险；
3. 一个最小实现切片；
4. 一个 SQLite / namespace / client 单点闭环；
5. 小样本集成验证；
6. fresh-machine / cross-platform；
7. 人工 release decision。

前一级证据不通过，不进入后一级。

### 11.2 风险与最小证明动作

| 风险假设 | 最小证明动作 | 通过条件 | 停止 / 回滚 |
| --- | --- | --- | --- |
| snapshot 会跨 scope | 两个 namespace 各写一条冲突记忆并构建 scoped snapshot | 输出仅含目标 scope | 任一泄漏立即停止 auto-reflection 扩张 |
| selected action 绕过 gate | stub 返回受禁 selected action | 结果 blocked，模型 action 不被标记 passed | 保持 decision experimental |
| doctor 会改库 | 对已知 DB 记录 checksum / schema / rows，再运行 read-only doctor | 三类 readback 完全不变 | 禁止用于 release soak 正式库 |
| migration 半完成 | 在 copy / rename / FK check 注入失败 | 原库可打开、行数一致、可再次迁移 | 恢复备份，阻断 schema 发布 |
| dashboard 暴露非本机 | 配置 `0.0.0.0` 启动 | 无 auth 时拒绝启动 | 保持 dashboard disabled |
| recall 只命中 ID 不可解释 | search 命中一个 claim 后读取 provenance | event / episode / reflection 链可读取 | 不宣布 trustworthy recall |
| package 不可复现 | clean machine 解包并运行核心故事 | 无 Rust toolchain、checksum 正确 | 保持 source-only MVP 口径 |

### 11.3 每阶段必须报告

- 全局建模了什么；
- 实际只执行了哪个最小切片；
- 收集了什么证据；
- 哪些能力故意没有扩张；
- 剩余风险和下一最小动作。

## 12. 成功指标

| 维度 | 指标 |
| --- | --- |
| 激活 | fresh-machine 到首次 remember + recall ≤ 10 分钟 |
| 隔离 | namespace leakage = 0 |
| 可解释 | recall provenance coverage = 100% |
| 可恢复 | backup / restore roundtrip = 100% |
| 迁移 | 每个 schema migration 有 version、backup anchor、failure test、FK check |
| 可用性 | provider 离线时 deterministic read path 仍可用 |
| 质量 | M3 评测集 recall@5 初始目标 ≥ 0.80 |
| 工程 | clean clone CI 通过 format、clippy、tests、status sync |
| 产品真实度 | implemented / partial / experimental / blocked 与代码一致 |
| 文档治理 | active 总计划始终只有一份 |

## 13. 参考 30 天执行节奏

### 第 1 周：真相与回归测试

- 完成 M0.1；
- 为 scope 泄漏、selected action 复检、doctor 写入和 migration failure 建立红灯测试；
- 冻结兼容策略和数据库备份锚点。

### 第 2 周：Foundation v2

- 实现 `MemoryScope` 和 scoped store queries；
- 拆分 doctor / init / migrate；
- 收口 dashboard loopback、tracing 与基础 CI。

### 第 3 周：Read Model v2

- 已完成 Event / Claim `search_memory` 与 `get_memory` 首片，以及 scoped Episode / Reflection provenance `search_memory` 首片；
- 已完成 exact scoped Claim revision history、scoped identity/commitment revision audit、evidence-relation runtime、跨类型 union、Episode / Reflection lookup 与 audited supersede 首片；继续完成 current-schema structural readback；
- 用两个 namespace 小样本验证。

### 第 4 周：真实客户端闭环

- 完成 Claim 之外的 history 与 correction contract；
- 在一个真实 MCP 客户端跑完整用户故事；
- 根据证据决定进入 M2，或继续修复 M1。

## 14. 停止条件

出现以下任一情况必须停在当前里程碑，不得用新增功能掩盖：

- scope 泄漏、数据丢失或迁移不可恢复；
- 需要破坏现有 MCP schema，但没有兼容层和迁移说明；
- doctor / smoke / release 命令可能接触未确认的正式数据库；
- 测试层级命令失效，或 active plan / reality gate 状态与当前分支不一致；
- 只读 projection 被当成 runtime capability；
- 需要远程监听、auth、第三方发布或凭据变更，但未获得单独授权；
- external evidence 缺失却准备声明 Local Alpha、Beta、GA 或 production-ready。

## 15. 文档迁移

| 文档 | 新角色 |
| --- | --- |
| `docs/positioning.md` | 继续作为命名与公共定位唯一来源 |
| `docs/project-status.md` | 只写当前实现与已验证事实 |
| `docs/roadmap.md` | 只写 Now / Next / Later 摘要 |
| 本文件 | 唯一 active execution plan |
| `docs/product/follow-up-reality-gates.md` | 证据状态与阻断项 |
| `codex/archive/pre-mainline-reset-2026-07-10` | 保存旧 progress tracker、历史 specs/plans、阶段快照和 release records |
| [archive.md](../archive.md) | 当前分支的归档索引、查阅方式与精确恢复方法 |

## 16. 当前进度与下一步

已完成：

- 2026-07-10 只读审计了代码架构、运行时能力、文档口径、测试清单和发布链；
- 建立新的产品主线、里程碑、证据门和停止条件；
- 建立本地主线整理分支与完整历史归档分支；当前文档树从 74 个文件收束到 36 个，40 份历史/实验文档仍可精确恢复；
- status-sync 已改读本计划，根目录误提交 SQLite 文件已移除并增加回流门禁；
- 测试与工具链已分为 `fast` / `core` / `full`；发布工具链退出默认构建但保留 full-feature 验证；status-sync 不再触发整套测试编译；
- 串行通过 `cargo fmt --check`、`git diff --check`、`cargo check`、三级测试、status-sync 和完整 all-feature Clippy；
- 未发布、未推送、未运行远程或 live-provider 操作。
- M0.2 已完成七个连续最小切片：snapshot scope、explicit evidence manifest、time window / stable order、scoped auto-reflection snapshot、active reflection runtime event-ID 等价性、只读 evidence/episode projection event-ID 等价性，以及 offline demo artifact event reference。
- M0.3 已完成四个限定切片并收口：trusted decision commitments + requested/selected dual gate、claim → evidence → episode distinct provenance join、validation / handled-ledger / commit failure atomicity，以及 experimental non-authoritative decision authority。
- M1.1.1、M1.1.2、M1.1.3、M1.1.4、M1.1.5、M1.1.6、M1.2.1、M1.2.2、M1.2.3、M1.2.4、M1.2.5、M1.2.6 与 M1.2.7 已完成：Event / Claim / Episode / Reflection search、跨类型 union、四类 lookup、Claim-linked reflection history、scoped identity/commitment revision audit、evidence-relation runtime 与 scoped Claim audited supersede 均使用显式 namespace 和既有治理写路径；versioned identity/commitment ledger、record-only history 与真实客户端退出门仍开放。
- 2026-08-09 只读复核把三个既有缺口正式纳入 M1.0 前置门：identity evidence-to-Episode 计数缺少完整 scope、Claim 普通 provenance 对 mixed-scope revision edge 的 Reflection ID redaction 不完整，以及合法 Unknown owner 写入与 namespace-derived read 的可达性不一致。它们不回滚 M1.1.3 的 scope-first Episode projection，但会阻塞后续 M1 feature 扩张。
- 2026-08-13 完成 M1.0.1：identity supporting-Episode 查询现在绑定完整 `MemoryScope`，并在分组/计数前同时限制 Claim 与 Evidence Event 的 owner + namespace；恶意跨 namespace evidence link 不再改变 identity revision 判断。
- 2026-08-13 完成 M1.0.2：Claim search/get 对 mixed-scope revision edge 整边隐藏，不再保留 Reflection ID 或对端 Claim 元数据；source 与 superseded-by 两个方向都有负向回归。
- 2026-08-13 完成 M1.0.3：冻结 canonical 写后可读矩阵，新写入拒绝 Unknown owner；只读 doctor inventory 统计 legacy Unknown 行且不改写。M1.0 三项前置门全部通过。
- 2026-08-13 完成 M1.1.4：`search_memory(record_type = Reflection)` 按同 scope Claim 端点归属 Reflection；record-only 行不可推断 namespace，mixed-scope edge 整边隐藏，`get_memory` 仍不开放 Reflection。

M0 已收口。仍属于后续路线图而非本轮完成声明的项目包括：repository-wide event-ID 统一、完整 recall contract、support bundle inventory、server-created snapshot handle、structured action validation、完整 policy arbitration、真实 binary package、fresh-machine 与 Windows runtime parity。本机私有 credential 轮换仍是用户侧动作，不纳入仓库提交。

当前里程碑是 M1 Trustworthy Recall；M1.0 三项 Scope/Data-Integrity Gates、M1.1.1–M1.1.6 四类 scoped search / evidence-relation / 跨类型 union，以及 Event/Claim/Episode/Reflection lookup、Claim reflection history、identity/commitment revision audit 与 scoped Claim audited supersede 已完成。下一执行顺序冻结为 current-schema structural readback → real-client closure；每项仍须独立领取。M2 fresh-machine / packaging 前还必须完成 exclusive init/migration gate。remote、tasks、OAuth、daemon writes、provider 扩张和正式发布仍不因 M0 或这些已完成切片而获得授权。
