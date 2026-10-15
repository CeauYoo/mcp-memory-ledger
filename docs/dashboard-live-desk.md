# Memory-chan Live Desk

Live Desk 是 loopback-only 的只读观测面板，仍属技术演示 / MVP。页面不会调用 MCP 写工具、触发模型或提供管理操作；健康检查仅验证面板 HTTP 服务，不等于 provider 连通、数据库完整性或完整 `doctor` 验证。

## 新核心的管理员入口

以下原有交互同样用于新核心 owner 监控，保留原页面样式。通过[核心 CLI](local-workspace-core.md)显式启用后，公开静态壳不含运行数据；登录使用管理员独立 Bearer，仅存在当前标签页内存。API 与 fetch SSE 均鉴权；不把值放入 URL/cookie/localStorage，刷新需重登，锁定会清凭据/数据/详情并取消请求。仅使用 CLI 打印的精确绑定 URL；Host/同源检查拒绝不匹配别名与跨站请求。

新核心视图缓冲来自共享 observer，因此能汇总连接该核心的多个桥；旧 standalone 仍是单进程缓冲。两者都不是全历史或全系统客户端计数，窗口上限不变。管理员可只读查看全部 workspace，普通桥不会获得此权限，也不能在网页中授予个人共享。当前实现只用合成值验证，未启用正式管理面；真实核心网页端到端证据尚待验收。

新增管理员前端验证脚本需要已有 Chrome、Node 22、Python 3 和一个全新的绝对短输出路径：

```sh
node scripts/dashboard-admin-browser-check.mjs --chrome /absolute/google-chrome --binary /absolute/bin/agent_llm_mm --output /absolute/new-short-output
```

传入 `--binary` 后创建隔离 mock 库/registry/合成能力文件，启动真实 Rust core 与 bridge，浏览器直接连接核心，验证登录、health/SSE、锁定/重登、刷新、Back、三语言与窄屏。省略 `--binary` 仅使用合成 Node API，不能算 Rust 后端端到端。Ubuntu CI 已接入真实 Rust 模式，截图与合成 report/cleanup 摘要作为产物保存，不上传 token、原始数据库、日志或浏览器 profile。云端核心/Chrome 启动因 Unix socket `EPERM` 被阻断，尚无本次实测浏览器通过声明，也不禁用沙箱绕过。

## 实际可用的交互

- 左侧导航切换全部、反思、决策、快照、持久历史和服务健康；窄屏保留全部入口。
- 记录视图可以切换当前进程 / 持久历史，并按状态、精确 namespace 和操作/ID/摘要关键词筛选。全部是浏览器内筛选，只覆盖已加载窗口。
- 选择记录按 ID 查询详情。刷新不强制选择最新项，不抢搜索框焦点；显式选择在窄屏定位到详情，关闭后回到原记录。所选记录离开筛选窗口时明确提示；找不到详情时报告已不可用。
- 支持暂停、恢复和手动刷新。暂停不阻止手动刷新或显式切换视图。SSE 事件促使刷新，2.5 秒轮询作补充；请求有 8 秒超时，整组刷新串行、突发请求合并，切源和暂停使旧响应失效。
- 非成功 HTTP 状态、无效 JSON 和网络异常可见；断线不再显示正常连接。保留上次成功数据时明确提示其可能过期。
- 简体中文、English、日本語覆盖静态文案、筛选、动态状态、错误与标签。初次按浏览器语言选择，手动选择保存在当前 origin 的 localStorage；禁用存储仍可用。用户记录内容和协议标识保持原文。

## 数据范围

顶部指标是**当前服务进程内缓冲区的保留记录数**，不是全历史、调用次数或成功写入次数；standalone不汇总其他进程，核心模式仅汇总连接本核心的桥。工具调用和关联自动反思可产生多条记录；缓冲区满后旧记录会离开内存。卡片中的反思计数只计原始 `kind=reflection`，不会被描述为“已处理冲突”。

列表最多加载最近 100 条。持久历史来自该服务连接的同一数据库，可能包含其他进程的记录；它也只显示最近 100 条，不能把空筛选结果理解为全库不存在。进程 PID、随机实例 ID、启动时间和容量来自 `GET /api/status`。实例 ID 标识面板实例，不认证调用者；历史 `actor_id=mcp-stdio` 不足以区分不同进程。

呈现分类兼容实际记录：`decide_with_snapshot` 和 `build_self_snapshot` 通常是 `kind=tool`，仍分别出现在决策、快照视图；反思视图包括 reflection/trigger 类型、`run_reflection` 和通过反思写路径实现更正的 `supersede_memory`。详情保留原始 kind。查询历史本身不会被冒充反思执行。

