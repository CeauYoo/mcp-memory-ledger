# 2026-10-15 体验复核、实时监控与 Codex 客户端验证

本轮仍为 technical MVP。工程检查只使用合成数据、mock provider 和回环 HTTP；未读取或交接凭据，未追加收费请求。历史真实运行、离线重评、实际 Codex MCP 客户端和新机器安装分别计数。原始对话、请求、数据库、冻结运行目录仅保留本机，不纳入 Git。

## 历史 30 场景的复核结论

以下是 2026-10-14 已有运行的只读复核，**不是修复后的真实复测**。原始 292 个文件及当时评分保留。

| 项目 | 结果与限制 |
|---|---|
| 执行 | 30 场景、90 轮、90 次物理请求、0 重试，全部 HTTP 200 / completed_text |
| 模型标识 | 请求 `gpt-5.6-luna`；90 次响应均自报 `gpt-6-luna`，无法据此认证实际模型身份 |
| Token | input 278563 / output 46932 / total 325495；reasoning 12588 属于 output 明细，不重复相加 |
| 费用 | 90 次 cost 均未知；不能确认累计美元支出或声称软件强制 5 美元封顶 |
| 延迟 | 中位数 4.588 秒，nearest-rank p95 9.800 秒；总历时 484.705 秒 |
| 原始自动评分 | 82 通过 / 8 失败，原评分不改写 |
| 仅重评保存的响应 | 5 个有限等义表示误判修正后为 87 / 3；无新模型调用，不能作为体验通过率 |
| 正文与证据联合人工复核 | 86 轮未发现实质问题，4 轮存在问题；26 个场景未发现问题，4 个场景有问题 |

四个问题轮次：S16-01 正文把执行准备排到运行日前（原输入缺少明确日期锚点，不能声称模型忽略了已知当前日期）；S19-01 将另一个 Claim 的来源事件用作目的地证据；S23-02、S25-02 因项目名与 namespace 身份不明而拒绝使用当前范围内正确值。

修补包括：S16 固定准备起点；明确当前客户端选择的项目名称与 namespace 绑定；要求来源 ID 从对应 Claim 原样复制；对指定 decisions 字段接受有限等义表示，同时保持量值、单位、时区、范围、状态及引用严格核对。新增正文日期检查可识别 S16 中显式过期或非法日期；不覆盖未写年份、含糊自然语言或全部计划语义。全部回答仍需人工复核。

## 一次隐藏输入后的定向与全量复测

入口为仓库根目录的 `python3 -E -s -B tools/experience/run.py`，macOS 可双击 `tools/experience/Start-Retest.command`。需要 Python 3.10+ 和已构建的 `target/debug/agent_llm_mm`；缺少工具时停止，不自动安装。完整合同见 [README.txt](../tools/experience/README.txt)。

1. 一次隐藏输入 key 后，自动执行 S16/S19/S23/S25 各三轮，共 12 轮，覆盖问题轮及相邻对话。
2. 定向阶段首次客观、正文日期、协议或存储检查失败即停止；不得自动继续全量或重跑。
3. 自动门通过后执行完整 30 场景 × 3 轮。正常 102 请求，两阶段共享最多 10 次显式重试，总次数不超过 112，每轮最多重试一次。自动门通过不代表正文语义通过。

每次完整 JSON 输入不超过 256 KiB，输出上限按场景为 2304/4096/4608 tokens；响应不超过 4 MiB。保留正常代理/TLS，无重定向、SDK 隐式重试或全程自动恢复。先持久预留尝试再发送，子运行在首请求前登记路径；收尾失败时总账标为未知并保留已知下界，不能把已发送请求计为零。原有 Rust provider certification 的两请求/16 KiB 合同保持独立。

当前体验 fixture 延续历史请求目标 `gpt-5.6-luna` 和 Responses 路由；此前 `gpt-6.1-luna` 的验收意向没有因此获得认证。返回模型名称仅作未核实标签。真实复测等待用户自己的凭据交接与执行；不搜索旧 key。服务商侧 5 美元总额度必须包含历史消耗，未知价格不能软件封顶。没有 SEND、别名确认或逐轮确认。

所有新输出位于已忽略的 `target/experience-acceptance/`；每次新建独立合成数据库。`runs/` 保存运行证据，`staged/` 保存阶段计划和汇总。未提供构建 receipt 时，二进制 SHA256 仅证明文件身份，不证明源码绑定。

## 实时网页在哪里

**体验复测监控**：隐藏输入后、首个模型请求前自动打开 `http://127.0.0.1:<自动端口>/`，终端打印可用地址。它是运行中的 HTTP 服务，每秒请求 `/api/status`，跨 MCP 子进程重启与阶段切换持续显示状态。端口由操作系统分配以避让占用。最终状态保持到 Ctrl+C；完成后不再发模型请求。静态 `report.html` 另存本地，不能代替此监控。

监控只提供状态、用量、延迟和合成场景标题；不提供完整回答、诊断文件、目录服务或数据库。仅监听 loopback，校验 Host，无 CORS。它不能作为远程服务或访问认证边界。

