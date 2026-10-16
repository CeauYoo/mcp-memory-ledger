# MCP Memory Ledger 正式化改进与主线同步计划

状态：`active supporting plan / implementation incomplete`

基线日期：`2026-08-25`

本文件把“做成正式项目”拆成可验收的改进项，并记录当前本地代码进入 GitHub 主线的同步路径。它是跨领域差距矩阵，不是第二份任务队列；具体实现顺序仍以 [active project plan](plans/README.md) 为唯一执行来源。

## 2026-10-15 当前差距对照

以下为当前实现核对，后文 2026-08-25 数字/矩阵保持历史基线，不重新变成待开发清单。执行顺序仍只由 active plan 管理。

| 范围 | 当前分类 | 依据与剩余条件 |
| --- | --- | --- |
| CORE-01 / CORE-03 | 已实现 | current-schema 结构读回与排他 init/migrate；见当前状态及生命周期测试 |
| CORE-02 | 部分实现 / 外部验收门 | 旧部署有实际协议/自然语言有限样本；新核心完整真实客户端故事尚未验收 |
| REL-01 / REL-03 | 有限实现 | 精确源码 portable 构建/校验/解包及 Windows wrapper 已有历史证据；新 Unix 核心候选须新证据，Windows 仅旧 stdio |
| GOV-01 / SEC-01 / SEC-04 / GIT-04 | 文档已补，治理门保留 | SECURITY、SUPPORT、拟定协作准则、CHANGELOG、Issue/PR 模板存在；未确认负责人，不伪造 CODEOWNERS、私密报告渠道可用性、响应 SLA 或签名 release |
| DATA-01 | 有限实现 / 非目标 | 显式备份恢复及 scoped export 已实现；retain-all，不擅自增加 destructive retention/compaction |
| DATA-02 | 未实施，须数据政策 | legacy Unknown owner 仅只读盘点；旧项目 namespace 到新 workspace ID 也没有自动 adopt/重写，不变更正式数据 |
| OPS-01 / OPS-02 / OPS-03 | 有限实现 | 有规模测量、索引与有界日志；新核心共享 observer，不等于生产容量/SLO、全历史计数或长期 soak |
| OPS-04 | 部分实现 | 新核心隔离崩溃/重连/恢复验证入口已纳入候选验证；断电、磁盘满、任意写入中断仍不得由正常重启推断 |
| UX-01 / UX-03 | 部分实现 | 核心接入、全局桥/显式桥样例、平台与数据切换边界已补；真实十分钟新机体验仍未测 |
| SEC-02 | 未实施 / 工具评估门 | 依赖漏洞/license/secret scanning 尚无已验证 CI 门；本轮不擅自安装新工具或创造例外负责人 |
| REL-02 / REL-04 / GIT-01–03 | 外部证据 / 执行动作门 | fresh-machine、人工 release、远端同步/保护/合并需各自当前证据和授权，不由本地文件更新关闭 |
| REL-05 / UX-02 / REMOTE-01–02 | 后置 / 独立需求 | SBOM/attestation、协议库升级、远程团队服务不扩大为当前核心必做项 |

本次只在助手云端实现并验证，不访问 Mac、不推送、不改正式客户端配置、不启用后台服务、不调用收费模型。新增代码、隔离测试通过、精确提交 CI、真实客户端与正式部署分别报告。

## 1. 结论

当前仓库已经是一个边界清楚、测试较完整的 local-first technical MVP，但还不能称为正式发布产品、Local Alpha、Beta、GA 或 production-ready 系统。

第一阶段“正式项目”应冻结为：

> 一个可从版本化 artifact 安装、可在 fresh machine 上完成核心记忆闭环、可备份恢复、可诊断支持、可审计发布，并由 CI 与人工 release decision 共同把关的本地 MCP Memory Ledger。

当前架构是 MCP `stdio + SQLite`。因此，本轮正式化默认不扩张为远程 HTTP、托管服务、多租户平台或持续自治 daemon。若商业目标实际是 SaaS / team service，需要另行冻结认证、授权、tenant isolation、rate limit、远程备份、运维 SLO 和成本模型，不能把本地产品门禁直接复用为线上服务验收。

## 2. 证据基线

任务开始时的可复核状态：

