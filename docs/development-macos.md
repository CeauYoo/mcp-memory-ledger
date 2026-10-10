# macOS 开发与接入指南

2026-10-09 文档导航：首次使用见[快速开始](quickstart.md)，当前操作见[数据库手册](database-operations.md)，可执行协议示例见[完整工作流](runnable-memory-workflow.md)。实现基线三平台验证与 Windows native / wrapper 区别见[当前状态](project-status.md)；下方分阶段追加记录不代表新的发布批准。

这份文档面向当前在 macOS 上开发、验证和接入 `agent_llm_mm` 的协作者。

## 1. 环境前提

- 已安装 `rustup`；仓库会通过 `rust-toolchain.toml` 选择 Rust `1.95.0`，并安装 `rustfmt` / `clippy` 组件
- `cargo` 可用
- 使用 `zsh` 或 `bash`
- 当前仓库内提供 macOS 原生入口脚本：
  - `./scripts/agent-llm-mm.sh bootstrap-local`
  - `./scripts/agent-llm-mm.sh init`
  - `./scripts/agent-llm-mm.sh doctor --read-only`
  - `./scripts/agent-llm-mm.sh migrate`
  - `./scripts/agent-llm-mm.sh serve`
  - `./scripts/run-self-revision-demo.sh`
  - `./scripts/generate-support-bundle.sh`
  - `./scripts/provider-certification-check.sh`
  - `./scripts/provider-live-certification-run.sh`

## 2. 进入项目目录

```zsh
cd ~/code/agent-llm-mm
```

请按你的本机实际路径替换上面的示例目录。

进入仓库后先确认当前固定工具链：

```zsh
rustc --version
cargo --version
```

`rustc` 应报告 `1.95.0`。支持下限与升级规则见 [Rust Toolchain Policy](toolchain-policy.md)；不要只改 `Cargo.toml` 或只改 `rust-toolchain.toml`。

## 3. 准备本地配置

优先用本地 bootstrap helper 生成本机私有配置文件：

```zsh
./scripts/agent-llm-mm.sh bootstrap-local
```

`bootstrap-local` 只把 `examples/agent-llm-mm.dev.example.toml` 复制到 `agent-llm-mm.local.toml`，不会生成 secret、不会覆盖已有配置、不会运行 `doctor`、不会启动 `serve` 或 daemon。如果需要自定义目标路径：

```zsh
./scripts/agent-llm-mm.sh bootstrap-local /absolute/path/to/agent-llm-mm.local.toml
```

显式目标路径可以是绝对路径或相对路径；相对路径按仓库根目录解析，不按调用者当前目录解析。跨目录调用脚本时建议传绝对路径，避免把配置写到非预期位置。

如果目标配置已存在，脚本会拒绝覆盖；这种情况下请手工编辑已有文件，或先选择一个新的目标路径。也可以继续手工选择 profile 并复制为本机配置：

