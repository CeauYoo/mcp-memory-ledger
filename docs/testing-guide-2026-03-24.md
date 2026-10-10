# Self-Agent MCP 测试指南（2026-03-24，按 2026-07-15 fresh 验证更新）

2026-10-09 文档导航：首次使用见[快速开始](quickstart.md)，当前操作见[数据库手册](database-operations.md)，可执行协议示例见[完整工作流](runnable-memory-workflow.md)。实现基线三平台验证与 Windows native / wrapper 区别见[当前状态](project-status.md)；下方分阶段追加记录不代表新的发布批准。

## 2026-10-10 portable package 与 wrapper 验证

- `python scripts/test-windows-wrapper.py`：必须找到真实 PowerShell 与 Cargo，不允许缺失工具时跳过成功。CI 在 Windows runner 上执行；其他平台运行不代替 Windows 证据。
- portable build 与无 Rust 解包测试、负向 fixture 命令见[本地包合同](portable-packages.md)。构建身份绑定精确 Git commit/tree，测试修改后的工作区须先形成可核对的提交快照。
- CI 产物保留在 runner 本地；本阶段不上传包、不创建 release/tag、不替用户执行客户端验收。

## 1. 目标

这份文档说明当前仓库应如何测试，覆盖：

- 代码格式与静态检查
- 自动化测试套件
- `namespace` / SQLite 迁移 / MCP `stdio` 的定向验证
- `doctor` 预检
- 手工 smoke test 的推荐方式

如果目标是判断“是否可以按当前 demo / MVP 口径发布”，请同时阅读 [Release Gate](release-gate.md)。本页偏向测试与回归，`release-gate.md` 负责收口发布前的最低 gate、self-revision 证据 gate、dashboard gate 和 sandbox 失败解释口径。

如果目标是判断“是否可以进入 Local Product Alpha / product alpha 口径”，请使用 [Local Alpha Release Gate](product/release-gate-local-alpha.md) 作为入口。本测试指南只保留参考入口，不复制产品 gate 的长清单；Local Alpha gate 会额外检查 product smoke evidence、dashboard local-only 边界、daemon disabled / observe-only 边界，以及 remote write / multi-tenancy 等产品文案限制。

当前工作目录（按实际环境替换）：

`~/code/agent-llm-mm`

命令执行环境要求：

- 安装 `rustup`；仓库固定 Rust `1.95.0` 与 `rustfmt` / `clippy`
- `cargo` 可用
- `bash` 或 `zsh`（用于 `scripts/agent-llm-mm.sh`）

---

## 2. 当前测试分层

测试数量不作为独立的文档状态源；[当前状态](project-status.md)只保留绑定精确提交的已验证数量，不让历史数字代替新 head 检查。固定入口按反馈成本分为三级：

| 层级 | 命令 | 使用场景 | feature 边界 |
| --- | --- | --- | --- |
| `fast` | `./scripts/test-tier.sh fast` | 日常逻辑修改后的短反馈 | 只跑 lib 与 decision/domain/evidence 核心契约 |
| `core` | `./scripts/test-tier.sh core` | 默认运行时、SQLite、MCP、dashboard、doctor、support bundle 回归 | 默认 features，不编译发布工具链 |
| `full` | `./scripts/test-tier.sh full` | 发布工具变更、合并前或 release gate | 等价于 `cargo test --all-features`，包含 `release-tools` |

`release-tools` 包含 Local Alpha evidence、release decision、product readiness、
provider certification 和 packaging 相关模块、二进制与测试；这些资产没有删除，
只是退出默认开发循环。测试文件增长时不要机械更新多份“当前总数”；将带提交的证据集中到状态页。

所有 13 个 bin target 都没有内部单元测试，因此关闭了 Cargo 的空 bin test harness；
需要真实进程的 E2E 仍通过 `CARGO_BIN_EXE_*` 启动实际二进制并保留在对应集成测试中。

静态检查同样分层：日常使用
`cargo clippy --all-targets -- -D warnings`；涉及 `release-tools` 或最终全功能验证时
使用 `cargo clippy --all-targets --all-features -- -D warnings`。

`./scripts/status-sync-check.sh` 只读取 active plan 与 reality gates，不编译整套测试；
它要求至少存在一项完成态 checkbox，并在缺少匹配 reality row 或对应状态未完成时失败，
避免零项解析被误报为同步成功。

`.github/workflows/ci.yml` 在 `ubuntu-latest` 与 `macos-latest` 上执行同一组
format、all-feature Clippy、`full` 与 status-sync 门禁。`tests/ci_contract.rs`
阻止平台或命令清单被静默削弱；Windows 仍是 M2 独立 runtime parity gate。

---

## 3. 测试前准备

### 3.1 环境要求

- 安装 `rustup`，并允许仓库选择 `rust-toolchain.toml` 中的 Rust `1.95.0`
- 可用的 `cargo`
- `bash` 或 `zsh`：用于 `scripts/agent-llm-mm.sh`

### 3.2 建议进入工作目录

```zsh
cd ~/code/agent-llm-mm
```

### 3.3 数据库隔离建议

未显式设置 `database_url` 时，服务会把默认 SQLite 文件放到当前平台的用户数据目录，并按“本机用户共享默认库”语义复用。为了避免和已有运行实例互相污染，手工测试时建议显式设置：

```zsh
cp examples/agent-llm-mm.example.toml agent-llm-mm.local.toml
```

然后修改 `agent-llm-mm.local.toml` 里的：

- `database_url`
- `provider`
- provider-specific 配置

如果只是跑现有自动化测试，不需要手工设置；测试本身已经隔离数据库。`doctor` 默认只读，对 missing / old / read-only 路径只报告 `database_lifecycle`，不会因无法写入而 bootstrap。需要建立或升级测试库时，必须显式运行 `init` 或 `migrate`。正式接入、手工测试和实验验证仍应使用不同数据库文件。

`bootstrap` 的默认启动回归会先显式初始化临时数据库，再以独立环境和管道
`stdin` / `stdout` 启动无子命令的真实二进制，完成 MCP `initialize` 与
`tools/list` 往返。它不依赖测试运行器的 stdin、用户默认库或本地配置；响应等待
有超时，失败时也会回收子进程。`serve` 仍不隐式创建或迁移数据库；缺失数据库的拒绝路径
由 `sqlite_lifecycle` 覆盖。定向验证可运行：

```bash
cargo test --test bootstrap default_command_serves_stdio_after_explicit_database_init -- --exact
cargo test --test bootstrap --test sqlite_lifecycle --test mcp_stdio
```

---

## 4. 推荐测试顺序

建议按下面顺序执行：

1. `cargo fmt --all -- --check`
2. `git diff --check`
3. `./scripts/test-tier.sh fast`
4. `cargo clippy --all-targets -- -D warnings`
5. `./scripts/test-tier.sh core`
6. `./scripts/status-sync-check.sh`
7. 对 scratch path 运行 `init`，再运行 `doctor --read-only`，确认 lifecycle status 为 current
8. `cargo test --test sqlite_lifecycle -v`
9. 涉及发布工具链或最终 full gate 时，再运行 `cargo clippy --all-targets --all-features -- -D warnings` 和 `./scripts/test-tier.sh full`
10. 如果改动涉及 automatic self-revision MVP，再补跑本指南里的 runtime coverage / diagnostics / evidence policy 定向验证
11. 如果改动涉及 demo package，先用 timestamped / scratch output 跑 `./scripts/run-self-revision-demo.sh target/reports/self-revision-demo/manual-$(date +%Y%m%d-%H%M%S)`；如果要按 Local Alpha 发布口径复核 `latest` 证据链，使用下一条 product smoke
12. 如果改动涉及 Local Alpha product smoke gate、启动包装脚本或本地产品化证据链，在 repo root 补跑 `./scripts/product-smoke-local.sh [config_path]`；如果当前目录不是 repo root，使用 `/path/to/agent-llm-mm/scripts/product-smoke-local.sh`，并在需要配置文件时传入绝对 config path
13. 如果改动涉及 wrapper，确认模式仍为 `serve|init|migrate|doctor|bootstrap-local`，doctor 只接受 `--read-only|--allow-bootstrap`，unsupported mode 返回 exit code `2`，并补跑 `cargo test --test bootstrap -v`
14. 如果改动涉及 first-run bootstrap smoke 或 `bootstrap-local -> init -> doctor --read-only` 路径，补跑 `bash -n scripts/first-run-bootstrap-smoke-local.sh` 和 `cargo test --test first_run_bootstrap_smoke -v`
15. 如果改动涉及 Local Alpha evidence summary、发布证据汇总或 gate status 输出，补跑 `bash -n scripts/local-alpha-evidence-summary.sh`、`cargo test --features release-tools --test local_alpha_release_evidence -v`，并用 `cargo run --quiet --features release-tools --bin local_alpha_evidence_summary -- --evidence-root .` spot-check JSON 输出；该 summary 只是本地只读 gate 状态汇总，不是自动认证
16. 如果改动涉及 Local Alpha release-gate refresh 或本机 gate 证据刷新流程，补跑 `bash -n scripts/local-alpha-release-gate-refresh.sh`、`cargo test --features release-tools --test local_alpha_release_evidence -v`，并按需执行 `./scripts/local-alpha-release-gate-refresh.sh [config_path]`；该 refresh 只产生本机可复现证据，不生成真实 fresh-machine、Windows runner、remote/team 或发布决策证据
17. 如果改动涉及 release engineering、release evidence directory、soak evidence 或候选发布说明，补跑 `bash -n scripts/release-soak-local.sh`、`cargo test --features release-tools --test local_alpha_release_evidence release_soak -v`，并按需执行 `./scripts/release-soak-local.sh <candidate-name> [config_path]`；该 soak 只生成本地 release evidence，不生成真实 fresh-machine、Windows runner、remote/team、上传、tag、安装包或发布认证证据
18. 如果改动涉及 SQLite 备份、恢复、schema migration 前置检查或 data lifecycle gate，补跑以下命令：
    ```bash
    bash -n scripts/backup-sqlite.sh
    bash -n scripts/restore-sqlite.sh
    cargo test --test sqlite_backup_restore -v
    cargo test --test sqlite_lifecycle -v
    ```
19. 如果改动涉及 product readiness、release decision artifact、产品措辞 gate、remote/team inventory/security gates、evidence relation、episode projection、layered memory projection 或 `doctor.system_layer_report`，补跑 `cargo test --features release-tools --test product_readiness -v`、`cargo test --features release-tools --test release_decision -v`、`cargo test --test product_completion_read_models -v`、`cargo test --test provider_config -v` 和 `./scripts/product-readiness-check.sh <candidate-name>` 的本地预检；这些检查只能核验本地门禁、doctor 只读架构层报告、runtime / declared-test-contract dependency-rule evidence、physics-informed non-claim / wording guard 和只读投影，不生成真实 fresh-machine、Windows runner、remote/team 产品模式、GA 或发布认证证据
20. 如果改动涉及 release evidence index、provider certification preflight 或 packaging preflight，补跑以下命令，并按需执行对应脚本：
    ```bash
    cargo test --features release-tools --test non_mvp_product_tracks -v
    bash -n scripts/release-evidence-index.sh
    bash -n scripts/provider-certification-check.sh
    bash -n scripts/packaging-preflight-check.sh
    ```
    这些 preflight 只读取本地 evidence/config shape；provider `--live` runner 只生成 provider preflight evidence files，用于记录本次配置下的 endpoint reachability、decision probe、self-revision parse probe、错误处理和 redaction review provenance；它不生成 installer、不上传文件、不认证 provider 质量、provider gateway、Local Alpha、Beta、GA、production-ready 或 release approval；provider live evidence 需要非空 JSON、匹配 provider、`status = "passed"`、expected `evidence_kind`、`mode = "live"`、非空 `generated_at`、`local_only = false`、`endpoint_reached = true`、`redaction_reviewed = true`、`request_outcome = "passed"` 和带显式 `exit_code = 0` 的成功 command evidence；packaging archive evidence 需要预期 archive 全部存在、非空、可解析为对应 `.tar.gz` / `.zip` archive，并与 manifest 的 name / size / SHA-256 匹配

如果当前机器没有 `pwsh`，PowerShell runtime 行为测试会跳过；这种情况下只代表 Rust 测试覆盖了 PowerShell 脚本文本契约和 no-clobber 静态断言，Windows runner 或 Windows 实机验证仍需单独记录。

如果只想快速回归某个变更，再执行对应的定向测试。若需要一份面向发布前核验的固定检查单，demo / MVP 发布直接使用 [Release Gate](release-gate.md)；Local Alpha / product alpha 发布使用 [Local Alpha Release Gate](product/release-gate-local-alpha.md)。

---

## 5. 全量验证

### 5.1 格式检查

```zsh
cargo fmt --check
```

通过标准：

- 命令退出码为 `0`
- 没有 diff 输出

### 5.2 补丁空白检查

```zsh
git diff --check
```

通过标准：

- 命令退出码为 `0`
- 没有 whitespace error、冲突标记或补丁格式问题

