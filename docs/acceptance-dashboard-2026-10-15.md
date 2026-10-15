# 2026-10-15 Live Desk 验收

## 源码与范围

修复起点为 `CeauYoo/mcp-memory-ledger:dev_work_dots` 的 `55590d9dd804d990281520e3016f438e9ee97160`。重新核对发现上游 PR #3 已由外部于 2026-10-15 合并，`dev-work` 为 `e45c14d76a3c6651115d55cdd3109959be69dba9`；本任务不执行合并。后续仅推送已授权的同一开发分支，并创建新的上游草稿 PR。精确提交与最终 CI 链接记录在该草稿 PR，不能继承旧 head 的绿灯。

本轮修复真实导航、静态假状态、刷新重置选中、断线吞错、历史未接入、中英日和响应式布局。详细合同、模块边界与复现命令见 [Live Desk](dashboard-live-desk.md)。原有图像保留；用户未跟踪文件不纳入提交。已安装服务和用户数据库未被重启、替换或写入。

## 本地证据

- `./scripts/test-tier.sh full`：702 项 Rust 测试通过，0 failed / ignored；其中 dashboard 的配置/HTTP/投影/缓冲测试共 18 项。
- `cargo clippy --all-targets --all-features -- -D warnings`、fmt、diff check、33 项 status-sync 通过。
- `node --test tests/dashboard/frontend.test.mjs`：7 项通过，覆盖真实操作分类、精确筛选、状态推导、无效数据、三语言键/占位符完整性、HTTP/JSON/超时和请求合并。
- 本机已有 Chrome `155.0.8059.41`：18 项真实浏览器检查通过，0 JavaScript 异常。初始合成记录经真实 MCP stdio 产生，SSE、轮询、HTTP API 与详情查询均经过候选 Rust 二进制。
- 检查覆盖导航、选中和键盘焦点保持、历史原始时间/nullable 摘要、快速切源、HTTP/JSON/离线恢复、失败请求取消慢兄弟请求、暂停/手动刷新/SSE 恢复、空来源/过期详情、语言保存、只读健康与全部 GET 请求。
- 视觉按原截图保留 `agent-llm-mm / live desk` 品牌、满幅角色横幅、彩色导航/卡片、侧栏贴纸、右侧详情和深色载荷；桌面恢复紧凑表格。删除新增宣传标题、介绍/计数/健康/载荷解释段落与多余区域，不把它们转移到 tooltip 或帮助中；范围仅保留进程、缓冲和最近窗口短标签。
- 三语言 × 320/375/768/1024/1440 CSS px 共 15 组布局检查通过；200% CSS 缩放、长操作名/namespace、恶意 HTML 文本、减少动画、移动详情与返回原行均通过。实际查看桌面、中文平板、日语 320px、移动详情及缩放控件截图，修正了占位加载后详情偏移和缩放下过窄卡片。
- 中断验证：向测试脚本发送 SIGINT，退出码 130，自有 Chrome、fixture、MCP 三个 PID 均消失；挂起自有子进程的强制退出路径约 2 秒完成。
- 独立审查发现并修复了 SSE timer 取消后未复位、移动详情远离点击位置、更正呈现分类遗漏、失败请求组残留、测试清理期限冲突。独立审查确认并发和清理阻断已关闭，并逐张对照原截图确认恢复视觉/信息密度且没有多余说明段落；精确提交 CI 结果在草稿 PR 汇总。

原始本机证据位于忽略的 `target/dashboard-review/`：`full-tests.log`、`clippy.log`、`browser-approved-style/report.json` 与 PNG、`browser-interrupt-final/interrupt-report.json`、`forced-cleanup.json`、`cleanup-signal-race.json`。运行目录使用新的合成数据库、mock provider、随机 loopback 端口和独立 Chrome profile；没有读取真实 API key，没有收费请求，没有修改系统代理。原始日志、数据库、profile 和用户截图均不提交 Git。

## CI 与尚未完成事项

Ubuntu/macOS 延续完整 Rust、离线工作流、源码绑定 portable package 与无 Rust PATH / 备份恢复检查；Windows 额外纳入 dashboard 四套 Rust 测试，三平台都运行前端纯函数/网络合同测试。真实浏览器检查在这台 Mac 执行，不宣称三平台均有 GUI 浏览器验收。

所有安装/运行证据属于**隔离运行验证**，不构成新机器、Safari/Firefox、全部 OS 版本、真实用户自然语言体验或付费模型质量证明。200% CSS 缩放不是浏览器 UI 缩放的完整等价测试。

部署仍需单独安排：用户正在运行的二进制包含旧页面，仓库修复不会热替换。更新计划是停止对应 MCP 服务、保留旧二进制、校验替换候选、重启并验证实例状态和工具重连；会短暂中断连接，未获运行时切换许可前不执行。无正式发布、部署或合并。
