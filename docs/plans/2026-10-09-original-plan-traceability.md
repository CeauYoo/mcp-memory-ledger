# 原始优化与方案合并讨论 → e14a59e 完整追踪矩阵

核验日期：2026-10-09（UTC）  
只读代码基线：`e14a59e77554891d5bfb03a88390713f07193102`  
原始输入：用户提供的《2026-10-09-agent-llm-mm-项目优化与方案合并-对话记录.md》，全文 287 行。下文 `原文 Lx–y` 指该文件；所有代码/测试路径相对仓库根目录。

## 结论与证据口径

当前提交交付的是“有界离线可用性候选”，不是 9 月合并方案 A–E 的全部实现。可信写入的主要安全目标已有实现，反馈数据合同与任务上下文也已有最小片；大部分存储演进、反馈支持性判定、全面质量评测、容量测量和经验候选尚未完成。

- **done**：该条有源码实现和对应测试/可执行验证入口。这里是静态核验结论，**本次没有执行测试、服务、模型或 CI**，不能把测试存在说成本轮通过。
- **partial**：实现只覆盖原要求的一部分，或只有小样本/模拟证据，剩余项明确列出。
- **unimplemented**：未在当前实现找到该能力；给出最近相关源码、缺失字段/接口或现有边界作为证据。
- **deferred**：当前计划明确后移；不表示原始要求已完成或被删除。
- 本报告不计算一个“完成百分比”。目标、实现建议、研究门槛和已有基础不能等权相加。
- 开始核验及最后一次基线读回（09:58 UTC 左右）checkout 为干净的 e14a59e。报告生成期间其他工作开始并行续建，出现未提交改动；本报告不追随这些变化，不把续建能力计入 e14a59e。该核验没有修改产品实现。

### 最容易误报的六个边界

1. `claim_id + expected_status` 的 CAS + `BEGIN IMMEDIATE` 已保护同一旧 Claim 只有一个替代后继；**没有**持久化 revision version 或客户端 `expected_version`。原文使用“例如 expected_version”，因此不能把没有该字段等同于现有并发保护失效，也不能将 state-CAS 描述成完整版本合同。
2. 全部 application ingest/reflection 成功写入都有同事务最小 receipt；普通 MCP operation diagnostics 仍是尽力记录。**最小成功审计已完成，完整审计产品合同尚未定义。**
3. `FeedbackMetadata` 记录 producer/target/version/expected/actual/verification/limitations，校验结构和证据 scope；**没有**通用“这个反馈支持这条替代结论”的候选/判定合同。`tool_reported` 只是 caller 声明。
4. 文本召回是 SQL `instr(lower(...))` 的 scoped literal scan；短中文已做精确子串 fixture，**不是 FTS5、中文分词或语义召回实现**。
5. `build_task_context` 实现完整 compact JSON UTF-8 bytes 的硬预算，包含 metadata；**不是 token 预算，也不包括 JSON-RPC 包装**。它没有完整冲突/过期/缺失信息解释。
6. 同一模型任务集 A/B/C + 单组件消融的**评测方法及离线执行框架**仍缺失；live-provider certification 只是接通/解析证据，付费模型质量实验是单独证据层。两者不能互相替代，也不应把“付费模型默认不调用”写成“对照实验整体延期”。

## 一、架构、范围与顺序

