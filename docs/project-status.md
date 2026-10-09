# 当前实现状态

## 2026-10-09 时间/独立范围/导出续建

Schema v6 补齐 nullable Claim recorded_at、独立 caller observed_at、持久纳秒排序键与 scope/time 索引。Reflection 的 origin/affected scope 与证据关系独立持久化；历史归属不明保持 unknown，targetless 读取严格限定来源和影响范围，不扩大权限。新增只读有界 export_memory（非恢复备份）。混合查询的 per-type LIMIT 前排序已修复，单类型旧浏览排序保留。详见 [时间](temporal-metadata.md)、[反思范围](reflection-scope-history.md)、[导出](scoped-export.md)。本地 613 项完整 Rust 测试、fmt、Clippy、status-sync 已通过；[阶段结果与独立证据](plans/2026-10-09-schema6-results.md)记录兼容性、上下文容量代价与实测结果。精确发布提交 CI 另行核验；下方 v5 完整结果是历史基线，不能代替 v6 验证。

## 2026-10-09 原方案 A–E 续建

已新增 schema v5、持久反馈候选与原子确定性提交、FTS5 可重建派生召回（短中文精确 fallback）、完整响应预算/解释，以及持久 Episode 和版本化语义/流程候选。新工具合同与保守边界见 [A–E 运行指南](memory-feedback-experience.md)。候选激活只改变可召回状态，不增加权限。本地最终完整测试 570 项、fmt、all-target/all-feature Clippy、status-sync 均通过，新增 stdio 闭环与备份恢复验证通过。三平台 CI 仍须读取草稿 PR 的最终精确提交结果；下方 e14a59e 结果仅是历史基线。

固定离线 A/B/C+消融与 10k/100k 容量工具已加入；实际结果以可复现报告为准。真实模型效果、token成本、完整数据模型规范化及旧大模块拆分仍明确未完成，见 [完整追踪矩阵](plans/2026-10-09-original-plan-traceability.md) 与 [续建增量映射](plans/2026-10-09-continuation-results.md)。

## 2026-10-09 早期可用性候选（e14a59e 历史基线）

新增第 11/12 个 MCP 工具 `recall_memory` / `build_task_context`：本地 literal 中英文文本召回、active Claim 优先、完整 provenance，以及 compact JSON UTF-8 硬字节预算。历史 `search_memory` 保持原合同；不声称 semantic retrieval、全文索引或大库性能。

更正 now 在写事务内复核 scope/evidence/state 并 CAS；ingest / supersede 支持 namespace+operation 作用域的可选 request_id，持久化原子 hash/result 回执，重放不重复 auto-reflection。Event 新增有界结构化 feedback，schema v4 显式迁移保留历史。来源标签是 caller supplied，不是认证或事实证明。

详见 [本地可用性合同](local-memory-usability.md)。执行型 installed-binary smoke 已在本地通过；它是安装模拟，不是fresh-machine证明。Windows native CI 已加入，平台结果以草稿PR精确head checks为准。完整MCP回归通过后仍不把legacy global self-model路径称为统一scope隔离；它可以按既有治理使用project证据影响全局identity/commitments。

## 2026-10-09 数据库安全前置门

已实现 current-schema 结构读回（canonical DDL/columns/FK/index、一致 read transaction）和排他 init/migrate。弱化同版本结构返回 `schema_structure_invalid`，不会报告 current 或自动修复。init 原子 create_new，失败保留文件而不清除潜在并发数据；migration write reservation 覆盖 backup、rehearsal、write。已用持有reader的确定性用例复现并修复提交时SQLITE_BUSY：writer入场仍快速拒绝，取得reservation后使用5秒busy timeout等待短暂reader锁；结构/行数读回在commit前完成。SQLite 生命周期 focused 回归覆盖并发和结构破坏；平台证据随精确提交 CI 单独核验。

## 2026-10-09 最新上游与有界候选

已重新拉取上游 `dev-work@c318caf`，确认包含 PR #2 的默认 stdio bootstrap 测试隔离修复。下述 8 月远端/本地分支差异是历史记录，不代表当前 GitHub 状态。可用性改进将沿唯一 active plan 的有界执行批次推进；规划中的能力未因此成为已实现，当前仍是 local-first technical MVP。

## 2026-08-25 正式化与远端主线同步基线

项目已进入“从 technical MVP 收束为可交付 Local Product Alpha”的正式化规划阶段，但尚未通过 M1、M2 或人工 release gate，因此当前对外口径仍是 local-first technical MVP。正式化的产品、工程、安全、发布和协作差距统一见[正式化改进与主线同步计划](formalization-improvement-plan-2026-08-25.md)；该文件是覆盖矩阵，不替代唯一 [active project plan](plans/2026-07-10-product-replan.md)。

本轮任务开始时，本地 `dev-work@43580ba` 工作区 clean，相对 `origin/dev-work@082be93` 领先 12 个线性提交，相对 `origin/main@6fcbb5f` 领先 28 个提交。GitHub 已存在 PR #1（`dev-work -> main`），但远端 head 仍是 `082be93`，旧 Linux/macOS checks 均失败。因此当前正确路径是先以 fresh 本地门禁验证候选，再推送现有 `dev-work` 更新 PR；只有新 head checks 通过并完成人工 review 后，才允许合并 `main`。

这里的 GitHub source sync 不等于生产部署。当前仓库没有唯一生产主机、服务单元、容器平台或部署清单，本轮不包含 SSH、服务重启、数据库迁移、live provider、tag 或 GitHub Release。

## 2026-07-10 主线收束

项目现在以[项目起点与主线原则](origin-and-principles.md)作为认知入口，以 `Truth and Safety -> Trustworthy Recall -> Local Product Alpha` 作为当前执行顺序。

本次仓库整理采用“当前主线 + 本地历史分支”结构：

- 主线整理基线：`1f7390d`；当前工作以实际 checkout 的本地分支为准，不在状态文档中固化临时分支名；
- 整理前完整归档：`codex/archive/pre-mainline-reset-2026-07-10@48f6eca`；
- 当前文档树由 74 个文件收束为 36 个；40 份历史/实验文档从当前分支移出但可精确恢复；
- 历史 specs、旧 plans、阶段快照、原始逐轮日志和旧 release records 已从当前文档树移出；
- 原始讨论整理稿、核心 runtime、正式 tests、开发/接入文档继续保留；
- dashboard、daemon、productization preflight 和 release-evidence 代码本轮没有拆除，只是不再主导首页和路线图；后续必须按依赖闭包单独决策。

归档不代表删除证据；具体查阅与恢复方式见[archive.md](archive.md)。

## 2026-07-14 M0.2 至 M0.5 收口

当前状态必须与 [active project plan](plans/2026-07-10-product-replan.md) 一起阅读。项目仍是 source-only、local-first technical MVP；M0.1 / M0.1.1 的仓库与工具链收束、M0.2 scoped snapshot 限定退出门、M0.3 governance correctness、M0.4 explicit database lifecycle，以及 M0.5 runtime boundary / CI 均已收口。Local Alpha、Beta、GA 和 production-ready 仍未通过对应 gate。

M0.2 的完成对象仅是显式 scoped snapshot 及其必需边界：scope/manifest/time 交集、stable recent-first order、scoped auto-reflection、active reflection 与只读 projection 的 event-ID 等价性，以及 offline demo artifact reference。省略 `namespace` 的 legacy MCP 调用仍是 unscoped 兼容路径；完整 recall contract、repository-wide event-ID 统一、support bundle inventory 和 M0.3 治理能力不属于本次完成声明。

本次代码级审计确认了以下必须优先公开的边界：