```zsh
cp examples/agent-llm-mm.dev.example.toml agent-llm-mm.local.toml
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

建议的 macOS SQLite URL 示例；dev、demo、prod-local 必须使用不同文件。按数据生命周期口径，`prod-local` 对应 formal 数据，`dev` / manual profile 对应 test 数据，demo profile 只对应 demo 数据：

```toml
database_url = "sqlite:///Users/<you>/Library/Application%20Support/agent-llm-mm/dev.sqlite"
```

## 4. 本机预检

本地产品化启动顺序是 config → explicit init/migrate → read-only doctor → serve。新库执行 `init`；旧库先用 `doctor --read-only` 查看状态，再显式 `migrate`。`serve` 和默认 `doctor` 都不会隐式 bootstrap。入口模式是 `serve|init|migrate|doctor|bootstrap-local`；doctor 可选 `--read-only`（默认）或显式 `--allow-bootstrap`。

优先使用仓库内脚本：

```zsh
./scripts/agent-llm-mm.sh init
./scripts/agent-llm-mm.sh doctor --read-only
```

示例 profile 是结构模板，包含占位 `database_url`。先复制到 `agent-llm-mm.local.toml`，替换为本机可写 SQLite 路径后，再检查本机私有配置；如果选择 prod-local profile，还必须同时替换 `base_url`、`model`，并配置本机私有 `api_key` 或 `api_key_env`：

```zsh
./scripts/agent-llm-mm.sh init agent-llm-mm.local.toml
./scripts/agent-llm-mm.sh doctor --read-only agent-llm-mm.local.toml
```

如果你想绕过脚本，也可以：

```zsh
cargo run --quiet --bin agent_llm_mm -- doctor --read-only
```

预期输出为 JSON，至少包含：

- `transport`
- `database_url`
- `provider`
- `database_lifecycle`
- `status`

如需记录“本地首启模拟”证据，可以运行：

```zsh
./scripts/first-run-bootstrap-smoke-local.sh
```

也可以传入一个不存在或为空的输出目录：

```zsh
./scripts/first-run-bootstrap-smoke-local.sh target/first-run-bootstrap-smoke/manual-check
```

该脚本会在隔离输出目录内调用 `bootstrap-local`，把生成配置的 `database_url` 改成同目录下的 `first-run.sqlite`，再显式运行 `init` 与 `doctor --read-only`，并写出 `init.json` / `doctor.json` / `summary.json`。它会清理环境变量干扰，不写真实 HOME，不启动 `serve`，不调用 product smoke，也不证明真实 fresh-machine install 或 Windows runtime parity。

## 5. 启动 MCP 服务

```zsh
./scripts/agent-llm-mm.sh serve
```

或：

```zsh
cargo run --quiet --bin agent_llm_mm -- serve
```

服务启动后会占用当前终端并等待 `stdio` JSON-RPC 输入，这是正常现象。

## 6. Codex 配置

推荐直接使用 macOS 原生入口脚本：

```toml
[mcp_servers.agent-llm-mm]
command = "/absolute/path/agent-llm-mm/scripts/agent-llm-mm.sh"
args = ["serve"]
env = { AGENT_LLM_MM_CONFIG = "/absolute/path/agent-llm-mm/agent-llm-mm.local.toml" }
transport = "stdio"
```

如果你不想经过脚本，也可以：

```toml
[mcp_servers.agent-llm-mm-cargo]
command = "cargo"
args = ["run", "--quiet", "--bin", "agent_llm_mm", "--", "serve"]
env = { AGENT_LLM_MM_CONFIG = "/absolute/path/agent-llm-mm/agent-llm-mm.local.toml" }
transport = "stdio"
```

## 7. 推荐验证顺序

```zsh
cargo fmt --check
git diff --check
cargo clippy --all-targets -- -D warnings
./scripts/test-tier.sh fast
./scripts/test-tier.sh core
./scripts/status-sync-check.sh
./scripts/agent-llm-mm.sh doctor --read-only
```

涉及发布证据、打包或 provider certification 工具时，再运行
`cargo clippy --all-targets --all-features -- -D warnings` 和
`./scripts/test-tier.sh full`。`release-tools` 不进入默认运行时构建。

发布前请按 [Release Gate](release-gate.md) 跑完整 gate；本节只是 macOS 日常验证入口。
如果判断 Local Product Alpha / product alpha 口径，还必须改用 [Local Alpha Release Gate](product/release-gate-local-alpha.md)；普通 `doctor` 通过不等于 Local Alpha 完成。
需要本机 release soak 证据时，可以运行 `./scripts/release-soak-local.sh <candidate-name> [config_path]`。该命令把数据库操作强制绑定到 `target/release-soak-runtime/<candidate-name>/release-soak.sqlite`，先显式 init，再只读 doctor；传入配置中的数据库路径不会成为 soak 写目标。它仍只生成本机候选证据，不生成真实 fresh-machine、Windows runner、remote/team、上传、tag、安装包或发布认证证据。

如果需要检查 provider live evidence preflight 缺口，先运行本地只读 preflight：

```zsh
./scripts/provider-certification-check.sh [config_path] [evidence_root] [output_dir]
```

如果需要为该 preflight 生成 bounded live evidence，可以显式运行：

```zsh
./scripts/provider-live-certification-run.sh --live [config_path] [evidence_root]
```

配置示例本身不是 live evidence；必须显式运行 live runner 才能产生 preflight 可读取的 live evidence。该 live runner 只写 provider preflight evidence files，记录本次配置下的 endpoint reachability、decision probe、self-revision parse probe、错误处理和 redaction review provenance；它不得输出 API key、URL userinfo、path secret、query secret、model id、request body、response body 或 provider-native payload。即便 preflight 之后显示 `live_certified = true`，含义也只限 config preflight 通过且四类 live evidence present，不代表 provider 输出质量、SLA、provider gateway、Local Alpha、Beta、GA、production-ready、production readiness 或 release approval。`--stub-evidence` 只生成本地模拟证据，不能让 `live_certified = true`。

## 7.1 本地支持包

需要本机排障材料时，可以生成脱敏 support bundle：

```zsh
./scripts/generate-support-bundle.sh target/support-bundles/manual-check
./scripts/generate-support-bundle.sh target/support-bundles/manual-check-config agent-llm-mm.local.toml
```

输出目录必须不存在或为空；生成器会拒绝非空目录，避免旧的本地文件混入可分享支持包。输出目录会包含 redacted `doctor` shape、config shape、bounded operation summaries、release metadata、product smoke summary 和 manifest。该支持包默认不复制完整 SQLite 数据库、不包含 raw TOML、不包含 provider payload，也不会上传数据。它只是 Local Alpha 诊断辅助，不代表生产支持通道或 Local Alpha 已完成。

## 7.2 本地接入排障

| Symptom | Likely Cause | Verification | Fix |
| --- | --- | --- | --- |
| `init` / `migrate` cannot write SQLite | database path not writable or sandbox restriction | 先用 `doctor --read-only` 查看 `database_lifecycle`，再检查 config 与 `AGENT_LLM_MM_DATABASE_URL` | 为显式 lifecycle command 设置可写 SQLite URL；不要为通过 doctor 而放宽正式库权限 |
| `serve` reports database not ready | database missing, stale, or runtime defaults incomplete | 运行 `doctor --read-only` 查看 status / schema version | missing 用 `init`；旧 schema 用 `migrate`；只有明确接受写入时才用 `doctor --allow-bootstrap` |
| MCP client starts the wrong binary | auxiliary `src/bin` target ambiguity | 检查客户端配置是否已显式带 `--bin agent_llm_mm`（如 `args = ["run", "--quiet", "--bin", "agent_llm_mm", "--", "serve"]`） | 使用 `agent-llm-mm.sh` 封装，或在客户端里固定 `cargo run --quiet --bin agent_llm_mm -- serve` |
| dashboard not visible | `[dashboard].enabled` 为 false，或端口不可用 | 查看配置里的 `[dashboard]` 与 `enabled`，并确认 `./scripts/agent-llm-mm.sh doctor` 输出中的 dashboard 信息 | 启用 `[dashboard].enabled = true`，并换到可用的 `127.0.0.1` localhost 端口 |
| model calls fail | provider 配置不完整 | 执行 `./scripts/agent-llm-mm.sh doctor`，确认 `provider`、`base_url`、`model` 均已回填 | 更新本地 TOML 的 provider 段；密钥只在本地文件里设置，不要提交 secrets |

## 7.3 SQLite 备份与恢复

完整数据生命周期边界见 [Data Lifecycle](product/data-lifecycle.md)。本节只保留 macOS 本地命令入口；正式数据、手工 test 数据和 demo 数据必须使用不同 `database_url`。这里的正式数据对应 `prod-local`，手工 test 数据对应 dev / manual profile。本地正式数据进入 productization 前，应先保守地按 [Local Alpha Data Safety Runbook](product/data-safety-local-alpha.md) 做 SQLite backup：

```zsh
./scripts/backup-sqlite.sh "sqlite:///Users/<you>/agent-llm-mm/formal/agent-llm-mm.sqlite"
```

默认 backup 目录是仓库内 `target/backups/sqlite/`，用于和常规 live database directory 分离；脚本会在写入前解析 live DB 目录和 backup 目录，如果 backup 目录等于 live DB 目录，或位于 live DB 目录的子目录下，会拒绝继续，并要求指定其它 backup 目录。backup 路径不能包含双引号、反斜杠、换行或 `..` 路径组件；`sqlite://` URL 也要求使用正斜杠路径，不能用反斜杠 escape，且会拒绝 invalid percent escape、控制字符、编码反斜杠和编码双引号。写出的 backup 和 restored database 文件会收紧到 owner-only 权限；restore 会先预留新目标路径，避免覆盖已有文件。如本机有 `shasum -a 256`，脚本会生成 `.sha256` checksum，缺少 checksum 工具时只提示降级。