| 维度 | 当前证据 | 结论 |
| --- | --- | --- |
| 本地工作区 | `dev-work@43580ba`，工作区 clean | 新功能已整理为提交，不是散落的未提交代码 |
| 开发分支差异 | `dev-work` 相对 `origin/dev-work` 为 `0 behind / 12 ahead` | 12 个本地提交尚未进入远端开发分支 |
| 默认分支差异 | `dev-work` 相对 `origin/main` 为 `0 behind / 28 ahead` | GitHub 默认分支仍停在旧技术 MVP 状态 |
| 新功能范围 | 12 个提交，41 个文件，约 6413 行新增 / 244 行删除 | 包含 M1.0.1–M1.0.3、M1.1.3–M1.1.6、M1.2.4–M1.2.7 及文档同步 |
| 合并入口 | GitHub PR #1：`dev-work -> main` | 应更新既有 PR，不另起重复主线或直接覆盖 `main` |
| PR 旧状态 | 远端 head 为 `082be93`，16 commits；Linux/macOS 旧检查均失败 | 旧 PR 不是可合并证据，必须由新 head 的 fresh CI 替代 |
| 本地候选门禁 | 2026-08-25 的 format、all-feature Clippy、full tier、status sync、diff check 全部通过；26 个完成项与 reality gates 一致 | 候选达到推送开发分支的本地证据门，仍需远端 fresh CI |
| MCP 能力 | 当前本地代码暴露 10 个工具 | scoped search/lookup/history/evidence relation/supersede 已形成首批闭环 |
| 核心未完成 | M1.3.0 current-schema structural readback、M1.3.1 real-client closure | 仍不能宣布 Trustworthy Recall 里程碑完成 |
| 产品交付 | `Cargo.toml` 仍为 `0.1.0`；无 tag、GitHub Release 或受支持 binary artifact | 当前仍是 source-only MVP |
| CI | GitHub Actions 只覆盖 Linux/macOS format、Clippy、full tier、status sync | Windows runtime、依赖安全、release provenance 尚未形成门禁 |
| 项目治理 | 有 LICENSE、NOTICE、CONTRIBUTING；缺 CHANGELOG、SECURITY、SUPPORT、CODE_OF_CONDUCT、CODEOWNERS、PR/Issue 模板 | 公开协作与安全响应还不完整 |
| 运行边界 | dashboard 仅 loopback 且无认证；无 remote HTTP/team mode | 不能部署成公网服务或宣称远程管理能力 |

这里的“线上代码”只指已配置的 GitHub `origin`。仓库没有唯一生产主机、服务单元、容器平台或部署清单，因此 Git 推送不等于生产部署，也不授权 SSH、服务重启、数据库迁移或 live provider 调用。

## 3. 正式化目标与非目标

### 3.1 业务目标

- 让本地 AI 客户端可以稳定地安装、初始化、记录、检索、查看证据、纠错和恢复记忆。
- 让每一次公开版本都能追溯到 commit、CI、artifact checksum、兼容矩阵、变更记录和人工发布结论。
- 让 scope 隔离、provenance、数据生命周期和故障恢复成为可验证合同，而不是文档承诺。

### 3.2 第一阶段主要用户

- 需要本地 MCP 记忆能力的 AI 客户端开发者；
- 评估 evidence-gated memory / self-revision 的工程与研究人员；
- 需要可检查、可恢复 SQLite 记忆底账的本地工具维护者。

### 3.3 非目标

- 本轮不建设公网 HTTP 服务、OAuth、远程写管理或多租户 SaaS；
- 不把 provider 连通性写成模型质量或 SLA；
- 不把本地测试通过写成 fresh-machine、Windows 或业务验收；
- 不因为“正式化”而扩大 automatic self-revision 或新增第二条 durable write path；
- 不在没有 release gate 和人工结论时使用 `stable`、`GA`、`production` 或 `v1.0` 标签。

## 4. 改进矩阵

优先级定义：`P0` 阻断当前主线或正式候选；`P1` 阻断可支持的 Local Alpha；`P2` 只在真实远程/商业需求成立后启动。

### 4.1 源码主线与协作治理