### 5.3 静态检查

```zsh
cargo clippy --all-targets --all-features -- -D warnings
```

通过标准：

- 命令退出码为 `0`
- 没有 warning

### 5.4 全功能测试

```zsh
./scripts/test-tier.sh full
```

重点覆盖：

- domain invariants
- application use cases
- SQLite adapter
- MCP `stdio` E2E
- failure modes
- 启动与配置基线
- release evidence、provider certification 与 packaging 工具

通过标准：

- 所有测试通过
- 没有失败、panic 或 `UnexpectedEof`

### 5.5 本机预检

```zsh
AGENT_LLM_MM_DATABASE_URL=sqlite:///private/tmp/agent-llm-mm-doctor.sqlite ./scripts/agent-llm-mm.sh doctor
```

```zsh
AGENT_LLM_MM_DATABASE_URL=sqlite:///private/tmp/agent-llm-mm-doctor-cargo.sqlite cargo run --quiet --bin agent_llm_mm -- doctor
```

预期输出为 JSON，至少包含：

- `transport`
- `database_url`
- `provider`
- `status`

通过标准：

- `status` 为 `ok`
- 未出现 bootstrap 或 runtime 初始化错误

---

## 6. 定向测试

### 6.1 `namespace` / SQLite 约束

```zsh
cargo test --test sqlite_store
```

重点覆盖：

- `events.namespace` / `claims.namespace` 持久化
- legacy schema 迁移
- `owner <-> namespace` 数据库级 `CHECK` 约束
- adapter 写入/读取兜底
- owner/namespace SQL 规则是否保持单一来源
- evidence query 是否按 namespace 先过滤、再排序和限流

特别关注这些测试名：

- `sqlite_store_bootstraps_all_tables`
- `sqlite_query_evidence_event_ids_filters_by_namespace_before_limit`
- `sqlite_query_evidence_event_ids_filters_by_kind_only`
- `sqlite_query_evidence_event_ids_rejects_zero_limit`
- `sqlite_bootstrap_backfills_namespace_for_legacy_event_rows`
- `sqlite_bootstrap_backfills_namespace_for_legacy_claim_rows`
- `sqlite_store_rejects_owner_namespace_mismatch_on_write`
- `sqlite_database_rejects_corrupt_namespace_owner_pair_before_read`
- `sqlite_owner_namespace_sql_rules_have_single_source`

适用场景：

- 修改了 `src/adapters/sqlite/schema.rs`
- 修改了 `src/adapters/sqlite/store.rs`
- 修改了 `Namespace` / `ClaimDraft` 相关规则

### 6.2 SQLite backup / restore 脚本门禁

```zsh
bash -n scripts/backup-sqlite.sh
bash -n scripts/restore-sqlite.sh
cargo test --test sqlite_backup_restore -v
```

这个门禁执行 bash 脚本；没有 `bash` 的平台会按现有脚本测试模式跳过 Rust 测试里的脚本调用。Windows 上需要 Git Bash、WSL 或等价 bash 环境来实际验证脚本行为。

重点覆盖：

- `backup-sqlite.sh` 生成本地备份后，`restore-sqlite.sh` 可以恢复到新的 SQLite 路径并保留数据内容
- restore 拒绝覆盖已有目标文件
- backup 拒绝把备份目录放到 live database 目录树内
- backup / restore 拒绝 in-memory SQLite 文件语义
- SQLite file URL 的 invalid percent encoding 会被拒绝
- restore 目标路径里的 `..` 组件会被拒绝，避免恢复写入绕过调用方指定的新路径边界

该门禁仍是本地脚本回归，不代表远程备份、云同步、定时 daemon、生产灾备、admin/auth 或团队模式能力。

### 6.3 MCP `stdio` 端到端

```zsh
cargo test --test mcp_stdio
```

重点覆盖：

- 工具是否正确暴露
- `ingest_interaction -> build_self_snapshot` 是否共享 runtime 状态
- `run_reflection` 是否影响 active snapshot
- 配置文件指定 provider 后是否真的走到对应 provider
- `run_reflection` 的显式 evidence 输入是否允许 inferred replacement
- `run_reflection` 的 query-based evidence 输入是否被正确校验
- `run_reflection` 的 `identity_update` / `commitment_updates` 是否真正落盘并反映到后续 snapshot
- `run_reflection` 的 `identity_update` / `commitment_updates` 缺少 resolved evidence 时是否返回 `invalid_params`
- baseline commitment 是否阻断 forbidden action
- 非法 namespace 是否返回 `-32602 invalid_params`

特别关注这些测试名：

- `server_exposes_expected_tools_over_stdio`
- `stdio_tools_share_runtime_state_across_calls`
- `decide_with_snapshot_over_stdio_uses_openai_compatible_provider_from_config_file`
- `conflicting_reflection_over_stdio_removes_claim_from_active_snapshot`
- `inferred_replacement_reflection_with_evidence_is_accepted_over_stdio`
- `reflected_claim_replacement_query_is_accepted_over_stdio`
- `reflection_identity_and_commitment_updates_are_applied_and_audited_over_stdio`
- `reflection_identity_or_commitment_updates_require_evidence_over_stdio`
- `replacement_evidence_query_limit_overflow_is_invalid_params_over_stdio`
- `fresh_stdio_runtime_blocks_forbidden_action_with_seeded_commitment`
- `invalid_namespace_is_reported_as_invalid_params_over_stdio`

适用场景：

- 修改了 `src/interfaces/mcp/dto.rs`
- 修改了 `src/interfaces/mcp/server.rs`
- 修改了应用层输入校验或错误映射

#### 6.3.1 M0.2 scoped snapshot 最小回归

```zsh
cargo test --test evidence_query_dto snapshot_dto_ -v
cargo test --test sqlite_store sqlite_snapshot_queries_do_not_leak_across_owner_or_namespace -v
cargo test --test application_use_cases build_self_snapshot_returns_store_backed_snapshot_and_respects_budget -v
```

这组回归验证：MCP DTO 只接受 namespace 并由 server-side conversion 推导 owner；省略 namespace 仍保持 legacy unscoped 兼容；显式 scope 会贯穿 application/query port/SQLite，并覆盖 self、world、两个 project 与两个 user namespace，确保 claims、event references 和 episode references 的跨 scope 注入为 0。它不单独证明 evidence manifest、time window / stable order、统一 event ID 或 scoped auto-reflection；manifest 与时间窗边界由后续专用回归覆盖。

#### 6.3.2 M0.2 evidence manifest no-widening 回归

```zsh
cargo test --test domain_snapshot event_reference -v
cargo test --test domain_snapshot raw_and_prefixed_event_ids_share_one_canonical_reference -v
cargo test --test domain_invariants namespace_and_memory_scope_deserialization_preserve_scope_invariants -v
cargo test --test evidence_query_dto snapshot_dto_ -v
cargo test --test sqlite_store sqlite_snapshot_manifest_intersects_scope_without_widening -v
cargo test --test mcp_stdio stdio_tools_share_runtime_state_across_calls -v
cargo test --test mcp_stdio mcp_tool_calls_append_operation_logs_with_correlation_id_and_snapshot_scope -v
```

这组回归验证：`MemoryScope` 反序列化拒绝 partial / mismatched 状态；显式 manifest（包括空数组）必须同时显式提供 `namespace`，DTO、application 与 store 都不能进入 legacy unscoped 兼容路径；manifest 最多 256 项，边界值可接受、超限会在 SQLite bind 构造前以 invalid params 拒绝，重复项使用保序线性去重；裸 event ID 与 `event:<id>` 等价并输出 canonical reference；空白、空 ID、重复前缀等非法项会被拒绝；SQLite 使用 `owner + namespace + event_id IN (...)` 做交集查询；越 scope ID 被排除，显式空 manifest 和空交集均返回空 evidence 且不回退到全 scope；tool operation log 记录 snapshot namespace。非法 manifest / time filter 在 optional auto-reflection 前被拒绝的跨过滤器顺序、时间窗与稳定排序由下一节证明；本节也不证明 snapshot 外的全仓 ID 统一或 scoped auto-reflection。

#### 6.3.3 M0.2 snapshot time-window / stable-order 回归

```zsh
cargo test --test domain_snapshot snapshot_time_window_accepts_equal_boundaries_and_rejects_reversed_bounds -v
cargo test --test evidence_query_dto snapshot_dto_ -v
cargo test --test application_use_cases build_self_snapshot_ -v
cargo test --test sqlite_store sqlite_snapshot_time_window_intersects_scope_manifest_and_orders_real_instants -v
cargo test --test sqlite_store sqlite_snapshot_preserves_submillisecond_time_window_precision -v
cargo test --test sqlite_store sqlite_snapshot_orders_episodes_by_latest_in_window_event_tuple -v
cargo test --test mcp_stdio server_preserves_tool_input_schemas_over_stdio -v
cargo test --test mcp_stdio invalid_snapshot_filters_are_rejected_before_auto_reflection_side_effects -v
cargo test --test failure_modes auto_reflection_scopes_trigger_window_to_input_namespace -v
```

这组回归验证：`recorded_after` / `recorded_before` 是 additive optional MCP 字段，但任一边界都要求显式 `namespace`；RFC3339 会归一为 UTC，闭区间两端相等可用，倒置窗口由 DTO 与 application 拒绝，并在 optional auto-reflection 前停止；显式窗口查无结果时保持空 evidence / episodes，不会回退扩大查询。SQLite 在同一路径上取 scope ∩ manifest（如有）∩ time window，把项目 canonical timestamp 与常见 legacy `Z` / offset 文本归一为固定宽度 UTC 秒 + 9 位小数秒键，并以专用亚毫秒边界用例证明不会退化到 SQLite `julianday()` 精度，再按 `(recorded_at DESC, event rowid DESC)` 稳定排序；legacy unbounded SQLite snapshot 也走 recent-first 查询。automatic reflection 的候选 scope、manifest 与由候选生成的闭区间会传入同一 snapshot/episode path，跨 namespace 事件不可进入；空候选保持 not-triggered。claims 没有 recorded timestamp，本组不宣称 claim recency filtering。

#### 6.3.4 M0.2 active reflection runtime event-ID 回归

```zsh
cargo test --test application_use_cases reflection_replaces_with_query_and_explicit_evidence_ids_without_duplication -v
cargo test --test application_use_cases reflection_rejects_invalid_explicit_event_reference_forms -v
cargo test --test failure_modes auto_reflection_normalizes_prefixed_proposal_evidence_ids_before_governance_and_audit -v
cargo test --test mcp_stdio inferred_replacement_reflection_with_evidence_is_accepted_over_stdio -v
cargo test --test sqlite_store sqlite_reflection_transactions_replace_identity_and_commitments_atomically -v
```

这组回归验证 active reflection runtime 的 explicit MCP/application evidence、query 合并和 auto-reflection proposal 都把裸 ID 与 `event:<id>` 解析为同一 raw event ID，按首次出现顺序去重，并拒绝空白、空 ID 与重复 `event:` 前缀。存在性校验与 SQLite 查询仍使用 raw ID；reflection audit `supporting_evidence_event_ids` 和 auto-reflection diagnostics 的 `*_event_ids` 是兼容字段，readback 继续断言 raw IDs，而 reference-shaped 输出才使用 canonical `event:<id>`。它不证明 repository-wide ID 统一；offline demo 由 6.3.6 单独覆盖，support bundle 等其他表面仍是 partial。

#### 6.3.5 M0.2 read-only evidence / episode projection event-ID 回归

```zsh
cargo test --test product_completion_read_models -v
cargo test --test domain_snapshot raw_and_prefixed_event_ids_share_one_canonical_reference -v
cargo test --test domain_snapshot invalid_event_references_are_rejected -v
cargo test --test bootstrap doctor_reports_self_revision_runtime_coverage -v
```

这组回归验证 evidence relation 的 `trigger_window_event_ids` / `selected_evidence_event_ids` 与 episode summary 的 `episode_event_ids` / `linked_evidence_ids` 都接受裸 ID 和 `event:<id>`，按底层 raw ID 保序去重后再执行 subset/no-widening、`event_count`、selected/rejected count 和 window rank。空白、空 ID、重复前缀 fail closed；projection JSON 的 `event_id` / `*_event_ids` readback 继续保持 raw IDs，不增加持久化、MCP tool 或 runtime read path。它不证明 repository-wide ID 统一；offline demo 由下一节单独覆盖，support bundle 和其他未授权表面仍是 partial。

#### 6.3.6 M0.2 offline self-revision demo event-ID 回归

```zsh
cargo test --test self_revision_demo_runner -v
cargo test --test demo_openai_compatible_stub -v
```

这组回归验证 deterministic demo 的真实边界：runner 不接收 caller-provided event ID；MCP ingest 返回的 raw `event_id` 必须经 `EventReference` fail-closed 解析，并在外部 `timeline.json` 中以 canonical `event_reference = event:<id>` 输出。snapshot 的 `evidence` 原本已是 canonical reference；内置与独立 stub 只返回空 `proposed_evidence_event_ids` 加受控 query，不比较或回显 event ID；SQLite artifact 的 `supporting_evidence_event_ids` 是明确 raw 兼容字段。它不改 SQLite schema、active runtime/projection、发布证据或 provider live path；support bundle 仅作为后续 inventory，repository-wide 统一仍为 partial。

