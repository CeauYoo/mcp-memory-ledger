# MCP Memory Ledger 路线图

状态：`active summary`
更新日期：`2026-10-09`

本文件只回答三个问题：现在做什么、接下来做什么、哪些方向暂不排期。具体任务、依赖、证据门与停止条件统一见 [2026-07-10 全新项目规划](plans/2026-07-10-product-replan.md)。

[正式化改进与主线同步计划](formalization-improvement-plan-2026-08-25.md)补充产品、工程、安全、发布和 GitHub 主线治理的跨领域差距矩阵；它不改变 `M1 -> M2 -> M3` 的依赖顺序，也不是第二份执行队列。

## 产品北极星

把现有 technical MVP 收束成一个可信、可检索、可审计、可恢复的本地 MCP Memory Ledger。

所有路线选择先服从[项目起点与主线原则](origin-and-principles.md)：原始事件保真、高层记忆可追溯、scope 隔离、历史可纠错、反思受治理。不能加强这些原则的能力默认不进入当前主线。

第一产品闭环是：

```text
记录 -> scoped 检索 -> 查看证据 -> 纠错/撤销 -> 备份恢复
```

不是：

```text
自动决策 -> 后台自治 -> 远程团队平台
```

当前仓库仍是 local-first technical MVP，不是 Local Alpha、Beta、GA 或生产级自治平台。

## 2026-10-09 原方案续建

最终有限收口包括 D09 丰富 Episode 证据视图、D10 可观察状态诊断、C09 caller-owned 次数/停止合同与 M01–M03 内部模块拆分；逐项状态与真正外部门见[最终 70 项核对](plans/2026-10-09-original-plan-final-reconciliation.md)。不把真实模型收益、用户机器安装或发布批准自动标为完成。

继续实施原文明确的数据模型要求：schema v6 时间键、独立 Reflection origin/effect 与安全 targetless history、只读 scoped export。未知历史保持未知；不以“迁移复杂”作为权限阻塞。模块拆分在该阶段验证后独立提交，避免与数据语义变化混淆。

本轮按原 A–E 讨论推进反馈候选、确定性支持合同、派生全文索引、离线对照/容量证据和有界经验候选。运行时边界与可复现入口见 [A–E 指南](memory-feedback-experience.md)；全量要求和遗留项见 [原文追踪矩阵](plans/2026-10-09-original-plan-traceability.md)。生命周期基础已通过后前移这组有限切片的理由写入唯一 active plan；不将 M2/M3 真实产品与模型效果门自动标成完成。

## 当前事实（历史切片说明；新 v5 入口见上）

已实现：

- Rust MCP `stdio` 服务与 12 个工具；`search_memory` 已覆盖显式 namespace 的 Event、Claim 与 scoped Episode / Reflection provenance recall 首片，`get_memory` 已覆盖 Event、Claim、Episode 与 scoped Reflection lookup 首片，`get_reflection_history` 已覆盖 exact scoped Claim revision chain 首片，`get_self_model_history` 已覆盖 scoped identity/commitment revision audit 首片，`get_evidence_relation` 已覆盖 scoped evidence-relation runtime 首片，`supersede_memory` 已覆盖 scoped Claim audited supersede 首片；
- SQLite 持久化与 ingest / reflection 事务；
- `run_reflection` 受治理的 durable write path；
- mock、OpenAI-compatible、OpenRouter 配置路径；
- 本地 dashboard、doctor、support bundle、backup / restore 和 release preflight；
- 测试已分为 `fast` / `core` / `full` 三级；发布证据、打包和 provider certification 工具由非默认 `release-tools` feature 承载。

当前不能视为完整产品能力：