| ID | 优先级 | 当前问题 | 改进动作 | 通过证据 |
| --- | --- | --- | --- | --- |
| GIT-01 | P0 | 本地 `dev-work` 与 `origin/dev-work` 相差 12 个提交 | 完整本地门禁通过后推送现有 `dev-work`，不重写历史 | 远端 `refs/heads/dev-work` SHA 与本地候选一致 |
| GIT-02 | P0 | PR #1 的说明、commit 数和 CI 证据停在旧 head | 让 PR #1 接收新 head；更新摘要、工具数、验证记录和边界 | PR head 为新候选；描述与 10-tool 事实一致；fresh checks 全绿 |
| GIT-03 | P0 | `main` 未建立可证明的合并门禁 | 保护 `main`，要求 PR、Linux/macOS CI 和人工 review；禁止 force push | GitHub branch rule readback；PR 合并记录；`origin/main` 含候选 commit |
| GIT-04 | P1 | 没有正式 changelog 与 release/tag 纪律落地文件 | 增加 `CHANGELOG.md`，采用 pre-release version；tag 只指向通过 gate 的 commit | changelog、signed/annotated tag、release note 和 commit 相互一致 |
| GOV-01 | P0 | 公开协作文件不完整 | 增加 `SECURITY.md`、`SUPPORT.md`、`CODE_OF_CONDUCT.md`、CODEOWNERS、PR/Issue 模板 | 链接可用；责任人、响应边界、漏洞报告路径明确 |

### 4.2 核心产品与数据正确性

| ID | 优先级 | 当前问题 | 改进动作 | 通过证据 |
| --- | --- | --- | --- | --- |
| CORE-01 | P0 | schema version 相同不代表表约束和索引真实一致 | 完成 M1.3.0：`table_info`、FK、index 与 DDL fingerprint structural readback | 被削弱的 same-version DB 不得报告 `current`；正负向回归通过 |
| CORE-02 | P0 | 10 个 MCP 工具主要由自动化测试证明，真实客户端闭环缺失 | 完成 M1.3.1：两个 namespace、重连、recall、provenance、supersede、history、provider offline | MCP transcript + SQLite readback；namespace leakage = 0 |
| CORE-03 | P0 | init/migrate 尚无完整排他与 moving-target 保护 | 完成 M2.0.1 exclusive init/migration lifecycle gate | 并发操作被排他或明确拒绝；失败只清理本次创建对象；数据不丢失 |
| DATA-01 | P1 | backup/restore 已有本地门禁，retention/export/tombstone/compaction 未闭环 | 定义生命周期政策和兼容策略，优先可恢复删除而非 hard delete audit | retention/export/restore 测试、版本兼容矩阵、人工切换记录 |
| DATA-02 | P1 | legacy `Owner::Unknown` 行可盘点但不可由 scoped read 找回 | 先报告数量与风险，再单独设计可回滚 rewrite/migration | migration preflight、备份、行级 readback、零跨 scope 泄漏 |

### 4.3 发布、安装与平台支持

| ID | 优先级 | 当前问题 | 改进动作 | 通过证据 |
| --- | --- | --- | --- | --- |
| REL-01 | P0 | 当前只有 source-only 仓库，没有受支持 artifact | 先产出版本化 macOS archive 与 SHA-256；不要提前做 installer/auto-updater | clean unpack、checksum、版本命令、commit 可追溯 |
| REL-02 | P0 | quick start 依赖源码和 Rust toolchain | 建立无需源码编辑/`cargo run` 的 first-run 路径 | fresh macOS 从下载到 remember/recall ≤ 10 分钟 |
| REL-03 | P1 | Windows 只有脚本和文档，没有真实 runtime parity | 增加 Windows CI/runner，或明确标记 unsupported | PowerShell init/doctor/serve/MCP/data roundtrip 证据 |
| REL-04 | P0 | 没有候选版本的人工作出结论 | 使用 candidate-specific evidence，记录 reviewer、open gates、rollback note | `approved` / `rejected` / `pending` 明确；pending 不得视为发布通过 |
| REL-05 | P1 | 没有自动 release artifact 构建、SBOM、provenance/attestation | 在本地 Alpha 稳定后增加可复现构建与供应链证据 | artifact hash、SBOM、build provenance 与 tag 一致 |

### 4.4 安全与供应链

