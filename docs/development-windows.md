# Windows 开发与接入指南

这份文档面向在 Windows 上开发、验证和接入 `agent_llm_mm` 的协作者。

## 1. 环境前提

- 已安装 `rustup`；仓库声明 Rust `1.95.0`、`rustfmt` 与 `clippy`，但该声明本身不构成 Windows runtime parity 证据
- `cargo` 可用
- 已安装 PowerShell 7
- 当前仓库内提供 Windows 入口脚本：
  - `pwsh -File .\scripts\agent-llm-mm.ps1 bootstrap-local`
  - `pwsh -File .\scripts\agent-llm-mm.ps1 init`
  - `pwsh -File .\scripts\agent-llm-mm.ps1 doctor --read-only`
  - `pwsh -File .\scripts\agent-llm-mm.ps1 migrate`
  - `pwsh -File .\scripts\agent-llm-mm.ps1 serve`

## 2. 进入项目目录

```powershell
Set-Location 'D:\Code\agent_llm_mm'
```

请按你的本机实际路径替换上面的示例目录。

## 3. 准备本地配置

优先用本地 bootstrap helper 生成本机私有配置文件：

```powershell
pwsh -File .\scripts\agent-llm-mm.ps1 bootstrap-local
```

`bootstrap-local` 只把 `examples/agent-llm-mm.dev.example.toml` 复制到 `agent-llm-mm.local.toml`，不会生成 secret、不会覆盖已有配置、不会运行 `doctor`、不会启动 `serve` 或 daemon。如果需要自定义目标路径：

```powershell
pwsh -File .\scripts\agent-llm-mm.ps1 bootstrap-local .\agent-llm-mm.local.toml
```

显式目标路径可以是绝对路径或相对路径；相对路径按仓库根目录解析，不按调用者当前目录解析。跨目录调用脚本时建议传绝对路径，避免把配置写到非预期位置。

如果目标配置已存在，脚本会拒绝覆盖；这种情况下请手工编辑已有文件，或先选择一个新的目标路径。也可以继续手工选择 profile 并复制为本机配置：

```powershell
Copy-Item .\examples\agent-llm-mm.dev.example.toml .\agent-llm-mm.local.toml
```

可选 profile：

- `examples/agent-llm-mm.dev.example.toml`: 本地开发和手工测试，默认 `provider = "mock"`，dashboard disabled。
- `examples/agent-llm-mm.prod-local.example.toml`: 正式本地数据，dashboard 只监听 `127.0.0.1`，daemon disabled；复制后必须替换 `database_url` 和 provider 占位值。
- `examples/agent-llm-mm.openrouter.example.toml`: OpenRouter 本地配置模板；默认通过 `api_key_env = "AGENT_LLM_MM_OPENROUTER_API_KEY"` 读取本机环境变量，通过 OpenAI-compatible `/chat/completions` transport 使用；配置示例本身不是 live evidence，必须显式运行 `provider-live-certification-run.sh --live` 才能生成 bounded live preflight evidence。
- `examples/agent-llm-mm.demo.example.toml`: self-revision demo runner 专用，通常不要手工复制为日常配置。

`examples/agent-llm-mm.example.toml` 只是通用入口说明，不再承载所有用途。然后编辑 `agent-llm-mm.local.toml`：

- 固定自己的 `database_url`
- 选择 `provider`
- dev/mock profile 不需要 API key；选择 `openai-compatible`、`openrouter` 或 prod-local profile 时，才在已忽略的 `agent-llm-mm.local.toml` 里填写 `base_url`、`model`，并二选一配置本机私有 `api_key` 或 `api_key_env`

常见 Windows SQLite URL 示例；dev、demo、prod-local 必须使用不同文件。按数据生命周期口径，`prod-local` 对应 formal 数据，`dev` / manual profile 对应 test 数据，demo profile 只对应 demo 数据：

```toml
database_url = "sqlite:///D:/agent-llm-mm/dev.sqlite"
```

## 4. 本机预检

本地启动顺序是 config → explicit init/migrate → read-only doctor → serve。新库用 `init`；旧库先查看 `doctor --read-only` 再显式 `migrate`；`serve` 不隐式 bootstrap。PowerShell 模式为 `serve|init|migrate|doctor|bootstrap-local`，doctor 可选 `--read-only`（默认）或显式 `--allow-bootstrap`。

```powershell
pwsh -File .\scripts\agent-llm-mm.ps1 init
pwsh -File .\scripts\agent-llm-mm.ps1 doctor --read-only
```

示例 profile 是结构模板，包含占位 `database_url`。先复制到 `agent-llm-mm.local.toml`，替换为本机可写 SQLite 路径后，再检查本机私有配置；如果选择 prod-local profile，还必须同时替换 `base_url`、`model`，并配置本机私有 `api_key` 或 `api_key_env`：

```powershell
pwsh -File .\scripts\agent-llm-mm.ps1 init .\agent-llm-mm.local.toml
pwsh -File .\scripts\agent-llm-mm.ps1 doctor --read-only .\agent-llm-mm.local.toml
```

预期输出为 JSON，至少包含：