#### 6.3.7 M0.2 收口门禁

```zsh
./scripts/test-tier.sh fast
./scripts/test-tier.sh core
cargo test --test status_sync
./scripts/status-sync-check.sh
```

M0.2 只有在 6.3.1–6.3.6 的行为边界由 `fast` / `core` 当前运行覆盖，且 active plan 的 `M0.2 Scoped Snapshot v2` 完成项与 reality-gate 的 `implemented` 行一致时才算收口。该完成状态不证明省略 `namespace` 的 legacy 调用已隔离，不证明完整 memory recall contract、repository-wide event-ID 统一或 support-bundle inventory 已完成，也不授权进入 M0.4、M1、remote 或 release 工作。

### 6.3A M0.3 trusted decision commitments / dual gate 回归

```zsh
cargo test --test decision_flow -v
cargo test --test mcp_stdio fresh_stdio_runtime_blocks_forbidden_action_with_seeded_commitment -- --exact
cargo test --test mcp_stdio provider_selected_forbidden_action_is_blocked_over_stdio -- --exact
```

这组回归验证：caller 即使从 snapshot 删除 baseline commitment，application 仍从当前 `CommitmentStore` 恢复服务端 policy context，并在 model call 前阻断 requested action；requested action 允许但 provider-selected action 违反同一 commitment 时，结果仍为 blocked、`decision = null`，并保留被拒绝的 `selected_action` 和有界 reason。允许动作继续保持 v2 response envelope 与 provider action-string contract。这个切片单独不证明其余 caller snapshot 字段可信或完整 policy arbitration。

### 6.3B M0.3 claim → evidence → episode provenance 回归

```zsh
cargo test --test failure_modes auto_reflection_ignores_unrelated_episodes_for_identity_support -- --exact
cargo test --test failure_modes auto_reflection_rejected_identity_attempt_does_not_start_cooldown_for_later_valid_retry -- --exact
cargo test --test sqlite_store sqlite_lists_only_episodes_reached_through_claim_evidence_links -- --exact
```

这组回归验证：匹配 proposed identity value 的 active claims 只有经 persisted evidence link 到达 episode event membership 时才贡献 distinct cross-episode support；全局无关 episode、无 provenance 的 claims 和空 claim 集均不计数。拒绝路径只记录 rejected trigger，不写 reflection 或 identity；具备至少两条真实 episode 路径的后续 retry 仍可通过。该切片复用现有表，不证明完整 provenance graph 或全部治理失败原子性。Scope 完整性由后续 M1.0.1 回归覆盖。

### 6.3C M0.3 governance failure atomicity 回归

```zsh
cargo test --test failure_modes auto_reflection_commit_failure_records_only_rejected_audit_and_rolls_back_deeper_updates -- --exact
cargo test --test failure_modes auto_reflection_handled_ledger_failure_rolls_back_reflection_updates -- --exact
cargo test --test sqlite_store sqlite_handled_ledger_failure_rolls_back_deeper_reflection_updates -- --exact
```

这组回归把 validation rejection、handled trigger ledger append failure 与 reflection transaction commit failure 分开验证。失败后 identity、commitments、supporting claims/evidence links、reflection 与 handled ledger 必须保持原值或不存在；事务外仅允许一条 `Rejected` trigger entry，且不得带 `reflection_id`、`handled_at` 或 cooldown。SQLite 回归使用 duplicate ledger primary key 让最后的 handled-audit 写入失败，并 readback identity、commitments、reflection 与 ledger 行数。它证明当前本地 transaction path 的原子性，不证明进程崩溃恢复、跨进程事务或分布式一致性。

### 6.3D M0.3 experimental decision authority 回归

```zsh
cargo test --test decision_flow -v
cargo test --test mcp_stdio decide_with_snapshot_over_stdio_uses_openai_compatible_provider_from_config_file -- --exact
cargo test --test mcp_stdio provider_selected_forbidden_action_is_blocked_over_stdio -- --exact
```

这组回归验证：允许的 provider action-string 保持 legacy `status = model_decision` 与 `{ "action": "..." }` payload，但 additive 返回 `decision_authority = experimental_non_authoritative`、`policy_scope = server_commitment_gate_only` 和 `not an authoritative policy decision` non-claim；blocked 路径返回 `not_applicable_blocked`。因此 commitment gate 未阻断只能解释为该 bounded literal check 未命中，不能解释为 structured action validation 或完整 policy passed。

M0.3 只有在 6.3A–6.3D 的行为边界由当前 `fast` / `core` 运行覆盖，且 active plan 的 `M0.3 Governance Correctness` 与四个子切片都和 reality-gate 的 `implemented` 行一致时才算限定收口。该完成状态不证明 caller snapshot 其余字段可信、server-created snapshot handle、完整 provenance graph、structured action validation、policy arbitration、crash recovery 或 distributed transaction 已实现，也不授权顺带进入 M0.4、M1、remote 或 release 工作。

### 6.3E M0.4 explicit database lifecycle 回归

```zsh
cargo test --test sqlite_lifecycle -v
cargo test --test bootstrap -v
cargo test --test sqlite_backup_restore -v
bash -n scripts/agent-llm-mm.sh scripts/first-run-bootstrap-smoke-local.sh scripts/release-soak-local.sh
```

这组回归验证：read-only doctor 对 missing / legacy / read-only file 不 create、migrate 或 seed；`init` 建立 schema version 3、三条 ledger、runtime defaults 和零 FK violation；legacy migration 在原库写入前建立 backup 与 restore rehearsal，在事务中保持行数并通过 FK/readback；rehearsal 失败时原库字节和 schema 保持可恢复；`serve` 拒绝 missing database；`doctor --allow-bootstrap` 的写权限必须显式。release soak 还必须把 lifecycle、doctor、product smoke 和 support bundle 绑定到 candidate-isolated database，并在 `release-boundaries.json` 记录 `formal_database_path_accepted = false`。

M0.4 不证明 remote backup、scheduled backup、cloud sync、production disaster recovery，或超出 SQLite 事务语义的 crash/power-loss guarantee。

### 6.3F M1.1.1 / M1.1.2 / M1.1.3 / M1.1.4 / M1.1.5 / M1.1.6 / M1.2.1 / M1.2.2 / M1.2.3 / M1.2.4 / M1.2.5 / M1.2.6 / M1.2.7 scoped read and correction 回归

```zsh
cargo test --test sqlite_store sqlite_event_recall -v
cargo test --test sqlite_store sqlite_claim_recall_is_scoped_status_aware_and_returns_provenance -v
cargo test --test sqlite_store sqlite_episode_recall -v
cargo test --test domain_snapshot raw_and_prefixed_claim_ids_share_one_canonical_reference -v
cargo test --test domain_snapshot invalid_claim_references_are_rejected -v
cargo test --test evidence_query_dto get_memory_dto -v
cargo test --test evidence_query_dto get_reflection_history -v
cargo test --test evidence_query_dto get_self_model_history -v
cargo test --test evidence_query_dto episode -v
cargo test --test sqlite_store sqlite_claim_reflection_history -v
cargo test --test sqlite_store sqlite_self_model_history_is_scoped_claim_attributed_and_hides_record_only_rows -- --exact
cargo test --test sqlite_store sqlite_supersede_memory_is_scoped_claim_correction_and_hides_cross_scope_targets -- --exact
cargo test --test evidence_query_dto supersede_memory -v
cargo test --test mcp_stdio search_memory -v
cargo test --test mcp_stdio episode -v
cargo test --test mcp_stdio search_memory_returns_scoped_claims_with_revision_provenance_over_stdio -v
cargo test --test mcp_stdio search_memory_invalid_or_empty_scope_fails_closed_over_stdio -v
cargo test --test mcp_stdio search_memory_survives_stdio_reconnect_with_offline_provider -v
cargo test --test mcp_stdio get_memory_returns_one_scoped_event_or_null_without_widening -v
cargo test --test mcp_stdio server_exposes_expected_tools_over_stdio -v
cargo test --test mcp_stdio server_preserves_tool_input_schemas_over_stdio -v
./scripts/test-tier.sh fast
./scripts/test-tier.sh core
cargo test --test status_sync -v
./scripts/status-sync-check.sh
```

这组回归证明：`search_memory` 只接受显式 namespace，且省略 additive `record_type` 时继续使用 Event；SQLite 在 filter / limit 前执行 owner + namespace 收窄，exact Event/Claim reference 都不会跨 scope 命中。Event 路径保留完整字段与现有 claim/episode provenance；Claim 路径支持 canonical/raw `claim_reference`、`claim_status`、`mode` 和 `1..=100` limit，省略 status 默认 `Active`，并返回 canonical claim/evidence references、episode references 与直接 source/superseded reflection links。`get_memory` 省略 `record_type` 时保持 canonical/raw Event ID 语义，显式 `record_type = Claim` 时接受 canonical/raw Claim ID；精确 Claim lookup 可返回任意状态，但 missing / cross-scope 仍为 `record: null`。DTO 回归还覆盖 raw Event ID `claim:*` 不被误判为 Claim。

M1.1.3 回归另外证明：`record_type = Episode` 在 `search_memory` 中开放，Episode 必须使用显式 namespace，optional `episode_reference` 只做非空 exact persisted-string 匹配并原样返回。SQLite 在 Episode filter、分组、排序和 limit 之前通过 Event owner + namespace 收窄，并按该 scope 内最新 Event timestamp、Event rowid 与 Episode reference 稳定排序；同一 Episode reference 跨 namespace 时只返回请求 scope 的 membership。结果 `recorded_at` 来自最新 scoped Event，provenance 仅包含 canonical recent-first same-scope Event references 与 canonical same-scope Claim references。断开重连且 provider 不可达时读取仍可用，semantic tables 不变，operation metadata 只记录 `record_type` 与 `result_count`。M1.2.4 再让 `get_memory(record_type = Episode)` 以 `limit = 1` 复用同一 opaque exact reference；missing / 跨 scope 返回 `record: null`，省略类型仍保持 Event。

M1.2.3 回归另外证明：`get_reflection_history` 要求显式 namespace 与 exact Claim anchor，接受 canonical/raw Claim ID，limit 默认 20 且只允许 `1..=100`；SQLite 递归读取 superseded/replacement 双向 chain，并按 newest-first 返回有界结果与 `has_more`。missing/cross-scope Claim 返回空；mixed-scope edge 整条隐藏；supporting evidence 只返回同 scope canonical references；malformed legacy evidence fail closed。断开重连且 provider 不可达时历史读取仍可用，semantic memory tables 不变，operation log metadata 只保存 `history_type`、`result_count`、`has_more`。

该证据只完成 `M1.1.1 Scoped Event Recall Read Model`、`M1.1.2 Scoped Claim Provenance Read`、`M1.1.3 Scoped Episode Provenance Read`、`M1.1.4 Scoped Reflection Provenance Read`、`M1.1.5 Scoped Evidence Relation Runtime Read`、`M1.1.6 Stable Cross-Type Record Union`、`M1.2.1 Scoped Event Lookup`、`M1.2.2 Scoped Claim Lookup`、`M1.2.3 Scoped Claim Reflection History`、`M1.2.4 Scoped Episode Lookup`、`M1.2.5 Scoped Reflection Lookup and Record-only History`、`M1.2.6 Identity and Commitment History` 与 `M1.2.7 Audited Supersede Contract`。M1.0.1 / M1.0.2 / M1.0.3 分别由 6.3G / 6.3H / 6.3I 单独证明。M1.1.4 由 6.3J 单独补充。M1.1.5 由 6.3K 单独补充。M1.1.6 由 6.3L 单独补充。M1.2.4 由 6.3M 单独补充。M1.2.5 由 6.3N 单独补充。M1.2.6 由 6.3O 单独补充。M1.2.7 由 6.3P 单独补充。它不证明 versioned identity/commitment ledger、record-only reflection history、current-schema structural readback、exclusive init/migration、完整 M1.1/M1.2/M1、真实本地客户端 transcript、fresh-machine、Windows、remote 或 Local Alpha；Episode reference normalization、schema migration/index 和规模化性能也未证明，当前仍是 MVP 表扫描边界。

### 6.3G M1.0.1 scoped identity evidence-to-Episode gate

```zsh
cargo test --test sqlite_store sqlite_lists_only_episodes_reached_through_claim_evidence_links -- --exact
cargo test --test sqlite_store sqlite_identity_support_ignores_cross_scope_evidence_links -- --exact
cargo test --test failure_modes auto_reflection_ignores_cross_scope_evidence_links_for_identity_support -- --exact
cargo test --test application_use_cases episode_store_default_preserves_legacy_calls_and_fails_closed_for_scoped_calls -- --exact
./scripts/test-tier.sh core
```

