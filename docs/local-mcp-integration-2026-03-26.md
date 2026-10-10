# 本机 MCP 接入

更新：2026-10-10。文件名保留历史链接兼容性；本页描述当前 schema7 / 32 工具版本。

先完成[快速开始](quickstart.md)的 build、明确配置、`init` 和 `doctor --read-only`。客户端负责启动服务进程；不要把自己的 shell 环境、工作目录或相对 SQLite 路径当作客户端必然继承的状态。

## 1. 通用 stdio 配置

在支持 JSON MCP server 配置的客户端中，按其设置入口填入同等内容：

```json
{
  "mcpServers": {
    "memory-ledger": {
      "command": "/absolute/path/mcp-memory-ledger/target/debug/agent_llm_mm",
      "args": ["serve"],
      "env": {
        "AGENT_LLM_MM_CONFIG": "/absolute/path/ledger-demo.toml"
      }
    }
  }
}
```

Windows `command` 示例为 `C:/code/mcp-memory-ledger/target/debug/agent_llm_mm.exe`。这些路径需要替换为真实绝对路径；配置中的 `database_url` 也须绝对化。JSON 容器键名和设置位置由客户端决定，上面不是所有客户端共用的导入格式。

MCP 使用 **stdio**，不是 HTTP URL。`stdout` 仅写协议，日志在 `stderr`。客户端应先完成 initialize / initialized 握手，再 `tools/list` / `tools/call`；一般客户端会自动处理这些步骤。

## 2. Codex TOML 示例

将 server 条目加入你实际使用的 Codex 配置：

```toml
[mcp_servers.memory-ledger]
command = "/absolute/path/mcp-memory-ledger/target/debug/agent_llm_mm"
args = ["serve"]
env = { AGENT_LLM_MM_CONFIG = "/absolute/path/ledger-demo.toml" }
```

二进制形式不依赖客户端 cwd 或 cargo 的 PATH。若选择 release build，command 必须同步改为 `target/release/agent_llm_mm`。此配置让客户端启动一个 stdio 子进程，不需要同时在终端手动启动另一份 `serve`。

仓库的[可复制配置样例](../examples/codex-mcp-config.toml)还给出 macOS wrapper 选项；Windows wrapper 用法见[平台指南](development-windows.md)。如果从任意目录用 cargo，必须显式传 `--manifest-path /absolute/path/mcp-memory-ledger/Cargo.toml`，并保证 Rust 工具链可用；直接 binary 更容易排查。

## 3. 连接后验证

1. 重新加载/连接 server，检查 `tools/list` 当前返回 32 个工具。
2. 用 `ingest_interaction` 向 `project/demo` 写入一条合成 Event/Claim，保存结果 ID。
3. 断开再连接；用 `recall_memory` 查原文，再 `get_memory` 查看 evidence/provenance。
4. 按[完整工作流](runnable-memory-workflow.md)更正并查看历史；确认其他 namespace 不混入。

参数以运行时 schema 为准，分类说明见[工具索引](tool-reference.md)。工具结果中的业务 JSON 位于 MCP `structuredContent`；错误需同时检查协议 error / 工具 `isError`，不要把文字描述当作提交成功。

## 4. 模型与权限边界

- 第一次使用明确的 mock TOML，不需要 API key。新反馈/经验和 deterministic read 不调用模型；旧 ingest/snapshot 的自动反思钩子可能在触发时使用配置的 provider。
- 真正的 provider 配置、密钥位置和 live preflight 见[provider contract](provider-contract.md)及平台指南。连接检查不是效果测量，也不能默认批准传输私人记忆或产生费用。
- 显式 scoped read 不扩大查询；省略 namespace 的部分 legacy self-model 路径仍保留旧兼容行为。
- `decide_with_snapshot` 的结果非权威且不会执行动作；candidate activation 只影响召回。客户端自身的权限与用户确认仍然独立有效。
- 无认证 dashboard 只允许 loopback；不要映射成公开 HTTP MCP 服务。

## 5. 排错顺序

核对实际 binary → 实际配置环境 → 绝对数据库路径 → 单独运行 `doctor --read-only` → 客户端 stderr → 重连。不要用 init/自动迁移掩盖路径错误，不要把 provider 密钥或整库贴入日志/issue。

升级与恢复见[数据库操作](database-operations.md)，诊断产物与测试门见[测试指南](testing-guide-2026-03-24.md)。

Schema7 global self-model version/diff and explicit compensation contract: [details](self-model-versions.md). Existing scoped read/export boundaries and experimental single-user limits remain.