| ID | 优先级 | 当前问题 | 改进动作 | 通过证据 |
| --- | --- | --- | --- | --- |
| SEC-01 | P0 | threat model 存在，但缺安全响应政策与发布前实现复核 | 把本地资产、provider secret、SQLite、support bundle、dashboard 边界映射到 SECURITY 与 release gate | 威胁/缓解/验证/owner 可追溯；高风险 open item 阻断 release |
| SEC-02 | P0 | CI 没有依赖漏洞、license policy 或 secret scanning 门禁 | 评估并引入 `cargo audit`/`cargo deny` 等等价门禁及依赖更新策略 | fresh CI 报告；允许/拒绝规则入库；例外有期限与 owner |
| SEC-03 | P0 | dashboard 无认证，只能依赖 loopback 配置保护 | 保持 loopback-only fail closed；任何 remote 设计必须先有 auth/authz/CSRF/rate-limit threat model | 非 loopback 启动拒绝；无文档暗示公网可用 |
| SEC-04 | P1 | release/support artifacts 的脱敏已有测试，但没有长期 secret-leak 处置流程 | 增加泄漏响应、凭据轮换和 artifact 撤回说明 | SECURITY/SUPPORT runbook + 演练记录 |

### 4.5 可靠性、性能与可观测性

| ID | 优先级 | 当前问题 | 改进动作 | 通过证据 |
| --- | --- | --- | --- | --- |
| OPS-01 | P1 | 当前主要是功能测试，没有明确容量/延迟基线 | 建立固定规模 SQLite benchmark、并发读写、长时间 soak 和资源上限 | p50/p95、吞吐、DB size、CPU/RSS 基线可复现 |
| OPS-02 | P1 | Episode/Reflection/union 等路径仍有 MVP table-scan 边界 | 先用 profile 和真实规模证明瓶颈，再增加 index/migration | query plan、基线对比、migration/rollback、无结果漂移 |
| OPS-03 | P1 | operation log/support bundle 已有，尚无正式 SLO/故障分类 | 定义本地产品可观察指标、错误码、correlation-id 覆盖和支持分级 | 关键路径均可关联；支持包不含 raw secret/data；故障可复现 |
| OPS-04 | P1 | SQLite 事务证据不等于进程崩溃、断电或磁盘故障恢复 | 增加 crash/restart、磁盘满、损坏库和恢复演练 | 失败模式 readback、数据损失边界、恢复步骤和停止条件明确 |

### 4.6 产品体验与兼容性

| ID | 优先级 | 当前问题 | 改进动作 | 通过证据 |
| --- | --- | --- | --- | --- |
| UX-01 | P1 | 使用文档完整但过长，缺可交付 artifact 的最短成功路径 | 为最终用户提供 5–10 分钟 quick start、故障入口和卸载/清理说明 | fresh user 无源码修改完成核心故事；文档命令可复制 |
| UX-02 | P1 | `rmcp 0.5.0` 与未来协议生态兼容风险仍开放 | 以 capability-neutral 独立迁移重新评估，不顺带引入 remote/tasks/OAuth | 工具 schema、stdio 隔离、兼容客户端矩阵与回滚通过 |
| UX-03 | P1 | 版本、配置、schema 和客户端兼容矩阵未形成一页式合同 | 建立兼容矩阵和 deprecation 窗口 | 每个候选明确 supported/tested/unsupported 组合 |

### 4.7 远程与商业化产品

| ID | 优先级 | 当前问题 | 改进动作 | 通过证据 |
| --- | --- | --- | --- | --- |
| REMOTE-01 | P2 | 当前没有远程 transport、认证、多租户或生产运维边界 | 仅在 Local Alpha 稳定且真实用户需求成立后立项独立 RFC | 需求、成本、threat model、tenant isolation、rollback 均获批准 |
| REMOTE-02 | P2 | 本地 SQLite/loopback 证据不能外推为 hosted service | 设计独立数据面、控制面、备份、SLO、on-call 与合规边界 | staging/canary/production 分阶段证据，业务 owner 人工验收 |

## 5. 分阶段实施建议

### Phase A — 主线重新同步

目标：让 GitHub 开发分支和 PR 准确反映本地候选。