这组回归验证：`list_episode_references_supporting_claims` 必须接收显式 `MemoryScope`；SQLite 在 Episode 分组/计数前同时限制 Claim 与 Evidence Event 的 owner + namespace。恶意跨 namespace evidence link 不能把外 scope Episode 计入 identity revision；legacy unscoped 与不支持该查询的 store 保持 fail closed。该切片复用现有表，不证明完整 provenance graph。

### 6.3H M1.0.2 mixed-scope Claim revision-edge redaction

```zsh
cargo test --test sqlite_store sqlite_claim_revision_links_hide_mixed_scope_edges -- --exact
cargo test --test mcp_stdio search_and_get_memory_hide_mixed_scope_claim_revision_edges_over_stdio -- --exact
cargo test --test mcp_stdio search_memory_returns_scoped_claims_with_revision_provenance_over_stdio -- --exact
./scripts/test-tier.sh core
```

这组回归验证：Claim search/get 遇到任一端越 scope 的 revision edge 时整边隐藏，不保留 Reflection ID、另一端 Claim ID、计数或存在性标志；source 与 superseded-by 两个方向、`search_memory` 与 `get_memory` 都覆盖。同 scope revision edge 仍可见。该切片复用现有表，不证明完整 revision graph。

### 6.3I M1.0.3 owner/namespace write-read reachability

```zsh
cargo test --test domain_invariants unknown_owner_is_not_accepted_for_new_writes -- --exact
cargo test --test domain_invariants canonical_namespace_pairs_derive_one_write_owner -- --exact
cargo test --test evidence_query_dto event_and_claim_dtos_reject_unknown_owner_for_new_writes -- --exact
cargo test --test sqlite_store sqlite_canonical_owner_namespace_writes_are_reachable_through_scoped_reads -- --exact
cargo test --test sqlite_lifecycle read_only_inspection_inventories_unknown_owner_rows_without_rewriting -- --exact
./scripts/test-tier.sh core
```

这组回归验证：新写入只接受 namespace-derived owner；MCP DTO 与 Claim validate 拒绝 `Owner::Unknown`；canonical self/world/user/project 写后可通过同一 scope 读回。只读 doctor inventory 统计 legacy Unknown 行，`rewrite_performed = false`，原行不被改写。scoped 查询不得用 `OR owner = unknown` 扩大结果。

### 6.3J M1.1.4 scoped Reflection provenance read

```zsh
cargo test --test sqlite_store sqlite_reflection_recall_is_scoped_claim_attributed_and_hides_record_only_rows -- --exact
cargo test --test evidence_query_dto search_memory_dto_adds_reflection_without_widening_get_memory_record_types -- --exact
cargo test --test evidence_query_dto search_memory_dto_validates_exact_reflection_filters_and_type_compatibility -- --exact
cargo test --test mcp_stdio search_memory_returns_scoped_reflection_provenance_and_hides_record_only_over_stdio -- --exact
cargo test --test mcp_stdio server_preserves_tool_input_schemas_over_stdio -- --exact
./scripts/test-tier.sh core
```

这组回归验证：`record_type = Reflection` 只在 `search_memory` 中开放；scope 只能从同 scope superseded Claim 派生，replacement 必须同 scope 或为空。record-only Reflection 不能推断 namespace；mixed-scope edge 整条隐藏；exact missing/cross-scope/record-only reference 返回空。`get_memory` 仍拒绝 Reflection。该切片没有 schema migration / index，也不证明 record-only history、Reflection lookup 或完整 M1。

### 6.3K M1.1.5 scoped evidence-relation runtime read

```zsh
cargo test --test sqlite_store sqlite_evidence_relation_runtime_is_scoped_intersect_only_and_hides_cross_scope_ids -- --exact
cargo test --test evidence_query_dto get_evidence_relation_dto_parses_mixed_references_and_defaults_selection -- --exact
cargo test --test evidence_query_dto get_evidence_relation_dto_rejects_invalid_scope_ids_and_limits -- --exact
cargo test --test mcp_stdio get_evidence_relation_returns_scoped_window_and_hides_cross_scope_ids_over_stdio -- --exact
cargo test --test mcp_stdio server_exposes_expected_tools_over_stdio -- --exact
cargo test --test mcp_stdio server_preserves_tool_input_schemas_over_stdio -- --exact
./scripts/test-tier.sh core
```

这组回归验证：第 8 个 MCP 工具 `get_evidence_relation` 要求显式 namespace 与 `trigger_window_event_ids`；裸 ID 与 `event:<id>` 保序去重后，SQLite 只保留同 owner+namespace Event。missing / cross-scope trigger ID 从窗口省略；selected 越 scoped window 时 fail closed。结果返回 canonical `event:<id>`、window_rank、selected / available-not-selected、binary weight 与 rejection reason。路径只读、provider-free，operation metadata 不保存原始 ID 列表。该切片没有 schema migration / index，也不证明 ranking、stable union 或完整 M1。

### 6.3L M1.1.6 stable cross-type record union

```zsh
cargo test --test sqlite_store sqlite_search_memory_union_is_scoped_stable_sorted_and_hides_cross_scope_types -- --exact
cargo test --test evidence_query_dto search_memory_dto_adds_union_record_types_without_widening_get_memory -- --exact
cargo test --test evidence_query_dto search_memory_dto_validates_union_filters_and_type_compatibility -- --exact
cargo test --test mcp_stdio search_memory_union_returns_scoped_mixed_records_and_preserves_event_default_over_stdio -- --exact
cargo test --test mcp_stdio server_preserves_tool_input_schemas_over_stdio -- --exact
./scripts/test-tier.sh core
```

这组回归验证：additive `record_types` 可请求 Event / Claim / Episode / Reflection 的 scoped union；省略 `record_type` / `record_types` 仍为 Event。两者同时出现、空数组、重复类型或类型专属 filter fail closed。单类型路径保持原 SQL 顺序与 tagged JSON。union 按 `recorded_at DESC`（Claim 无 timestamp 在后）、type rank、id DESC 收口并截断，跨 scope 记录不进入结果。该切片没有 schema migration / index，也不证明 lookup/history/correction 或完整 M1。

### 6.3M M1.2.4 scoped Episode lookup

```zsh
cargo test --test evidence_query_dto search_memory_dto_adds_episode_without_widening_get_memory_record_types -- --exact
cargo test --test evidence_query_dto get_memory_dto_rejects_invalid_episode_references -- --exact
cargo test --test mcp_stdio search_memory_returns_scoped_episode_provenance_over_stdio_without_semantic_writes -- --exact
cargo test --test mcp_stdio server_preserves_tool_input_schemas_over_stdio -- --exact
./scripts/test-tier.sh core
```

这组回归验证：显式 `get_memory(record_type = Episode)` 把 `id` 当作 opaque exact persisted Episode reference，并以 `limit = 1` 复用同一 scoped search。missing、大小写不同或跨 scope ID 返回 `record: null`；省略类型仍按 Event 解析。空白或首尾空白 ID fail closed。该切片没有 schema migration / index，也不证明完整 M1。

### 6.3N M1.2.5 scoped Reflection lookup

```zsh
cargo test --test evidence_query_dto search_memory_dto_adds_reflection_without_widening_get_memory_record_types -- --exact
cargo test --test evidence_query_dto get_memory_dto_rejects_invalid_reflection_references -- --exact
cargo test --test mcp_stdio search_memory_returns_scoped_reflection_provenance_and_hides_record_only_over_stdio -- --exact
cargo test --test mcp_stdio server_preserves_tool_input_schemas_over_stdio -- --exact
./scripts/test-tier.sh core
```

这组回归验证：显式 `get_memory(record_type = Reflection)` 把 `id` 当作 opaque exact persisted reflection ID，并以 `limit = 1` 复用同一 scoped search。missing、cross-scope 与 record-only 都返回 `record: null`，三者不可区分。record-only 行没有 Claim anchor，因此不能进入 scoped lookup 或 history。该切片没有 schema migration / index，也不证明 versioned identity/commitment ledger 或完整 M1。

### 6.3O M1.2.6 scoped identity/commitment revision audit

```zsh
cargo test --test sqlite_store sqlite_self_model_history_is_scoped_claim_attributed_and_hides_record_only_rows -- --exact
cargo test --test evidence_query_dto get_self_model_history_dto_requires_history_type_and_defaults_limit -- --exact
cargo test --test evidence_query_dto get_self_model_history_dto_rejects_invalid_scope_and_limits -- --exact
cargo test --test mcp_stdio get_self_model_history_returns_scoped_identity_and_commitment_audits_over_stdio -- --exact
cargo test --test mcp_stdio server_exposes_expected_tools_over_stdio -- --exact
cargo test --test mcp_stdio server_preserves_tool_input_schemas_over_stdio -- --exact
./scripts/test-tier.sh core
```

这组回归验证：第 9 个 MCP 工具 `get_self_model_history(namespace, history_type, limit?)` 要求显式 namespace 与 `Identity` / `Commitment`；limit 默认 20、范围 `1..=100`，并用 `has_more` 表示截断。历史只读取现有 reflection 审计列，归属规则与 Reflection search 相同：必须存在同 scope superseded Claim，replacement 为空或同 scope。record-only 更新与 mixed-scope edge 保持不可见。路径只读、provider-free，operation metadata 仅含 `history_type`、`result_count`、`has_more`。该切片没有 schema migration / 新写路径 / rollback，也不把现态 `identity_claims` / `commitments` 表变成版本账本。

### 6.3P M1.2.7 scoped audited supersede

```zsh
cargo test --test sqlite_store sqlite_supersede_memory_is_scoped_claim_correction_and_hides_cross_scope_targets -- --exact
cargo test --test evidence_query_dto supersede_memory_dto_parses_canonical_claim_and_event_references -- --exact
cargo test --test evidence_query_dto supersede_memory_dto_rejects_invalid_scope_target_and_empty_evidence -- --exact
cargo test --test mcp_stdio supersede_memory_replaces_scoped_claim_without_hard_delete_over_stdio -- --exact
cargo test --test mcp_stdio server_exposes_expected_tools_over_stdio -- --exact
cargo test --test mcp_stdio server_preserves_tool_input_schemas_over_stdio -- --exact
./scripts/test-tier.sh core
```

这组回归验证：第 10 个 MCP 工具 `supersede_memory` 要求显式 namespace、Claim 与至少一条 evidence；replacement 必须留在同一 namespace。SQLite 先确认 target 与 evidence 都属于请求 owner+namespace，再调用既有 `run_reflection` 事务。默认 search 只返回新 Active Claim；旧 Claim 仍为 `Superseded` 且 history 可回看。missing / cross-scope 输入 fail closed。operation metadata 只保存 `correction_type` 与 `durable_write_path = run_reflection`。该切片没有 schema migration / 第二条 write path / hard delete，也不证明 Event/Episode/Reflection 纠错或完整 M1。

### 6.4 Provider 合规预检

新增 provider 前先阅读 [Provider Readiness Checklist](provider-contract.md)。下面这组命令只是当前共享 provider 路径的最小验证；如果 checklist 里仍有 `partial` 或 `gap` 且新 provider 依赖该行为，新增 provider 的同一变更必须补齐对应专用回归或记录明确例外。

```zsh
cargo test --test provider_config -v
cargo test --test openai_compatible_model -v
cargo test --test mcp_stdio decide_with_snapshot_over_stdio_uses_openai_compatible_provider_from_config_file -v
cargo test --test mcp_stdio decide_with_snapshot_over_stdio_uses_openrouter_provider_from_config_file -v
cargo test --test mcp_stdio ingest_interaction_auto_reflection_uses_openrouter_provider_from_config_file -v
cargo test --test support_bundle support_bundle_reports_openrouter_config_shape_without_provider_secrets -v
```

重点覆盖：

- config validation behavior
- `doctor` redaction behavior
- timeout handling
- non-success HTTP status behavior
- malformed JSON behavior
- decision action parsing
- self-revision proposal parsing
- evidence policy parsing
- config-selected provider 是否真实走到 MCP `stdio` provider path

当前覆盖映射：

- `tests/provider_config.rs`
  - `default_config_uses_mock_provider_when_no_config_file_is_present`
  - `load_from_path_reads_openai_compatible_provider_from_toml_file`
  - `load_from_path_reads_openrouter_provider_from_toml_file`
  - `load_prefers_config_path_from_environment`
  - `load_prefers_database_url_env_over_default_config_file`
  - `openrouter_example_config_parses_without_live_looking_secret`
  - `doctor_fails_when_openai_provider_config_is_missing_api_key`
  - `doctor_reports_openrouter_provider_without_exposing_api_key`
  - `doctor_fails_when_openrouter_provider_config_is_missing_model`
  - `doctor_report_does_not_contain_api_key_in_serialized_output`
- `tests/openai_compatible_model.rs`
  - `openai_compatible_model_parses_first_assistant_message_into_action`
  - `openai_compatible_model_rejects_empty_action`
  - `openai_compatible_model_surfaces_non_success_status`
  - `openai_compatible_model_parses_self_revision_proposal_from_assistant_message`
  - `openai_compatible_model_defaults_missing_machine_patch_in_self_revision_proposal`
  - `openai_compatible_model_accepts_fenced_json_self_revision_proposal`
  - `openai_compatible_model_parses_self_revision_evidence_policy`
  - `openai_compatible_model_fails_gracefully_on_malformed_json_response`
  - `openai_compatible_model_surfaces_timeout_as_error`