**正式 MCP 运行监控**：原有 Memory-chan Live Desk 保留。在独立 mock 配置中启用：

```toml
[dashboard]
enabled = true
host = "127.0.0.1"
port = 8787
```

先为新的合成数据库执行 `init`，再用相同配置启动 `./scripts/agent-llm-mm.sh serve /absolute/path/mock.toml`，浏览 `http://127.0.0.1:8787/`。以运行配置的实际端口为准；占用时显式选其他 loopback 端口。dev 模板默认关闭，prod-local 模板默认打开。服务随 MCP 进程存活；单独打开静态报告不会启动服务。页面每 2.5 秒轮询 summary/events，后端另提供 SSE；持久 operation history 与当前会话事件分开。

本机用新 mock 数据库、自动端口和实际二进制验证页面/health 为 200，合成 ingest 后事件计数由 1 增至 3、tool/reflection 各增加 1，SSE 收到数据，持久历史可查询。探测结束关闭该服务；不声称旧 8787 监听仍在运行。体验复测每轮重启 MCP，故关闭其单进程 dashboard，由上述独立体验监控持续展示总流程。

## 实际 Codex app-server 客户端

`tools/codex_client/acceptance.py` 使用已安装的 Codex CLI experimental app-server MCP API；本机验证版本 `codex-cli 0.162.0-alpha.17.2`。不是模拟 Codex，也不调用模型来选择工具。

```sh
python3 -E -s -B tools/codex_client/acceptance.py \
  --codex /absolute/path/to/codex \
  --binary /absolute/path/to/agent_llm_mm \
  --output target/reports/codex-client-new-run
```

输出目录必须不存在。脚本创建临时 HOME/CODEX_HOME 与新 mock 数据库，仅用进程参数添加五个 MCP 工具，模型 provider 指向不提供模型服务的 loopback 端口；从不发送 `turn/start`。不加载历史凭据，不修改用户全局配置、项目配置或信任设置；遇到任何服务端审批请求停止。

本机结果：3 个真实客户端进程、2 次重连、17 次 MCP 工具调用、0 模型轮次；写入、重连检索、来源 Event、显式更正、幂等重放、旧值 Superseded、新值 Active、历史、外部 namespace 的 Claim/Event/history 拒绝及未知查询为空全部通过。回执包含客户端版本、二进制哈希和明确边界。

**未验证**：桌面 UI 注册、用户日常聊天中的模型自主工具选择、新机器安装。experimental API 可能随客户端版本变化；失败时停止，不降级成模拟后声称真实客户端通过。

如需桌面持续接入，在独立受信任项目中使用[最小五工具示例](../examples/codex-mcp-acceptance.example.toml)，替换二进制和独立 mock 配置绝对路径，显式 init 后加入该项目的 `.codex/config.toml` 并重载。Codex 只加载受信任项目配置；新增持久配置或信任由用户确认。此次只准备方案，未执行全局写入。官方说明：[Codex MCP](https://learn.chatgpt.com/docs/extend/mcp?surface=cli)。

## 离线验收与剩余门禁

```sh
python3 -E -s -B -m unittest discover -s tools/experience -p 'test_*.py' -v
python3 -E -s -B -O -m unittest discover -s tools/experience -p 'test_*.py' -v
```

覆盖实际 MCP 与合成 HTTP 的 90 轮及 102 请求两阶段路径、共享次数预算、来源错配、旧正文日期、失败账本、监控动态更新/自动端口/路径与 Host 拒绝。三平台 CI 新增离线套件，并继续执行已有 Rust、包构建、空运行 PATH、备份恢复到新路径及只读验证；CI 不要求安装 Codex、不执行收费请求。

本机已通过 36 项 Python 离线测试（普通及 `-O`）、审查修正后的 8 项阶段/监控回归、702 项 all-feature Rust 测试（0 failed/ignored）、fmt、33 项状态同步、132 个相对文档链接检查。实际 Codex 脚本最终版本再次通过 17 次 MCP 工具调用；原真实运行 292 个文件哈希复核无变化。独立审查修正了 Windows UTF-8 读写、阶段/全程计数标签和未知总账时的用量完整性声明。

首轮新 CI 的 Windows 36 项体验测试仅因诊断文件 POSIX `0600` 断言失败（Windows 返回 `0666`）；90 轮功能步骤和另一个 102 请求阶段测试已执行完成。后续修正只对 POSIX 检查完整模式位，Windows 检查可写标志，全部功能断言保留。Windows 的访问限制取决于输出目录 ACL，`chmod/st_mode` 不能作为仅当前用户可读的证据；本工具不修改系统 ACL。

原始 2026-10-13 [隔离安装验证](acceptance-2026-10-13.md)保留历史身份；本轮最终精确提交 CI 以草稿 PR #3 的对应 SHA/checks 为准，不继承旧提交绿灯。以上仍只证明本机或 CI 隔离运行，不能标为 fresh-machine。

尚待：用户触发的修复后定向及全量真实复测、全部正文人工复核、服务商账单核对和实际模型身份、桌面持续注册及普通聊天体验、新机器验收。无合并、正式发布或部署。