| ID | 原要求 / 原文位置 | 状态 | 当前源码、测试、文档证据 | 剩余边界或下一动作 |
|---|---|---|---|---|
| F01 | 保留 Rust + SQLite + MCP stdio + domain/application/ports/adapters；本地技术 MVP（L44、146、150–152） | done | `Cargo.toml`；`src/{domain,application,ports,adapters,interfaces}/`；`src/interfaces/mcp/server.rs`；`README.md:9–23` | 不需要语言/数据库替换、多 crate 或微服务。 |
| F02 | Event / Claim / Evidence / Reflection 分开、可追溯（L48） | done | `src/adapters/sqlite/schema.rs` 的 events/claims/evidence_links/reflections；`src/application/get_memory.rs`；`tests/mcp_stdio.rs:1888,2736` | 完整 provenance graph 仍未实现，勿扩大声明。 |
| F03 | 修订复用 `run_reflection`，避免第二 durable self-model 路径（L49、150、218） | done | `src/application/supersede_memory.rs:74–114`；`src/application/auto_reflect_if_needed.rs:802`；`src/domain/self_revision.rs:7`；`tests/correction_atomicity.rs:511` | 对 application/runtime 路径成立；低层 store/import API 不属于同一合同。 |
| F04 | 确定性读取不依赖模型（L50） | done | `recall_memory::execute` 和 `build_task_context::execute` 仅依赖 read/text ports；`tests/mcp_stdio.rs:3427`；`scripts/local-memory-smoke.py` | 离线可用不证明相关性或真实任务收益。 |
| F05 | 延续显式 init/migrate、结构核验、备份恢复（L51、165、255） | done | `src/adapters/sqlite/lifecycle.rs:326–373`；`tests/sqlite_lifecycle.rs:63,79,213,243,349,421,444,470,534,621`；`tests/sqlite_backup_restore.rs:11,48` | 当前 schema v4；本轮未重跑，不推断生产 crash recovery 或所有平台通过。 |
| F06 | 项目只管记忆、证据、修订；行动/规划/环境控制归接入 Agent（L208–210、277） | done | `docs/origin-and-principles.md`；`README.md:13`；`docs/plans/2026-07-10-product-replan.md:446–458` | `decide_with_snapshot` 仍 experimental，不是自主执行器。 |
| F07 | 合并进既有分层路线，不另造平行架构；投影≠持久化层（L223–225） | done | 唯一 active plan `docs/plans/2026-07-10-product-replan.md`；`docs/product/memory-layering-roadmap.md:13–17,31–47`；`src/domain/memory_layer_projection.rs` | 完整 A–E 映射本身仍需补回 active plan，避免只保留此次缩小批次。 |
| F08 | 评测可前置，Local Product Alpha → Retrieval Quality；提前 runtime 要说明理由（L253–257） | done | active plan `:146–163` 明确把最小 recall/context 从 M3 前移，FTS/ranking/50 场景留 M3；`:134–142` 保留 M1→M2→M3 | 不能将本次前移泛化为完整 M3 已通过；测量框架可先落地。 |
| F09 | schema/interface/验证改变同步 README/status/testing/platform docs（L165） | partial | `README.md:15–23`；`docs/project-status.md:3–9`；`docs/testing-guide-2026-03-24.md:1554–1555`；`docs/local-memory-usability.md`；`AGENTS.md` | 有本次切片文档；active plan 尾部 `:583` 仍写旧顺序“structural readback → real-client closure”，与中段完成态不完全同步；既有 README 后段一些“no migration/index”的历史切片语句需明确限定切片，避免读成全局。 |

## 二、B：修订一致性、版本与审计

| ID | 原要求 / 原文位置 | 状态 | 当前源码、测试证据 | 剩余边界或下一动作 |
|---|---|---|---|---|
| B01 | 明确可修订状态、终态和是否允许分叉（L61、67；阶段 A/B L263–264） | done | `run_reflection.rs:225–248` 拒绝 Superseded；`store.rs:2056–2089` 禁止终态重开，支持 Active/Disputed→Disputed/Superseded；`docs/local-memory-usability.md:49`；`tests/correction_atomicity.rs:182,205` | 当前允许 Disputed→Disputed 记录；没有显式统一状态迁移枚举/矩阵测试，可在 B 收口补齐，不能默认为新的版本。 |
| B02 | 同一事务复核 target/evidence 的存在、scope、状态（L68） | done | `run_reflection.rs:170–276`；`store.rs:1824–1835,2028–2054`；`tests/correction_atomicity.rs:166,182` | 查询证据可事务外取候选，权威校验在写事务内；legacy global-world exception 明确保留。 |
| B03 | 条件更新，未命中明确冲突（L69） | done | `store.rs:2074` SQL 为 `UPDATE claims SET status = ? WHERE claim_id = ? AND status = ?`，rows_affected≠1 返回 InvalidParams；`tests/correction_atomicity.rs:217` | 是 state-CAS，不是 revision-CAS；错误消息可解释但未定义独立机器可读 conflict code。 |
| B04 | 在事务内检查“版本”；版本变化与提交同事务（L68–71） | partial | `schema.rs` claims 只有 id/owner/namespace/subject/predicate/object/mode/status，无 version；`ports/claim_store.rs` StoredClaim 无 version；`supersede_memory.rs:23–31` 无 expected_version | 以 claim ID 作为不可变版本身份 + terminal CAS 完成单后继目标；若继续承诺显式版本/stale-reader 冲突，需合同/迁移/API/测试。不能直接写“版本冲突全部完成”。 |
| B05 | 两个请求竞争同一旧版本，结果明确且不分叉（L73） | done | `store.rs:1829` BEGIN IMMEDIATE；`tests/correction_atomicity.rs:205` 并发只一成功、1 reflection、2 claims、1 evidence link；`:296` 同 key 两请求同结果 | 当前测试同进程并发 SQLite 请求；独立进程竞争和明确冲突分类仍可增强，不自动宣称分布式线性一致。 |
| B06 | 可选幂等 key，同 key/内容返回原结果，同 key/不同内容拒绝（L70） | done | `src/ports/write_receipt.rs` SHA-256 operation+namespace+key 身份与 payload hash；`ingest_interaction.rs:85–113`；`run_reflection.rs:173–180`；`tests/correction_atomicity.rs:265,296,333,418,449` | 对外字段名为 request_id，typed payload/order 必须相同；raw/canonical 输入归一和顺序语义按文档，不承诺任意语义等价 payload 同 key。 |
| B07 | 响应丢失/重启重试不重复写、不重做自动反思（L73） | done | `tests/correction_atomicity.rs:265` reopen replay；`:333` ingest replay；`server.rs:231` `if !result.replayed`；`scripts/local-memory-smoke.py` restart/replay | keyed application writes 范围；unkeyed 请求是不同写入，未承诺任意操作 exactly-once。 |
| B08 | revision、evidence、audit、幂等结果一起提交；失败无半链（L71、73） | done | `run_reflection.rs:287–399`；`ingest_interaction.rs:134–157`；store poisoned transaction；`tests/correction_atomicity.rs:217,357`；`tests/failure_modes.rs` | 现有行为/注入测试不是断电、磁盘故障或所有 crash points 的完整证明。 |
| B09 | 必要成功审计进业务事务，诊断日志可独立（L75） | done | `run_reflection.rs:380–399`、`ingest_interaction.rs:149–157` 无 key 也有最小 receipt；`store.rs:1877–1892,2011–2026`；`tests/correction_atomicity.rs:357,395` | 必要审计为 hash/result-ID 最小回执；`server.rs:1042,1087,1125` 普通诊断日志仍 best effort，这是允许的分层。 |
| B10 | 若要求每次写入的“完整审计”，需定义完整性范围（L75 条件性要求） | partial | reflections 保存修订内容/证据，receipt 保存 hash/result；`docs/local-memory-usability.md:55` 明确 receipt 不可按诊断日志清理 | 无审计 retention/防篡改/结构版本/全 API 覆盖保证；低层导入不在此范围。应先定义 audit manifest 与保留政策，不能把最小 receipt 等同完整审计。 |
| B11 | 项目修订与身份修订区分（关联 L107、218） | partial | scoped supersede 无 identity/commitment patch；`tests/correction_atomicity.rs:469,511`；`docs/local-memory-usability.md:51–53` | direct/automatic/targetless legacy global self-model 路径仍可受 project evidence 影响；尚无所有路径统一的作用/影响 scope 合同。 |

