# 本机核心与固定工作空间桥

## Task continuity increment

The current source adds six scoped task tools (31 allowed / 7 global denied). They use the same per-call namespace binding, grant refresh and revocation checks, including receipt replay. [Task data](task-ledger.md) is knowledge, not execution authority. The [Codex adapter](codex-task-context-adapter.md) uses the bridge and does not open the ledger directly; adapter state is separate from registry and DB backup.


旧 project namespace 不会通过 register 自动接入。基础事实首片支持先 `adopt-preview` 再 `adopt` 创建新 UUID 映射；v2 registry 允许保留旧域，握手与逐调用固定核验 namespace。高级历史支持范围、锁与 standalone 后续写入边界见[旧域显式接入](workspace-legacy-adoption.md)。

状态：2026-10-15 开发切片，隔离验收进行中。它是 technical MVP 的可选入口，不替换现有部署，也不安装开机服务。完整证据见[验收矩阵](workspace-core-acceptance.md)。

## 两种运行方式

| 入口 | 生命周期与权限 |
|---|---|
| `serve` | 原兼容模式，每个客户端启动自己的运行时；32 个旧工具。namespace 是数据过滤参数，不是会话授权。 |
| `core` + `bridge` | macOS/Linux Unix socket，Windows 本机命名管道 candidate；一个核心持有数据库和共享操作缓冲，多个桥只转发 stdio。连接绑定注册的 workspace；开放 25 个经过范围检查的工具。 |

Windows 新增本机命名管道 candidate，继续支持原 `serve`。`--socket` 仍接收私有目录中的绝对文件路径，内部按当前用户 SID 与路径散列为本机管道名，不创建 socket 文件，不接受 TCP/远端 pipe。平台实现共用 session/framing/退出排空，须以本轮 Windows CI 与包烟测作为运行证据，不能继承旧模式通过。详见 [Windows 指南](development-windows.md)。

这是同一操作系统用户下的可信启动合同，防止模型用工具参数访问其他工作空间。它不隔离能自行读数据库、改注册表或启动任意程序的恶意同 UID 进程。`workspace_id`、路径和客户端标签不是密码。不要把 Unix socket 或无认证的旧 dashboard 转发到网络。

## 最小显式配置

以下仅用于自己新建的隔离目录。替换绝对路径；已有正式数据库必须先单独规划切换，不要混用 `serve`、迁移程序与 `core` 同时写同一个库。

```sh
mkdir -m 700 /absolute/local-state
mkdir -m 700 /absolute/local-state/ipc
/absolute/bin/agent_llm_mm workspace --registry /absolute/local-state/workspaces.json register --root /absolute/project-a
```

首次普通注册返回稳定 `id` 和 `project_namespace = project/<id>`；显式 adopted workspace 保留旧 project namespace。同一 canonical root 重复注册返回原 ID 与原 namespace。注册表原子替换、私有权限、结构验证；损坏或锁文件冲突失败关闭，不覆盖原文件。Windows 锁删除/共享争用会短暂重试：有可见普通锁最多等待2秒，未确认锁存在的访问拒绝仅100毫秒宽限；持续拒绝保留原错误，绝不自动删除未知锁。

核心继续使用现有 TOML 格式：

```toml
transport = "stdio"
database_url = "sqlite:///absolute/local-state/memory.sqlite"
[model]
provider = "mock"
[dashboard]
enabled = false
[daemon]
enabled = false
```

新库先显式初始化，再在独立终端启动核心：

```sh
AGENT_LLM_MM_CONFIG=/absolute/local-state/core.toml /absolute/bin/agent_llm_mm init
AGENT_LLM_MM_CONFIG=/absolute/local-state/core.toml /absolute/bin/agent_llm_mm core --socket /absolute/local-state/ipc/core.sock --registry /absolute/local-state/workspaces.json
```