- `build_self_snapshot` 已增加 additive `namespace` 输入；server 会从 namespace 唯一推导 owner，并把 scope 贯穿 application、query port 和 SQLite store。`MemoryScope` 的反序列化只接受匹配的 owner + namespace 或全空 legacy 状态，partial / mismatched scope 会被拒绝。显式 scope 下的 active claims、event references 与 episode references 会在查询层按 owner + namespace 收窄；省略 namespace 时仍保留旧 unscoped 行为作为 MCP 兼容层，因此不能把兼容调用描述为已隔离。
- `build_self_snapshot` 已增加 additive `evidence_manifest`：只要显式提供 manifest（包括空数组）就必须同时显式提供 `namespace`，并在 query construction 前限制为最多 256 项；DTO、application 与 store 三层都会拒绝未完整收窄或超限的 manifest 查询，且 snapshot 参数会在可选 auto-reflection 之前完成校验；裸 event ID 与 `event:<id>` 会先规范化并保序线性去重，再与 server-owned owner/namespace scope 在 SQLite 查询中同时约束。越 scope ID 被排除；显式空 manifest 或空交集返回空 evidence，不会回退为全 scope。tool operation log 使用 snapshot namespace，auto-reflection 诊断保留独立 namespace。
- `build_self_snapshot` 已增加 additive `recorded_after` / `recorded_before`：任一时间边界都要求显式 `namespace`，使用 inclusive 边界，RFC3339 输入归一到 UTC，倒置窗口由 DTO / application fail closed。SQLite evidence 查询在同一条 SQL 中取 owner/namespace、manifest（如有）与时间窗交集；SQL 将项目 canonical timestamp 与旧库常见 `Z` / offset 文本转换为固定宽度 UTC 秒 + 9 位小数秒排序键，避免 SQLite date function 折叠亚毫秒差异，再以 `rowid DESC` 稳定 tie-break。episode 按窗口内最新合格事件元组排序，避免独立 `MAX(recorded_at)` / `MAX(rowid)` 来自不同事件。显式窗口空交集保持为空；claims 没有 recorded timestamp，因此仍只做 scope filtering。legacy unbounded snapshot 继续兼容，但 snapshot evidence / episode SQLite 读取已统一 recent-first；手工写入且超出项目 canonical / 常见 legacy 形式的畸形时间文本会 fail closed，而不是扩大 bounded 查询。
- automatic self-revision 已从触发 namespace 派生完整 owner + namespace scope：先冻结 trigger window，再取授权 scope 与 trigger manifest 的交集，并以该受限窗口构建 revision snapshot 与 episode read；无有效交集保持 fail-closed，不回退到历史全量。active reflection runtime 的 MCP/application/model proposal evidence 输入均接受裸 event ID 与 `event:<id>`，以底层 raw ID 保序去重；SQLite 查询、evidence links、reflection audit 与 auto-reflection diagnostics 的明确 `*_event_ids` 兼容字段继续存取 raw ID，reference-shaped 输出才使用 canonical `event:<id>`。
- `decide_with_snapshot` 仍接收调用方 snapshot，但 application 会在 gate 与 provider 调用前用当前服务端 commitment store 覆盖其中的 commitments，并对 requested action 与 provider-selected action 复用同一 commitment gate；selected action 被拒绝时返回 blocked、保留被拒绝的 `selected_action`，且 `decision = null`。允许路径保留兼容的 `model_decision` / action-string payload，同时明确返回 `decision_authority = experimental_non_authoritative` 与 `policy_scope = server_commitment_gate_only`；`gate.blocked = false` 不是完整 policy-passed verdict。identity / claims / evidence / episodes 仍是 caller-provided，尚无 server-created snapshot handle 或完整 policy binding，因此仍不能描述为完整可信策略执行器。
- cross-episode identity support 已不再使用全局 episode 数量推断：application 先选出与 proposed identity value 匹配的 active claims，再通过只读 store port 与 SQLite `evidence_links` → `episode_events` join 计算 distinct supporting episodes；无关 episode、无 evidence link 的 claim 和空 claim 集都不会提高支持数。M1.0.1 进一步要求该查询绑定完整 `MemoryScope`，并在分组/计数前同时限制 Claim 与 Evidence Event 的 owner + namespace；跨 namespace evidence link 不再改变 identity revision 判断。该切片复用现有 schema，仍不是完整 provenance graph。
- governance failure atomicity 已由 application fault injection 与真实 SQLite transaction 回归共同覆盖：validation rejection 不进入 reflection transaction；handled-ledger append 或 commit 失败时，pending identity / commitment / claim-evidence / reflection / handled audit 均回滚，随后事务外只记录 rejected trigger audit。该证据不等于进程崩溃恢复、跨进程事务或分布式一致性。
- `doctor` 默认执行只读检查；missing / old / read-only 数据库只报告状态，不创建、迁移或 seed。只有显式 `init`、`migrate` 或 `doctor --allow-bootstrap` 可以改变数据库；`serve` 只接受 current database。
- SQLite 当前 schema version 为 3，并使用 `schema_migrations` ledger。legacy rebuild 在原库写入前建立 backup anchor 和 restore rehearsal，随后在事务内执行 row-count preservation、`foreign_key_check`、ledger 与表行数 readback；这仍不是 remote backup、scheduled backup、cloud sync 或 production DR。
- dashboard HTTP 路由仍无认证；启用 dashboard 时配置校验只接受 `localhost` 或 loopback IP，非 loopback host 会在启动前失败。它仍只是本机只读界面，不能暴露到公网反向代理，也不能描述为 remote/admin 能力。
- episode summary、memory layer 和 richer semantics projection 主要是 read-only 定义与测试切片；evidence relation 已有 scoped MCP runtime 首片，仍不是 ranking / widening engine。
- `.github/workflows/ci.yml` 已在 Linux/macOS 上声明 format、all-feature Clippy、full tests 与 status sync；Rust 固定为 `1.95.0`，CLI tracing 固定写 stderr。当前仍无真实 binary package、fresh-machine / Windows 完整证据或正式 release approval。
- `rmcp` 仍固定在 `0.5.0`。对官方 `2.2.0` 的隔离探针在机械迁移后 compile 通过，但存在 router dead-code warning，且 MCP `stdio` 回归为 47/48；因此本轮不升级，具体破坏面与测试矩阵见 [兼容性 spike](spikes/rmcp-compatibility-2026-07-14.md)。

新的优先级是：`Truth and Safety -> Trustworthy Recall -> Local Product Alpha -> Retrieval Quality -> optional Remote/Autonomy`。旧 productization、P1/P2/P3 和 non-MVP plans 保留为历史记录，不再决定下一步。

## 2026-07-15 M1.1.1 / M1.1.2 / M1.2.1 / M1.2.2 / M1.2.3 scoped read 首片

M1 已开始，当前完成两个 `search_memory` 读取切片和 Event / Claim 两个 `get_memory` lookup 切片。`search_memory` MCP 工具要求显式 `namespace`，省略 additive `record_type` 时继续查询 Event；服务端据此派生 owner，并在 SQLite 查询内先按 owner + namespace 过滤，再应用对应 record type 的过滤与 `1..=100` limit。Event 查询支持 exact event reference、kind、inclusive RFC3339 time window 和 recent-first 稳定排序；`get_memory` 复用同一 read service，按 stable ID 返回单条 Event 或 Claim，未命中或跨 scope ID 返回 `record: null`。Event 记录保留 canonical `event:<id>`、recorded_at、owner、namespace、kind、summary，以及从现有 `evidence_links` 和 `episode_events` 批量读取的 claim IDs / episode references。