## 三、C：反馈合同、支持性验证与闭环

| ID | 原要求 / 原文位置 | 状态 | 当前源码、测试证据 | 剩余边界或下一动作 |
|---|---|---|---|---|
| C01 | 结构化预期结果、实际观察、验证结论（L216、246–249） | done | `src/domain/feedback.rs:9–48`；Event 可选 feedback；schema v4 `events.feedback_json`；`tests/feedback_provenance.rs:88,119,202` | 字段已持久化，observed_version 可选，JSON 必须拒绝未知字段。 |
| C02 | 记录谁/哪个工具、对象及版本、验证方式、限制（L246–249） | done | `FeedbackMetadata` producer/observed_target/observed_version/verification_method/verification_result/limitations；`tests/feedback_provenance.rs:119` | 身份/版本字符串为 caller 声明，不是认证 producer 或受检工件证明。 |
| C03 | 区分模型声称通过与工具报告通过；不得当普遍真理（L251） | done | `FeedbackSourceKind::{CallerReported,ToolReported,ModelAsserted}`；`feedback.rs:1–2`；`docs/local-memory-usability.md:61–67` | 只实现显式来源声明及边界；若后续要验证真实工具来源，需要可信 adapter/证据封装合同，不应凭标签自动提高可信度。 |
| C04 | 反馈引用存在、同 scope、边界有界且失败原子（L244） | done | `feedback.rs:50–100`；`ingest_interaction.rs:114–133`；`tests/feedback_provenance.rs:88,171` | 结构验证与存在性已做；证据内容是否支持目标结论没有做。 |
| C05 | 反馈明确支持哪条结论及其适用限制（L249） | partial | 反馈有 evidence_refs、target/version、limitations；Claim→Event 链可引用反馈 Event | 没有明确 supported_claim/candidate 引用、predicate/expected conclusion 或支持/反驳/不相关关系；Claim 的任意同 scope Event 仍可作为结构证据。 |
| C06 | 将“结构合法/证据存在”与“证据支持结论”分开校验（L218、242–251） | unimplemented | `run_reflection.rs:251–276` 读取 Event 后只检查存在/scope；`:289` ClaimDraft.validate 主要 evidence count；`auto_reflect_if_needed.rs:566–618` 管理 identity/commitment 结构 | 未消费 feedback.verification_result/observed_target/observed_version 来判定 Claim 修订支持性，也无含 unsupported/contradicted/inconclusive 状态的 validator。不能用现有 provenance 校验代替。 |
| C07 | 外部结果入账→修订候选→规则校验→提交→重新召回（L265） | partial | ingest feedback、手工 supersede、recall/history 能串接；`scripts/local-memory-smoke.py` 包含 feedback/correction/history/recall；旧 `auto_reflect_if_needed.rs:308–367` 有 model self-revision proposal→governance→commit | 没有针对结构化反馈的普通 Claim 候选端到端合同；旧 auto-reflection proposal 只含 identity/commitment patch，不是通用反馈候选。需要 accepted/rejected/insufficient 的完整故事。 |
| C08 | 新结论可追溯，旧结论保留，证据不足拒绝（L265） | partial | `tests/mcp_stdio.rs:2736` 保留旧 Superseded Claim + history；`tests/application_use_cases.rs:507,692,728` 缺失证据拒绝；`tests/feedback_provenance.rs:171` scope/不存在拒绝 | 结构性证据不足已有负例；“存在但不支持、版本错误、模型自述、限制不允许泛化”的内容支持性负例缺失。 |
| C09 | 何时检索/反思/停止：次数、体积、无新证据停止（L219） | partial | `auto_reflect_if_needed.rs:301–305` recursion guard；`:482–531` cooldown/unchanged evidence/episode watermark suppression；`build_task_context.rs` bytes cap | 已有反思触发抑制和上下文体积；没有统一每任务 retrieval/reflection 次数与成本账本，也无规则消融证明。先加确定性规则，不引入 RL 控制器。 |