- `transport`
- `database_url`
- `database_lifecycle`
- `provider`
- `status`

当前 `scripts/first-run-bootstrap-smoke-local.sh` 是 bash 本地首启模拟脚本；在 Windows 上只能从 Git Bash、WSL 或等价环境运行。它模拟 `bootstrap-local -> init -> doctor --read-only`，不会替代 PowerShell runtime parity 证据。

## 5. 启动 MCP 服务

```powershell
pwsh -File .\scripts\agent-llm-mm.ps1 serve
```

服务启动后会占用当前终端并等待 `stdio` JSON-RPC 输入，这是正常现象。

## 6. Codex 配置

推荐使用 PowerShell 入口脚本：

```toml
[mcp_servers.agent-llm-mm]
command = "pwsh"
args = ["-File", "D:/Code/agent_llm_mm/scripts/agent-llm-mm.ps1", "serve"]
env = { AGENT_LLM_MM_CONFIG = "D:/Code/agent_llm_mm/agent-llm-mm.local.toml" }
transport = "stdio"
```

## 7. 推荐验证顺序

```powershell
cargo fmt --check
git diff --check
cargo clippy --all-targets -- -D warnings
cargo test
pwsh -File .\scripts\agent-llm-mm.ps1 doctor --read-only
```

涉及发布证据、打包或 provider certification 工具时，再运行
`cargo clippy --all-targets --all-features -- -D warnings` 和
`cargo test --all-features`。`scripts/test-tier.sh` 是 bash 入口；Windows 原生
PowerShell 环境使用上面的等价 Cargo 命令。

发布前请按 [Release Gate](release-gate.md) 跑完整 gate；本节只是 Windows 日常验证入口。Release gate 中的 `./scripts/agent-llm-mm.sh doctor` 在 Windows 上对应 `pwsh -File .\scripts\agent-llm-mm.ps1 doctor`。
如果判断 Local Product Alpha / product alpha 口径，还必须改用 [Local Alpha Release Gate](product/release-gate-local-alpha.md)；普通 `doctor` 通过不等于 Local Alpha 完成。
当前 macOS 本机验证环境没有 `pwsh`，所以 PowerShell `bootstrap-local` 行为需要 Windows runner 或 Windows 实机补充 runtime parity 证据；Rust bootstrap 测试仍保留脚本文本契约和 no-clobber 静态断言。
当前 release soak runner 是 bash 脚本：`./scripts/release-soak-local.sh <candidate-name> [config_path]`。在 Windows 上请从 Git Bash、WSL 或等价 bash 环境运行；它只生成本机候选证据，不替代 Windows runner parity、真实 fresh-machine、安装包或发布认证证据。
当前 provider certification preflight / live evidence runner 也是 bash 脚本：`./scripts/provider-certification-check.sh [config_path] [evidence_root] [output_dir]` 和 `./scripts/provider-live-certification-run.sh --live [config_path] [evidence_root]`。在 Windows 上请从 Git Bash、WSL 或等价 bash 环境运行；配置示例本身不是 live evidence，必须显式运行 `--live` runner 才能产生 preflight 可读取的 live evidence。`--live` runner 只写 provider preflight evidence files，记录本次配置下的 endpoint reachability、decision probe、self-revision parse probe、错误处理和 redaction review provenance；即便 preflight 显示 `live_certified = true`，也只表示 config preflight 通过且四类 live evidence present，不证明 provider 输出质量、SLA、provider gateway、Local Alpha、Beta、GA、production-ready、production readiness 或 release approval。`--stub-evidence` 只生成本地模拟证据，不能让 `live_certified = true`。

## 7.1 本地接入排障

| Symptom | Likely Cause | Verification | Fix |
| --- | --- | --- | --- |
| `init` / `migrate` cannot write SQLite | database path not writable or sandbox restriction | 先用 `doctor --read-only` 查看 lifecycle 状态，再检查 config 与环境覆盖 | 为显式 lifecycle command 设置可写路径；不要为只读 doctor 放宽正式库权限 |
| MCP client starts the wrong binary | auxiliary `src/bin` target ambiguity | 检查 MCP 客户端配置里的参数是否带 `--bin agent_llm_mm`（如 `args = ["run", "--quiet", "--bin", "agent_llm_mm", "--", "serve"]`） | 统一使用 PowerShell 脚本入口，或在客户端里固定 `cargo run --quiet --bin agent_llm_mm -- serve` |
| dashboard not visible | `[dashboard].enabled` 为 false，或端口不可用 | 查看 TOML 的 `[dashboard]` 区块和 `enabled`，以及 `pwsh -File .\scripts\agent-llm-mm.ps1 doctor` 输出 | 设置 `[dashboard].enabled = true`，并改用可用的本地端口（如 `127.0.0.1:8787`） |
| model calls fail | provider 配置不完整 | 执行 `pwsh -File .\scripts\agent-llm-mm.ps1 doctor`，确认 `provider`、`base_url`、`model` 已配置 | 在本地 TOML 更新 provider 信息；密钥仅放本地文件，不要提交 |

## 7.2 SQLite 备份与恢复