恢复时默认原则是 restore to a new path first，然后用新的 `database_url` 验证：

```zsh
./scripts/restore-sqlite.sh target/backups/sqlite/formal.sqlite.20260511-120000.12345.bak \
  "sqlite:///Users/<you>/agent-llm-mm/restore-check/formal-restore.sqlite"
```

确认 restored database 可用后，再由人工决定是否切换正式配置。不要把 backup 直接覆盖回现有正式 SQLite 文件。

## 8. Self-Revision Demo Package

如果要在本机快速验证 automatic self-revision MVP 的完整证据链，写到 timestamped / manual 目录，避免绕过 Local Alpha product smoke 的 staging / promote 路径直接覆盖 `latest`：

```zsh
./scripts/run-self-revision-demo.sh target/reports/self-revision-demo/manual-$(date +%Y%m%d-%H%M%S)
```

Local Alpha 发布证据的 `latest` 目录由 `./scripts/product-smoke-local.sh` 负责更新。

该脚本会构建本地二进制、启动 deterministic `openai-compatible` stub provider，并通过真实 MCP `stdio` 服务生成：

- `doctor.json`
- `snapshot-before.json`
- `snapshot-after.json`
- `decision-before.json`
- `decision-after.json`
- `timeline.json`
- `sqlite-summary.json`
- `report.md`