## 四、D：召回、context 与存储模型

| ID | 原要求 / 原文位置 | 状态 | 当前源码、测试证据 | 剩余边界或下一动作 |
|---|---|---|---|---|
| D01 | 保留历史筛选、另加显式文本召回（L85） | done | `src/application/search_memory.rs:284–313` 原 browse；`recall_memory.rs` 新用例；`server.rs:302,322` 新 tools；`tests/mcp_stdio.rs:3165,3427` | 12 MCP tools；不是给 search_memory 静默改变排序。 |
| D02 | Event summary + Claim 内容本地文本检索，scope-first（L86） | done | `src/adapters/sqlite/text_recall.rs:24–73` scope SQL WHERE 在 LIMIT 前；active Claim subject/predicate/object 与 Event summary；同文件 `bilingual_scope_literal_and_claim_priority_eval` | 仅这些字段/类型；Episode/Reflection text 仍 deferred。 |
| D03 | 建本地全文索引、比较 FTS5（L86、91） | deferred | `text_recall.rs` 用 instr literal scan；`schema.rs` 无 virtual table / FTS；active plan `:150`、usability `:102` 明确后移 | 当前本地文本能力不能算原始“全文索引”完成。M3 用真实漏召回/查询计划比较 FTS5 或其他本地索引。 |
| D04 | 中文分词、短词、代码标识符分别验证，trigram 不算中文已解决（L91） | partial | `text_recall.rs` fixture 北京/咖啡/记忆、ASCII COFFEE、字面 `%_"*\\`；`recall_memory.rs` terms 单测 | 当前短 CJK 子串和特殊符号有覆盖；无分词、标识符命名变体、混合中英、同义改写评测；无 FTS tokenizer 比较。 |
| D05 | 避免有 timestamp 的记录淹没 Claim（L81、87） | done | `text_recall.rs:24` 独立类型候选且 Claim first；fixture 的 130 条 matching Event 洪水、limit=1 返回 active Claim | 仅新 recall；历史 union 故意保留旧排序。不应声称历史浏览问题“修改完毕”。 |
| D06 | 任务排序考虑 relevance / valid state / recency / type budget（L87） | partial | recall 为 Claim-first、matched_terms DESC、stable ID ASC；active filter | relevance 是字面命中项数；无 recency 权重、独立类型预算/多样性分配，Claim 多时可饿死 Event。需固定数据比较，不急于复杂 ranking。 |
| D07 | 返回命中理由和原引用，get_memory 深查（L88） | done | `RecallMatch {matched_terms,record}` + 原 SearchMemoryRecord provenance；`recall_memory.rs:54–67,80–127`；fixture 校验 evidence/episode refs；`get_memory.rs` | “理由”只有 match count，未列具体命中字段/term 或状态/关系得分解释。扩展解释属于 D10。 |
| D08 | 有真实同义漏召回证据才加可选 embedding/可替换 adapter（L89、147） | deferred | `TextMemoryStore` 已是检索 port；README/usability 明确无 embeddings | 需先固定 synonym/paraphrase negative cases 和 FTS 对照；索引版本/失效/成本也是触发后的合同。无需默认依赖远程模型。 |
| D09 | 接收任务描述/scope/budget，输出有效 Claim 与情景/证据（L233–236） | partial | `BuildTaskContextInput {namespace,query,limit,max_bytes}`；完整 Claim/Event 记录和 provenance | query 可作简短任务文本但不是任务建模；仅 Claim/Event，Episode 有引用无按需情景记录组装，未处理任务目标/缺失槽位。 |
| D10 | 输出冲突、过期、未验证信息、选择与截断原因（L237–238） | partial | `ContextOmissions {byte_budget,candidate_limit}`；matched_terms；active-only Claim | 没有独立 conflict/stale/unverified/missing 列表或逐条 selection reason；历史 Event 仍可能匹配，不能当当前 Claim。 |
| D11 | 预算内完整内容，可重建短期工作记忆，不急增表（L217、240） | done | `build_task_context.rs:42–119` exact serialization 固定点计数；whole-record pack、跳过 oversized；tests `every_budget_counts_exact_serialized_utf8_and_metadata` / `oversized_first_record_does_not_block_later_small_record` / `rejects_tiny_budget_and_includes_all_at_exact_boundary` | 完成的是 JSON UTF-8 bytes；不是 token 限额或 JSON-RPC total cap。没有新增工作记忆表，符合原路线。 |
| S01 | 新记录明确排序键；未知历史创建时间保持未知（L97、99、106、112） | unimplemented | `schema.rs::claims_table_sql` 无时间；`store.rs:49–60` 仍 RFC3339 SQL 转换表达式；`search_memory.rs` Claim recorded_at 为 None | 需要 Claim recorded/effective time 语义及规范化排序列迁移；未知历史时间 nullable，不能用迁移时间冒充创建时间。 |
| S02 | Reflection 独立作用范围与影响范围，区分身份级/项目级（L98、107） | unimplemented | `schema.rs::REFLECTIONS_TABLE_SQL` 无 owner/namespace/effect scope；当前 scope 通过 Claim endpoints 推导；`tests/mcp_stdio.rs:2404,2571` record-only 隐藏 | record-only scoped history 仍不可见；需要“来源 scope vs 影响 scope”合同，legacy global patches 迁移/兼容策略。 |
| S03 | 需要查询约束的证据关系逐步关系表化（L108） | partial | `evidence_links`、`episode_events` 已关系表化；reflections.supporting_evidence_event_ids 与 events.feedback_json.evidence_refs 仍 JSON | 先选实际需要逆向查询/约束的 reflection/feedback 关系；回填、FK、去重、scope 与未知 legacy 处理都需迁移测试。 |
| S04 | 基于查询计划加 scope/time/reverse relation indexes（L100、109） | unimplemented | `schema.rs` 只有 PK/关联复合 PK，没有二级 scope/time/reverse 索引；grep 当前 src/tests/scripts 无 EXPLAIN QUERY PLAN benchmark | 先实测 EXPLAIN 与容量；关联表 `(claim_id,event_id)` PK 不等于 event→claim reverse index。 |
| S05 | 派生全文索引可重建，账本是事实源（L110、150） | deferred | 当前不存在独立文本派生索引；`TextMemoryStore` 直接读事实表 | 未来索引需 build/version/rebuild/readiness/recovery 和检索结果等价性；现在不能宣称“重建已支持”。 |
| S06 | 表达式索引 vs 规范化排序列，以测量决定（L112） | unimplemented | 现有长时间转换表达式 + 无对应索引，见 S01/S04 | 这是需记录的技术决策，不是已验证性能瓶颈。benchmark 后选方案，并保留时区/小数秒稳定排序。 |