M1.1.2 为 `search_memory` 增加 `record_type = Claim`。Claim 查询要求同一显式 namespace，支持 canonical/raw `claim_reference`、`claim_status`、`mode` 与 `1..=100` limit；DTO 省略 `claim_status` 时默认 `Active`。结果返回 canonical `claim:<id>`、owner、namespace、subject/predicate/object、mode、status，以及 canonical evidence event references、episode references 和直接 source/superseded reflection links。claims 当前没有 `recorded_at`，因此 Claim 查询会拒绝 event reference、event kind 和时间过滤；跨 scope exact claim reference 返回空结果，不扩大查询。M1.2.2 又让 `get_memory` 在显式 `record_type = Claim` 时接受 canonical/raw Claim ID；省略类型仍保持 Event 语义，避免历史 raw Event ID 的判型回归。精确 Claim lookup 不套用 search 的默认 Active 过滤，因而可返回任意状态 Claim。

M1.2.3 新增第 7 个 MCP 工具 `get_reflection_history(namespace, claim_reference, limit?)`。它接受 canonical/raw Claim ID，以 exact scoped Claim 为锚点，递归沿 superseded/replacement 双向关系读取同一 revision chain，并按 `recorded_at DESC, reflection rowid DESC` 返回 newest-first 结果；limit 默认 20，合法范围为 `1..=100`，并用 `has_more` 表示截断。missing/cross-scope Claim 返回空历史；只要 revision edge 任一端不属于请求 scope，整条 mixed-scope edge 就隐藏。每条 reflection 只返回同 scope 的 canonical evidence event references；operation metadata 仅含 `history_type`、`result_count`、`has_more`。

Event、Claim 与 Claim history 查询共用独立的只读 application/port，不复用 reflection evidence narrowing，不调用 model provider，也不修改 events、claims、evidence、episodes、reflections、identity 或 commitments。真实 `stdio` 回归覆盖跨 scope 干扰、exact-reference no-widening、非法参数 fail-closed、断开重连和不可达 provider。M1.2.3 没有 schema migration 或新 index，当前 SQLite 实现保留 technical-MVP 表扫描性能边界。五个首片完成仍不代表 identity/commitment history、record-only reflection、episode/reflection `get_memory`、supersede/correction、真实客户端 M1 退出门或 Local Alpha。

## 2026-08-09 M1.1.3 Scoped Episode Provenance Read

M1.1.3 为现有 `search_memory` 增加显式 `record_type = Episode` 与 optional `episode_reference`。Episode reference 在本片保持 opaque：按持久化字符串精确匹配并原样返回，不做 `episode:` canonical/raw 猜测或改写。省略 `record_type` 仍查询 Event。M1.2.4 / M1.2.5 之后 `get_memory` 已接受显式 Episode 与 scoped Reflection lookup。

SQLite 从 `episode_events -> events` 出发，在 exact filter、分组、排序和 `1..=100` limit 之前按 server-derived owner + namespace 收窄。同一个 reference 即使关联多个 namespace，也只返回请求 scope 的 membership，不暴露其他 scope 的计数或存在性。结果的 `recorded_at` 由该 scope 内最新 Event 派生，provenance 返回 canonical recent-first Event references 与 canonical same-scope Claim references；路径只读、provider-free，operation metadata 仅记录 record type 与结果数。

该首片没有新增 Episode table、schema migration 或 index，也没有把只读 projection 的 caller-provided `objective / outcome / lesson` 当成持久化事实。M1.1.4 / M1.1.5 / M1.1.6 / M1.2.4 / M1.2.5 / M1.2.6 / M1.2.7 已补上 scoped Reflection search/lookup、evidence-relation runtime、跨类型 union、Episode lookup、scoped identity/commitment revision audit 与 scoped Claim audited supersede，但 versioned identity/commitment ledger、record-only reflection history、Event/Episode/Reflection 纠错、真实客户端退出门和 Local Alpha 仍开放。

同日只读复核还确认三项既有缺口，并已作为 M1.0 前置门写回 active plan。2026-08-13 已完成全部三项：identity supporting-Episode 查询现在同时限制 Claim 与 Event scope；Claim search/get 对 mixed-scope revision edge 整边隐藏；新写入拒绝 `Owner::Unknown`，只读 doctor 盘点 legacy Unknown 行且不改写。它们不回滚本切片已验证的 scope-first Episode projection。

## 2026-08-13 M1.0.1 Scoped Identity Evidence-to-Episode Gate

M1.0.1 让 identity auto-reflection 的 supporting-Episode 查询绑定完整 `MemoryScope`。`list_episode_references_supporting_claims` 现在接收 server-derived owner + namespace；legacy unscoped 调用 fail closed。SQLite 在 Episode 分组/计数前 JOIN `claims` 与 `events`，两端都必须匹配请求 scope。恶意跨 namespace 的 persisted evidence link 不再把外 scope Episode 计入 `cross_episode_support_count`，因此也不能单独把 identity revision 从拒绝变成通过。

该切片没有 schema migration、新 index 或新 MCP tool。它不冻结 Unknown owner 的写读可达性合同。

## 2026-08-13 M1.0.2 Mixed-Scope Claim Revision Edge Redaction

M1.0.2 让普通 Claim search/get 与 Claim history 使用同一 fail-closed edge 策略。`load_claim_revision_links` 在 source 与 superseded-by 两个方向都要求两端同属请求 scope；任一端缺失或越 scope 时整边隐藏，不返回 Reflection ID、对端 Claim ID、计数或存在性标志。同 scope revision 与 replacement 为空的 dispute edge 仍可见。`search_memory` 与 `get_memory` 共用该 store 路径。

该切片没有 schema migration 或新 MCP tool。它不构成完整 revision graph。

## 2026-08-13 M1.0.3 Owner-Namespace Read-Write Reachability Contract

M1.0.3 冻结新写入的 owner/namespace 合同：`self` → `Self_`，`world` → `World`，`user/*` → `User`，`project/*` → `World`。MCP Event/Claim DTO 与 Claim `validate` 拒绝 `Owner::Unknown`。namespace-derived scoped read 继续精确匹配 owner + namespace，不使用 unscoped fallback 或 `OR owner = unknown`。schema 仍允许 legacy Unknown world/project 行；只读 `inspect_database` / `doctor` 报告 `unknown_owner_inventory`，`rewrite_performed = false`，改写需另行批准。

该切片没有 schema migration 或自动 rewrite。它不让 legacy Unknown 行通过 scoped read 找回。

## 2026-08-13 M1.1.4 Scoped Reflection Provenance Read

M1.1.4 为现有 `search_memory` 增加显式 `record_type = Reflection` 与 optional exact `reflection_reference`。Reflection 表没有 owner/namespace；scope 只从 Claim 端点派生。一条 Reflection 属于请求 scope，当且仅当 superseded Claim 在该 owner + namespace 内，且 replacement 要么为空、要么也在同一 scope。record-only Reflection 没有 Claim anchor，因此不能推断 namespace，也不会出现在 scoped search 或 exact reference 命中中。mixed-scope revision edge 整条隐藏。

结果返回 persisted reflection ID、recorded_at、请求 scope 的 owner/namespace、summary，以及同 scope canonical Claim / evidence references。`get_memory` 仍只接受 Event / Claim。该切片没有 schema migration / index，也不覆盖 identity/commitment history、record-only history 或 Reflection lookup。

## 2026-08-13 M1.1.5 Scoped Evidence Relation Runtime Read