- `tests/evidence_query_dto.rs`
  - `evidence_query_dto_parses_recency_window_fields`
  - `evidence_query_dto_rejects_invalid_recency_timestamp`
  - `evidence_query_dto_rejects_zero_limit`
- `tests/operation_log.rs`
  - `operation_log_redacts_summary_json_before_persisting`
  - `operation_log_queries_by_correlation_id`
- `tests/mcp_stdio.rs`
  - `decide_with_snapshot_over_stdio_uses_openai_compatible_provider_from_config_file`
  - `decide_with_snapshot_over_stdio_uses_openrouter_provider_from_config_file`
  - `ingest_interaction_auto_reflection_uses_openrouter_provider_from_config_file`

未来新增 provider 前的阻断缺口：

- self-revision proposal 的 malformed JSON 仍只通过 proposal 解析路径间接覆盖；新增 provider 前需要按 provider contract 补齐更明确的 self-revision proposal 错误断言

### 6.5 领域不变量

```zsh
cargo test --test domain_invariants --test domain_snapshot
```

重点覆盖：

- inferred claim 的证据门槛
- `identity_core` 不能被普通 ingest 直接改写
- namespace 默认派生和 owner 匹配
- snapshot evidence 预算与 gate 行为

### 6.6 应用层编排

```zsh
cargo test --test application_use_cases --test failure_modes
```

重点覆盖：

- ingest 事务顺序
- reflection 状态流转
- inferred replacement 在有显式 evidence 时可通过
- query-based evidence 会被去重、限流并做上限校验
- replacement claim 的 evidence link 会写入
- deep reflection 会更新 `identity_core` / `commitments` 并写入审计字段
- snapshot 组装
- failure mode 回归

特别关注这些测试名：

- `reflection_rejects_inferred_replacement_without_external_evidence`
- `reflection_accepts_inferred_replacement_with_explicit_evidence`
- `reflection_can_update_identity_and_commitments_with_audited_supporting_evidence`
- `reflection_preserves_baseline_commitment_when_updates_replace_commitments`
- `reflection_rejects_identity_update_without_supporting_evidence`
- `reflection_rejects_identity_update_when_evidence_query_resolves_empty`
- `reflection_without_replacement_claim_disputes_old_claim_and_updates_identity`
- `reflection_rejects_missing_replacement_evidence_event_ids`
- `reflection_rejects_empty_identity_update_even_with_supporting_evidence`

### 6.7 automatic self-revision runtime coverage

```zsh
cargo test --test mcp_stdio ingest_interaction_can_trigger_conflict_auto_reflection_when_explicit_conflict_hints_present -v
cargo test --test mcp_stdio ingest_interaction_does_not_auto_reflect_conflict_without_explicit_conflict_hints -v
cargo test --test mcp_stdio ingest_interaction_does_not_auto_reflect_conflict_with_non_conflict_trigger_hints -v
cargo test --test mcp_stdio ingest_interaction_returns_success_even_when_conflict_auto_reflection_fails -v
cargo test --test mcp_stdio decide_with_snapshot_can_trigger_conflict_auto_reflection_without_breaking_decision_flow -v
cargo test --test mcp_stdio blocked_decide_with_snapshot_does_not_auto_reflect_conflict_hints -v
cargo test --test mcp_stdio build_self_snapshot_can_trigger_periodic_auto_reflection_once_for_explicit_namespace -v
cargo test --test mcp_stdio build_self_snapshot_returns_snapshot_when_best_effort_periodic_auto_reflection_fails -v
cargo test --test mcp_stdio ingest_interaction_auto_reflects_once_and_does_not_recurse_inside_run_reflection -v
```

重点覆盖：

- 当前 MCP-wired automatic path 是否仍然准确限定为：
  - `ingest_interaction -> failure`
  - `ingest_interaction -> conflict`
  - `decide_with_snapshot -> conflict`
  - `build_self_snapshot -> periodic`
- `ingest_interaction -> conflict` 是否仍要求显式 `trigger_hints` 包含 `conflict` 或 `identity`
- `decide_with_snapshot` 的 conflict auto-reflection 是否仍要求显式 conflict-compatible `trigger_hints`，且只在非 blocked 决策后运行
- `build_self_snapshot` 的 periodic auto-reflection 是否仍要求显式 `auto_reflect_namespace`
- best-effort auto-reflection 失败是否不会把主 MCP 成功路径改写成 MCP 错误
- `run_reflection` 是否仍是唯一 durable write path / persistence funnel

运行时 contract matrix 的权威定义见 `docs/project-status.md` §8 的「runtime hook contract matrix」表。本指南不再复制该表，只核对其语义不变。

实现细节核对：

- `ingest_interaction:failure` 当前对应 `failure` 或 `rollback` trigger hints，加上 failure evidence threshold。
- `build_self_snapshot:periodic` 属于 snapshot tool flow，但 best-effort reflection attempt 发生在 `build_self_snapshot::execute` 之前；它不是后台 scheduler。

### 6.8 automatic self-revision diagnostics

```zsh
cargo test --test failure_modes auto_reflection_returns_structured_diagnostics_for_recursion_guard_skip -v
cargo test --test failure_modes auto_reflection_returns_structured_diagnostics_for_not_triggered_case -v
cargo test --test failure_modes auto_reflection_returns_structured_diagnostics_for_rejected_proposal -v
cargo test --test failure_modes auto_reflection_returns_structured_diagnostics_for_suppressed_trigger -v
cargo test --test failure_modes auto_reflection_applies_model_proposed_evidence_subset_but_preserves_full_trigger_window_in_handled_ledger -v
cargo test --test failure_modes auto_reflection_repeated_suppression_does_not_extend_existing_cooldown -v
cargo test --test bootstrap doctor_reports_self_revision_runtime_coverage -v
```

重点覆盖：

- structured diagnostics 是否返回可直接检查的 summary contract：
  - `trigger_type`: `failure` / `conflict` / `periodic`
  - `namespace`
  - `trigger_key`
  - `outcome`: `handled` / `rejected` / `suppressed` / `not_triggered` / `skipped`
  - `suppression_reason`
  - `rejection_reason`
  - `cooldown_boundary`
  - `cooldown_state`: `none` / `set` / `active`
  - `evidence_window_size`
  - `selected_evidence_event_ids`
  - `durable_write_path = run_reflection`
- suppressed cooldown 是否保持已有窗口而不是在重复 suppression 时被无界延长
- `doctor` 输出是否保守暴露 runtime hook coverage 与 `self_revision_write_path`
- `doctor` 输出的 runtime hook list 是否仍然精确等于上面的 4 条 contract matrix，且 `self_revision_write_path = run_reflection`
- `doctor` 输出 runtime hooks 不应被解读成新增 MCP tool、后台 daemon 或“所有请求自动反思”

判读要点：

- `rejected` 表示触发器已经命中并进入 proposal 路径，但模型未给出可接受 proposal；此时应检查 `rejection_reason`，而不是 `suppression_reason`。治理校验失败会记录 rejected ledger 并以错误返回，不作为成功返回的 diagnostics summary。
- `suppressed` 表示这次触发被已有 ledger 状态压住，例如 `cooldown_active`、`evidence_window_unchanged` 或 `episode_watermark_unchanged`；此时应检查 `suppression_reason` 与 `cooldown_boundary`。
- `not_triggered` 和 `skipped` 仍然是前台、按调用发生的诊断结果：它们说明“本次调用未进入 durable write”，不表示系统存在后台自治流程。
- `selected_evidence_event_ids` 只表示已进入 handled durable write 的实际证据子集；`rejected`、`suppressed`、`not_triggered`、`skipped` 没有 durable write selection，应通过 `evidence_window_size` 读取本次触发窗口规模。
- `durable_write_path = run_reflection` 只是说明一旦进入 durable write，唯一允许的落盘路径仍是 `run_reflection`；它不代表新增 MCP tool、后台 daemon、额外 hook 或独立 self-revision worker。

### 6.9 self-revision evidence policy

```zsh
cargo test --test failure_modes auto_reflection_rejects_model_proposed_evidence_outside_trigger_window -v
cargo test --test failure_modes auto_reflection_applies_model_proposed_evidence_subset_but_preserves_full_trigger_window_in_handled_ledger -v
cargo test --test failure_modes auto_reflection_intersects_proposed_evidence_query_with_current_trigger_window_when_ids_are_empty -v
cargo test --test failure_modes auto_reflection_applies_query_limit_within_current_trigger_window_when_ids_are_empty -v
cargo test --test failure_modes auto_reflection_rejects_model_proposed_evidence_ids_that_do_not_match_query_policy -v
cargo test --test failure_modes auto_reflection_rejects_empty_proposed_evidence_query_instead_of_widening -v
cargo test --test failure_modes auto_reflection_scopes_trigger_window_to_input_namespace -v
cargo test --test failure_modes auto_reflection_rejects_namespace_filter_with_no_trigger_window_intersection -v
cargo test --test failure_modes auto_reflection_rejects_noop_proposal_when_query_has_no_trigger_window_intersection -v
cargo test --test openai_compatible_model openai_compatible_model_parses_self_revision_evidence_policy -v
```

重点覆盖：

- proposal 首阶段 evidence contract 是否包含 `proposed_evidence_event_ids`、`proposed_evidence_query` 与 `confidence`
- model 提议的 evidence id 是否仍必须落在当前 trigger window 内
- project / user scoped conflict 和 periodic trigger window 是否在 proposal narrowing 前排除 sibling namespace 的事件
- 当 model 同时提供 explicit ids 和 `proposed_evidence_query` 时，这些 ids 是否仍必须满足 query 在当前 trigger window 内的过滤约束
- handled ledger 是否保留完整 evidence window，而不是只保留 model 选择的子集
- `proposed_evidence_query` 在 explicit ids 为空时是否只会对当前 trigger window 做交集收口，并在有交集时只按当前窗口内候选应用 `limit`
- `proposed_evidence_query` 的 `recorded_after` / `recorded_before` 是否按 inclusive recency window 参与 trigger-window 内过滤
- `proposed_evidence_query` 在 explicit ids 为空且 query 无交集时是否拒绝处理，而不是绕过 query 改用 full trigger window
- record-only / no-op proposal 是否同样不能绕过 no-match query rejection
- `proposed_evidence_query` 当前是否仍不会在 id 为空时自动 widening / ranking

### 6.10 automatic self-revision MVP 定向验证

这是当前 self-revision MVP 的最低定向回归集。只要你改了下面任一部分，就至少补跑这 7 条：

- `src/application/auto_reflect_if_needed.rs`
- `src/interfaces/mcp/server.rs`
- `src/interfaces/mcp/dto.rs`
- `src/adapters/sqlite/store.rs`
- `src/ports/trigger_ledger_store.rs`
- `src/support/config.rs` 里与启动/数据库加载语义相关的代码

命令：

```zsh
cargo test --test application_use_cases auto_reflection_runs_once_for_repeated_failure_and_records_handled_ledger -v
cargo test --test sqlite_store sqlite_trigger_ledger_records_namespace_periodic_watermark_and_cooldown -v
cargo test --test mcp_stdio ingest_interaction_auto_reflects_once_and_does_not_recurse_inside_run_reflection -v
cargo test --test mcp_stdio ingest_interaction_can_trigger_conflict_auto_reflection_when_explicit_conflict_hints_present -v
cargo test --test mcp_stdio ingest_interaction_does_not_auto_reflect_conflict_without_explicit_conflict_hints -v
cargo test --test mcp_stdio decide_with_snapshot_can_trigger_conflict_auto_reflection_without_breaking_decision_flow -v
cargo test --test mcp_stdio build_self_snapshot_can_trigger_periodic_auto_reflection_once_for_explicit_namespace -v
```

如果改动包含 `src/support/config.rs`，再追加：

```zsh
cargo test --test provider_config -v
```

覆盖点：

- 应用层会在重复 failure 窗口里只自动修订一次，并把 handled ledger 正确落盘
- SQLite adapter 会持久化 trigger ledger 的 `namespace`、`episode_watermark` 和 `cooldown_until`
- stdio runtime 的 4 条当前 MCP-wired automatic path 都会被最低回归集直接覆盖：
  - `ingest_interaction -> failure`
  - `ingest_interaction -> conflict`
  - `decide_with_snapshot -> conflict`
  - `build_self_snapshot -> periodic`
- direct `run_reflection` 不会递归回自动链路

额外注意：

- `decide_with_snapshot` / `build_self_snapshot` 仍要求显式 `auto_reflect_namespace`，`decide_with_snapshot` 还要求显式 conflict-compatible `trigger_hints`，并且只在非 blocked 决策后才会 best-effort 触发
- 不要把这组测试解读成“所有 MCP 入口都会自动反思”
- 当前 auto-reflection 仍通过已有 `run_reflection` 写入 identity / commitments，不存在新的 durable write 通道