这条 demo 不需要真实 API key，也不会访问外网。

## 9. Dashboard 面板

在 `agent-llm-mm.local.toml` 中启用：

```toml
[dashboard]
enabled = true
host = "127.0.0.1"
port = 8787
```

然后启动：

```zsh
./scripts/agent-llm-mm.sh serve
```

浏览器访问 `http://127.0.0.1:8787/`。该面板只读，不会调用 `run_reflection` 或修改 SQLite。启用 dashboard 时，配置校验只接受 `localhost` 或 loopback IP；`0.0.0.0`、LAN IP 和域名会在启动前被拒绝。`base_path = "/agent-llm-mm"` 只表示路径挂载，不是认证、授权或公网暴露控制。不要在没有单独产品化 gate / auth 决策前把 dashboard 暴露到公网反向代理。

当前面板标题为 `Memory-chan Live Desk`，内嵌两份生成图物料：

- `src/interfaces/dashboard/static/memory_chan_hero.png`
- `src/interfaces/dashboard/static/memory_chan_sidebar.png`

这些物料只用于 dashboard 本机观测页面，归属与声明见仓库根目录 [NOTICE](../NOTICE)。

## 10. 额外说明

- `agent-llm-mm.local.toml` 已被 `.gitignore` 忽略，不应提交。
- 未显式设置 `database_url` 时，默认库会落到当前平台的用户数据目录，并按“本机用户共享”语义复用。
- 正式数据、手工测试数据和 demo 数据必须分开使用不同数据库文件；prod-local 只用于要保留、检查或备份的本地正式数据。
- 如果多个本机客户端共用同一 SQLite 文件，需要预期 SQLite 单写者模型带来的锁等待和状态互相影响。
- 所有示例 profile 都保持 `[daemon].enabled = false`；后续 daemon 观察模式必须走单独 gate。

## 2026-10-09 schema-v5 feedback and experience verification

The [A–E runtime guide](memory-feedback-experience.md) describes explicit migration, read-only index inspection/rebuild and new MCP tools. Existing v4 databases require explicit `migrate` before serving; keep the generated backup and restore to a new path. New deterministic checks are `cargo test --test feedback_candidates --test experience_workflow --test indexed_text_recall --test sqlite_lifecycle`. The Python evaluation/capacity scripts accept an existing binary and never require paid model calls; see [methodology](evaluation-methodology.md). Use `python3` on macOS and `python` on Windows, with the native binary suffix. Exact-head platform evidence remains distinct from configured workflows and from formal fresh-machine/release gates.

## Schema v6 temporal/scope/export

Run explicit `migrate` for an existing v5 database; inspect the generated backup and restore only to a new path. [Temporal fields](temporal-metadata.md) keep unknown historical Claim times null and preserve raw timestamps; [Reflection scope](reflection-scope-history.md) never turns effect scope into read authority. [Export](scoped-export.md) is bounded readonly interchange, not a replacement for backup. Verify with `cargo test --test temporal_metadata --test sqlite_temporal_store --test reflection_scope_history --test scoped_ledger_export --test schema6_migration` and `scripts/temporal-scope-export-smoke.py` using the platform's Python and binary path. Rust lifecycle/format/full gates remain required.


### Final original-plan context and caller-budget regression

`cargo test --test context_diagnostics --test caller_operation_budget --test mcp_stdio --test schema6_migration` covers scoped rich Episode context, honest bounded diagnostics, optional caller counts/stops over actual local MCP subprocesses, and historical receipt/fingerprint replay across migration. Run the normal full/fmt/Clippy/status-sync gates afterward. The fixed offline evaluator keeps its original tasks and byte budgets; richer metadata costs must be reported rather than hidden by retuning fixtures. See [context](context-diagnostics.md), [caller budget](caller-operation-budget.md), and [module extraction](implementation-module-boundaries.md) contracts.

## Schema v7 global self-model versions

Use explicit `migrate` for older databases, including v6; retain its backup anchor and restore only to a new path. Migration captures current projections as baseline0 with unknown historical effective time. `serve` and index repair never seed or repair the version ledger. See [version contract](self-model-versions.md) and [database operations](database-operations.md). Run `cargo test --test schema7_migration --test self_model_versions --test version_api` and the full/fmt/Clippy/status-sync gates against this source; prior schema6 counts remain historical. Versioned writes and reads reject projection drift. Local deterministic tests are not actual user-client or fresh-machine evidence.