M1.1.5 把既有只读 `build_evidence_relation_report` 收敛为第 8 个 MCP 工具 `get_evidence_relation(namespace, trigger_window_event_ids, selected_evidence_event_ids?, selection_basis?)`。显式 namespace 派生 owner；trigger window 接受裸 ID 与 `event:<id>`，保序去重后再与 SQLite 中同 owner+namespace 的 Event 做 intersect-only 收窄。missing / cross-scope trigger ID 从窗口省略；selected 必须落在 scoped window 内，否则 fail closed。结果返回 canonical `event:<id>`、caller-order `window_rank`、selected / available-not-selected、bounded binary weight 与 rejection reason。路径只读、provider-free；operation metadata 仅含 `report_type`、`trigger_window_size`、`selected_count`、`result_count`。

该切片没有 schema migration / index，也不引入 ranking、widening、stable union、Episode/Reflection lookup 或 correction。既有 projection JSON 的 raw `event_id` 合同保持不变。

## 2026-08-13 M1.1.6 Stable Cross-Type Record Union

M1.1.6 为 `search_memory` 增加 additive `record_types`。省略 `record_type` 与 `record_types` 时仍查询 Event；两者同时出现、空数组或重复类型 fail closed。单类型路径保持原来的 SQL 顺序与 tagged JSON。两个及以上类型组成 scoped union：先分别按同一 owner+namespace 与 `1..=100` limit 读取，再按 `recorded_at DESC`（Claim 无 timestamp 排在最后）、type rank（Event / Episode / Reflection / Claim）、id DESC 稳定排序并截断。union 拒绝类型专属 filter；Claim 在 union 中仍默认 Active。`get_memory` 现已覆盖 Event / Claim / Episode / scoped Reflection。

该切片没有 schema migration / index，也不开放 Episode/Reflection lookup、identity/commitment history 或 correction。

## 2026-08-13 M1.2.4 Scoped Episode Lookup

M1.2.4 让 `get_memory(namespace, id, record_type?)` 在显式 `record_type = Episode` 时接受 opaque persisted Episode reference，并以 `limit = 1` 复用同一 scoped search service，返回与 Episode search 相同的 provenance record。省略类型仍保持 Event 语义，因此 `episode:*` 不会被重判为 Episode。空白或首尾空白 ID fail closed；missing、大小写不同或跨 scope ID 返回 `record: null`。本片不推断 `episode:` canonical/raw 等价。

## 2026-08-13 M1.2.5 Scoped Reflection Lookup and Record-only History

M1.2.5 让 `get_memory(record_type = Reflection)` 接受 opaque persisted reflection ID，并以 `limit = 1` 复用同一 scoped search。归属与 M1.1.4 相同：必须存在同 scope superseded Claim，replacement 为空或同 scope。missing、cross-scope 与 record-only 都返回 `record: null`，三者不可区分。record-only 行没有 Claim anchor，因此不能进入 scoped lookup 或 history；本片不新增第二条 history 工具。

## 2026-08-13 M1.2.6 Identity and Commitment History

M1.2.6 新增第 9 个 MCP 工具 `get_self_model_history(namespace, history_type, limit?)`。`history_type` 为 Identity 或 Commitment；limit 默认 20、范围 `1..=100`，并用 `has_more` 表示截断。结果来自现有 reflection 审计列，只包含能通过同 scope Claim 归属的行。record-only identity/commitment 更新保持不可见。该首片没有 schema migration、新写路径或 rollback，也不把现态 `identity_claims` / `commitments` 表变成版本账本。

## 2026-08-13 M1.2.7 Audited Supersede Contract

M1.2.7 新增第 10 个 MCP 工具 `supersede_memory(namespace, claim_reference, replacement_claim, replacement_evidence_event_ids, summary)`。它先按显式 namespace 校验 target Claim 与 evidence 都在同一 scope，再调用既有 `run_reflection::execute`。旧 Claim 被标为 `Superseded` 并留下 reflection 审计，默认不 hard delete。missing / cross-scope Claim 或 evidence fail closed；replacement 必须留在请求 namespace。该首片没有 schema migration、第二条 durable write path、identity/commitment 更新或 Event/Episode/Reflection 纠错。

## 项目定位

当前仓库更准确的定位是：

“MCP Memory Ledger 的 local-first memory MVP / technical demo，包含受限的 evidence-gated self-revision 实验能力”

它已经完成了工程闭环、本机接入闭环，以及受治理的 automatic self-revision MVP；但还没有完成原始设计里更完整的“自我机制”产品语义，也不能对外包装成完整自治系统。

## 已实现

### 1. 最小运行链路

当前最小链路已经打通：

`events -> claims -> self_snapshot -> decision -> reflection`

对应到 MCP 工具层，当前可用的 10 个工具是：

- `ingest_interaction`
- `search_memory`
- `get_memory`
- `get_reflection_history`
- `get_self_model_history`
- `get_evidence_relation`
- `supersede_memory`
- `build_self_snapshot`
- `decide_with_snapshot`
- `run_reflection`

### 2. SQLite 持久化

- 已使用文件型 SQLite
- 支持跨重启保持数据
- `AppConfig::load()` 的默认启动路径可通过 `AGENT_LLM_MM_DATABASE_URL` 显式覆盖数据库位置
- `AppConfig::load_from_path()` 会保留显式文件里的 `database_url`
- 若显式文件省略 `database_url`，`load_from_path()` 仍会通过 `AppConfig::default()` 继承环境变量派生出来的默认数据库路径

### 3. `namespace` 最小闭环

当前已支持：

- `self`
- `world`
- `user/<id>`
- `project/<id>`

并且这些语义已贯穿：

- domain 校验
- SQLite 约束
- MCP DTO 输入
- event / claim / trigger ledger 持久化；legacy event rows 会按 owner 规则回填 namespace

### 4. 反思最小正向路径

`run_reflection` 已支持：

- 审计友好的 supersede / dispute 行为
- 显式 `replacement_evidence_event_ids`
- 一套窄化的结构化 `replacement_evidence_query` 首批能力，支持 namespace / owner / kind / limit 过滤，空查询结果仍返回 `invalid_params`
- 一条最小可用的 `identity_core` / `commitments` 深层修订路径，并把 supporting evidence 与请求的更新内容写入 reflection 审计记录
- `replacement_evidence_event_ids` 同时接受裸 ID 与 `event:<id>`；空白、空 ID 和重复 `event:` 前缀返回 `invalid_params`，显式/query 合并按底层 raw ID 保序去重
- 缺失 evidence event id 时返回 `invalid_params`；审计 `supporting_evidence_event_ids` 因字段兼容性继续 readback raw IDs

### 5. 本机接入入口

- `scripts/agent-llm-mm.sh bootstrap-local`
- `scripts/agent-llm-mm.sh doctor`
- `scripts/agent-llm-mm.sh serve`
- 可作为本机 MCP 子进程被 Codex 类客户端接入

`bootstrap-local` 是 Local Alpha first-run 配置引导器：默认把 `examples/agent-llm-mm.dev.example.toml` 复制到 `agent-llm-mm.local.toml`，或复制到显式传入的目标路径。显式目标为相对路径时按仓库根目录解析；跨目录调用建议传绝对路径。它拒绝覆盖已有配置，父目录不存在时拒绝继续，不生成 secret，不运行 `doctor`，不启动 `serve` 或 daemon，也不代表安装包、远程 bootstrapper、GA 或 production-ready 能力。

`first-run-bootstrap-smoke-local.sh` 是 Local Alpha first-run 本地模拟证据脚本：它在不存在或为空的隔离输出目录里运行 `bootstrap-local`，把生成配置的 `database_url` 改成同目录 SQLite，再运行显式 `init` 与 `doctor --read-only` 并写出 `init.json` / `doctor.json` / `summary.json`。它会清理环境变量干扰，不写真实 HOME，不启动 `serve`，不调用 product smoke 或 demo wrapper，也不代表真实 fresh-machine install、Windows runner parity、installer、远程 bootstrapper、GA 或 production-ready 能力。