## 五、容量、并发配置、运维与工程可维护性

| ID | 原要求 / 原文位置 | 状态 | 当前源码、测试证据 | 剩余边界或下一动作 |
|---|---|---|---|---|
| O01 | 1 万/10 万条、多 namespace、长修订链固定容量基线（L120） | unimplemented | 当前 text fixture 主要 130+130 事件；`scripts/local-memory-smoke.py` 小型工作流；未找到容量 fixture/bench runner | 不能把小 fixture 和 smoke 时长当容量基线。 |
| O02 | 记录 p50/p95、内存、DB size、锁等待、响应 bytes（L121） | unimplemented | 无聚合 latency/RSS/DB size/lock-wait benchmark 结果；smoke 仅本地总时长，usability `:88` 明示 simulation | 建机器可读统一 report，保留 seed、构建/硬件、commit、payload bytes。 |
| O03 | 同时写、读与 dashboard 查询（L122） | partial | correction concurrency 测试、dashboard HTTP/recorder 单独 tests、migration lock tests 均存在 | 无三路固定负载并行容量实验；并发正确性回归不等同 workload 性能实验。 |
| O04 | 查看热点 SQL 计划再调索引、连接参数、分页（L116、123） | unimplemented | `lifecycle.rs:861–873` 仍 SqlitePool::connect_with 默认池行为；无 normal serve 显式 pool/journal/busy policy；现有 limit 不等于 cursor pagination | 迁移专用 0→5000ms busy timeout 不能冒充全局 runtime 参数治理。 |
| O05 | 是否启用 WAL 连同 backup/checkpoint/recovery 验证，仍单写者（L125） | partial | migration `BEGIN IMMEDIATE` + bounded reader wait 已实装，`tests/sqlite_lifecycle.rs:470,621`；普通 backup restore tests 存在 | 无显式 runtime WAL 决策或 checkpoint/sidecar/recovery 矩阵；不能直接启用 WAL 然后宣称冲突问题解决。 |
| O06 | 独立平台验收、可交付闭环（L163） | partial | `.github/workflows/ci.yml:15–47` Linux/macOS；`:49–62` Windows native subset；`scripts/local-memory-smoke.py` 安装二进制模拟；`tests/mcp_stdio.rs` 真实进程 harness | 本报告未读远端 exact-SHA runs；workflow 配置不是通过证据；fresh-machine/native user client/manual release gates 仍 open（active plan `:401–424`）。 |
| M01 | SQLite crate 内拆分 read / transaction write / row mapping（L133） | partial | 已有 `lifecycle.rs` / `schema.rs` / `text_recall.rs`；`store.rs` 仍 3178 行且 query/tx/mapping 混合 | 本次新 text_recall 模块不等于原要求完成；拆现存 store，保留 port/SQL 行为与 public contract。 |
| M02 | MCP 参数转换、业务调用、日志包装分离（L134） | partial | `dto.rs` 950 行承担参数转换；`server.rs` 1742 行仍混合 handler/runtime/logging | DTO 分离已有，日志包装与 orchestration 仍集中；可逐 tool 提取边界，不增加新 crate。 |
| M03 | auto-reflect 拆候选/证据/策略/提交（L135） | partial | `auto_reflect_if_needed.rs` 已有 detect_trigger_candidate / resolve_governed_evidence_window / validate_self_revision / apply_validated_self_revision 命名步骤，仍单文件 1136 行 | 逻辑阶段存在；模块边界与独立阶段 fixtures 尚未充分分离，不应说“完全没做”或“重构完成”。 |
| M04 | 保留 release-tools feature，不急拆 crate（L136） | done | `Cargo.toml:12–14` 及多个 required-features；`scripts/test-tier.sh` | 与 broad refactor 分开，保持默认构建轻量及 full-feature tests。 |
| M05 | 用合法/非法配置和重复请求行为补文本合同（L137） | done | `tests/provider_config.rs`、`tests/dashboard_config.rs:58,80,94`、`tests/daemon_config.rs:22`；`tests/correction_atomicity.rs:265,296,333` | ci_contract 文本检查仍可留作辅助；测试数量不是效果指标。 |

