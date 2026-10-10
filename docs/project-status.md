# 当前实现状态

更新：2026-10-10。当前定位：**local-first technical MVP，尚未正式发布 binary package**。历史运行时实现基线为 `9ba0050d3ffb9228b02e2d0895b4e76d11ead989`；本轮新增源代码绑定的本地 portable package 构建/解包验证，Windows wrapper 行为测试和修复，以及 schema7 全局 self-model 追加版本。包可在本地生成，不代表人工发布批准。

## 已实现

| 能力 | 当前合同与证据入口 |
| --- | --- |
| 32 个 MCP stdio 工具 | [按用途索引](tool-reference.md)，以运行时 `tools/list` 的 schema 为准 |
| Scoped Event / Claim / Episode / Reflection 浏览、lookup、修订与证据关系 | 显式 namespace、scope-first 查询、不跨范围扩大；[工作流](runnable-memory-workflow.md) |
| 原子 Claim 纠错与持久 request_id 回执 | [更正工作流](runnable-memory-workflow.md)、[反馈候选合同](memory-feedback-experience.md) |
| 持久反馈候选 | 提案、确定性验证、拒绝、事务内再验证与原子提交；来源标签不认证真实性 |
| 派生 FTS5 与短 CJK fallback | 可检查/重建；基于原文再次验证 scope 和字面匹配；不提供语义 embeddings |
| 有界任务上下文 | 完整 Claim/Event 优先、来源关联 Episode、可选诊断；[完整 JSON 字节合同](context-diagnostics.md) |
| Caller-owned 操作预算 | 仅 recall/context/reflection 三条路径；可观察 stop/error receipt；[非持久配额](caller-operation-budget.md) |
| 丰富 Episode 与版本化经验候选 | source-linked、inspect/reject/revise/rollback/activate；候选只供召回，不自动执行 |
| Schema 7 | [记录/观察时间与纳秒排序](temporal-metadata.md)、[独立 Reflection origin/affected scope](reflection-scope-history.md)、规范化来源关系 |
| 全局 self-model 追加版本 | 初始化/迁移真实 baseline、CAS、同范围来源 diff 与显式补偿回滚；[实验性单用户合同](self-model-versions.md) |
| 只读 scoped export | 有界、一致、同范围、无写日志；[交换而非备份](scoped-export.md) |
| 安全本地生命周期 | 显式 init/migrate、只读 doctor、结构读回、排他 writer admission、迁移前备份/恢复演练；[操作手册](database-operations.md) |
| 内部模块整理 | [SQLite、MCP、自动反思私有模块边界](implementation-module-boundaries.md)，未新增框架或权限 |

新 Claim 有记录时间；历史未知时间保持 null。Reflection 的 origin 与 affected scope 各有职责，后者不是另一项可见性授权。旧 receipt/fingerprint 字节兼容性有迁移回归，不能通过迁移伪造历史或改变已成功请求的含义。

## 本轮交付：schema7 全局 self-model 版本

本地实现增加 append-only 全局序列、只读同范围 written-component history/diff、可选 expected-version guard 和需显式确认的补偿回滚。旧库迁移只捕获当前投影，不伪造过去的有效时间；漂移 fail closed。新全局写、Claim 效果、来源、版本和 receipt 同事务。新代码的本地测试/独立审查与精确提交 CI 必须分别记录，旧 schema6 数值不替代验证。

本轮源代码本地验证：673 项 all-feature Rust 测试全部通过（0 failed/ignored/filtered），严格 all-target/all-feature Clippy、fmt、独立只读审查、三组实际二进制离线闭环、9 项 Python fixture 与 33 项包合同测试通过。二进制/源码身份和范围见[schema7 阶段结果](plans/2026-10-10-schema7-results.md)。精确提交 CI 与发布仍单独验证，不继承历史 head 绿灯。

## 本轮交付：可追溯包与 Windows wrapper

- [Portable package](portable-packages.md)：从精确 Git commit/tree 构建本机原生二进制，包含校验信息、许可、隔离 mock 配置与使用说明；验证先检查归档和来源，再在无 Rust 的子进程环境完成真实二进制闭环。
- PowerShell wrapper：补齐 literal 路径、仓库根目录相对路径、调用者配置环境恢复及多余参数拒绝；真实平台行为由 `scripts/test-windows-wrapper.py` 在 Windows CI 执行。
- 这些检查是受控本地/CI 安装模拟；M1 真实用户客户端、真实 fresh-machine 十分钟验收、live provider 和人工 release decision 未因此完成。
- Windows 回归按真实原生进程检查参数透传；Python 数据库验证显式关闭连接，避免临时目录清理持有文件锁。Windows 的各 Python 检查分成独立 CI 步骤，后续成功命令不能掩盖前一项失败。
- 任何精确提交是否通过，以该提交 PR checks 为准；下方保留原实现基线，不将旧绿灯作为本轮结果。