### 6. SQLite backup / restore 本地门禁

- `scripts/backup-sqlite.sh` 与 `scripts/restore-sqlite.sh` 已纳入 `tests/sqlite_backup_restore.rs` 脚本级回归测试
- 当前门禁覆盖：backup -> restore-to-new-path roundtrip、restore 拒绝覆盖已有目标、backup 拒绝 live database 目录树内备份、in-memory SQLite 拒绝、invalid percent encoding 拒绝和 `..` restore target 拒绝
- restore 仍默认写入新 SQLite 路径；正式 `database_url` 是否切换必须在 restored DB 跑过 `doctor` 后由人工决定
- 这只是 Local Alpha 本地 data lifecycle gate，不是远程备份、云同步、定时 daemon、生产灾备、admin/auth 或团队模式能力

### 7. provider adapters

- 已实现 `openai-compatible` 模型适配器
- 已实现 OpenRouter adapter 本地切片：通过 OpenAI-compatible `/chat/completions` transport 接入，覆盖 config parser、doctor、support bundle、MCP `stdio` decision path 和 self-revision path 的本地 stub 回归
- 已支持通过本地 TOML 配置文件选择 provider
- runtime 已能按配置在 `mock`、`openai-compatible` 与 `openrouter` 间切换
- `doctor` 会输出 provider / base_url shape / model，但不会泄露 API key、URL userinfo、path content 或 query secret
- `doctor.provider_matrix` 已输出当前只读 provider matrix：`mock`、`openai-compatible`、`openrouter` 为 `supported` / configurable；`azure-openai`、`local` 为 `planned-only` / not configurable，并列出缺失的 config parser、doctor diagnostics、model adapter、error handling、redaction 和 MCP stdio tests
- planned-only provider 仍会被配置解析拒绝，不能被当成已实现 adapter
- OpenRouter 当前不代表真实 OpenRouter live provider 已认证，也不是 provider gateway

### 8. automatic self-revision MVP

- 已新增 `self_revision` 领域契约，包含 trigger type、proposal rationale 和 machine patch 最小结构
- `ModelPort` 已支持 `propose_self_revision`
- `mock` 与 `openai-compatible` adapter 已实现最小 proposal 行为
- proposal 首阶段已支持 `proposed_evidence_event_ids`、`proposed_evidence_query` 与 `confidence`；这些字段当前用于收口证据候选与置信度，其中 `proposed_evidence_query` 在 explicit ids 为空时可作为 bounded narrowing hint，对当前 trigger window 做交集收口，并在有交集时按当前窗口内的候选顺序应用 `limit`；project / user scoped conflict 与 periodic trigger window 会先排除 sibling namespace 事件；`recorded_after` / `recorded_before` recency window 已按 inclusive 边界参与过滤；若没有交集，不再绕过 query 改用 full trigger window。proposal explicit IDs 与 query 结果都用 `EventReference` 解析为 raw ID 后再和冻结 trigger window 比较，裸/前缀表示不会重复或绕过候选边界；explicit ids 非空时，这些 ids 也必须满足 query 在当前 trigger window 内的过滤约束，但不代表 richer widening / ranking engine 已落地
- 已新增 trigger ledger 持久化，能记录 handled / rejected / suppressed 结果、episode watermark 和 cooldown，并通过 structured diagnostics 暴露 `trigger_type`、`namespace`、`trigger_key`、outcome、rejection / suppression reason、`cooldown_state`、cooldown boundary、evidence window size 与 selected evidence ids
- 已新增 `auto_reflect_if_needed` 协调器，负责 trigger 判定、proposal 请求、治理校验和写入前收口
- 当前 MCP-wired automatic path 已谨慎扩到 4 条：`ingest_interaction -> failure`、`ingest_interaction -> conflict`、`decide_with_snapshot -> conflict`、`build_self_snapshot -> periodic`
- `ingest_interaction` / `decide_with_snapshot` / `build_self_snapshot` 上的 best-effort auto-reflection 失败都不会把已经成功的 MCP 主路径改写成额外的 MCP 错误
- 通过治理的 automatic self-revision 最终仍复用 `run_reflection` 作为 identity / commitments 的 durable write path
- 直接 `run_reflection` MCP tool 不会递归触发 auto-reflection

当前 runtime hook contract matrix 如下（本表为这 4 条 runtime hook contract 的权威单一来源，README 与 testing-guide 引用本表而不另行复制，避免漂移）：

| Hook | Trigger Input | Runs When | Does Not Do |
| --- | --- | --- | --- |
| `ingest_interaction:failure` | repeated or explicit failure signal | after successful ingest path | does not turn successful ingest into MCP error if best-effort reflection fails |
| `ingest_interaction:conflict` | explicit `trigger_hints` containing `conflict` or `identity` | after successful ingest path | does not infer conflict from arbitrary text alone |
| `decide_with_snapshot:conflict` | explicit `auto_reflect_namespace` and conflict-compatible `trigger_hints` | after non-blocked decision | does not run when commitment gate blocks the decision |
| `build_self_snapshot:periodic` | explicit `auto_reflect_namespace` | during snapshot build with periodic policy | does not create a background scheduler |

Implementation notes:

- `ingest_interaction:failure` currently means `failure` or `rollback` trigger hints plus the failure evidence threshold.
- `ingest_interaction:conflict` accepts only conflict-compatible hints (`conflict` or `identity`); arbitrary conflict-looking text with no hints or unrelated hints such as `commitment` does not start this hook.
- `build_self_snapshot:periodic` is part of the snapshot tool flow, but the best-effort reflection attempt runs before `build_self_snapshot::execute`; it is not a scheduler.

这代表“自动 self-revision MVP”已经存在，但它仍然是受限、保守、局部接线的 demo 能力。

### 9. self-revision demo package

- 已新增 deterministic `openai-compatible` stub provider binary
- 已新增 demo runner binary，复用真实 MCP `stdio` 服务和原有 4 个写入/快照/决策/反思工具跑 canonical scenario；后续新增的 `search_memory` / `get_memory` / `get_reflection_history` 不在该旧 demo story 内
- 已新增 macOS shell wrapper：`./scripts/run-self-revision-demo.sh`
- 运行后会生成 `doctor.json`、snapshot before / after、decision before / after、timeline、SQLite summary 和 Markdown report
- 该 demo 只证明当前 MVP 的可重复证据链，不新增 MCP tool、daemon、Web UI 或新的 durable write path

### 10. Local read-only dashboard service

- 已支持通过 `[dashboard]` 配置随 `serve` 启动只读 HTTP 面板
- 面板展示运行时 operation 事件，并保持 MCP `stdio` 输出不被污染
- HTTP surface 只注册 GET route；写方法会返回 `405 Method Not Allowed`，dashboard route 不调用 `run_reflection`
- 当前 UI 为 `Memory-chan Live Desk`，使用内嵌生成图物料与 CSS 装饰复刻清新活力二次元观测面板
- 生成图物料位于 `src/interfaces/dashboard/static/`，版权/归属说明已记录在 `NOTICE`
- 当前 dashboard event recorder 仍是 bounded in-memory recorder，不是 durable operation-log database
- MCP tool 调用已生成 `mcp-tool-call-<uuid-v4>` correlation id；dashboard 成功/失败事件、auto-reflection 诊断事件和 detail projection 会保留该 id
- 已知 MCP tool 调用在 object-shaped arguments 进入项目 handler 后，成功与 handler-reached 失败都会追加 tool-level `operation_log` 元数据，包含 entrypoint、status、namespace、correlation id 和受限摘要；framework-level 解析/路由失败不在该 handler-level 记录范围内；失败路径记录不改变 MCP error code / error message 语义
- dashboard 已提供本机只读 `GET /api/operation-log` 与 `GET /api/operation-log/{id}` durable history JSON API，可按 `limit`、`namespace`、`kind`、`correlation_id` 做受限查询；history 列表默认最多返回 100 条，单次查询最大 100 条
- correlation id 与 operation-log 元数据仅用于观测排障，不替代 `run_reflection`，也不写 identity / commitments / reflection audit