socket 路径要短（macOS Unix socket 有路径长度限制），父目录必须属于本用户且为 `0700`、祖先不能是符号链接。数据库必须已经存在且 schema 当前；核心不自动初始化/迁移。数据库与 socket 的持锁防止两个合作核心同时占用；这不是对原有独立 SQLite 程序的强制排他锁。

macOS 的 `/var`、`/tmp` 可能是系统符号链接。registry、socket 和管理员 capability 文件的管理路径须使用实际物理路径（例如先进入自己创建的状态目录，再用 `pwd -P` 获取路径）；不要直接把含这些别名的临时路径传给管理入口。测试 fixture 会先规范化其自己新建的临时目录，产品对符号链接管理路径的拒绝规则保持不变。工作空间 root 则按前述合同支持 canonical alias。

可以在客户端全局配置一次桥，并为每个可信项目显式注册。使用[全局 TOML 示例](../examples/codex-workspace-bridge-global.toml)，桥以启动时的真实 cwd 在 registry 中精确匹配 canonical root；未知目录拒绝，不向上搜索、不自动注册，也不回退个人域：

```toml
[mcp_servers.memory-ledger]
command = "/absolute/bin/agent_llm_mm"
args = ["bridge", "--socket", "/absolute/local-state/ipc/core.sock", "--registry", "/absolute/local-state/workspaces.json", "--client-label", "codex"]
```

这只适用于启动桥时正确传入项目 cwd 的客户端。不要把 MCP 的 cwd 固定为安装目录；固定 cwd 客户端仍须使用下面的显式绑定。连接建立后绑定不变，移动会话必须重建连接。这是全局配置入口的实现合同，实际客户端兼容性须单独验收。

也可以为项目显式配置桥；[完整 TOML 示例](../examples/codex-workspace-bridge.toml)：

```toml
[mcp_servers.memory-ledger]
command = "/absolute/bin/agent_llm_mm"
args = ["bridge", "--socket", "/absolute/local-state/ipc/core.sock", "--workspace-id", "REPLACE-WITH-REGISTERED-UUID", "--workspace-root", "/absolute/project-a", "--client-label", "codex"]
```

桥不加载 TOML、provider 环境变量或数据库，不需要 Cargo/Rust。客户端注册、项目规则和信任仍由用户管理；该实现不会改现有 Codex 配置。bridge 断开不停止核心。Ctrl-C 只停止自己启动的核心及其桥连接。断线不自动重试/重放写入，调用方应使用原 `request_id` 明确重连核验。

## 根目录、移动与权限

- 未注册根、ID/root 不匹配、注册表不可用都拒绝连接；没有回退到 `user/default`。
- 连接固定在启动时绑定。当前实际 Codex 未提供可用 roots；会话移动项目时既有 MCP 进程仍留在旧 cwd。必须重建连接并明确指定新 root/ID，模型不能通过 initialize、roots 通知或工具参数改绑定。
- root 的符号链接 canonicalize 到已注册目录，可以是同一个身份；被改指向其他目录后，逐调用检查会拒绝不匹配。注册表与 socket 的祖先路径拒绝符号链接。
- 普通 clone/worktree/相邻目录默认分别注册为不同 ID；不会根据 Git remote、父路径或名称猜为同一项目。
- 目录移动后旧根不可用会拒绝工具调用。管理员可用 `workspace ... bind --id UUID --root /new/root` 显式加入新路径，保留 ID/记忆；然后重连。`bind` 是增加别名，不删除旧绑定，也不自动识别替换旧路径的目录。若保留旧路径或允许恶意同 UID 修改路径，不构成安全隔离；不要把路径身份当文件系统不可伪造证明。

管理员可停用、重新启用或精确撤销 root；这些操作不是 MCP 工具，不删除账本、workspace ID 或已有 grants：

