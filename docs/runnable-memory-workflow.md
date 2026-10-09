# 最小到完整的本地记忆工作流

先完成[快速开始](quickstart.md)和[MCP 客户端接入](local-mcp-integration-2026-03-26.md)。以下使用 **mock provider + 隔离测试库 + 合成数据**；不会请求外部模型。保留一个 namespace `project/demo`，别与正式数据共用。

## 1. 可复制的最小 MCP 参数

下面 JSON 是 `tools/call.arguments`，工具名写在各步骤标题中；客户端负责 MCP 握手和包装。返回业务结果在 `structuredContent`。`<EVENT_ID>`、`<CLAIM_ID>`、`<EVIDENCE_ID>` 必须由前一步真实结果替换，不能原样发送。

### 记录：ingest_interaction

```json
{
  "request_id": "demo-original-1",
  "event": {
    "owner": "World",
    "namespace": "project/demo",
    "kind": "Observation",
    "summary": "演示：项目城市是北京"
  },
  "claim_drafts": [{
    "owner": "World",
    "namespace": "project/demo",
    "subject": "项目",
    "predicate": "城市",
    "object": "北京",
    "mode": "Observed"
  }],
  "episode_reference": null
}
```

保存返回的 `event_id` 为 `<EVENT_ID>`。相同 payload/request_id 重试不重复写入。可选 `observed_at` 在顶层；不填表示未知观察时间，与服务器 recording time 不同。

### 断开重连后检索：recall_memory

```json
{"namespace":"project/demo","query":"北京","limit":10}
```

找到返回 `records` 中 Claim 的 `record.id`，保存为 `<CLAIM_ID>`；不要仅凭 Event ID 猜测 Claim ID。不同 namespace 的同词记录不会混入。

### 查看证据：get_memory

```json
{"namespace":"project/demo","record_type":"Claim","id":"<CLAIM_ID>"}
```

检查 `record.provenance.evidence_event_references`。这是来源关系，不表示内容经过独立真实性认证。

### 记录新的更正证据：ingest_interaction

```json
{
  "request_id":"demo-new-evidence-1",
  "event":{
    "owner":"World",
    "namespace":"project/demo",
    "kind":"Observation",
    "summary":"演示更正：项目城市现在确认为上海"
  },
  "claim_drafts":[],
  "episode_reference":null
}
```

保存返回 `event_id` 为 `<EVIDENCE_ID>`。即使没有 Claim，也要提供 `claim_drafts: []`。

### 显式更正：supersede_memory

```json
{
  "request_id":"demo-correction-1",
  "namespace":"project/demo",
  "claim_reference":"<CLAIM_ID>",
  "replacement_claim":{
    "owner":"World",
    "namespace":"project/demo",
    "subject":"项目",
    "predicate":"城市",
    "object":"上海",
    "mode":"Observed"
  },
  "replacement_evidence_event_ids":["<EVIDENCE_ID>"],
  "summary":"依据新的演示证据更正城市"
}
```

这是明确的调用方更正，不是假定反馈已自动验证。相同请求可重放；旧 Claim 留作 Superseded，不被删除。

### 看历史：get_reflection_history

```json
{"namespace":"project/demo","claim_reference":"<CLAIM_ID>","limit":10}
```

### 装入任务上下文：build_task_context

```json
{"namespace":"project/demo","query":"上海","limit":10,"max_bytes":8192}
```

检查新 Claim 和来源，留意省略/诊断字段。旧 Event 仍是历史证据，不能因字面命中就当作当前结论。byte cap 覆盖整个紧凑结果 JSON，不含 MCP 外壳，也不是 token 预算。

## 2. 自动执行完整离线示例

先 `cargo build --locked --bin agent_llm_mm`，再用平台的 Python launcher 与 binary 路径执行。每个输出目录必须是新的空目录，Windows binary 加 `.exe`：

```text
python scripts/local-memory-smoke.py --binary target/debug/agent_llm_mm --output target/reports/workflow-restart
python scripts/evaluate-memory-loop.py --binary target/debug/agent_llm_mm --output target/reports/workflow-feedback
python scripts/temporal-scope-export-smoke.py --binary target/debug/agent_llm_mm --output target/reports/workflow-temporal
```

这些脚本显式生成 mock 配置与合成库，并检查实际 stdio 结果：

- smoke：安装副本、写入/重连、request_id 重放、纠错与备份恢复。
- evaluator：反馈 → 提案 → 验证 → 原子提交 → 召回；Episode → 经验候选 → 激活 → 版本/拒绝/回滚。
- temporal：观察/记录时间、独立 Reflection 范围、安全 history、只读 export。

输出保留本地；断言失败即测试失败。它们不证明 fresh-machine 安装、真实模型收益或正式发布。

## 3. 反馈候选与经验生命周期

自动验证的反馈路径比上面的显式 supersede 更窄：先 `get_feedback_target_version`，把确切 target/version/old/new 值绑定到 Event.feedback，再 propose/validate/commit。`tool_reported` 不认证来源；非空 limitations 等会阻止 v1 自动提交。没有反馈 revise 工具；需要新证据时依照抑制规则重新提案，或拒绝旧候选。完整字段与边界见[反馈/经验合同](memory-feedback-experience.md)，可执行请求实现在[固定 evaluator](../scripts/evaluate-memory-loop.py)。

丰富 Episode 来源必须同 scope；semantic/procedural 候选需要显式激活才可召回。revision/rollback 生成新的 pending version，需要再次激活；不会执行流程、改变权限或修改全局策略。

## 4. 有界 context 与停止条件

完整 Claim/Event 优先，其后才是来源关联的丰富 Episode 和可选 diagnostics。样本不完整；不同值可能只是多值信息，不能据此推断矛盾；年龄不等于过期。详见[上下文合同](context-diagnostics.md)。

可在 recall/context/reflection 上提供 `caller_budget`，从成功或 admitted-error receipt 携带 `next_used`，在 `allowed=false` 时停止并检查原因。这是合作式调用方计数，不是持久 session/quota；省略/null 保持兼容。见[预算示例](caller-operation-budget.md)。

## 5. 运维与结果解释

用[数据库操作](database-operations.md)做显式迁移、备份和恢复到新路径；[export](scoped-export.md)只用于有界数据交换，不是备份。历史未知归属不扩大可见范围；origin/affected scope 见[Reflection 合同](reflection-scope-history.md)。

最终结果与限制见[当前状态](project-status.md)及[70 项核对](plans/2026-10-09-original-plan-final-reconciliation.md)。固定离线通过不自动证明真实模型效果、token 节省、生产容量或用户客户端验收。
