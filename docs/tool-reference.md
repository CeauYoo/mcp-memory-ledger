# MCP 工具与合同索引

当前注册 **32 个工具**。本页按用途导航；精确 required/optional 字段与枚举以正在运行的 `tools/list` schema 和 [DTO](../src/interfaces/mcp/dto.rs) / [server 注册](../src/interfaces/mcp/server.rs)为准。CLI 命令和 MCP 工具是不同入口。

## 1. 日常记忆与任务上下文（6）

| 工具 | 用途与容易混淆的边界 |
| --- | --- |
| `ingest_interaction` | 写 Event / Claim，`event` 与 `claim_drafts` 必填；可选 request_id。自动反思钩子可能触发配置的 provider |
| `search_memory` | scoped 过滤/浏览，不是全文 query；默认 Event，可显式 Claim/Episode/Reflection 或 union |
| `get_memory` | exact namespace + id；省略类型保持 Event，Claim/Episode/Reflection 需显式类型 |
| `recall_memory` | active Claim + 历史 Event 字面检索；最多 8 个 OR terms、512 UTF-8 query bytes、limit 1..100 |
| `build_task_context` | 完整记录/来源/元数据/可选 caller receipt 共用 JSON byte cap；有界 Episode 与可选诊断 |
| `supersede_memory` | 明确的同域 Claim 更正；旧 Claim 留作 Superseded，事务/CAS/可选 request_id，不改身份或承诺 |

Event/Claim 支持 canonical/raw ID；Episode/Reflection ID 为 opaque persisted reference。get_memory 未命中/跨 scope 返回 null；scoped 浏览/历史返回空结果，不扩大查询。更正写入遇到缺失或越界目标/证据会拒绝。旧 Episode membership projection 与丰富 Episode detail 不同。

普通 recall 为 ASCII 大小写不敏感的 OR 字面匹配，CJK literal；三字符以上利用 FTS5 trigram，短词精确 fallback。不是中文分词、拼写纠错、同义词或 embeddings。完整语义见[召回合同](memory-feedback-experience.md)，上下文见[diagnostics](context-diagnostics.md)。

## 2. 反馈 → 更正候选（6）

`get_feedback_target_version` → `propose_feedback_candidate` → `get_feedback_candidate` → `validate_feedback_candidate` → `commit_feedback_candidate`，或 `reject_feedback_candidate`。

- 先读取确切 Claim 的 fingerprint；proposal 的 candidate_id 由服务生成。
- 新反馈写操作要求 request_id；reject 另需 reason。相同业务 payload + key 可重放，换 payload 不可复用 key。
- v1 只更改 object，保留 subject/predicate/mode/scope；证据须精确对齐 target/version/old/new 值，commit 在事务内再次验证。
- 非空 limitations、缺证据、不确定 verification 等会阻止自动 commit。没有 feedback revise 工具；取得新证据后按抑制规则新建提案，必要时拒绝旧候选。
- caller_reported / tool_reported 是来源标签，不是认证或语义真值证明。

完整[反馈生命周期与重试合同](memory-feedback-experience.md)。一般 `supersede_memory` 是显式更正 API，不能等同为上述受验证候选自动提交。

## 3. 丰富 Episode（3）

- `record_episode`：调用方提供 episode_id/request_id/content；title/objective/actions/observations/outcome/lesson/limitations/source_event_refs 全部字段必需。
- `get_episode_detail`：读取持久丰富 Episode。
- `list_episode_details`：有界 scoped 列表。

observations 与同 scope 的 source_event_refs 不可空。内容是来源关联的调用方报告；Episode 快照不可覆盖，修正用新 Episode。详见[经验合同](memory-feedback-experience.md)。

## 4. 语义/流程经验候选（7）

| 工具 | 生命周期 |
| --- | --- |
| `propose_experience_candidate` | 调用方提供 candidate_id；semantic/procedural 内容以 pending 保存 |
| `get_experience_candidate` | 默认 current，可指定历史 version |
| `list_experience_candidates` | scoped current heads |
| `set_experience_candidate_status` | 带 expected_version/request_id 的显式 activate/reject/supersede |
| `revise_experience_candidate` | 新内容追加为 pending version |
| `rollback_experience_candidate` | 复制指定旧版本为新的 pending，另行激活 |
| `recall_experience_candidates` | 仅 active current heads；Unicode 小写 AND 字面 terms，与普通 recall 的 OR 合同不同 |