## 2026-10-10 包与 Windows 精确提交证据

包/wrapper 阶段 `0ca8bccc64cadb3f1eae9d1a74bad602a1c60bc9` 的 [CI run 38035347484](https://github.com/yooyui/mcp-memory-ledger/actions/runs/38035347484) 三平台全部成功：Linux x86_64 与 macOS ARM64 各 634 项 all-feature Rust 测试，Windows x86_64 为 247 项 native Rust 测试；三平台均通过 13 项 Python fixture、40 项包回归和精确源码构建/无 Rust 解包闭环。Windows PowerShell 7.6.6 实际通过 12 项 wrapper 测试，其中原生参数/环境测试覆盖 20 种组合。

首轮 Windows 检查发现并修复了 doctor 参数绑定、SQLite 连接关闭和测试 PATH 基线问题；各 Python CI 步骤独立传播失败。以上是指定 host 与该提交的证据，不是所有 Windows/macOS 版本、真实 fresh-machine、用户客户端或正式发布认证。Schema7 后续 head 须重新执行自己的 CI。

## 验证证据（明确绑定提交）

实现基线 [9ba0050](https://github.com/CeauYoo/mcp-memory-ledger/commit/9ba0050d3ffb9228b02e2d0895b4e76d11ead989) 的 [CI run 37930021202](https://github.com/yooyui/mcp-memory-ledger/actions/runs/37930021202) 三平台成功：

- Ubuntu / macOS：各 633 项 all-feature Rust 测试。
- Windows：247 项 native Rust 测试，不代表全部 shell wrapper parity。
- 三平台：各 9 项 Python fixture 测试、installed-copy 重启/备份恢复、反馈/经验 evaluator、时间/范围/export 可执行工作流。
- 同源本地：551 项默认 / 633 项 all-feature Rust 测试，无失败或忽略；fmt、严格 Clippy、32 项 status-sync 及 diff 检查通过。

这些是实现基线的证据；后续提交必须单独核验 CI，不能继承旧 head 的绿灯。[测试指南](testing-guide-2026-03-24.md)提供复现入口，[最终 70 项核对](plans/2026-10-09-original-plan-final-reconciliation.md)记录源码身份和逐项边界。

固定离线代理指标 8/10，两个语义改写 miss 原样保留；50 个额外 query variants 单独计数，不能算作 60 项独立任务。测量同时记录延迟退化、额外上下文字节和存储成本；shared-host 合成数据不是生产容量保证。真实模型任务成功率、token 和费用收益尚未测量。详见[评估方法](evaluation-methodology.md)。原始运行输出保留本地；已发布历史证据不改写身份或数值。

## 部分实现 / 仍有限制

- 显式 scoped snapshot 已实现；省略 namespace 的旧接口兼容路径仍未统一隔离。namespace 不是 authn/authz。
- legacy 全局 self-model 治理仍是 experimental。`decide_with_snapshot` 只执行服务端 commitment gate，结果标记 `experimental_non_authoritative`，不是完整 policy verdict。
- identity/commitment 已有有界追加版本、记录边界 effective-time、同范围来源 diff 与显式补偿回滚；迁移前有效时间保持 unknown，不提供回溯调度或跨范围全局快照。
- 诊断是有界且不完整的可观察样本；缺失诊断不证明没有冲突，记录年龄不代表过期。
- 索引损坏可触发字面读取 fallback，但不保证受损 trigger 下写入仍健康；检查与重建是全库工具。
- 所有历史与成功回执 retain-all；没有自动 retention、tombstone 或 compaction。

## 尚未关闭的产品与研究门

1. 用户真实 MCP 客户端闭环及 fresh-machine 安装证据。
2. 正式发行包与人工 release approval、实际用户机器支持范围及 rollback note；本地构建器和 CI wrapper 验证属于实现基础。
3. 经单独授权和预算约束的真实同模型效果 / token / 成本实验。
4. 更广泛 self-model 生命周期政策、语义检索，以及需独立需求和权限的 remote/team/autonomy。

当前没有正式 Alpha/Beta/GA 或 production-ready 声明。启用 dashboard 仍只允许 loopback，daemon 仍 observe-only。

## 文档权威与历史

[路线图](roadmap.md)回答先后顺序，[唯一 active plan](plans/2026-07-10-product-replan.md)管理执行，[reality gates](product/follow-up-reality-gates.md)约束完成声明。

旧 schema、工具数、验证基线和分支状态已完整保留在[带日期的历史状态](project-status-history-2026-10-09.md)。历史 70 项 baseline、schema5 / schema6 阶段报告都是当时证据；以最终核对解释其增量，不覆盖或美化旧结果。