### 6.11 self-revision demo package

如果改动涉及下面任一部分，需要补跑 demo package 定向验证：

- `src/bin/demo_openai_compatible_stub.rs`
- `src/bin/run_self_revision_demo.rs`
- `scripts/run-self-revision-demo.sh`

runner 必须按时间顺序在 reflection 前生成 `decision-before.json`、在 reflection 后生成 `decision-after.json`。`decide_with_snapshot` 会在每次调用时读取服务端 commitment store，因此不得依靠修订后回放旧 snapshot commitments 来证明 decision shift。
- `examples/agent-llm-mm.demo.example.toml`
- automatic self-revision runtime hook / provider / MCP `stdio` 相关代码

推荐命令：

```zsh
cargo test --test demo_openai_compatible_stub --test self_revision_demo_runner --test openai_compatible_model --test mcp_stdio -v
./scripts/run-self-revision-demo.sh target/reports/self-revision-demo/manual-$(date +%Y%m%d-%H%M%S)
```

通过后，指定 output dir 下至少应有；按 Local Alpha 发布口径复核 `latest` 时，改用 `./scripts/product-smoke-local.sh`，不要直接让 demo wrapper 写入 `latest`：

- `doctor.json`
- `snapshot-before.json`
- `snapshot-after.json`
- `decision-before.json`
- `decision-after.json`
- `timeline.json`
- `sqlite-summary.json`
- `report.md`

重点确认：

- negative conflict 场景不会增加 handled conflict ledger
- positive conflict 场景会增加 handled conflict ledger
- after snapshot 会出现 revised commitment
- before / after decision action 会发生变化
- `doctor.json` 仍声明 durable write path 是 `run_reflection`

---

### 6.12 Local Alpha product smoke script

如果改动涉及 Local Alpha 发布 gate、本地启动包装、`doctor` 配置传递，或 self-revision demo 证据链的产品化入口，需要补跑 product smoke：

```zsh
./scripts/product-smoke-local.sh
```

如果要同时验证某个本地配置文件的 bootstrap / doctor 路径，可以传入可选 config path：

```zsh
./scripts/product-smoke-local.sh agent-llm-mm.local.toml
```

这里的 `agent-llm-mm.local.toml` 是本地用户配置占位路径，文件必须已存在；这个 smoke 只验证 `doctor` 的 config path 解析与传递，不会把 config path 传给 deterministic demo wrapper。

以上 repo-relative 示例要求当前目录是 repo root。如果从其他当前目录运行，使用脚本绝对路径：

```zsh
cd /tmp && /path/to/agent-llm-mm/scripts/product-smoke-local.sh
```

如果从其他当前目录运行且要传入配置文件，config path 也使用绝对路径：

```zsh
cd /tmp && /path/to/agent-llm-mm/scripts/product-smoke-local.sh /path/to/agent-llm-mm/agent-llm-mm.local.toml
```

通过标准：

- 脚本可从 repo root 或其他当前目录调用，并能定位 repo root
- 可选 config path 必须存在；脚本会解析成绝对路径后只传给 `./scripts/agent-llm-mm.sh doctor`
- `doctor` 退出码为 `0`
- 脚本会先把 demo wrapper 输出到 staging 目录，8 个 artifact 均通过后才替换 `target/reports/self-revision-demo/latest`
- `target/reports/self-revision-demo/latest` 下 8 个 required demo artifacts 均存在且非空

限制说明：`scripts/run-self-revision-demo.sh` 当前只接受第 1 个参数作为 output dir，不接受 config path。因此 product smoke 的 config path 只覆盖 `doctor`，self-revision demo 仍使用现有 deterministic demo 契约。这条 product smoke 是 Local Alpha gate 的产品化入口，不替代 demo / MVP 的 [Release Gate](release-gate.md)，也不表示 GA / production-ready。

---

### 6.13 Local first-run bootstrap smoke

如果改动涉及 `bootstrap-local`、本地配置引导、`doctor` 配置路径、默认数据库环境变量隔离，或 Local Alpha first-run 证据，需要补跑：

```zsh
bash -n scripts/first-run-bootstrap-smoke-local.sh
cargo test --test first_run_bootstrap_smoke -v
./scripts/first-run-bootstrap-smoke-local.sh target/first-run-bootstrap-smoke/manual-check
```

通过标准：

- 输出目录不存在或为空时脚本成功；非空目录会在写任何 artifact 前拒绝
- 脚本生成 `agent-llm-mm.local.toml`、`doctor.json`、`summary.json` 和同目录下的 `first-run.sqlite`
- `doctor.json.status = "ok"`、`provider = "mock"`、`self_revision_write_path = "run_reflection"`
- `doctor.json.daemon_enabled = false`，且 `daemon_observe_only.writes_allowed = false`
- `summary.json.fresh_machine_simulation = true`，`real_fresh_machine_evidence = false`
- 脚本会清理 `AGENT_LLM_MM_CONFIG` / `AGENT_LLM_MM_DATABASE_URL` 干扰，不写真实 HOME，不启动 `serve`，不调用 `product-smoke-local.sh` 或 demo wrapper，不调用远端命令

这条 smoke 只证明当前 checkout 内的本地首启模拟链路，不替代 product smoke、真实 fresh-machine install、Windows runner parity、installer、packager、remote bootstrapper 或 GA 证据。

---

### 6.14 Provider live preflight evidence runner

如果改动涉及 provider certification preflight、live evidence schema、provider URL / credential redaction，或显式 provider evidence runner，需要补跑：

```zsh
bash -n scripts/provider-live-certification-run.sh
cargo test --features release-tools --test provider_live_certification -v
cargo test --features release-tools --test non_mvp_product_tracks provider_certification -v
```

通过标准：

- 配置示例本身不是 live evidence；需要 live preflight evidence 时必须显式运行 runner
- 显式传入 `--live` 时，runner 只可生成 bounded live evidence files，不可输出 provider-native payload 或声明 provider 质量、SLA、provider gateway、Local Alpha、Beta、GA、production-ready、production readiness 或 release approval
- 省略或同时传入 `--live` / `--stub-evidence` 必须被拒绝，不能伪装成通过的 live check，也不能把第二个 mode flag 当作 config path
- `--stub-evidence` 只生成 stub/simulated evidence，文件标记 `local_only = true`，且 preflight 仍保持 `live_certified = false`
- live evidence 只有同时满足 provider、`status = passed`、expected `evidence_kind`、`mode = live`、非空 `generated_at`、`local_only = false`、`endpoint_reached = true`、`redaction_reviewed = true`、`request_outcome = passed`，以及 `name = provider-live-certification`、`command = scripts/provider-live-certification-run.sh --live` 或 `command = ./scripts/provider-live-certification-run.sh --live`、显式 `exit_code = 0` 的成功 command evidence 才算 present；即便 live evidence complete，config preflight 失败时 `live_certified` 仍必须 blocked
- 输出不得包含 API key、URL userinfo、URL path 内容、query secret、model id、request body 或 response body

---

### 6.15 Packaging archive evidence

如果改动涉及 packaging preflight、候选 archive 命名、checksum manifest 或 release packaging 证据，需要补跑：

```zsh
bash -n scripts/packaging-archive-evidence.sh
cargo test --features release-tools --test packaging_archive -v
cargo test --features release-tools --test non_mvp_product_tracks packaging_preflight -v
```

通过标准：

- 缺失 archive、零字节 archive、纯文本占位 archive 或截断 archive 必须失败且不能写 manifest
- 成功 manifest 只包含候选名、archive 文件名、size、SHA-256、`local_only = true`、`complete = true` 和 non-claims，不能写绝对路径
- packaging preflight 只有在四个平台 archive 都存在、可解析为对应 `.tar.gz` / `.zip` archive，并与 manifest 的 name / size / SHA-256 匹配时才满足 `binary_archive`
- `installer`、`service_manager`、`auto_updater` 仍保持 `not_implemented`，所以 `packaging_ready` 仍为 false

---

### 6.16 Local release soak runner

如果改动涉及 release engineering、候选发布证据目录、soak evidence、release note 或产品化发布口径，需要补跑本地 release soak：

```zsh
bash -n scripts/release-soak-local.sh
cargo test --features release-tools --test local_alpha_release_evidence release_soak -v
./scripts/release-soak-local.sh local-alpha-YYYYMMDD.1-rc.1
```

如果要验证某个本地配置文件的 `doctor` / product smoke / support bundle 分支，可传入可选 config path：

```zsh
./scripts/release-soak-local.sh local-alpha-YYYYMMDD.1-rc.1 agent-llm-mm.local.toml
```

通过标准：

- candidate name 只能包含字母、数字、点、下划线或短横线，且不能包含 `..`
- evidence directory 写入 `target/reports/releases/<candidate-name>/`，目录必须不存在或为空
- lifecycle database 强制写入 `target/release-soak-runtime/<candidate-name>/release-soak.sqlite`；任何传入 config 的数据库路径都不得成为 soak 写目标
- 目录内包含 `git-head.txt`、`git-status-before.txt`、`git-status-after.txt`、`command-summary.tsv`、`commands/`、`secret-scan.log`、`artifact-scan.log`、`support-bundle-files.txt`、`support-bundle-sha256.txt`、`product-smoke-latest-files.txt`、`product-smoke-latest-sha256.txt`、`local-alpha-evidence-summary.json`、`local-alpha-evidence-summary.md`、`compatibility-matrix.json`、`release-boundaries.json` 和 `release-soak-summary.md`
- 运行顺序覆盖 explicit `init`、`doctor --read-only`、dashboard HTTP、product smoke、first-run simulation、support bundle、scans 和 Local Alpha evidence summary
- support bundle secret scan 不应发现未脱敏 secret-like marker；raw artifact scan 不应发现 `.sqlite`、`.toml` 或 `.log`

这条 soak 只生成本地候选证据；它不生成真实 fresh-machine evidence、Windows runner evidence、remote/team evidence、上传、source tag、binary package、installer、service manager、auto-updater、release decision 或 GA / production-ready 证明。

---

## 7. 手工 Smoke Test

`MCP` `stdio` 是 JSON-RPC 交互协议，手工敲消息成本较高。当前项目更推荐直接运行自动化 E2E 测试，而不是纯手工交互。

如果你仍然想做一次最小人工验证，推荐下面的方式。

### 7.1 使用独立数据库启动服务

```zsh
cd ~/code/agent-llm-mm
cp examples/agent-llm-mm.example.toml agent-llm-mm.local.toml
./scripts/agent-llm-mm.sh serve
```

这会启动 MCP `stdio` 服务。由于它等待 JSON-RPC 消息，终端表面上会“挂住”，这是正常现象。

### 7.2 更实用的人工验证方式

另开一个终端，直接跑现有 E2E：

```zsh
cd ~/code/agent-llm-mm
cargo test --test mcp_stdio -- --nocapture
```

原因：

- 这条测试已经覆盖真实二进制
- 使用真实 `stdio`
- 覆盖 `initialize / tools/list / tools/call` 全链路
- 比手工拼 JSON-RPC 更稳定

### 7.3 手工验证 openai-compatible provider

如果你要专门确认 provider 路径已经不是 `mock`，推荐跑：

```powershell
cargo test --test openai_compatible_model -- --nocapture
cargo test --test mcp_stdio decide_with_snapshot_over_stdio_uses_openai_compatible_provider_from_config_file -- --nocapture
cargo test --test mcp_stdio decide_with_snapshot_over_stdio_uses_openrouter_provider_from_config_file -- --nocapture
cargo test --test mcp_stdio ingest_interaction_auto_reflection_uses_openrouter_provider_from_config_file -- --nocapture
```

新增 provider 的验收应先按 [Provider Readiness Checklist](provider-contract.md) 补齐 `partial` / `gap` 对应的专用回归或记录明确例外，再执行上述手工验证。

### 7.4 手工验证 evidence-aware reflection

如果你要专门手测 reflection 的显式证据行为，推荐先跑自动化：

```powershell
cargo test --test application_use_cases reflection_accepts_inferred_replacement_with_explicit_evidence
cargo test --test mcp_stdio inferred_replacement_reflection_with_evidence_is_accepted_over_stdio
```

如果必须走手工 `stdio` 路径，`run_reflection` 的关键入参如下：

```json
{
  "reflection": {
    "summary": "Two external observations support promoting the inferred replacement."
  },
  "supersede_claim_id": "<event_id>:claim:0",
  "replacement_claim": {
    "owner": "Self_",
    "subject": "self.role",
    "predicate": "is",
    "object": "principal_architect",
    "mode": "Inferred"
  },
  "replacement_evidence_event_ids": [
    "evt-reflection-1",
    "evt-reflection-2"
  ]
}
```

预期：

- `replacement_evidence_event_ids` 中的每个 ID 都必须对应一条已持久化的 `events` 记录
- 返回 `replacement_claim_id`
- 不是 `invalid_params`
- 后续 snapshot 中 active claim 应变为 replacement 对应的新命题