历史版本曾 active 不代表现在可召回。候选拒绝/回滚保留历史，stale expected_version 不覆盖新状态。procedural steps 永远只是文本；不触发动作、权限、identity、commitment 或后台写入。详见[版本与查询限制](memory-feedback-experience.md)。

## 5. 反思与实验性自模型（7）

- `get_reflection_history`：以 exact scoped Claim 为锚的双向修订链。
- `get_self_model_history`：保留原有 scoped identity/commitment 修订审计，不输出全局聚合快照。
- `get_self_model_versions`：独立的实验性版本入口，必填 namespace，必须显式 `allow_global_version_metadata:true`；limit 默认 20、范围 1..100，`before_version` 为 exclusive cursor，按版本倒序；仅限制返回记录数，不是字节或计算量上限。返回 global `current_version`、`has_more`/`next_before_version` 与可公开的 written component patch。全局版本号可透露其他 namespace 的活动，因此仅限实验性单用户边界，不代表认证/租户隔离。只返回 source reflection 为 verified、恰好一个相同 origin scope、且全部非空证据仍持久存在于相同 scope 的组件；baseline、unknown、mixed 与 inherited aggregate snapshots 不公开。每个组件的 diff 保留顺序与重复项，报告 changed/前后数量；只有 previous component source 同样满足 verified/same-scope/durable-evidence 时才包含 previous_patch，否则仅说明已脱敏，不返回旧值或 hashes。head 与当前 projection 校验和分页读取共用一个 SQLite read transaction；漂移时以不含内容的通用错误 fail closed。
- `get_evidence_relation`：trigger window 与 scope 相交、selected subset 检查，不排名或扩大证据。
- `build_self_snapshot`：显式 namespace/manifest/time 可收窄；省略 namespace 的 legacy 路径保留兼容性。
- `decide_with_snapshot`：服务端 commitment gate 的实验非权威结果，不执行动作。
- `run_reflection`：受治理事务；MCP targetless 记录需要 origin_namespace，禁止借此携带 replacement/identity/commitment patch 或 rollback。全局组件更新追加版本，可用 `expected_self_model_version` 进行全局 head CAS。`self_model_rollback:{target_version,components:["identity"/"commitments"],confirm:true}` 只回滚明确选择的组件并追加 compensation version；需 existing Claim target、explicit origin_namespace、同域 durable evidence、expected version 与 request_id，不能混入普通全局 patch。其他组件保留当前值，不删除历史。可选 `request_id` 的 typed receipt 在移动 DTO 字段前生成，相同 key/payload 可重放、不同 payload 拒绝；普通请求的 receipt namespace 使用 explicit origin_namespace，省略时沿用 `self` fallback。新增 expected/rollback/request_id 字段缺省不序列化，旧 typed payload bytes 保持不变。

[Reflection origin/affected scope 合同](reflection-scope-history.md)区分来源与影响范围；未知/混合历史不猜测归属。全局 self-governance 是实验边界，namespace 不等于用户授权。

## 6. 导出与检索维护（3）

- `export_memory`：有界只读 scoped interchange，不是整库备份或自动脱敏。
- `inspect_retrieval_index`：全数据库只读深检，参数 `{}`。
- `rebuild_retrieval_index`：全数据库显式派生索引重建，参数 `{}`，不重写 ledger facts。

维护工具无 namespace 参数。[操作手册](database-operations.md)说明何时诊断、重建或恢复，[export 合同](scoped-export.md)说明一致性与完整依赖省略规则。

## 共用边界

- `caller_budget` 仅适用于 recall_memory / build_task_context / run_reflection；携带 next_used，按 receipt 停止，不能当作持久服务端 quota。见[预算合同](caller-operation-budget.md)。
- `recorded_at` 与 caller `observed_at` 分开；未知历史为 null，不据年龄判断失效。见[时间合同](temporal-metadata.md)。
- 所有示例优先 mock/合成数据；工具 schema 只表示输入合同，不批准访问外部服务或执行经验步骤。