```sh
/absolute/bin/agent_llm_mm workspace --registry /absolute/local-state/workspaces.json disable --id UUID
/absolute/bin/agent_llm_mm workspace --registry /absolute/local-state/workspaces.json unbind --id UUID --root /absolute/old-project
/absolute/bin/agent_llm_mm workspace --registry /absolute/local-state/workspaces.json bind --id UUID --root /absolute/new-project
/absolute/bin/agent_llm_mm workspace --registry /absolute/local-state/workspaces.json enable --id UUID
```

`unbind` 使用 `list` 返回的已存绝对路径，或 Windows 上等价的普通磁盘/UNC 绝对路径表示（规范化产生的 `\\?\` 前缀与路径分隔符只作词法对应）。它不访问旧目录、不解析符号链接、不按目录名称大小写折叠、不匹配父目录或相似路径，旧目录已删除也可撤销。目标 ID 不存在或该精确路径属于其他 ID 会报错；该 ID 已无此路径时重复撤销是无写入的幂等成功。可撤销最后一个 root，此后任何会话解析都拒绝。`active` 与 roots 分开保存：解绑不改变 active；停用后 register/bind 不会隐式启用；enable 要求至少一个已绑定的现存目录。旧 registry 缺少 active 字段时视为 true。

停用或解绑在旧连接的下一次 tools/list、tools/call 生效，包括 durable receipt replay 之前；已经获准 dispatch 的操作不追溯取消。重新启用或恢复原 root 绑定可恢复该固定连接访问，移动到新 root 的连接仍须重新建立。个人 grants 保留不等于停用期间可读取。

个人共享需要明确授权：

```sh
/absolute/bin/agent_llm_mm workspace --registry /absolute/local-state/workspaces.json grant --id UUID --namespace user/default --read true --write false
/absolute/bin/agent_llm_mm workspace --registry /absolute/local-state/workspaces.json grant --id UUID --namespace user/default --read false --write false
```

第一条只读共享，第二条撤销。写授权必须同时有读权限；每次调用重新加载注册表，已打开的连接也会看到撤销。省略 namespace 只使用项目默认范围；指定个人范围必须有对应 grant。每次写入的嵌套 Event/Claim 必须属于同一范围且 owner 一致，不能静默纠正错误 owner。并发执行中的已批准操作不会被追溯取消。

## 工具、监控与上限

7 个旧全局入口在受限会话中隐藏并拒绝：`build_self_snapshot`、`decide_with_snapshot`、`run_reflection`、`get_self_model_versions`、`get_self_model_history`、`inspect_retrieval_index`、`rebuild_retrieval_index`。未知工具默认拒绝。核心还从运行时禁用 ingest 自动反思；非空 trigger_hints 拒绝。旧全局 Episode label 不接受写入，使用范围内的 `record_episode`。Episode/candidate 等调用方指定 ID 仍是全库唯一，建议使用随机 UUID；跨项目同 ID 冲突会拒绝，不能声称 ID 是项目内局部唯一。

核心共享内存缓冲和持久操作历史。正常工具结果含可信 workspace/client/session 的脱敏审计元数据，不保存查询、请求内容或模型结果；`export_memory` 保持完全无审计写入。原有业务账本可能另有其自身事务记录，不能把所有 operation_log 行都当工具调用次数。

默认核心不开 HTTP，且拒绝启用旧无认证 dashboard。管理员可显式通过 CLI 使用自己的私有 capability 文件，两个参数必须同时提供：

```sh
AGENT_LLM_MM_CONFIG=/absolute/local-state/core.toml /absolute/bin/agent_llm_mm core --socket /absolute/local-state/ipc/core.sock --registry /absolute/local-state/workspaces.json --admin-bind 127.0.0.1:8788 --admin-token-file /absolute/local-state/admin.capability
```

文件必须为本用户拥有的私有普通文件（不接受符号链接或硬链接，最多4096字节），值由管理员通过自己的安全流程准备；本轮仅使用合成测试值，没有创建、保存或启用生产凭据。不要在命令行、URL、源码、支持报告或 workspace 桥配置中放入值。`[dashboard].enabled` 仍须为 false；此入口单独启动 owner-gated 监控并复用核心 observer/store。

浏览器打开 CLI 打印的精确 loopback 地址；Host 必须匹配绑定 IP/端口，不能擅自换成 localhost 别名。静态页面和允许的资源公开提供，但不包含运行数据。所有数据 API、status/health 和 SSE 都要求独立 Bearer；页面登录后仅在当前标签页内存保存值，通过 authenticated fetch（包括SSE）发送，刷新页面需要重新登录。锁定会清凭据、已加载数据和详情并取消请求。无 URL token、凭据 localStorage 或 cookie 授权；Origin 若存在须严格同源，跨站请求拒绝，不开放 CORS。

这是一位本机 owner 查看全部 workspace 的只读监控，不是普通 workspace 用户登录或网页授权管理。原页面样式保留；个人共享仍使用显式 `workspace grant` CLI，普通桥不获得管理员能力。实现与离线合成验证不等于真实核心浏览器端到端验收，最终证据见验收矩阵。

运行上限为 64 个连接、共享 32 个正在执行的工具调用、8 KiB 绑定报文、1 MiB 单个入站、4 MiB 单个出站 MCP JSONL 帧，每连接最多 8 个未完成请求；绑定和 MCP 初始化各有 5 秒超时。拒绝 batch、重复未完成 ID 和无 ID 的工具调用；数字请求 ID 只接受 0..=2^32-1 的整数，以匹配当前协议库。超额工具请求立即拒绝，不排队写拒绝审计，只进入有界实时缓冲；export 仍不记录。字节限制不等于 token 或美元上限。核心没有后台模型任务，当前范围工具也不使用模型。

## 验收与恢复

### 从旧 standalone 数据切换

旧 `project/foo` 不会自动变成新注册产生的 `project/<uuid>`。现有基础事实首片可通过 `adopt-preview` 盘点，再显式 `adopt` 新建 UUID → 旧 namespace 映射；不重标历史、不跨域复制，也不能用个人 grant 绕过项目隔离。高级历史未支持且会明确保守阻断，详见[支持集合](workspace-legacy-adoption.md)。先保留原配置、数据库与 registry 备份。被阻断的旧库继续保留原模式，不要清洗历史绕过检查。执行后仍需管理员停止或重新配置旧 standalone 写入客户端；不要把接入映射误认为客户端已切换，也不要让旧 `serve` 与核心并写同一个库。

注册表也是恢复资产：数据库备份不包含注册表。保留其稳定 ID、根绑定、active 状态和授权；不要把私有注册表上传到支持报告。`bind` 只增加根别名；移动目录后须显式 `unbind` 撤销旧路径，避免以后重新创建该目录时恢复旧绑定。

使用 `scripts/verify-local-core.py --binary /absolute/bin/agent_llm_mm --output /absolute/isolated-output` 做无 Rust 运行 PATH、两项目写入、重连、证据、更正、历史、强制终止、回执重放及恢复到新路径核验。它只操作自己创建的进程和数据库，运行结果保留在显式输出目录。该脚本实现/通过状态以[矩阵](workspace-core-acceptance.md)为准。

恢复必须同时保留所需的数据库备份及工作空间注册表：稳定 ID 决定 namespace。恢复到新目录后显式绑定根，不能重新生成 ID 并假定旧记忆会自动归属。管理员 `grant --read false --write false` 可撤销个人共享；完整数据库恢复、旧版本回退继续遵守[数据库操作](database-operations.md)，不在活动核心上做原地迁移。


核心生命周期诊断区分 `active_requests()`（含仍可取消的鉴权前处理）与 `dispatched_requests()`（已通过授权及最后取消检查的工具调用，计数保留至审计结束）。关闭仍等待所有已进入 handler 的工作结束后才释放数据库锁；已进入 handler 不表示工具一定执行，取消可在 dispatch 前使它正常提前结束。恢复/取消验收必须等待后者的明确接纳信号，不能用 handler 数量或固定睡眠推断数据库写入已开始。