如果你要验证最小 deep reflection 更新，可在上述基础上再加：

```json
{
  "identity_update": {
    "canonical_claims": [
      "identity:self=staff_architect",
      "identity:style=evidence_first"
    ]
  },
  "commitment_updates": [
    {
      "owner": "Self_",
      "description": "prefer:evidence_backed_identity_updates"
    },
    {
      "owner": "Self_",
      "description": "forbid:write_identity_core_directly"
    }
  ]
}
```

额外预期：

- 后续 `build_self_snapshot` 返回的新 `identity` 与 `commitments` 已更新
- `reflections` 表会保留 supporting evidence 与请求更新内容的 JSON 审计字段

### 7.5 手工验证 automatic self-revision runtime hooks

如果你要专门观察 automatic self-revision MVP，而不是只看最终 snapshot，优先跑自动化：

```zsh
cargo test --test application_use_cases auto_reflection_runs_once_for_repeated_failure_and_records_handled_ledger -v
cargo test --test mcp_stdio ingest_interaction_auto_reflects_once_and_does_not_recurse_inside_run_reflection -v
cargo test --test mcp_stdio decide_with_snapshot_can_trigger_conflict_auto_reflection_without_breaking_decision_flow -v
cargo test --test mcp_stdio build_self_snapshot_can_trigger_periodic_auto_reflection_once_for_explicit_namespace -v
```

当前你应期待的是：

- 第二次重复 failure 触发会因为 ledger cooldown 被 suppress
- 已成功的 `ingest_interaction` 不会因为 post-ingest auto-reflection 失败而变成 MCP error
- 已成功的 `decide_with_snapshot` / `build_self_snapshot` 也不应因为 best-effort auto-reflection 失败而变成 MCP error
- direct `run_reflection` 只执行显式请求，不会再触发一轮自动修订
- 这组验证只覆盖当前 4 条已接线 hook；不代表所有 MCP entry point 都会自动反思

### 7.6 手工验证 self-revision demo package

如果你想看一套可读 report，而不是逐条跑 MCP `stdio` 测试：

```zsh
./scripts/run-self-revision-demo.sh target/reports/self-revision-demo/manual-$(date +%Y%m%d-%H%M%S)
```

然后打开：

```text
target/reports/self-revision-demo/manual-<timestamp>/report.md
```

这条路径使用本地 deterministic provider，不需要真实 API key，也不会访问外网。
Local Alpha 发布证据的 `latest` 目录由 `./scripts/product-smoke-local.sh` 负责更新，不要在手工 demo 中直接覆盖它。

### 7.7 手工验证 Local Alpha product smoke

如果你想从 Local Alpha gate 的产品化入口复核本机 `doctor` 和 self-revision demo 证据链：

```zsh
./scripts/product-smoke-local.sh
```

带本地配置文件时：

```zsh
./scripts/product-smoke-local.sh agent-llm-mm.local.toml
```

以上 repo-relative 示例要求当前目录是 repo root；从其他当前目录运行时，使用 `/path/to/agent-llm-mm/scripts/product-smoke-local.sh`，如果要传入配置文件，也传入绝对 config path。

注意：config path 只传给 `doctor`；deterministic self-revision demo wrapper 仍只接收 output dir。

### 7.8 手工验证 dashboard 面板

如果你要手工查看只读 dashboard：

```zsh
cp examples/agent-llm-mm.example.toml agent-llm-mm.local.toml
./scripts/agent-llm-mm.sh serve
```

然后访问：

```text
http://127.0.0.1:8787/
```

如果改动涉及 dashboard，至少补跑：

```zsh
cargo test --test dashboard_config --test dashboard_recorder --test dashboard_projection --test dashboard_http
cargo test --test mcp_stdio dashboard_enabled_does_not_corrupt_mcp_stdout_and_records_tool_event -v
```

dashboard HTTP 测试会监听本机端口，受限沙箱中可能需要在允许本地监听的环境运行。启用 dashboard 时，`DashboardConfig::validate` 只接受 `localhost` 或 loopback IP，明确拒绝 `0.0.0.0`、LAN IP 与域名；disabled 配置可以保留未启用的 host 值但不会启动监听。该面板只读，不会调用 `run_reflection` 或修改 SQLite；`dashboard_rejects_write_methods_on_read_only_routes` 覆盖 POST / PUT / PATCH / DELETE 返回 `405 Method Not Allowed`，`dashboard_serves_html_summary_events_detail_and_health` 覆盖 HTML、JSON API、health 和 SSE 只读 GET surface。

如果改动涉及 CLI tracing 或 `stdio` 隔离，补跑：

```zsh
cargo test --test bootstrap cli_tracing_writes_to_stderr_without_corrupting_json_stdout -- --exact
cargo test --test mcp_stdio -v
```

command-level tracing 在解析 CLI 后初始化并固定写入 stderr。`doctor` 的 JSON 与
`serve` 的 MCP protocol frames 必须保持在 stdout，任何日志进入 stdout 都是阻断错误。

如果改动涉及 dashboard 视觉或静态物料，还需要确认：

- `GET /` 包含 `Memory-chan Live Desk`
- 静态 HTML visual contract 覆盖动态 ID 省略、指标卡/operation chain 自适应网格、侧栏贴纸 `contain` 显示、移动端顶部状态条重排、hero 文案遮罩和移动端无横向溢出
- `GET /assets/memory-chan-hero.png` 返回 `content-type: image/png`
- `GET /assets/memory-chan-sidebar.png` 返回 `content-type: image/png`
- 生成图物料的仓库归属说明已经同步到 `NOTICE`

---

## 8. 迁移验证

如果你改了 SQLite schema 或 migration，至少跑下面两条：

```powershell
cargo test --test sqlite_store sqlite_store_bootstraps_all_tables
cargo test --test sqlite_store sqlite_bootstrap_backfills_namespace_for_legacy_claim_rows
```

这两条分别验证：

- 新建数据库的 schema 是否正确
- 旧数据库升级后是否完成 namespace 回填和强约束恢复

如果你改了 `claims` 表约束，再加跑：

```powershell
cargo test --test sqlite_store sqlite_database_rejects_corrupt_namespace_owner_pair_before_read
```

---

## 9. 常见问题排查

### 9.1 `cargo fmt --check` 失败

现象：

- 输出 diff

处理：

```powershell
cargo fmt
cargo fmt --check
```

### 9.2 `mcp_stdio` 失败，出现 `UnexpectedEof`

优先怀疑：

- 服务端启动后 panic
- `tools/call` 的 DTO 解析失败
- SQLite bootstrap 或 migration 出错
- provider 配置文件无法解析

排查顺序：

1. 先跑 `cargo test --test mcp_stdio -- --nocapture`
2. 再跑 `cargo test --test sqlite_store`
3. 如果是最近改了 DTO，优先检查 `src/interfaces/mcp/dto.rs`
4. 如果是 provider 路径，优先检查 `agent-llm-mm.local.toml`

### 9.3 SQLite 相关测试失败

优先怀疑：

- schema 与 adapter SQL 不一致
- legacy migration 没把旧表升级到最新约束
- `owner <-> namespace` 规则和数据库 `CHECK` 不一致

优先检查：

- `src/adapters/sqlite/schema.rs`
- `src/adapters/sqlite/store.rs`
- `src/domain/types.rs`
- `src/domain/claim.rs`
- `src/support/config.rs`

### 9.4 数据库路径加载语义和预期不一致

优先检查你是走哪条配置路径：

- `AppConfig::load()`：默认启动路径，会在读取配置文件后继续接受 `AGENT_LLM_MM_DATABASE_URL` 覆盖
- `AppConfig::load_from_path()`：显式文件加载路径，会保留文件里显式给出的 `database_url`

这意味着：

- 如果你通过脚本或默认启动路径运行服务，同时又设置了 `AGENT_LLM_MM_DATABASE_URL`，最终数据库位置可能不是 TOML 文件里写的那个
- 如果你在测试里直接调用 `load_from_path()`，显式文件里的 `database_url` 不会再被环境变量覆盖
- 但如果该文件省略 `database_url`，`load_from_path()` 仍可能通过 `AppConfig::default()` 继承环境变量派生出的默认路径

### 9.5 `invalid_params` 变成 `internal_error`

说明错误映射回退了。

优先检查：

- `src/error.rs`
- `src/interfaces/mcp/server.rs`

预期行为：

- 调用方参数错误返回 `-32602`
- 基础设施或服务端异常才返回 `-32603`

---

## 10. 修改后最低测试门槛

如果你只是改了一处小逻辑，最低建议如下：

### 改 domain / claim / namespace 规则

```powershell
cargo test --test domain_invariants --test domain_snapshot --test application_use_cases
```

### 改 SQLite schema / migration / store

```powershell
cargo test --test sqlite_store
```

### 改 reflection 输入 / DTO / 证据门槛

```powershell
cargo test --test application_use_cases --test failure_modes --test mcp_stdio
```

### 改 automatic self-revision / trigger ledger / runtime hook wiring

```zsh
cargo test --test application_use_cases auto_reflection_runs_once_for_repeated_failure_and_records_handled_ledger -v
cargo test --test sqlite_store sqlite_trigger_ledger_records_namespace_periodic_watermark_and_cooldown -v
cargo test --test mcp_stdio ingest_interaction_auto_reflects_once_and_does_not_recurse_inside_run_reflection -v
cargo test --test mcp_stdio decide_with_snapshot_can_trigger_conflict_auto_reflection_without_breaking_decision_flow -v
cargo test --test mcp_stdio build_self_snapshot_can_trigger_periodic_auto_reflection_once_for_explicit_namespace -v
cargo test --test failure_modes auto_reflection_returns_structured_diagnostics_for_suppressed_trigger -v
```

### 改 self-revision demo package

```zsh
cargo test --test demo_openai_compatible_stub --test self_revision_demo_runner --test openai_compatible_model --test mcp_stdio -v
./scripts/run-self-revision-demo.sh target/reports/self-revision-demo/manual-$(date +%Y%m%d-%H%M%S)
```

发布前使用对应 release gate 的 freshness 口径：这些 artifact 必须来自同一次成功运行，不能只因为已有旧文件就视为通过。Local Alpha 发布前不要直接让 demo wrapper 写入 `latest`，应使用 `./scripts/product-smoke-local.sh` 的 staging / promote 路径。

### 改 Local Alpha product smoke gate / wrapper

```zsh
bash -n scripts/product-smoke-local.sh
./scripts/product-smoke-local.sh
./scripts/product-smoke-local.sh agent-llm-mm.local.toml
(cd /tmp && /path/to/agent-llm-mm/scripts/product-smoke-local.sh)
(cd /tmp && /path/to/agent-llm-mm/scripts/product-smoke-local.sh /path/to/agent-llm-mm/agent-llm-mm.local.toml)
test -s target/reports/self-revision-demo/latest/report.md
git diff --check
```

其中 `./scripts/product-smoke-local.sh` 是 repo root 示例；`agent-llm-mm.local.toml` 是本地用户配置占位路径，文件必须已存在，且只覆盖 `doctor` config path；从其他当前目录运行时，改用 `/path/to/agent-llm-mm/scripts/product-smoke-local.sh` 这类绝对脚本路径，并用绝对 config path 验证配置分支。

这组命令只覆盖 Local Alpha product smoke 入口；demo / MVP 发布前仍以 `release-gate.md` 为准，Local Alpha 发布前仍以 `docs/product/release-gate-local-alpha.md` 为准。

### 改 Local Alpha support bundle

```zsh
cargo test --test support_bundle -v
bash -n scripts/generate-support-bundle.sh
rm -rf target/support-bundles/manual-check
./scripts/generate-support-bundle.sh target/support-bundles/manual-check
rm -rf target/support-bundles/manual-correlation-check
./scripts/generate-support-bundle.sh target/support-bundles/manual-correlation-check --correlation-id mcp-tool-call-018fbc89-9ac1-4f5d-8b2a-1f6f5f27b205
find target/support-bundles/manual-check -maxdepth 1 -type f -print | sort
rg -n 'api_key|api-key|x-api-key|Authorization|Bearer|sk-|provider_token|openai_api_key|password|secret|sqlite:///|token=' target/support-bundles/manual-check || true
find target/support-bundles/manual-check \( -name '*.sqlite' -o -name '*.toml' -o -name '*.log' \) -print
tmp_log="$(mktemp target/support-bundles/manual-log.XXXXXX.log)"
printf 'INFO request prompt=hidden Authorization: Bearer sk-manual token=abc sqlite:///Users/example/private.sqlite\n' > "${tmp_log}"
rm -rf target/support-bundles/manual-check-with-log
./scripts/generate-support-bundle.sh target/support-bundles/manual-check-with-log --log-file "${tmp_log}"
test -s target/support-bundles/manual-check-with-log/local-log-excerpts.json
rg -n 'api_key|api-key|x-api-key|Authorization|Bearer|sk-|provider_token|openai_api_key|password|secret|sqlite:///|token=' target/support-bundles/manual-check-with-log || true
find target/support-bundles/manual-check-with-log \( -name '*.sqlite' -o -name '*.toml' -o -name '*.log' \) -print
rg -n 'API key|redact|support bundle|excluded|doctor|generate-support-bundle' docs/product/support-bundle-local-alpha.md docs/product/release-gate-local-alpha.md
git diff --check
```