### 11. Local support bundle generator

- 已新增首版本机支持包生成入口：`./scripts/generate-support-bundle.sh <output_dir> [config_path] [--log-file <path>] [--correlation-id <id>]`
- 生成器输出 `manifest.json`、`doctor.json`、`config-shape.json`、`operation-summaries.json`、`release-metadata.json`、`product-smoke-summary.json` 和 `local-log-excerpts.json`
- `doctor` / config 只保留脱敏 shape：SQLite URL 会泛化为 `sqlite://<local-path>`，provider credential 只输出布尔值，provider URL 会移除 userinfo、path content 与 query；support bundle 的 `doctor.json` 不执行 runtime bootstrap
- operation summaries 通过 read-only SQLite 连接读取最多 25 条 durable operation-log metadata，不输出 request / response / diagnostic payload summary；user/project namespace 只输出 shape，secret-like operation id / correlation id 会替换为 `<redacted-metadata>`；可显式传入生成型 `--correlation-id mcp-tool-call-<uuid-v4>` 只导出匹配 correlation id 的 metadata，并在 `operation-summaries.json.filter` 记录过滤条件；数据库或 `operation_log` 表不存在时会标记 unavailable，不创建或迁移数据库
- 非生成型、非 canonical 或非 v4 的 correlation id filter 会在创建 bundle 输出目录前被拒绝，避免把 secret-like 文本写进诊断包 metadata
- local log excerpts 只在显式 `--log-file <path>` 时生成 bounded / redacted 摘要，secret-like config/log 文件名会折叠成 `<local-path>/<redacted-name>`；不自动扫描日志目录、home、系统日志、browser profile、SSH/cookie/session、shell history 或 `target/` 输出，也不复制原始 `.log` 文件
- 默认不复制完整 SQLite 数据库、不包含 raw TOML、不包含 provider payload、不上传数据，也不新增 identity / commitments / reflection 的 durable write path
- `manifest.json` 会输出 `safety_checks`，明确记录 read-only、未 runtime bootstrap、未包含 SQLite/TOML/raw log/provider payload
- local support bundle 仍只是 Local Alpha 诊断辅助；它不代表生产支持通道、远程上传能力、observe-only daemon diagnostics 或 Local Alpha 完整 gate 已完成

### 12. Local Alpha evidence summary

- 已新增本地只读证据汇总入口：`./scripts/local-alpha-evidence-summary.sh`
- Rust 入口为 `src/bin/local_alpha_evidence_summary.rs`，核心汇总逻辑在 `src/support/local_alpha_evidence.rs`
- 汇总器读取已有 Local Alpha evidence：product smoke `latest` 目录、first-run bootstrap `summary.json`、Windows parity `summary.json` 和 support bundle 目录；`first_run_simulation` gate 只表示本地首启模拟证据，真实 fresh-machine 仍由 `first_run_bootstrap` gate 单独阻断
- 真实 fresh-machine summary 现在需要明确 real evidence kind、`fresh_machine_simulation = false`、`captured_at`、`source_checkout`，以及带显式 `exit_code = 0` 的成功 `bootstrap-local` / `doctor` command evidence；Windows parity summary 现在需要 `windows_runtime_parity` kind、`captured_at`，以及带显式 `exit_code = 0` 的成功 bootstrap / doctor / product-smoke command evidence。缺少这些外部证据 metadata 时，对应 gate 会保持 `open` / `not_verified`
- 输出 JSON 包含 `overall_status`、`local_only`、`summary_boundary` 和 gate 列表；每个 gate 至少包含 `name`、`status`、`evidence_path` 或 `reason`
- 可选输出 Markdown，并保守声明所有 gate 都 `satisfied` 时也只是 `ready_for_human_review`，仍需要人工 release decision
- 当 real fresh-machine evidence 或 Windows parity evidence 缺失时，overall / gate status 会保持 `in_progress` / `open` / `not_verified`，不会宣称 Local Alpha 完成
- 该能力不启动 `serve`，不运行 product smoke，不上传文件，不触发 daemon 写，也不新增 durable write path；`run_reflection` 仍是唯一 durable identity / commitment / reflection 写路径

### 13. Observe-only daemon diagnostics

- `doctor` 已输出 `daemon_observe_only` 本机只读诊断字段
- 该字段固定声明 `mode = "observe_only"`、`local_only = true`、`write_gate_approved = false`、`writes_allowed = false`、`remote_listener_enabled = false`
- 当 `[daemon].enabled = true` 时，诊断会读取本地 `operation_log` 中 `operation_kind = tool / trigger` 且 `status = failed / suppressed` 的有界候选计数，当前每个 kind/status 读取最多 25 条
- 诊断还暴露 `data_sources`、`cooldown_status`、`in_flight_task_count` 和 `read_errors`，用于本机 preflight 排查
- `DaemonHandle` 现在有本地生命周期回归：disabled handle 会快速退出，observe-only handle 可 start / stop，drop 会 abort 未停止的本地 lifecycle task；`serve` 仅在 `[daemon].enabled = true` 时启动 observe-only handle，并在 stdio service 退出后停止它；`writes_allowed = false`、`remote_listener_enabled = false`
- 这不是 daemon 写能力：observe-only lifecycle tick 不调用 `run_reflection`，不写 identity / commitments / claims / events / reflections，也不启动 remote listener 或代表后台自治 / Local Alpha 已完成；`doctor` 只做 preflight 诊断，不启动 daemon loop

### 14. Productization follow-up reality gates

- 已新增追踪文档：`docs/product/follow-up-reality-gates.md`
- 该文档把后续产品化模块按 `implemented`、`partial`、`simulation-only`、`planning-gate`、`blocked claim`、`not-implemented` 等状态拆开
- 它明确标出 Local Alpha release gate、fresh-machine first-run、Windows parity、daemon lifecycle、remote/team/auth/security、release engineering 和 multi-layer memory 等模块中仍带假设或缺 fresh evidence 的部分
- 它不是新的产品能力声明，而是二次跟进追踪入口；后续每个模块只有在代码、测试、文档和 fresh evidence 对齐后才能从 open 状态移动

### 15. P1/P2/P3 follow-up gate slices