## 六、A / E：实验、经验沉淀与明确延期的研究路线

| ID | 原要求 / 原文位置 | 状态 | 当前源码、测试证据 | 剩余边界或下一动作 |
|---|---|---|---|---|
| A01 | 固定任务样本，区分错误召回、错误修订、缺证据（L160、167、263） | partial | 双语 fixture、correction/feedback negative tests 分别存在 | 没有统一固定任务 manifest/分类/expected outcome/report；不是同一数据上可比较的 A–E 基线。 |
| A02 | 命中率、旧结论误用、scope leakage、context volume（L138） | partial | literal fixture 对 exact IDs/provenance/旧 Claim 排除做断言；context sweep 校验 bytes | 无正式 recall@k/误用率聚合、版本化标注集、分层指标；literal 命中 5 queries 不能代表一般 Agent 效果。 |
| A03 | 同一模型、同一任务集：无长期记忆 / ledger+recall / feedback+revision（L269–273） | unimplemented | `ModelPort` 有 decide/propose；demo 用 deterministic stub；没有三条件统一 runner/任务重置/随机种子/等预算 manifest | 可先做完全离线 runner + deterministic fixtures；stub 结果只能证明机制，不证明真实 LLM 收益。 |
| A04 | 成功率、错误修订率、耗时、token 成本（L275） | unimplemented | 当前 ModelDecision 仅 action（`src/ports/model_port.rs:30–33`），未提供统一 usage/cost；没有此实验 report schema | 没有实际 usage 时输出 not_measured，不将 JSON bytes 推成 token；paid cost 需 model/price/time/version provenance。 |
| A05 | 单组件消融，控制额外计算收益（L221、275） | unimplemented | 未找到 ablation runner；自修订 before/after demo 只有特定故事 | 至少移除 recall、feedback gating、revision、context packing 分别重跑，固定模型任务和计算预算；预算不相等时明确不可归因。 |
| A06 | 真实模型收益证据与 provider 连接认证分离（L269–275 的证据要求） | deferred | `src/support/provider_live_certification.rs:248,334` probe 明示不构成 self-revision quality evidence；active plan `:161` 不默认调用付费/远程模型 | provider-live 文件即使 passed 也不是 A/B/C 效果报告。真实模型评测须独立授权/固定模型版本与数据共享范围；离线 harness 不能一并推迟。 |
| E01 | 补完整 Episode goal/objective/outcome/lesson/evidence（L220、267） | partial | `episode_projection.rs` input/read-only projection；`tests/product_completion_read_models.rs:253,320,363`；schema 只有 episode_events，无 durable Episode record | objective/outcome/lesson 是 caller-fed projection，不是持久化事实。需要迁移、scope、生命周期、backup/export gates 后新增 durable record。 |
| E02 | 多 Episode 提炼可检查语义知识候选（L220、267） | deferred | `memory_layer_projection.rs` semantic 标 partial 来自 Claim；`memory-layering-roadmap.md:111–122` 是未来计划 | 无 semantic candidate 类型/store/generator/inspection/rejection；既有 Claim 不等于完整知识沉淀层。 |
| E03 | 流程候选可检查、拒绝、版本化（L220、267） | deferred | `memory_layer_projection.rs` procedural 明确 not_implemented；roadmap `:124–134` | 无 procedure candidate 类型/version/rejection/provenance store 或搜索合同。 |
| E04 | 激活与回滚另设合同，不自动扩大权限（L220、267） | deferred | roadmap `:129–132` 写 explicit activation/rollback；当前无 runtime procedural activation | 候选生成可以单独实现，默认 inert；激活不能从“模型提案”自动得出，不要求原始 A–E 完成前先做自治执行器。 |
| E05 | 新层满足迁移恢复生命周期前置（L255） | partial | 现有 explicit schema/lifecycle/backup 已有；`memory-layering-roadmap.md:31–47` 还要求 export/retention/deletion | 当前 retention/compaction/export 生命周期尚未统一实现，不可因为 v4 migration 安全就宣布所有新层前置完成。 |
| R01 | 自治控制器、多 Agent 竞争、RL、自动长期策略只保留研究候选（L277） | deferred | active plan M4 `:446–458`；memory layering roadmap slow variables `:136–146` | 继续明确不排期；原方案没有授权实现这些。 |
| R02 | remote DB / retrieval service / team 需真实需求再评估（L148） | deferred | active plan M4、`docs/product/remote-team-mode-boundary.md` | 不因本轮召回/写入工作引入服务端多租户、auth 或新云依赖。 |