1. 完成本地 CI-equivalent gate 与文档回读。
2. 提交本次正式化文档和真相修正。
3. 推送 `dev-work` 到现有 `origin/dev-work`，不 force push。
4. 读取远端 SHA，确认 PR #1 自动更新。
5. 等待 Linux/macOS fresh checks；失败时保留 PR open，不合并 `main`。
6. 更新 PR 摘要与验证信息；由人工 reviewer 决定是否合并。

退出门：远端开发分支与候选 SHA 一致，PR head fresh checks 全绿，旧失败检查不再是当前 head 的状态。

### Phase B — M1 Trustworthy Recall 收口

目标：把自动化能力变成真实客户端可复核闭环。

依次完成 `M1.3.0 structural readback -> M1.3.1 real-client closure`。任何 scope 泄漏、same-version schema 误判或纠错历史丢失都阻断进入 M2。

### Phase C — Local Product Alpha 候选

目标：从 source-only MVP 进入可安装、可恢复、可支持的本地候选。

完成 M2.0.1、macOS binary archive/checksum、fresh-machine、backup/restore、support bundle、安全/治理基础文件、版本/changelog 和人工 release decision。Windows 没有证据时必须明确标为 `not verified` 或 `unsupported`。

### Phase D — Controlled Beta 与 GA 评估

目标：只在真实使用证据成立后扩大承诺。

至少需要受控用户 dogfood、长期数据安全/恢复证据、性能基线、依赖安全、支持流程和独立 release review。Hosted/remote 仍需另行决策，不随本地 Beta 自动获得授权。

## 6. 推荐分支与发布流

当前过渡期使用：

```text
feature/codex branch
  -> dev-work（集成与完整本地门禁）
      -> PR #1 / 后续 PR（CI + review）
          -> main（公开集成主线）
              -> candidate tag
                  -> release decision
```

规则：

- `main` 不直接 force push；正常功能通过 PR 合并。
- `dev-work` 只接收依赖闭合、文档同步且至少完成 focused checks 的提交。
- 进入 `main` 前必须运行 Linux/macOS CI；平台证据不能互相替代。
- tag 和 release note 只能指向通过候选 gate 的 exact commit。
- GitHub push、PR merge、tag/release、production deployment 是四个独立动作；前一步成功不自动授权后一步。

## 7. 正式化验收清单

| ID | 结果 | 证据 | 类型 | Owner | 通过条件 |
| --- | --- | --- | --- | --- | --- |
| A-01 | 远端开发分支同步 | local/remote SHA readback | automated | CI / maintainer | `origin/dev-work` 等于候选 SHA |
| A-02 | 主线合并准备完成 | PR head、fresh checks、diff、review | automated + human-required | GitHub CI + reviewer | checks 全绿且 reviewer 明确同意 |
| A-03 | Trustworthy Recall 收口 | structural DB tests、真实 MCP transcript、SQLite readback | automated + agent-observed | maintainer | M1.3.0/1 通过，scope leakage = 0 |
| A-04 | Local Product Alpha 可安装 | artifact、checksum、fresh-machine transcript | agent-observed | release reviewer | 无 Rust toolchain 完成核心故事 ≤ 10 分钟 |
| A-05 | 数据可恢复 | backup/restore/read-only verify/manual switch | automated + human-required | maintainer | roundtrip = 100%，切换经人工确认 |
| A-06 | 安全与支持可操作 | security policy、dependency gate、support runbook | automated + human-required | security/release owner | 无未处置 P0；报告与响应路径可用 |
| A-07 | 公开口径准确 | README/status/roadmap/release note scan | automated + human-required | docs owner / reviewer | 无 Local Alpha/Beta/GA/production 越级声明 |
| A-08 | 发布决定明确 | release decision record | human-required | release reviewer | `approved` 才允许发布；`pending` 不是通过 |

## 8. 本轮边界

本轮允许：本地文档与真相修正、完整本地验证、提交到当前 `dev-work`、推送现有 GitHub `origin/dev-work`、读取 PR/CI 状态。

本轮不自动执行：直接覆盖 `main`、合并 PR、创建 tag/GitHub Release、修改仓库设置/branch protection、SSH/生产部署、数据库迁移、live provider 或付费调用。`main` 合并保留为 PR fresh CI 通过后的人工验收动作。