- Product readiness checker 已提供候选级本地只读门禁汇总，会把真实 fresh-machine、Windows parity、release decision、remote/team、安全/auth、daemon writes 和产品措辞缺口保持为 blocked
- Release decision artifact 生成器已能写 source-only decision 模板，并在 evidence summary 仍为 `in_progress` 时拒绝 approved 决策
- `status-sync-check` 已收敛为轻量 plan/reality gate 矛盾检测；勾选完成的计划项如果没有对应 reality row、对应状态仍不完整，或 active plan 根本没有可检查的完成态 checkbox，都会 fail closed；它不再为了核对精确测试总数编译整套测试
- Support bundle manifest 已增加非 manifest 文件的 SHA-256 integrity 列表；daemon observe-only diagnostics 已输出 write/remote blockers
- `decide_with_snapshot` response envelope 使用 `protocol_version = 2`，已有 `decision_id`、requested/selected action、bounded local confidence metadata、policy checks 和 non-claims；M0.3.4 additive 增加 `decision_authority` 与 `policy_scope`，同时保留旧 `blocked` / `decision` / `status` 字段和 provider action-string contract
- Evidence relation read model 已能只读展示 trigger window 内 selected evidence、available-not-selected rows、rejected count、relation status、window rank、rejection reason、bounded binary selection weight 和 no-widening policy；trigger window 与 selected `*_event_ids` 同时接受裸 ID / `event:<id>`，先解析为 raw ID、保序去重后执行 subset/no-widening 与 count/rank；JSON `event_id` readback 保持 raw ID。`doctor.system_layer_report.evidence_relation_contract` 同步公开 v2 contract、read-only/no-widening/binary-weight policy、allowed status、selected/unselected weight、rejection reason 和 additive v2 字段；它不拉取 trigger window 外证据，也不是完整 ranking / scoring engine
- Episode summary projection 已能以只读 local metadata 表达 objective、outcome、linked evidence ids，不写 identity 或 commitments；episode / linked `*_event_ids` 同时接受裸 ID / `event:<id>`，按 raw ID 保序去重后执行 subset 校验与 `event_count`，JSON `linked_evidence_ids` readback 继续保持 raw ID
- Provider matrix planned-only 行已输出 missing implementation checklist，避免把 future provider 当作可配置 adapter
- `doctor` 已输出 `remote_team_capability_inventory` 与 `remote_team_security_gates`，所有 remote/team 能力和 security/auth 前置门禁仍默认 blocked，support bundle upload 为 false
- `doctor.system_layer_report` 已输出只读 architecture layer summary，八层固定为 substrate / signal / memory / policy / control_loop / actuator / interface / release_boundary；报告本身 `writes_performed = false`，actuator 仍只锚定 `run_reflection` 且不新增写授权；`evidence_relation_contract` 将 evidence semantics v2 的 no-widening / bounded binary selection metadata 暴露为 doctor-level 机器可读契约；dependency rules 现在带 `status`、`grants_capability = false` 和逐项 evidence，其中 daemon / remote-write 等当前状态使用 `runtime` evidence，release-boundary / memory-write 等边界使用 `declared_test_contract` evidence 并标为 `declared-test-contract`，只公开验证命令而不声称 doctor 已运行测试；physics principle mappings、Phase 0-8 coverage 和 non-claims 均为报告/门禁信息，不是 runtime、solver、controller 或 scientific validation 能力
- Multi-layer memory projection 已提供只读分层状态：working / episodic / semantic / self-model 为 partial，procedural 为 not implemented；不新增 durable self-model 写路径
- Product wording guard 已接入 product readiness，阻断 Beta、GA、production-ready、remote/team、remote write admin、complete self-governance、physics-informed runtime、solver/controller 和 scientific validation 等缺少 gate 的候选措辞

## 部分实现

### 1. `decide_with_snapshot`

- commitment gate 是真实能力
- 下游模型调用已可走 `openai-compatible` 或 OpenRouter
- 当前返回 envelope 已有 `protocol_version = 2`、`decision_id`、requested/selected action、bounded local confidence metadata、policy checks、non-claims 和 commitment-gate metadata
- 返回的 provider action string 显式标记为 `experimental_non_authoritative`，policy scope 只覆盖 `server_commitment_gate_only`；允许结果不等于完整 policy passed
- 原有 `blocked` / `decision` 字段保留，`decision` 内仍是最小 `action` 字符串

因此它更适合作为最小决策闭环和集成验证能力，而不是完整决策引擎。

### 2. provider 生态

- 当前 provider 边界已经抽出来
- 仓库内目前实现了 `mock`、`openai-compatible` 与 `openrouter`
- 当前 provider matrix 只是只读合同和 doctor 诊断；它不会让 `azure-openai`、`local` 变成可运行 provider
- OpenRouter 支持范围仍限于 OpenAI-compatible chat completions transport 的本地验证，不是 live-provider 认证

### 3. `self_snapshot`

- 当前有统一 `SnapshotBudget`
- 显式 `namespace` 会生成 server-owned `MemoryScope`；调用方不能单独伪造 owner
- claims / event references / episode references 已按精确 owner + namespace 在 query port / SQLite 层过滤
- explicit `evidence_manifest` 要求同时显式传入 `namespace`，并已支持裸/前缀 ID 等价输入、canonical `event:<id>` 输出和 scope intersection/no-widening
- explicit `recorded_after` / `recorded_before` 要求同时显式传入 `namespace`，并已对 evidence / episodes 使用 inclusive time intersection 与 recent-first 稳定 SQL 排序
- automatic reflection 会从触发 namespace 派生完整 owner + namespace scope，把当前候选固定为 explicit manifest，并以候选最早/最晚记录时间约束其 snapshot 与 episode read；无合格候选不会回退到历史全量
- 省略 `namespace` 仍只保留 legacy unscoped、无时间窗的兼容路径；需要隔离的调用方必须显式传入 scope
- 当前 budget 主要控制 evidence 数量

它还没有完成 repository-wide event ID 统一：本轮已覆盖 active reflection runtime、只读 evidence/episode projections，以及 offline demo 的外部 `timeline.json` baseline `event_reference`；demo snapshot evidence 原本已是 canonical reference，stub 只产生空 `proposed_evidence_event_ids` 与 query，SQLite artifact 的 `supporting_evidence_event_ids` 继续保持 raw 兼容。support bundle 仅记录为后续 inventory，未在本轮修改；它也不是对 `identity / commitments / claims / episodes` 分层预算的完整模型。claims 当前没有时间列，因此 snapshot time window 不会伪装成 claim recency filter。

### 4. `episodes`

- 当前主要是 `episode_reference -> event_id` 级别的轻量聚合

它还不是带 `goal / outcome / lesson / self_effect` 的完整自传式建模。

### 5. `identity_core` / `commitments` 深层修订

- 当前已经能通过 `run_reflection` 最小更新 `identity_core`
- 当前已经能通过 `run_reflection` 最小更新 `commitments`
- 反思审计会记录 supporting evidence 与请求的更新载荷
- 当前 deeper-update 输入、evidence、baseline commitment 和审计边界以 `src/application/run_reflection.rs`、`src/domain/rules/reflection_policy.rs` 及对应测试为当前事实来源；旧设计说明保存在归档分支

但它仍然只是首版收口，不是 richer schema、版本化 slow-variable 层或完整策略系统。

### 6. 默认数据库作用域

- 已可稳定落盘
- 默认语义已收口为“本机用户共享的持久化默认库”
- 若需要按项目、按环境或按实验隔离，应显式配置不同的 `database_url`
- 首次使用先执行 `init`；旧库先执行 `doctor --read-only`，再显式执行 `migrate`。`serve` 不再隐式 bootstrap。
- release soak 会用 `AGENT_LLM_MM_DATABASE_URL` 强制覆盖为 candidate-specific isolated database，不把传入配置中的未确认正式库路径作为写目标。

### 7. self-revision 触发面与运行形态

- 当前领域层 trigger type 已覆盖 `failure / conflict / periodic`
- 当前协调器和 ledger 也已接通这 3 类 trigger 的最小 runtime coverage
- 当前 MCP-wired automatic path 只有这 4 条，而且需要按各自边界显式触发：
  - `ingest_interaction -> failure`
  - `ingest_interaction -> conflict`
  - `decide_with_snapshot -> conflict`
  - `build_self_snapshot -> periodic`