## 七、建议补全的有界 A–E 里程碑

这些是对原方案的**剩余交付拆解**，不声称已实现，不替换既有 M1→M2→M3 队列。把这组编号并入唯一 active plan，另标“基线可先做 / runtime 受原 gate 约束”。每个里程碑同时更新对应行为、状态和验证文档。若只完成本次可用性候选，仍必须保留这些未完成条目及触发条件。

### A：冻结合同与可重现基线（现在可做，无需付费模型）

交付：

1. 一份修订 transition/branching/identity-version ADR：区分 state-CAS、不可变 Claim-ID 版本、显式 optimistic revision token；定义 conflict reason、Disputed→Disputed 的意义、legacy self-model scope。
2. 版本化任务 manifest：至少 50 个 scoped cases（数量来自既有 M3），包含中英短词/代码标识符、噪声洪水、同义改写、旧/冲突 Claim、空/不足/跨 scope 证据、反馈版本错配。标注 relevant IDs、当前结论、应拒绝原因和 context bytes；未知语义明确 unknown。
3. 独立 A/B/C 离线 runner 和机器可读报告：A 无长期记忆，B ledger+recall，C feedback+constrained revision；固定任务、模型标识/seed、初始账本与预算；单组件消融开关。可先用 deterministic model adapter，不伪称真实模型效果。
4. 固定 10k/100k 容量生成器：多 namespace、长链；记录 SQL plan、query p50/p95、RSS/CPU、DB size、locks/busy、response bytes，读/写/dashboard 并行负载。只建立基线，不凭感觉设吞吐保证。

验收：同 seed 和 commit 可复现 functional outputs；报告将 wrong_recall / wrong_revision / insufficient_evidence 分开；token/cost 不可得时填 not_measured；scope leakage=0，provenance 100% 的断言失败会红灯。性能数值带运行环境和原始样本。运行时间不同不影响功能可复现定义。

停止：任一 scope 泄漏、数据丢失、评测标注矛盾时先修基础；不能用更多测试数量替代失败样本解释。

### B：把可信账本合同收完整

交付：

1. 对已实现的 state-CAS/receipts 做完整 transition matrix、直接/自动/scoped 路径 coverage；增加独立进程同目标/同 key/different payload 竞争和 restart 故障点测试。
2. 根据 A 的 ADR 做**一个明确选择**：保留 Claim-ID-as-version 并写明不支持什么 stale edits，或加入 expected_revision + durable revision counter（兼容旧调用、schema migration、旧数据版本初始化语义）。不能以字段名字完成架构，也不能以“CAS 已有”省略版本语义。
3. 定义最小成功审计必填项/版本/保留：业务结果、evidence links、receipt 原子一致；诊断日志单独保留；禁止 prune receipt 破坏 replay。若需要更完整审计，另列数据字段和隐私约束。
4. Reflection origin scope / effect scope 合同先落 ADR；project→global legacy 更新仍 experimental，新增路径不得继承含糊例外。

验收：竞争只一后继；同 key 同 typed payload 原 IDs；不同 payload 冲突；audit/receipt failure 全回滚；旧 terminal 不复活；未知/跨 scope fail closed。新增 schema 必須通过当前结构核验、migration rehearsal、restore-to-new-path。

停止：不满足迁移/恢复/审计一致性就停在 B，不扩候选自动提交。

### C：实现真正的最小反馈候选闭环

交付：

1. 新增非执行型 RevisionCandidate 合同（可先内存/测试夹具）：target Claim/version、proposed replacement、feedback Event refs、明确 supported predicate、limitations；candidate 不等于已批准写入。
2. 确定性 validation decision：accepted / insufficient / contradicted / stale-target / out-of-scope；反馈来源声明和可信工具产出证明分开。先选择一个可机械验证的小领域，如“某 commit 的某检查通过/失败”，不做任意自然语言真值判定。
3. 只有校验通过的候选通过既有 run_reflection 提交，带原子 receipt/audit；错误版本、inconclusive/not_performed、缺证据、模型声称却无验证、超出 limitations 都有拒绝或明确 unknown 的负例。
4. 完整 fixture 故事：入账→proposal→validate→commit→recall→history；旧记录完整，新结论追溯原反馈；无新证据停止，次数受限。

