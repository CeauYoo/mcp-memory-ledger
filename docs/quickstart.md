# 快速开始：先跑通本地 mock

目标：从源码构建，用一个隔离数据库完成 MCP 记忆闭环。当前没有经过正式发布验收的安装包；源码构建不是 fresh-machine 产品验收。

## 1. 环境与构建

- 安装 Rust/rustup，仓库 `rust-toolchain.toml` 固定 **1.95.0**；详见[工具链政策](toolchain-policy.md)。
- 以下 cargo 命令在仓库根目录执行；Python 3 仅用于可执行示例与验证。
- shell wrapper 细节分别见 [macOS](development-macos.md) 和 [Windows](development-windows.md)。Linux 可直接运行二进制，不假定全部平台 wrapper 一致。

```text
cargo build --locked --bin agent_llm_mm
```

产物为 `target/debug/agent_llm_mm`（Windows 为 `target/debug/agent_llm_mm.exe`）。需要 release profile 时用 `cargo build --locked --release --bin agent_llm_mm`，路径相应改为 `target/release/…`。不要在 MCP 客户端中使用不同构建路径。

## 2. 先用自动隔离示例

```text
python scripts/local-memory-smoke.py --binary target/debug/agent_llm_mm --output target/reports/first-memory-smoke
```

Windows 将 binary 改为 `target/debug/agent_llm_mm.exe`；使用 `python3` 的系统替换 launcher。输出目录必须是新的空目录；重复运行请选择新目录。

脚本会复制已构建二进制，生成明确的 mock 配置和合成库，经真实 `stdio` 完成初始化、写入、重连、中文检索、证据读取、纠错、历史与备份恢复。它不会访问付费模型，但会在指定目录写测试文件。成功以退出码 0 和脚本断言为准；生成的原始数据只留本地。

## 3. 配置自己的测试库

先创建可写目录，再保存本机私有 TOML，例如 `/absolute/path/ledger-demo.toml`：

```toml
transport = "stdio"
database_url = "sqlite:///absolute/path/ledger-demo.sqlite"

[model]
provider = "mock"
```

Windows 使用真实绝对路径，例如 `sqlite://C:/Users/you/ledger/ledger-demo.sqlite`；TOML 内推荐正斜杠，避免反斜杠转义。文件名和目录仅为占位，请替换，不要与正式库复用。

为接下来的进程设置环境变量 `AGENT_LLM_MM_CONFIG`，值为该 TOML 的绝对路径：

macOS / Linux shell：

```sh
export AGENT_LLM_MM_CONFIG=/absolute/path/ledger-demo.toml
./target/debug/agent_llm_mm init
./target/debug/agent_llm_mm doctor --read-only
```

Windows PowerShell：

```powershell
$env:AGENT_LLM_MM_CONFIG = "C:/Users/you/ledger/ledger-demo.toml"
.\target\debug\agent_llm_mm.exe init
.\target\debug\agent_llm_mm.exe doctor --read-only
```

`init` 只用于新库；已有旧库走[显式迁移](database-operations.md)。`doctor` 默认为只读，配置成功不代表真实 provider 已连通。不要为了消除报错改用 `doctor --allow-bootstrap`。

配置加载顺序：显式 `AGENT_LLM_MM_CONFIG`，否则当前目录 `agent-llm-mm.local.toml`，否则默认配置；环境变量还可覆盖部分字段。因此仅覆盖数据库地址不能保证使用 mock。首次验证使用上述显式配置，并检查是否遗留其他 `AGENT_LLM_MM_*` 环境变量。

## 4. 连接客户端并使用

按 [MCP 接入](local-mcp-integration-2026-03-26.md)把 binary、`serve` 参数和同一配置环境交给客户端。`serve` 是长期运行的 stdio 进程，会等待协议输入；终端没有聊天提示或网页是正常现象。数据库缺失或过期时它拒绝启动，不隐式 init/migrate。

接着运行[最小请求与完整工作流](runnable-memory-workflow.md)。首次用一个明确 namespace，例如 `project/demo`；把服务返回的 ID 传给后续工具，别手写猜测 ID。

## 常见阻塞

- 工具未出现：核对绝对 binary 路径、`.exe`、`args = ["serve"]`、环境和 stderr；客户端需重新连接。
- `missing` / `migration_required`：确认配置实际指向的库；新库 init，旧库备份后 migrate。
- `schema_structure_invalid`：同版本结构受损，保留现场并查[运维手册](database-operations.md)，不要删库或强行当作 current。
- 中文 query 无结果：先检查 namespace 与原文；字面检索不保证同义改写命中。