- `ingest_interaction -> conflict` 仍要求显式 `trigger_hints` 包含 `conflict` 或 `identity`
- `decide_with_snapshot` 与 `build_self_snapshot` 仍要求调用方显式传 `auto_reflect_namespace`
- `decide_with_snapshot` 的 conflict auto-reflection 还要求显式 conflict-compatible `trigger_hints`，并且只会在非 blocked 决策后 best-effort 运行，不会改变原有 decision payload 形状
- 当前没有新增单独的 auto-reflection MCP tool，也没有后台 daemon、定时调度器或“所有入口统一自动反思”的运行形态

因此，当前仓库可以准确描述成“已实现 trigger-ledger-backed automatic self-revision MVP”，但不能描述成“完整自治 self-governing agent”。

## 未实现

- versioned identity/commitment ledger 与 record-only Reflection history
- 更广泛 crash/power-loss 和实际 fresh-machine 证据（结构读回与排他 init/migrate 已于 2026-10-09 实现）
- Event/Episode/Reflection 纠错、identity/commitment 作为 `supersede_memory` 的一部分，以及真实 MCP 客户端“记录 → 重连 → 检索 → 查看证据 → supersede → 回看历史”退出证据
- richer 自动 evidence lookup（当前 `replacement_evidence_query` / `proposed_evidence_query` 仍只是 namespace / owner / kind / inclusive recency window / limit 的窄化 evidence-oriented 查询基础；只读 relation projection 已有首片，但不是 full ranking/weighting engine）
- richer evidence weighting / full ranking engine
- `identity_core` 的 richer schema 与版本化形成机制
- `commitments` 的 richer schema、升级 / 失效策略与更细粒度生命周期
- 更多 provider 类型（Azure、本地模型网关；provider 质量、SLA 和 gateway 认证仍由 provider certification preflight 保持 blocked）
- richer `claim / episode / identity` schema
- durable working memory / procedural memory 的独立建模
- 持续后台自治运行、独立 daemon 与更完整的多层 memory 自治系统

## 当前验证状态

截至 `2026-08-25`，测试与工具链已完成分层减重；本轮 fresh 本地 CI 等价门禁已通过以下运行入口，`status-sync` 读回 26 个完成计划项与 implemented reality gates 一致：

- `cargo fmt --check`
- `git diff --check`
- `cargo check`
- `./scripts/test-tier.sh fast`
- `./scripts/test-tier.sh core`
- `./scripts/test-tier.sh full`
- `cargo test --test status_sync -v`
- `./scripts/status-sync-check.sh`
- `cargo clippy --all-targets --all-features -- -D warnings` 通过，当前静态质量基线无 warning

结果与边界：

- `fast` 只跑 lib 与 decision/domain/evidence 核心契约，服务短反馈；
- `core` 使用默认 features，覆盖默认运行时、SQLite、MCP、dashboard、doctor 与 support bundle；
- `full` 使用 `--all-features`，保留 Local Alpha evidence、release decision、product readiness、provider certification 与 packaging 的完整验证；
- 默认 Cargo 目标从 13 个二进制 / 31 个集成测试目标收敛为 5 个二进制 / 24 个集成测试目标；完整目标没有删除；
- 13 个无内部单元测试的 bin target 不再生成空 test harness；依赖实际二进制的 MCP、demo、Local Alpha 与 provider runner E2E 仍通过；
- `status-sync-check` 不再编译并枚举整套测试，只检查非空的 active-plan 完成态 / reality gate 对齐与根目录 SQLite fixture；
- 文档不再复制易漂移的测试总数和 suite count。

以下 product smoke、doctor、support bundle 和 release evidence 已在 M0.4 收口中重新生成或由当前测试验证：

- `doctor` 返回 JSON，且 `status = ok`
- self-revision demo package 生成 release gate 要求的 8 个核心 artifact，并证明 before / after decision shift
- Local Alpha product smoke 通过 staging / promote 流程刷新 `target/reports/self-revision-demo/latest`
- Local Alpha support bundle 生成允许的 JSON 文件，敏感词扫描无未脱敏命中，且未包含 `.sqlite`、`.toml` 或原始 `.log` 文件
- Local release soak runner 生成 source-only `compatibility-matrix.json` 和 `release-boundaries.json` blocker artifacts；它们记录边界，不生成真实 Windows / fresh-machine / daemon write / release approval 证据
- Local release soak runner 可生成 `target/reports/releases/<candidate-name>/` 候选证据，先在 `target/release-soak-runtime/<candidate-name>/` 显式初始化隔离数据库，再让 read-only doctor、product smoke 和 support bundle 使用该覆盖路径；`release-boundaries.json.database_isolation` 明确记录隔离开启且不接受正式库路径。它仍不生成 Windows runner、真实 fresh-machine、remote/team、daemon writes、上传、tag、安装包或发布认证证据
- Release evidence index 可把候选 evidence root 下的 Local Alpha evidence summary 与 product readiness gate 合并为 present / missing / not_verified / blocked 的本地只读索引；它只输出 JSON/Markdown，不生成缺失 evidence、不批准 release、不上传文件
- Provider certification preflight 可校验本地 provider config shape 并列出 live evidence preflight 缺口；显式 live evidence runner 只生成 provider preflight 可读取的 evidence files，配置示例本身不是 live evidence，必须显式运行 runner 才能生成 live evidence；runner 用于记录本次配置下的 endpoint reachability、decision probe、self-revision parse probe、错误处理和 redaction review provenance，不输出 API key、URL userinfo、path secret、query secret、model id、request body、response body 或 provider-native payload；live evidence 只有在 provider、status、expected evidence_kind、`mode = live`、非空 `generated_at`、`local_only = false`、`endpoint_reached = true`、`redaction_reviewed = true`、`request_outcome = passed` 和带显式 `exit_code = 0` 的成功 command evidence 同时满足时才算 present；stub/simulated runner 只能生成本地模拟证据，不能让 `live_certified = true`；即便 preflight 输出 `live_certified = true`，也只表示 config preflight 通过且四类 live evidence present，不代表 provider 输出质量、SLA、provider gateway、Local Alpha / Beta / GA / production-ready、production readiness 或 release approval
- Packaging preflight 可区分 source-only soak artifacts 与真实 binary archive / installer / service manager / auto-updater evidence；新增 archive manifest / SHA-256 evidence 生成入口只校验已存在 archive 是否可解析为对应 `.tar.gz` / `.zip`、文件大小和 SHA-256，不构建二进制；缺少真实打包证据、纯文本占位、截断 archive、零字节占位、部分平台 archive、缺失 manifest 或 manifest mismatch 时保持 blocked，不创建 tag、安装包、上传或发布认证证据
- Richer memory semantics projection 已提供只读 evidence relation / episode summary / semantic claim / procedural memory / durable self-model write 状态投影；它不新增 durable memory layer 写路径，也不是 full ranking engine
- `first-run-bootstrap-smoke-local.sh` 已提供 `bootstrap-local -> init -> doctor --read-only` 的本地 fresh-machine simulation evidence，包含 env 隔离、输出目录隔离、`init.json` / `doctor.json` / `summary.json` 和 isolated SQLite 证据；但真实 fresh-machine install / Windows runner 实机验证仍需单独记录，PowerShell runtime parity 仍只能视为待补证据
- SQLite backup / restore 本地脚本门禁已覆盖 roundtrip、拒绝覆盖、拒绝 live DB 子目录备份、拒绝 in-memory / invalid URL 和拒绝 `..` restore target；这不是远程备份、云同步或生产灾备证明

## 对外描述建议

如果要给协作者一句话描述当前项目，建议用下面这个口径：

“这是一个 Rust 编写的本机 MCP `stdio` memory demo，已经打通事件写入、快照构建、最小决策门控、反思修订，以及由 trigger ledger、证据门槛和慢更新约束保护的 automatic self-revision MVP；但它仍然是本地 demo，不是完整自治代理系统。”