- 显式 namespace / manifest / time-window 的 M0.2 scoped snapshot 已收口，但省略 namespace 的 legacy 兼容调用仍未隔离，且尚无完整 recall read model；
- 已有正式的 Event / Claim `search_memory` 与 `get_memory` 首片、Episode / Reflection `search_memory` provenance 首片、Episode / scoped Reflection `get_memory`、跨类型 `record_types` union、Claim-linked `get_reflection_history` 首片、scoped `get_self_model_history` 首片、scoped `get_evidence_relation` 首片，以及 scoped `supersede_memory` 首片；versioned identity/commitment ledger 与 record-only reflection history 仍未实现；
- M1.0 三项 scope/data-integrity 前置门已通过：identity evidence-to-Episode 计数同时限制 Claim/Event scope，Claim search/get 对 mixed-scope revision edge 整边隐藏，新写入拒绝 Unknown owner 且只读 doctor 盘点 legacy Unknown 行；
- decision 已改用服务端 commitments，并同时 gate requested / provider-selected action；允许的 action-string 结果显式标记为 `experimental_non_authoritative`，且 policy scope 只覆盖 server commitment gate。但其他 snapshot 字段仍由 caller 提供，尚无完整 trusted snapshot handle 或 provenance binding；
- cross-episode identity support 已使用 claim → evidence → episode 的真实 distinct join，并在 M1.0.1 绑定完整 `MemoryScope`；跨 namespace evidence link 不再改变 identity revision 判断，但仍没有完整 provenance graph；
- M0.4 已完成：`init` / `migrate` / 默认只读 `doctor` / 显式 `doctor --allow-bootstrap` 已拆分，`serve` 不再隐式改库；
- SQLite 使用 schema version 与 migration ledger；legacy migration 先建立 backup anchor 和 restore rehearsal，再在事务中执行 FK / 表行数 readback；
- dashboard 无认证，但启用时配置已强制 localhost / loopback；remote/public exposure 仍未实现；
- daemon 仍是 observe-only idle lifecycle；
- memory layer / episode-summary 等投影尚未形成统一 runtime read path；evidence-relation 已有 scoped runtime 首片，仍不是 ranking engine；
- 已有 Linux/macOS GitHub Actions 与固定 Rust toolchain；仍没有真实二进制交付或 fresh-machine / Windows 完整证据。

## Complete — M0 Truth and Safety Reset

目标：在继续扩功能前，修正产品可信性和数据安全边界。

工作流：

1. 统一 active plan、状态与证据入口，清理仓库卫生问题。
2. **已完成（M0.2，2026-07-14）**：建立 `MemoryScope`，让显式 scoped snapshot 和 auto-reflection 只读取允许的 namespace / trigger window，并使用 recent-first 稳定排序；完成 active reflection、只读 evidence/episode projection 与 offline demo artifact 的限定 event-ID 收口。省略 namespace 的 legacy unscoped 兼容、repository-wide ID 统一和 support bundle inventory 不计入该退出门。
3. **已完成首个切片（M0.3，2026-07-14）**：decision 使用服务端 commitment store 覆盖 caller commitments，同时复检 requested / provider-selected action；被拒绝的 selected action 不返回 authoritative decision payload。
4. **已完成第二个切片（M0.3，2026-07-14）**：用 claim → evidence → episode 的 distinct store join 替换全局 episode 数量推断，无关 episode 不计入 identity support。
5. **已完成第三个切片（M0.3，2026-07-14）**：验证治理拒绝、handled-ledger 写入失败与 transaction commit 失败均不留下部分 identity / commitment / reflection 更新，失败后只保留 rejected audit。
6. **已完成第四个切片并收口（M0.3，2026-07-14）**：v2 envelope additive 返回 `decision_authority = experimental_non_authoritative` 与 `policy_scope = server_commitment_gate_only`，保留 legacy 字段和 provider action-string contract，不把有界字面量 gate 误报为完整 policy passed。
7. **已完成（M0.4，2026-07-14）**：拆分显式数据库生命周期；只读 doctor 对 missing / old / read-only 路径不 create / migrate / seed；migration ledger、事务、备份、恢复演练、FK / 行数读回和 release-soak 隔离库均有回归证据。
8. **已完成（M0.5，2026-07-14）**：无认证阶段强制 dashboard loopback；CLI tracing 固定写入 stderr。
9. **已完成（M0.5，2026-07-14）**：Linux/macOS CI 执行 format / all-feature Clippy / full tests / status sync。
10. **已完成（M0.5，2026-07-14）**：固定 Rust `1.95.0` 与 `rust-version = 1.95`。
11. **已完成 spike（M0.5，2026-07-14）**：`rmcp 0.5.0` → `2.2.0` 直接升级 no-go；后续迁移必须独立且不得引入 remote/tasks/OAuth。

