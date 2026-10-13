# 2026-10-13 验收工具与隔离运行验证

本轮边界：技术 demo / MVP；只修改 `CeauYoo/mcp-memory-ledger` 的 `dev_work_dots`，上游草稿 PR #3 目标 `dev-work`。无合并、发布、部署、真实模型调用或用户配置修改。

## 证据分层

| 验证 | 当前口径 | 能证明什么 |
|---|---|---|
| `--stub-evidence` | 离线模拟文件；0 请求 | 文件格式、脱敏防线，不证明连通 |
| 回环 HTTP fixture | `offline/protocol-fixture` | 三协议 wire、解析、边界、失败路径 |
| `--live` 对外部 endpoint | 必须另行凭据交接并获调用授权 | 仅实际调用成功路径；不自动证明错误处理、模型效果、费用或 MCP 客户端验收 |
| Python MCP harness + 实际二进制 | 隔离运行验证，mock provider | 写入、重连、检索、证据、更正重放、历史、跨 namespace 负向检查 |
| 候选包 unpack + 空 PATH | 本机隔离运行验证 | 无 Cargo/Rust 的启动与恢复，不能称新机器验收 |
| 用户 Codex 客户端 | 尚未执行 | 需要用户配置最小条目与真实 UI/客户端结果 |

2026-10-13 正常网络、默认代理设置下，对 `https://yooyui.chat` 公开首页无认证 HEAD 返回 HTTP 200。没有请求模型路由；不能从首页推断 `gpt-6.1-luna` 可用。

## 请求和费用合同

- 使用共享 HTTP 层，保留系统默认代理；无隐式重试、重定向、provider fallback。
- 每次 runner 最多 2 次 POST（decision，然后 self-revision）；任何失败停止后续调用。每次完整序列化 JSON 输入最多 16384 字节；不是精确 tokenizer token 数。
- Chat 常规运行输出 `max_tokens=2048`；验收输出上限 512。Responses 用 `max_output_tokens`，Anthropic 用 `max_tokens`；native 配置更低值优先。每次调用独立计数，不是跨运行的总额度管理器。
- 新 evidence 目录在调用前独占创建，已有目录不覆盖。`request-accounting.json` 在首请求前及每次探测后保存最小账本；最终 `run-accounting.json` 汇总状态，记录尝试次数、每次输入字节、输出上限、HTTP 状态和仅数字 usage；缺失值保持 `null`。失败也落盘，不将失败计作免费。被中断/崩溃或磁盘失败的运行仍需从服务商账单核对。
- USD 单价和账单未知，`actual_cost_usd=null`、`software_enforced_usd_cap=false`。用户的 5 美元专用总额度必须由服务商侧硬配额/预付额度及账单核对管理；本工具不能声称软件强制该美元上限。
- 两次成功请求仅将 decision/revision 项标 passed；错误处理与人工脱敏审核保持 `not_tested`。模拟 fixture 永不满足 live preflight。真实 MCP 客户端验收是独立项。

## 可复现离线验证

```sh
cargo test --offline --all-features
cargo clippy --offline --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
./scripts/status-sync-check.sh
python3 scripts/local-memory-smoke.py --binary target/debug/agent_llm_mm --output target/reports/acceptance-memory
python3 scripts/temporal-scope-export-smoke.py --binary target/debug/agent_llm_mm --output target/reports/acceptance-scope
python3 -m unittest discover -s tests -p 'test_portable_package.py'
python3 scripts/portable-package.py build --offline --candidate local-mvp-acceptance-rc.1 --source-ref HEAD --output target/acceptance-package --target-dir target
```

用生成的 archive/build receipt 调用 `portable-package.py verify`，显式传入 `--expected-commit`、`--expected-tree` 和全新 `--output` 目录。验证器校验源码与包哈希，在临时 HOME 和空 PATH 下启动实际二进制，用 mock 配置完成闭环，并经 SQLite backup API 备份、恢复到新路径。恢复前后核对 dump、integrity、foreign keys、只读 doctor 的数据库字节和 MCP 检索/历史/证据。Windows CI 同样运行验收 fixture 与候选包验证。

## 用户客户端的最小后续步骤

本机仅检查标准 Codex 配置文件是否存在，未读取其内容，未修改配置或读取凭据。用户需自行在所用客户端配置中加入[已有 TOML 示例](local-mcp-integration-2026-03-26.md#2-codex-toml-示例)的独立 server 条目，指定候选二进制绝对路径、`args=["serve"]` 和隔离 mock TOML 的绝对路径。重载后依次完成[完整工作流](runnable-memory-workflow.md)：写入合成记录、断开/重连检索、查看证据、更正及历史、换 namespace 验证不可见。不要切换现有正式数据库。

真实模型调用另行由用户控制凭据交接、服务商硬额度、协议/API root 与模型可用性确认。本任务不收集、搜索、复制或保存凭据，不执行该步骤。

## 执行结果

- 独立只读审查完成；两项证据可靠性缺陷已修复并补回归，复查无剩余阻断项。
- 首轮全量 699 项 Rust 测试、严格 Clippy、fmt、33 项状态同步通过；审查后新增 3 项回归随最终检查执行，最终计数与精确提交 CI 记录在 PR #3。
- 40 项候选包合同测试、13 项 Python fixture、实际二进制重连/更正/备份与 temporal/scope/export smoke 通过。固定离线 evaluator 保持 8/10（两个不支持的 paraphrase 保留），不能解释为付费模型效果。
- 在本机临时 HOME、空运行 PATH 下，实际二进制 init、只读 doctor、MCP 写入/重连/证据/更正重放/历史、foreign direct lookup/history/recall 隔离、备份并恢复到新路径、恢复后历史与证据一致性均通过。模型调用数 0，namespace 泄漏 0。
- 源码绑定候选包与最终三平台 CI 的精确 SHA/运行链接，以 PR #3 对应提交结果为准；没有发布候选包或创建正式 release。
- 真实模型、真实用户 Codex 客户端、新机器验收仍未执行。