历史详情保留原始 UTC `occurred_at`（对外为 `timestamp`）、actor、关联 ID 和已存储的 request/response/diagnostic 摘要字符串及 redaction version；缺失摘要仍为 null。页面只以文本展示，不将记录内容作为 HTML 执行。已有摘要脱敏不是通用秘密发现服务，面板不应对外暴露。

## 多个 MCP 会话共用端口

每个 stdio MCP 会话拥有自己的 dashboard 生命周期。`required = false` 时，如果端口已被占用，会话继续提供 MCP、保留有限进程事件缓冲，并每秒尝试一次本机端口绑定；占用者退出释放端口后，一个存活会话可接管。等待绑定不发送 HTTP 或模型请求，也不终止占用者。只有 `AddrInUse` 进入等待；其他启动错误仍告警后禁用 dashboard。`required = true` 仍在首次绑定失败时终止启动。

等待与监听均随所属 MCP 会话关闭而取消，不建立独立后台服务。接管产生新的 `/api/status` 实例 ID 和进程 ID，页面会清除旧实例选择并重新加载。进程缓冲仍只含接管者自己的事件，不能恢复旧进程的内存事件；同一数据库的持久历史保持可查。不同数据库应配置不同端口，否则端口释放后展示的数据源可能变成另一个会话的数据库。

本修复不提供全部客户端实时事件聚合、不保证所有 MCP 会话关闭后的网页可用，也不能追溯已经退出进程的终止原因。仓库通过不代表已安装运行时已更新。

## 代码边界

- `lifecycle.rs`：会话持有的监听与可取消端口等待；
- `http.rs`：固定只读路由、元数据与 SSE；`assets.rs`：编译时资源白名单，无任意文件读取。
- `projection.rs`：事件和历史记录的纯投影；`recorder.rs`：有限进程缓冲。
- `static/index.html` / `dashboard.css`：保留原版满幅 Memory-chan 横幅、彩色图标/卡片、侧栏角色、右侧详情与深色载荷；桌面紧凑表格，窄屏重排。主页面只保留操作/状态/范围短标签，无额外说明段落或帮助区。
- `i18n.mjs`：三语言文案与语言选择；`state.mjs`：纯筛选、排序、连接状态与数据校验。
- `transport.mjs`：HTTP 校验、取消、超时和串行刷新；`render.mjs`：安全文本/键控记录渲染；`app.mjs`：交互与生命周期。

`base_path` 可嵌套，例如 `/qa/desk`；无尾斜杠入口重定向到 `/qa/desk/`，模块、图片和 API 保持在该路径下。它不是认证机制。所有新增路由仍拒绝 POST/PUT/PATCH/DELETE。

## 可复现隔离验证

基础验证：

```sh
cargo test --test dashboard_config --test dashboard_recorder --test dashboard_projection --test dashboard_http
node --test tests/dashboard/frontend.test.mjs
```

macOS 上使用已安装 Chrome、Node 22+ 和 Python 3；脚本不下载软件。输出目录必须尚不存在：

```sh
cargo build
node scripts/dashboard-browser-check.mjs \
  --chrome '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' \
  --binary target/debug/agent_llm_mm \
  --output target/dashboard-browser-qa
```

`--python` 可指定已有 Python 路径。该脚本创建 mock 配置、独立 SQLite、随机 loopback 端口和独立 Chrome profile；通过真实 MCP stdio 写入 28 条合成观测、创建快照/决策和一次参数失败。记录每次检查、截图、请求方法和合成 MCP 证据。故障代理仅用于注入 HTTP/JSON/延迟故障与不可信长文本。浏览器断线通过 CDP 模拟，不改变系统代理。

覆盖三语言 × 320/375/768/1024/1440 CSS 像素、200% CSS 缩放重排、键盘焦点、减少动画、选择保持、历史、状态/命名空间筛选、失败恢复和只读请求。CSS 缩放不能代替每个平台的浏览器 UI 缩放；Chrome 证据也不代表 Safari/Firefox 或 Windows/macOS 全版本兼容。未安装 Node/Chrome 时先报告缺失项，不擅自安装。

这是**隔离运行验证**，不是用户真实数据库体验、付费模型效果或新机器安装。脚本不读取用户配置或凭据。报告和截图留在忽略的输出目录，不提交数据库、日志、profile 或用户截图。

## 更新已安装运行时

仓库构建和 CI 通过不会更新已安装的运行中二进制。当前进程仍会提供旧版内嵌资源，刷新浏览器不能加载新代码。更新需要在用户认可的维护时段让 MCP 客户端停止该服务、校验并替换候选二进制、重新启动，再确认 `/api/status`、页面版本行为和工具重连；这会短暂中断 MCP 连接，应预留原二进制作为回退。未获运行时切换许可前，不停止用户进程、不改数据库路径或客户端配置。
