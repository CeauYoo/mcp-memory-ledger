# 本机 MCP 接入

## 任务续接与 Codex 接入

新增任务工具与 CLI 见[任务账本](task-ledger.md)，原生配置预览、项目默认任务选择和宿主支持边界见[Codex 适配器](codex-task-context-adapter.md)。Schema 8 需要显式迁移，先备份，不能自动替换运行中的数据库。预览不改全局配置；未取得真实宿主注入证据，不将输出成功称为接入成功。Windows 核心/任务 CLI 与 Windows hook 自动配置支持分别记录。

所有状态、构建与日志放在操作者确认的非同步目录；不改变现有部署。


旧 namespace 接入必须显式预览与 adopt，不能把现有 workspace 默认域改名。新 core 应使用 registry 实际返回的 project namespace（普通注册为 UUID 域，v2 adopted workspace 保留旧域）；参见[首片支持合同](workspace-legacy-adoption.md)。

## 2026-10-15 先选连接模式

- 原 `serve`：客户端每次启动独立运行时，32个工具；下面的JSON/TOML例子均属此模式。全局注册一次不意味着后台只有一个进程，namespace只是数据范围。
- 新 `core`＋`bridge`：macOS/Linux Unix socket、Windows 本机命名管道 candidate，固定workspace连接，25允许/7拒绝、个人范围显式授权；实现与集成验收进行中。按[核心接入指南](local-workspace-core.md)及[桥配置样例](../examples/codex-workspace-bridge.toml)在隔离环境验证，Windows 接入与 ACL 前提见 [平台指南](development-windows.md)，本轮原生 CI 验证前不称验收通过。桥不读取provider配置或密钥，不自行启动核心。

实际Codex roots探针67条断言证明本版本没有可用自动root通知依据；会话目录更新不改变已启动MCP的cwd。桥可以明确指定注册ID/root，或使用[一次全局配置样例](../examples/codex-workspace-bridge-global.toml)在启动时按真实cwd精确解析已注册目录。后者不向上搜索、不自动注册；固定cwd客户端须保留显式绑定。移动会话后重建连接，不能由模型参数重绑或自动猜项目。个人共享读写逐调用重新核验，撤销约束旧连接。

旧部署d9e33d3已完成20项协议、11项网页检查和一个真实用户自然语言主动召回/来源核对/更正样本；新核心真实客户端仍待验收。完整范围见[矩阵](workspace-core-acceptance.md)。早期独立HOME/mock的17次app-server MCP调用、[最小五工具示例](../examples/codex-mcp-acceptance.example.toml)及脚本仍见[当时记录](acceptance-2026-10-15.md)。这些工作不会自动修改用户注册、全局配置或项目trust。

更新：2026-10-15。文件名保留历史链接兼容性；以下兼容配置说明对应schema7的32工具standalone模式，新核心按顶部入口单独验收。

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

1. 原standalone模式重新加载/连接server，检查`tools/list`返回32个工具；新核心应只列25个，并另外验证直接调用被隐藏的7个工具也被拒绝。
2. 原standalone模式用`ingest_interaction`向`project/demo`写入合成Event/Claim并保存ID；新核心必须使用registry返回的实际project namespace，不得把示例namespace当成已授权范围。
3. 断开再连接；用 `recall_memory` 查原文，再 `get_memory` 查看 evidence/provenance。
4. 按[完整工作流](runnable-memory-workflow.md)更正并查看历史；确认其他 namespace 不混入。

参数以运行时 schema 为准，分类说明见[工具索引](tool-reference.md)。工具结果中的业务 JSON 位于 MCP `structuredContent`；错误需同时检查协议 error / 工具 `isError`，不要把文字描述当作提交成功。

## 4. 模型与权限边界

- 第一次使用明确的 mock TOML，不需要 API key。新反馈/经验和 deterministic read 不调用模型；旧 ingest/snapshot 的自动反思钩子可能在触发时使用配置的 provider。
- 真正的 provider 配置、密钥位置和 live preflight 见[provider contract](provider-contract.md)及平台指南。连接检查不是效果测量，也不能默认批准传输私人记忆或产生费用。
- 显式 scoped read 不扩大查询；省略 namespace 的部分 legacy self-model 路径仍保留旧兼容行为。
- `decide_with_snapshot` 的结果非权威且不会执行动作；candidate activation 只影响召回。客户端自身的权限与用户确认仍然独立有效。
- 原无认证dashboard只允许loopback；新核心默认关闭监控HTTP，管理员Bearer只通过独立库API/隔离fixture验证，本轮没有正式网页登录。不要把任一入口映射成公开HTTP MCP服务。

## 5. 排错顺序

核对实际 binary → 实际配置环境 → 绝对数据库路径 → 单独运行 `doctor --read-only` → 客户端 stderr → 重连。不要用 init/自动迁移掩盖路径错误，不要把 provider 密钥或整库贴入日志/issue。

升级与恢复见[数据库操作](database-operations.md)，诊断产物与测试门见[测试指南](testing-guide-2026-03-24.md)。

Schema7 global self-model version/diff and explicit compensation contract: [details](self-model-versions.md). Existing scoped read/export boundaries and experimental single-user limits remain.

## 2026-10-13 接入验收边界

本轮不读取或修改用户现有 Codex 配置。上面的条目是用户自行接入的最小步骤；Python stdio harness 成功只标为隔离运行验证，不标为真实 Codex 客户端或新机器验收。分层结果、费用限制与复现命令见[本轮验收记录](acceptance-2026-10-13.md)。

## 可选动作检查预览

`codex-actions` 是独立、可选的额外参数检查入口，配置只生成候选，不自动改 Codex/MCP 注册或信任设置。沿用 `codex-context` 的已选 session binding；不会用 cwd 猜任务或把 task.source 当权限。默认只匹配 apply_patch。详见 [完整命令和边界](codex-action-checks.md)。现有 MCP 使用不依赖此入口。