验收：有效/不足/矛盾至少三类故事在同一 harness 中可观察；scope leakage=0；错误修订负例不改变 Claim 状态、不留半 audit；不修改 global identity/commitments、不执行外部动作。

停止：无法确定语义支持时返回 unknown/拒绝，不以 tool_reported 标签替代证据。

### D：任务相关召回、schema 与容量收口

运行时依赖：保留已有最小 recall/context；全面 ranking/index 扩张仍按 M1/M2→M3 gate。A 的基线/SQL 测量可先准备。

交付：

1. 比较现有 literal、FTS5/tokenizer、recency/type-budget、relation weight；中文短词、代码 ID、同义改写分组报告。采用有证据获益的最小方案；FTS 不适配短 CJK 时保留确定性 fallback，不强行把 trigram 当完成。
2. 版本化可重建派生索引：账本事实不变、schema/index version、rebuild/recovery、scope-first；embedding 仅在明确 paraphrase 漏召回门触发，且可选。
3. Claim 新记录时间/排序键、历史未知时间 nullable；Reflection scope/effect scope；需要检索的 feedback/reflection evidence edges 关系表化。以 explain 选择 scope/time/reverse indexes，保持 FK/legacy 兼容。
4. context 增加选择理由、conflict/stale/unverified/missing 标记以及最小 Episode 证据视图；保持 exact bytes hard cap。若加入 token 预算，必须另设明确 tokenizer/version，不重命名 bytes 当 tokens。
5. normal runtime pool/journal/busy 参数明确化，只有实测需要才调；如选择 WAL，同步 checkpoint/backup/restore/reader-writer/restart 矩阵。分页选择由大样本/response bytes 决定。
6. crate 内逐步拆 store read/tx/mapping、MCP orchestration/logging、auto-reflect stages；前后以同一 golden fixtures 和回归比较，保留 release-tools。

验收：50+ cases 的 recall@5 初始目标≥0.80（来自 active plan），provenance=100%、scope leakage=0；旧结论误用率单列且不能因排除 Claim 而忽略历史 Event；同预算与 A 基线比较。10k/100k 给出 p50/p95/锁等待等前后结果，不能只报平均时间；无造假历史时间，无 silent permission broadening。

停止：质量退化、scope leak、索引/账本不可恢复不一致、不可解释迁移时停止/回滚本片；没有充分收益时保留简单方案并记录负结果。

### E：经验候选，不自动激活权限

依赖：已有 layering-roadmap 的 lifecycle/export/retention/backup prerequisites；先 richer Episode，再 semantic/procedural candidate。

交付：

1. 持久化 Episode goal/outcome/lesson/linked evidence、scope、版本和有界 lifecycle；不能再把 caller-provided 只读投影当事实层。验证迁移、readback、snapshot、backup/restore。
2. 从明确 Episode 输入生成 semantic/procedural candidate，保存来源、限制、矛盾、版本、review state（proposed/rejected/accepted/deprecated）；inspect/search/reject 不改变历史。
3. 明确 activation/rollback 合同，默认所有候选 inert。当前里程碑可止于可检查候选；运行时执行/自动权限扩张仍不在范围。
4. 用 A/B/C 和消融 fixture 验证经验候选是否真正降低错误，保留负结果；真实模型效果证据单独采集并标清授权/费用/模型版本。

验收：候选可检查、拒绝、版本化和追溯；互相矛盾的案例不会覆盖旧事实；读取或接受候选不自动扩大 action 权限；所有新层 migration/recovery 有独立证据。

停止：没有生命周期/恢复门先不增加新 durable 层；无法证明支持性时保留候选状态；不以 E 为由加入自治控制器、RL、多 Agent 竞争或远程团队服务。

## 八、跨阶段发布与证据要求

- 源码/测试存在、focused 本机测试通过、完整 CI、精确提交平台证据、真实客户端、fresh-machine、人类 release decision 是不同证据级别。
- 每片记录 exact SHA、命令、平台、测试/报告路径、失败或未执行项；不能复制历史“passed”到新提交。
- 现有安装脚本复制一次构建的二进制、synthetic MCP transcript、backup/restore，是有价值的 local installation simulation；不等于真正 fresh macOS ≤10 分钟或任意客户端验证。
- 原 A/B/C 是机制效果实验；provider live certification 是连通/解析测试；模型付费实验是授权后的外部验证。为这三种证据分别建状态，不合并 checkbox。
- 本次只读映射没有取得 e14a59e 的远端 CI 结果；后续若有 exact-SHA CI 证据，可补充为单独验证条目，不能推导为原 A–E 完成。
- 建议总述：**“已实现可靠纠错/重试、结构化反馈 provenance、离线字面召回与有界上下文；原合并方案部分落地，反馈支持性候选闭环、评测/容量框架、查询模型演进和经验候选仍按 A–E 逐片推进。”**