退出门：

- namespace leakage = 0；
- recent-first snapshot 顺序稳定；
- 模型返回受禁 action 时仍被阻断；
- `doctor --read-only` 前后 DB checksum / schema / row count 不变；
- migration 故障后可恢复且行数一致；
- 非 loopback dashboard 配置被拒绝；
- clean clone CI workflow 覆盖 Linux/macOS，且同一组固定门禁在本机通过。

M0 未通过前，不开始新 provider、daemon 写能力、remote/team 或正式发布扩张。

## Active — M1 Trustworthy Recall

目标：完成用户真正需要的本地记忆闭环。

已完成十六片（截至 2026-08-13）：`M1.0.1`–`M1.0.3` 三项 Scope/Data-Integrity Gates、`M1.1.1` Event / `M1.1.2` Claim / `M1.1.3` Episode / `M1.1.4` Reflection scoped search、`M1.1.5` scoped evidence-relation runtime、`M1.1.6` 跨类型 union，以及 Event/Claim/Episode/Reflection lookup、Claim reflection history、identity/commitment revision audit 与 scoped Claim audited supersede。Event / Claim search 和 lookup 继续使用显式 namespace、SQL scope-first filtering、provider-free read 与跨 scope empty/null；Episode search 与 lookup 以同 scope Event membership 投影 Episode，并把 persisted reference 当作 opaque exact ID。Reflection search 与 lookup 只通过同 scope Claim 端点归属，隐藏 mixed-scope edge，并排除 record-only 行。additive `record_types` 在同一 scope 内合并已稳定的四类 tagged record，并按 recorded_at / type / id 收口。第 8 个 MCP 工具 `get_evidence_relation` 把既有只读 projection 收敛为 scoped trigger-window ∩ selected-subset 合同。第 10 个 MCP 工具 `supersede_memory` 把 Claim 纠错收敛为显式 namespace + evidence 的 `run_reflection` 门面，默认不 hard delete。完整 M1、versioned identity/commitment rollback ledger 和真实客户端退出门仍开放；v6 已新增安全归属的 record-only history。

计划能力：

- M1.0 前置门、四类 scoped search、evidence-relation runtime、跨类型 union、四类 lookup、identity/commitment revision audit 与 scoped Claim audited supersede 已通过；current-schema structural readback、exclusive lifecycle、可靠更正/重试、有界反馈、本地literal recall与context已实现；下一步精确提交的平台/用户闭环验证；
- 已完成 scoped Episode / Reflection provenance search、evidence-relation runtime、跨类型 union、Episode / Reflection lookup、self-model history 与 Claim supersede 首片；current-schema structural readback、exclusive lifecycle、可靠更正/重试、有界反馈、本地literal recall与context已实现；下一步精确提交的平台/用户闭环验证；
- `search_memory`；
- `get_memory`：当前 Event / Claim / Episode / scoped Reflection；安全归属的 record-only Reflection 已有 v6 读取；unknown/mixed 历史仍隔离；
- `get_reflection_history`：已完成 Claim-linked 首片；Claim history 仍沿修订链；独立 record-only 通过 v6 scoped search/get 读取；
- `get_self_model_history`：已完成 scoped identity/commitment revision audit 首片；不是 versioned ledger 或 rollback；
- `supersede_memory`：已完成 scoped Claim audited supersede 首片；复用 `run_reflection`，默认不 hard delete；
- 已完成 current-schema structural readback gate；同版本但约束被削弱的数据库不能报告 `current`；
- 保留 ID、scope、时间、status、mode 和 provenance 的返回结构；
- provider 离线时仍可用的 deterministic read path。

冻结顺序：`structural readback + exclusive lifecycle → reliable corrections/replay → feedback provenance → minimal offline recall/context → real-client closure`。