完整数据生命周期边界见 [Data Lifecycle](product/data-lifecycle.md)。本节只保留 Windows 本地命令入口；正式数据、手工 test 数据和 demo 数据必须使用不同 `database_url`。这里的正式数据对应 `prod-local`，手工 test 数据对应 dev / manual profile。本地正式数据进入 productization 前，应先保守地按 [Local Alpha Data Safety Runbook](product/data-safety-local-alpha.md) 做 SQLite backup。当前 backup / restore helper 是 bash 脚本；在 Windows 上请从 Git Bash、WSL，或等价 bash 环境运行。Git Bash drive URL 形态 `sqlite:///D:/...` 会被脚本转换为 `D:/...`，不会按 Unix 路径 `/D:/...` 处理：

```bash
./scripts/backup-sqlite.sh "sqlite:///D:/agent-llm-mm/formal/agent-llm-mm.sqlite"
```

默认 backup 目录是仓库内 `target/backups/sqlite/`，用于和常规 live database directory 分离；脚本会在写入前解析 live DB 目录和 backup 目录，如果 backup 目录等于 live DB 目录，或位于 live DB 目录的子目录下，会拒绝继续，并要求指定其它 backup 目录。backup 路径不能包含双引号、反斜杠、换行或 `..` 路径组件；`sqlite://` URL 也要求使用正斜杠路径，不能用反斜杠 escape，且会拒绝 invalid percent escape、控制字符、编码反斜杠和编码双引号。写出的 backup 和 restored database 文件会收紧到 owner-only 权限；restore 会先预留新目标路径，避免覆盖已有文件。如本机有 `shasum -a 256` 或 `sha256sum`，脚本会生成 `.sha256` checksum，缺少 checksum 工具时只提示降级。

恢复时默认原则是 restore to a new path first，然后用新的 `database_url` 验证：

```bash
./scripts/restore-sqlite.sh target/backups/sqlite/formal.sqlite.20260511-120000.12345.bak \
  "sqlite:///D:/agent-llm-mm/restore-check/formal-restore.sqlite"
```

确认 restored database 可用后，再由人工决定是否切换正式配置。不要把 backup 直接覆盖回现有正式 SQLite 文件。

## 8. 额外说明

- `agent-llm-mm.local.toml` 已被 `.gitignore` 忽略，不应提交。
- 正式数据、手工测试数据和 demo 数据必须分开使用不同数据库文件；prod-local 只用于要保留、检查或备份的本地正式数据。
- 如果多个本机客户端共用同一 SQLite 文件，需要预期 SQLite 单写者模型带来的锁等待和状态互相影响。
- 所有示例 profile 都保持 `[daemon].enabled = false`；后续 daemon 观察模式必须走单独 gate。

## 2026-10-09 schema-v5 feedback and experience verification

The [A–E runtime guide](memory-feedback-experience.md) describes explicit migration, read-only index inspection/rebuild and new MCP tools. Existing v4 databases require explicit `migrate` before serving; keep the generated backup and restore to a new path. New deterministic checks are `cargo test --test feedback_candidates --test experience_workflow --test indexed_text_recall --test sqlite_lifecycle`. The Python evaluation/capacity scripts accept an existing binary and never require paid model calls; see [methodology](evaluation-methodology.md). Use `python3` on macOS and `python` on Windows, with the native binary suffix. Exact-head platform evidence remains distinct from configured workflows and from formal fresh-machine/release gates.

The evaluation fixtures are UTF-8 even on a Windows installation whose default text code page is cp1252. Fixture loaders specify UTF-8 explicitly; the Python unit suite includes a forced-cp1252 regression. Do not work around a decoding failure by skipping Chinese fixtures or relaxing expected results. The initial v5 Windows CI passed all 184 native Rust tests and the complete feedback/experience workflow before exposing this fixture-test decoding issue; verification must be rerun against the encoding-fix commit.

## Schema v6 temporal/scope/export

Run explicit `migrate` for an existing v5 database; inspect the generated backup and restore only to a new path. [Temporal fields](temporal-metadata.md) keep unknown historical Claim times null and preserve raw timestamps; [Reflection scope](reflection-scope-history.md) never turns effect scope into read authority. [Export](scoped-export.md) is bounded readonly interchange, not a replacement for backup. Verify with `cargo test --test temporal_metadata --test sqlite_temporal_store --test reflection_scope_history --test scoped_ledger_export --test schema6_migration` and `scripts/temporal-scope-export-smoke.py` using the platform's Python and binary path. Rust lifecycle/format/full gates remain required.


### Final original-plan context and caller-budget regression

`cargo test --test context_diagnostics --test caller_operation_budget --test mcp_stdio --test schema6_migration` covers scoped rich Episode context, honest bounded diagnostics, optional caller counts/stops over actual local MCP subprocesses, and historical receipt/fingerprint replay across migration. Run the normal full/fmt/Clippy/status-sync gates afterward. The fixed offline evaluator keeps its original tasks and byte budgets; richer metadata costs must be reported rather than hidden by retuning fixtures. See [context](context-diagnostics.md), [caller budget](caller-operation-budget.md), and [module extraction](implementation-module-boundaries.md) contracts.
