# 项目起点与主线原则

状态：`current`

## 灵感起点

MCP Memory Ledger 来自一次关于“有限空间中的信息压缩”和“LLM 是否能形成长期记忆”的连续讨论。讨论最终把问题从“怎样存更多信息”推进到了一个更重要的命题：

> 自我不是一组固定记忆，而是一套治理“哪些过去可以约束哪些未来”的机制。

当前分支保留经过整理的[原始讨论提炼稿](llm-agent-memory-self-dialogue-2026-03-23.zh-CN.md)。逐轮原始日志、早期设计稿、实施计划和阶段快照保存在本地归档分支 `codex/archive/pre-mainline-reset-2026-07-10`，查阅方法见[归档说明](archive.md)。

## 项目真正要解决的问题

项目的核心不是打造一个“什么都能做的自治 Agent”，而是回答四个窄而困难的问题：

1. 哪些交互值得成为长期记忆？
2. 一条高层记忆如何追溯到原始事件？
3. 新旧信息冲突时，怎样纠错而不洗掉历史？
4. 哪些过去形成的承诺可以安全地约束未来行动？

因此，主线始终是：

```text
原始事件
  -> 带归属和证据的命题
  -> scoped 检索与回灌
  -> 冲突 / supersession
  -> 受治理的反思与承诺
```

## 五条不可丢失的原则

### 1. 原始事件负责保真

交互、工具结果和观察先进入追加式事件底账。摘要、投影和索引不能替代原始事实。

### 2. 高层记忆必须带证据

Claim、episode、identity 或 commitment 都必须能回到具体 event；没有 provenance 的“记忆”只能是草稿或推断。

### 3. 归属和 scope 是产品边界

`self`、`world`、`user/<id>`、`project/<id>` 不能只在写入时区分，读取、snapshot、检索和反思也必须保持同一隔离边界。

### 4. 更正历史，不重写历史

冲突通过 `disputed`、`superseded`、版本和审计记录处理，默认不 hard delete，也不让当前目标重新解释全部过去。

### 5. 自我修订必须慢、少、可解释

Identity 和 commitments 只允许通过带证据的治理路径改变；automatic reflection 是受限实验能力，不是持续自治的许可证。

## 当前产品主线

当前唯一主线按以下顺序推进：

1. **Truth and Safety**：scope、provenance、policy gate、只读诊断和可恢复 migration。
2. **Trustworthy Recall**：search、get、history、supersession 和真实客户端闭环。
3. **Local Product Alpha**：可运行 artifact、fresh-machine、backup / restore 和人工 release decision。
4. **Retrieval Quality**：在真实评测集上优化检索与生命周期。

具体任务、证据门和停止条件见[当前 active plan](plans/README.md)。

## 不再作为主线的内容

以下内容可以作为实验、运维辅助或未来研究存在，但不得反向决定产品路线：

- 发布证据生成器和 preflight 数量扩张；
- dashboard 视觉或远程管理；
- team / multi-tenant / OAuth；
- write-capable daemon 和持续自治；
- physics-inspired 命名、solver/controller 类比；
- 为了跟随协议而提前增加 remote transport 或 experimental tasks。

这些方向只有在核心记忆闭环提出真实需求时，才允许重新进入候选池。

## 新能力准入问题

任何新功能在进入 active plan 前，至少要明确回答：

| 问题 | 通过要求 |
| --- | --- |
| 它改善哪一个核心用户任务？ | 必须落在记录、检索、证据、纠错、恢复之一 |
| 它保护哪条核心不变量？ | 必须说明 scope、provenance、历史或治理边界 |
| 不做它会阻断当前里程碑吗？ | 如果不会，默认进入 Later / Labs |
| 最小证明动作是什么？ | 必须有可观察、可失败的证据门 |
| 失败如何回滚？ | 写操作、schema 和发布行为必须可恢复 |

如果这些问题没有清楚答案，就不应因为“看起来先进”而进入主线。

## 当前实现与原始命题的对应关系

| 原始命题 | 当前状态 |
| --- | --- |
| 原始事件底账 | `implemented` |
| Claim 与 evidence link | `implemented` |
| owner / namespace 写入约束 | `implemented` |
| explicit scoped snapshot | `implemented` |
| legacy unscoped compatibility / complete recall | `partial` |
| 可检索、可解释的 memory read contract | `partial` |
| 冲突、supersession 与 reflection audit | `implemented` |
| 可信 commitment gate | `partial` |
| 慢变量 identity 与版本回滚 | `partial` |
| 完整自治、自我或远程团队平台 | `out of scope` |

这张表比功能数量更重要：后续整理、开发和发布都必须先看它是否仍然成立。