这组命令验证首版本地 support bundle 生成器、脚本入口、脱敏边界、read-only operation-log 查询、显式 `--correlation-id mcp-tool-call-<uuid-v4>` operation summary 过滤、显式 `--log-file` 本地日志摘要/摘录，以及文档口径。输出目录必须不存在或为空；测试会覆盖非空目录被拒绝，避免旧的本地文件混入可分享支持包。敏感词扫描应无实际泄露；`find` 命令不应打印 `.sqlite`、`.toml` 或原始 `.log` 文件。operation summary 过滤只接受生成型 `mcp-tool-call-<uuid-v4>` correlation id，并仍只输出 metadata，不输出 request / response / diagnostic payload summary；user/project namespace 只输出 shape，secret-like operation metadata 会被替换。日志摘录只允许显式传入单个本地文件，secret-like config/log 文件名会折叠成 `<local-path>/<redacted-name>`；不允许扫描默认日志目录、home、系统日志、browser profile、SSH/cookie/session、shell history 或 `target/` 输出。超大日志只读取有界尾部窗口，并以 `line_number_scope = "tail"` 标记行号语义。该生成器不会创建或迁移缺失 SQLite 数据库，也不会通过 runtime bootstrap seed 默认 identity / commitments；它仍是本地诊断辅助，不代表远程上传、生产支持通道或 Local Alpha 完成。

### 改 release engineering / local soak evidence

```zsh
bash -n scripts/release-soak-local.sh
cargo test --features release-tools --test local_alpha_release_evidence release_soak -v
rm -rf target/reports/releases/manual-local-soak
./scripts/release-soak-local.sh manual-local-soak
test -s target/reports/releases/manual-local-soak/release-soak-summary.md
test -s target/reports/releases/manual-local-soak/command-summary.tsv
test -s target/reports/releases/manual-local-soak/local-alpha-evidence-summary.json
test -s target/reports/releases/manual-local-soak/support-bundle-sha256.txt
test -s target/reports/releases/manual-local-soak/product-smoke-latest-sha256.txt
test ! -s target/reports/releases/manual-local-soak/secret-scan.log
test ! -s target/reports/releases/manual-local-soak/artifact-scan.log
git diff --check
```

这组命令验证本地 release soak runner、candidate-specific evidence directory、doctor / dashboard HTTP / product smoke / first-run simulation / support bundle / evidence summary 串联，以及 support bundle secret/raw-artifact scan 和 support bundle / product smoke SHA-256 manifest。它不创建 release tag、安装包、Windows runner evidence、真实 fresh-machine evidence、remote/team evidence、上传或发布认证证据；`local-alpha-evidence-summary.json` 若仍为 `in_progress` / `not_verified`，必须保留 open gate。

### 改 daemon observe-only gate

```zsh
rg -n 'observe-only|run_reflection|forbidden|daemon|remote listener' docs/product/daemon-observe-only-gate.md docs/product/release-gate-local-alpha.md docs/roadmap.md docs/project-status.md
cargo test --test daemon_config -v
git diff --check
```

这组命令验证 daemon 观察模式的边界文档、doctor diagnostics、`serve` 中 `[daemon].enabled = true` 时的 observe-only handle wiring、本地 handle start / stop / drop-abort 生命周期。Local Alpha 仍保持 daemon disabled by default；observe-only 阶段不能调用 `run_reflection`，也不能声明后台自治或 write-capable daemon。

### 改 daemon observe-only diagnostics / doctor 输出

```zsh
cargo test --test daemon_config -v
cargo test --test operation_log -v
AGENT_LLM_MM_DATABASE_URL=sqlite:///private/tmp/agent-llm-mm-doctor.sqlite ./scripts/agent-llm-mm.sh doctor --read-only
rg -n 'daemon_observe_only|observe-only|writes_allowed|remote_listener_enabled|operation_log' README.md docs/product/daemon-observe-only-gate.md docs/product/release-gate-local-alpha.md docs/project-status.md
git diff --check
```

这组命令验证 `doctor.daemon_observe_only` 的本机只读诊断字段、daemon 默认关闭、observe-only 写入 gate、operation-log status 查询，以及文档口径。doctor 不执行 runtime bootstrap，也不启动 daemon handle；数据库不是 current 时只报告 operation-log unavailable。diagnostics 不能调用 `run_reflection`、不能新增 identity / commitments / claims / events / reflections 语义写入，也不能声明 daemon 已具备后台自治。

### 改 correlation id / operation log observability

```zsh
cargo test --test dashboard_projection --test dashboard_http --test mcp_stdio --test operation_log -v
cargo test --test mcp_stdio mcp_tool_failure_does_not_persist_provider_error_payload_in_operation_log -v
cargo test --test mcp_stdio dashboard_failed_tool_event_does_not_expose_provider_error_payload -v
```

这组命令验证 MCP tool call 级 correlation id、dashboard 详情投影、`/api/operation-log` 本机只读 durable history 查询和 handler-level MCP tool operation-log 元数据。失败路径记录不改变 MCP error code / error message 语义，且 durable diagnostic 与 dashboard failure event 只保留安全分类元数据，不落 raw request / provider payload；该链路只是 observability metadata，不代表新增 identity / commitments / reflection 的旁路写入能力。`rmcp` framework-level 解析/路由失败（例如非 object `arguments`）不进入项目 handler，因此不声明为 durable operation-log 覆盖范围。

```zsh
cargo test --test mcp_stdio non_object_mcp_tool_arguments_do_not_reach_handler_operation_log -v
rg -n 'correlation_id|mcp-tool-call|run_reflection|operation-log' docs/product/correlation-id-contract.md docs/product/release-gate-local-alpha.md
git diff --check
```

### 改 `src/support/config.rs`

```zsh
cargo test --test provider_config -v
```

### 改 MCP DTO / server / 错误映射

```powershell
cargo test --test mcp_stdio
```

### 普通提交前快速回归

```zsh
cargo fmt --check
git diff --check
./scripts/test-tier.sh fast
cargo clippy --all-targets -- -D warnings
./scripts/status-sync-check.sh
./scripts/test-tier.sh core
AGENT_LLM_MM_DATABASE_URL=sqlite:///private/tmp/agent-llm-mm-doctor.sqlite ./scripts/agent-llm-mm.sh doctor
```

demo / MVP 发布前核验不使用这段简表作为最终依据；请按 [Release Gate](release-gate.md) 执行完整 MVP gate。Local Alpha / product alpha 发布前核验使用 [Local Alpha Release Gate](product/release-gate-local-alpha.md)。

---

## 11. 当前结论

截至 `2026-08-09`，推荐把下面七条当作普通提交前基线；demo / MVP 发布前仍以 [Release Gate](release-gate.md) 为准；Local Alpha / product alpha 发布前以 [Local Alpha Release Gate](product/release-gate-local-alpha.md) 为准：

```zsh
cargo fmt --check
git diff --check
./scripts/test-tier.sh fast
cargo clippy --all-targets -- -D warnings
./scripts/status-sync-check.sh
./scripts/test-tier.sh core
AGENT_LLM_MM_DATABASE_URL=sqlite:///private/tmp/agent-llm-mm-doctor.sqlite ./scripts/agent-llm-mm.sh doctor
```

如果这七条都通过，说明当前工作树至少满足：

- 编码规范通过
- 编译与静态检查通过
- 当前 active plan 与 reality gate 同步，且根目录没有误回流的 `not-a-sqlite-url` SQLite 文件
- `namespace`、SQLite migration、MCP `stdio`、reflection 闭环和 automatic self-revision MVP 基线都可继续追加定向验证
- 本机运行时 bootstrap 正常

## 2026-10-09 database prerequisite regressions

Run `cargo test --test sqlite_lifecycle` for canonical structural readback, weakened same-version columns/keys/FKs/CHECK/index rejection, v2-to-current migration, concurrent init, pre-existing sidecar preservation, and external same-count writers including WAL. Default doctor stays read-only. Failed init deliberately retains the reserved database for diagnosis; inspect before manual cleanup. `schema_structure_invalid` is not automatically repaired. Full source gates remain required before publishing a completed implementation stage.

## Bounded usability candidate regressions

- `cargo test --test correction_atomicity --test feedback_provenance`: transaction target/state/scope revalidation, concurrent replay/supersession, receipt-insert rollback, schema-v4 migration, typed feedback and same-scope evidence.
- `cargo test --lib`: fixed bilingual literal-query fixtures, active-claim priority under event floods, scope/provenance checks, query limits, and exact UTF-8 context-budget sweeps. This small deterministic fixture is not a public/model-judged benchmark or large-corpus latency result.
- `python3 scripts/local-memory-smoke.py --binary target/debug/agent_llm_mm --output target/reports/local-memory-smoke`: actual stdio installed-copy restart/correction/history/restore story; requires an empty output directory. Windows uses `python` and `.exe`. No remote model call.
- Full/fmt/all-feature Clippy/status-sync remain mandatory. Windows native CI is a defined subset, not proof of shell-wrapper parity or a published installer. See [workflow contract](local-memory-usability.md).

### Migration reader-lock regression

`migration_waits_for_existing_reader_before_commit` deterministically reproduced SQLITE_BUSY before the fix. Migration admission still fails fast when another writer owns the database; after its own reservation, SQLite waits at most five seconds for transient reader locks so rollback-journal COMMIT can complete. DELETE/WAL writer-resumption tests remain enabled on Windows. Exhausting the bound returns an error and preserves the backup rather than silently retrying or deleting data.

## Original-plan continuation regressions (schema v5)

- `cargo test --test feedback_candidates --test correction_atomicity`: target-content version conflict, evidence support alignment, model/inconclusive/missing rejection, candidate immutability, no-new-evidence stop, commit/reject races and all-or-nothing receipts/history.
- `cargo test --test experience_workflow`: same-scope source links, expected_version race, inspect/reject/activation, immutable versions and pending rollback, complete JSON-byte budget, no authority mutation, recovery.
- `cargo test --test indexed_text_recall --test sqlite_lifecycle --test feedback_provenance`: explicit v0/v2/v3/v4→v5 migration, canonical readback, original-record preservation and restore rehearsal; FTS/CJK/punctuation/legacy IDs, trigger synchronization, missing/corrupt index detection and explicit rebuild, VACUUM/restore.
- `python3 scripts/evaluate-memory-loop.py --help` and `python3 scripts/benchmark-memory-capacity.py --help`: fixed offline mechanism comparisons, ablations and reproducible capacity/query-plan reports. Read [methodology](evaluation-methodology.md); proxy success is not real LLM success, bytes are not tokens, and query variants are not independent tasks.
- Final verification also requires fmt, all-target/all-feature Clippy, full test tier, status-sync and exact-head platform CI. Historical v4 counts above are not reused as v5 results.

Windows fixture portability: `python -m unittest discover -s tests/fixtures/memory-evaluation -p "test_*.py"` also forces a cp1252 default decoder in a regression and requires both multilingual JSON fixtures to retain their UTF-8 content. Earlier archived two-test metric logs predate this additional encoding regression; they are historical evidence, not its result.

## Schema v6 temporal, scope and export regressions

- `cargo test --test temporal_metadata --test sqlite_temporal_store`: distinct caller observation/application recording time, historical null, old receipt/fingerprint compatibility, replay immutability, nanosecond/extreme-offset/leap-second ordering, union prelimit and standalone compatibility, real range-index query plans.
- `cargo test --test reflection_scope_history --test scoped_ledger_export --test schema6_migration`: independent source/effect metadata, safe targetless history, normalized durable evidence FK/rollback, ambiguous-history quarantine, bounded snapshot export and no export diagnostics writes, real v5 migration/raw preservation/backup recovery.
- `python3 scripts/temporal-scope-export-smoke.py --binary target/debug/agent_llm_mm --output target/reports/temporal-scope-export`: real provider-free stdio timestamp/replay/history/export journey; use `python` and `.exe` on Windows. No real user database or remote model is used.

Earlier schema-v5 570-test/evaluation artifacts remain historical. New-stage results must carry matching source/binary manifests and exact-head CI; do not transfer old performance numbers to the new schema.


### Final original-plan context and caller-budget regression

`cargo test --test context_diagnostics --test caller_operation_budget --test mcp_stdio --test schema6_migration` covers scoped rich Episode context, honest bounded diagnostics, optional caller counts/stops over actual local MCP subprocesses, and historical receipt/fingerprint replay across migration. Run the normal full/fmt/Clippy/status-sync gates afterward. The fixed offline evaluator keeps its original tasks and byte budgets; richer metadata costs must be reported rather than hidden by retuning fixtures. See [context](context-diagnostics.md), [caller budget](caller-operation-budget.md), and [module extraction](implementation-module-boundaries.md) contracts.