本次有界可用性候选的范围、验收与停止条件见唯一 active plan 的“2026-10-09 有界可用性候选执行批次”。最小文本召回与 context 预算因用户闭环需要从 M3 前移；全面检索调优及正式 Alpha 发布门仍不越级。

退出门：真实 MCP 客户端完成“写入 → 重连 → 检索 → 查看证据 → supersede → 回看历史”，且两个干扰 namespace 没有任何数据混入。

本次最小召回不等于完整 M3：固定双语fixture覆盖短中文词、字面符号、scope隔离和旧Claim排除；这一早期基线没有 embedding/FTS；后续已实现 FTS 与 10k/100k 合成观测，仍无生产性能承诺。完整context预算覆盖metadata，排除JSON-RPC包装。legacy direct/automatic全局self-model治理仍为experimental，不借namespace声明多用户安全隔离。

## Next — M2 Local Product Alpha

目标：从 source-only MVP 进入可验证的本地产品 alpha。

计划能力：

- 已完成 exclusive init/migration lifecycle gate；并发初始化原子争用路径，迁移持有 write reservation；失败初始化保留文件供诊断，不删除他人数据；
- 版本化 macOS binary archive 与 checksum；
- 无需 Rust toolchain 的 first-run；
- fresh-machine 核心用户闭环；
- Windows runtime parity 的真实证据或明确 unsupported 口径；
- backup / restore-to-new-path / read-only verify / manual switch；
- 一个 real provider 的连通与解析证据；
- 人工 release decision 与 rollback note。

退出门：fresh-machine 首次闭环不超过 10 分钟、namespace leakage = 0、backup / restore 100% 通过、artifact 可追溯且公开文档没有越级声明。

## Later — M3 Retrieval Quality and Lifecycle

目标：在可信读取与真实交付稳定后，提高记忆质量和长期维护能力。

- 冻结 retrieval evaluation set；
- 比较 structured query、FTS5、recency 和 relation weight；
- retention、tombstone、compaction 与 export；
- identity / commitment version、diff、effective time、rollback target；
- 将有效 projection 接入正式 read path，其余保留在 labs 或删除；
- 在可靠 scope、trigger、idempotency 和 rollback 之后再扩 automatic self-revision。

初始质量门：recall@5 ≥ 0.80、provenance coverage = 100%、namespace leakage = 0。

## Not Scheduled — M4 Remote and Autonomy

当前不排期：

- Streamable HTTP remote server；
- OAuth / authz / rate limit / tenant isolation；
- remote dashboard / admin；
- write-capable daemon、queue、retry、dead letter；
- MCP experimental tasks；
- multi-agent orchestration；
- full procedural memory / self-model autonomy。

只有 M2 稳定、真实用户需求成立、两周 dogfood 无数据丢失或 scope 泄漏，并完成独立 threat model 与 rollback runbook 后，才允许重新评估。

## 执行顺序

```text
M0 Truth and Safety
  -> M1 Trustworthy Recall
      -> M2 Local Product Alpha
          -> M3 Retrieval Quality and Lifecycle
              -> M4 Remote and Autonomy (only if separately approved)
```

每次只领取一个最小切片。测试或 readback 不通过时停在当前阶段，不以新增 preflight、projection 或文档包装替代产品能力。

## 文档入口

- 项目起点：[origin-and-principles.md](origin-and-principles.md)
- 公共定位：[positioning.md](positioning.md)
- 当前事实：[project-status.md](project-status.md)
- 唯一 active plan：[2026-07-10-product-replan.md](plans/2026-07-10-product-replan.md)
- 证据状态：[follow-up-reality-gates.md](product/follow-up-reality-gates.md)
- 正式化差距与主线同步：[formalization-improvement-plan-2026-08-25.md](formalization-improvement-plan-2026-08-25.md)
- 计划索引：[plans/README.md](plans/README.md)
- 历史归档：[archive.md](archive.md)

2026-03 至 2026-06 的历史 specs、execution plans、阶段快照和发布记录已移至 `codex/archive/pre-mainline-reset-2026-07-10`，不再作为当前下一步来源。
