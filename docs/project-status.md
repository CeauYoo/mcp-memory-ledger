# 当前实现状态

本切片云端全量 929 项通过、14 项 AF_UNIX 环境阻断，无其他失败；精确提交 CI 待运行。详细范围见[第二切片验证记录](task-experience-validation-2026-10-16.md)。

## 当前整合与验证状态（2026-10-16）

当前源码已合入动作检查、Schema 9 经验采用反馈、只读界面与项目发现。云端定向回归、严格 Clippy、Node 13 项和宿主探针离线 9 项通过，最终全量结果如下，下一精确提交 CI 待核验。首批 `20d4037` 的 Ubuntu CI 全部通过；macOS / Windows 在同源 artifact fixture 的路径规范化检查失败，最小测试路径修补已合入、本地对应测试通过，原生平台仍待复验，不能报告首批三平台全绿。后续切片不继承首批通过结果。

T16 模型草稿、P4 检索与真实宿主探针仍在实现/验证中，尚未标为完成；未操作用户机器或部署。

## T09–T11 源码已整合，真实宿主待验收

已新增 deny-only patch 参数检查、独立 policy epoch、默认 latest_active 任务刷新、私有检查回执、配置差异/备份快照及 CAS 撤回候选。无授权 token，无普通记忆写入权限晋升。T11 不包含实际安装/apply；配置必须另行确认后部署。详细范围见 [动作检查合同](codex-action-checks.md)。

领域/适配定向测试与严格 Clippy 已通过；真实 host 仍 blocked/unverified，Windows shell preview 与 checked writes 均 unsupported。默认 patch-only 不拦 shell；显式 strict-local 会拒绝未分类本地工具，也不能覆盖 stdin/hosted/故障旁路。不能继承首批/历史 CI 绿灯。

## P3 经验采用反馈与界面已合入源码

Schema 9 的 `record_experience_use` / `list_experience_uses` 及本地 CLI 记录任务与经验精确版本、采用决定、适用条件、行动/结果 Event 来源、评估和局限。关联后续观察保留原历史；当前资格与失效原因单独重算。MCP 沿用逐调用项目授权和撤销检查，CLI 不自动初始化或迁移。

`derive_experience_from_use` 显式接收调用方 Episode/候选内容，校验原 use 的来源关系，复用既有写入和确定性回执逐步生成 pending 候选；可恢复但不是跨步骤原子事务，也不自动激活。T13 的有界记录核心已实现；T12/T15 当前版本重查与失效支持、T14 调用方归纳组合为部分实现，T16 模型草稿仍在推进。详见[经验反馈合同](experience-use-feedback.md)。

原 owner-only 任务界面已加入三语言经验报告、精确版本和结果来源，历史采用与当前重新评估资格分列；无新增写权限按钮或“自动生效”声明。Node 13 项及脚本语法通过，owner-only Rust/API 定向测试通过；最终整合、真实浏览器和三平台结果待对应验证。以上均不证明独立效果、因果收益、可信授权或真实宿主采用。

## 任务续接首批源码实现

Schema 8 增加任务要求修订、候选隔离、checkpoint 和有界续接上下文；6 个新任务 MCP 工具复用范围与撤销策略。原界面提供 owner-only 只读任务摘要，旧无凭据 observer 不暴露任务内容。原生 Codex 适配器提供显式项目默认任务、生命周期上下文输出和配置预览。

- [任务合同与迁移](task-ledger.md)
- [Codex 适配与真实证据边界](codex-task-context-adapter.md)
- [网页任务摘要](dashboard-live-desk.md)

本轮云端 Rust 878 项通过，14 项 IPC 环境阻断；严格 Clippy、Node 11 项及 Python 127 项通过。详细范围见[首批验证记录](task-continuity-validation-2026-10-16.md)；后续精确提交 CI 状态见本页顶部。

首批代码与当前整合版本分别记账。调用方报告不等于认证用户输入或独立结果验证；hook 输出不等于宿主接受。真实 Codex 会话启动受云端沙箱限制，自动注入尚未验收；Windows 核心仍已支持，但新 hook 的 Windows shell 配置预览未验证。旧 schema 须显式迁移；scoped export 不包含任务数据，完整数据库备份包含。

动作检查与经验反馈已形成上述有界实现，P4 检索增强仍在推进；不因代码落盘关闭整阶段验收。实际用户机器配置和部署仍另行确认。


## 当前执行入口与已完成基线（2026-10-16）

已批准按[完整唯一实施计划](plans/2026-10-16-task-continuity-and-host-integration.md)分阶段推进。P0 与 T01–T08 首批后已继续整合动作检查及 P3/P5 切片，当前处于集成审查与回归；真实宿主注入等未闭合项仍单列，批准不等于全部完成。

基线 `7448b02a7de8d5dd32c32a149fdd024f798bbf0f` 的 [CI 38860968271](https://github.com/yooyui/mcp-memory-ledger/actions/runs/38860968271) 三平台全部成功：Ubuntu/macOS 各 848 项 Rust，Windows 514 项 configured Rust；三平台精确来源候选包通过，Windows named-pipe core 的 42 次 MCP 工具调用、隔离、确认写后重启/重放、备份恢复与清理通过。Ubuntu 真实 Chrome 检查通过。

workspace 生命周期、只读配置生成、原生 DB-only 恢复、有界 reflection/experience 历史接入及 Windows 核心均已实现并获得上述范围验证；[独立审查](independent-security-review-2026-10-16.md)及修复已完成限定范围。它们不再是待实现项目。仍非全面安全认证、真实用户安装、任意在途崩溃保证或正式发布；真实宿主注入与新增功能须获得新证据，用户机器安装另行确认。

### 以下为历史阶段记录

下文带日期的“待验证/未提交/Windows 未支持”仅保留当时事实，由上面精确基线结论更新；旧阶段路线不再拥有执行优先权，当前任务只由唯一实施计划管理。


## 2026-10-16 跨平台与历史整合检查

本轮候选源码已合并复杂历史接入、Windows 本机核心和生命周期隐私修复。Linux 云端全套 Rust 846 项去重结果为 **834 通过、12 项 AF_UNIX `EPERM` 环境阻断**；不是全套通过。严格 all-feature Clippy、格式、diff、status-sync 33 项、Python 125 项和 Node 9 项通过。初轮构建目录符号链接造成的 fixture 拒绝已通过调整忽略的构建布局解决，产品源码未因此修改；具体复现与原始失败见[测试指南](testing-guide-2026-03-24.md#integrated-candidate-2026-10-16)。

已完成独立、有界源码安全复核并修复发现的生命周期隐私/路径问题及选定 TLS 依赖风险，见[报告](independent-security-review-2026-10-16.md)及[审查源码清单](integrated-source-manifest.sha256)。这不是安全认证或“无漏洞”结论。精确提交三平台 CI、Windows 原生 ACL/命名管道与候选包、真实 Chromium、新核心自然语言客户端、fresh-machine 和发布批准仍待各自证据；未部署。

数据库生命周期隐私加固：Unix 新建库与迁移备份创建即 owner-only，排练使用私有临时目录；现有库权限不改，备份无覆盖发布。Windows 使用专用 ACL helper，原生执行验收仍待完成。详见[数据库操作](database-operations.md)。

## 2026-10-16 Windows 本机核心 candidate

新增 Windows 本机命名管道 `core` + stdio `bridge` 实现，与 Unix 共用绑定、逐调用撤销、资源限额和关闭排空逻辑。新建 IPC/状态文件使用当前用户私有 DACL；已有文件/目录须当前用户所有，访问授权只接受当前用户、SYSTEM 和 Administrators。它不隔离同用户任意进程或特权管理员，也不是远端多租户服务。`serve` 兼容入口保留。

当前为待精确提交 Windows 原生 CI 验证的 candidate，不能把 Linux 编译或此前 legacy Windows CI 当作本轮通过。验证包括真实桥多会话、范围拒绝、重连/恢复，以及解包二进制烟测；无用户电脑操作、生产 ACL 修改、凭据生成、服务安装或部署。入口与限制见 [Windows 指南](development-windows.md)。


旧域 `adopt-preview` / `adopt` 已扩展为 scoped-history-v2：可归属 reflection、experience 全版本、feedback/trigger 历史按双向来源闭包检查，不重写数据；其他项目无关联历史可保留。unknown/mixed/缺失来源及 baseline 后 global self-model 仍明确不支持。见[接入合同](workspace-legacy-adoption.md)。当前源码验证与精确提交 CI 分开记录，不继承上一次 CI 或正式部署结论。

## 2026-10-16 当前切片：生命周期、配置输出与原生恢复

当前工作区已接入三项功能，**尚未提交，最终同源验证与新提交 CI 待完成**。这是 M1.W1/W2 的可用性补齐及 M2 实用功能的有限前移；不等待 M1 外部验收才开始实现，也不自动关闭任何里程碑。

| 新能力 | 当前合同 |
| --- | --- |
| `workspace unbind / disable / enable` | 精确解绑存储 root；停用保留 ID、授权与账本；旧连接下一请求含 replay 前重新拒绝；启用须有有效 root，已派发操作不追溯取消。见[生命周期](local-workspace-core.md) |
| `workspace inspect / status / client-config` | 只读 registry JSON / stdout TOML，不读账本或连接核心，不修改配置/权限；配置可用性不等于运行健康。见[配置生成](workspace-client-config.md) |
| `backup / restore --db ... --output ...` | SQLite online backup、一致 WAL 快照、current schema7 校验、新目标 no-clobber；registry 独立、不自动迁移/切换/激活。见[运维](database-operations.md) |

此前提交 `f8eb9ed` 的 [CI run 38709687717](https://github.com/yooyui/mcp-memory-ledger/actions/runs/38709687717) 已全绿：Ubuntu/macOS 各 794 项 Rust 测试、Windows 429 项，以及 Ubuntu 真实 Chromium 核心网页检查。该历史提交的 Windows 仅验证 legacy；这些结果不覆盖本次 Windows candidate。新增功能的独立源码审查与云端同源回归结果见本页顶部，跨平台精确 CI 仍待完成；新核心真实客户端自然语言、fresh-machine、真实模型/费用与人工发布仍开放。无 Mac 操作、真实模型调用、推送或正式部署替换。

以下 2026-10-15 段落是此前集成阶段的证据快照；其中“最终 CI 待完成”已由上述旧提交 CI 更新，但不代表当前新增改动已通过。

## 2026-10-15 当前切片：工作空间核心（进行中）

[本机核心与固定桥](local-workspace-core.md)纳入[唯一 active plan](plans/README.md)。Unix 核心统一持有数据库和 observer；桥按 `--workspace-id`/`--workspace-root` 显式固定绑定，或通过 `--registry` 在启动时按真实 cwd 精确解析；后者可全局配置一次，但固定 cwd 客户端须用显式模式，连接不会跟随项目移动。registry 每次调用刷新个人读写授权。25 个范围工具允许，7 个全局工具隐藏/拒绝；core ingest 关闭自动反思。核心默认不开监控 HTTP，管理员 CLI 与标签页内存登录入口已接入，公开静态壳不含运行数据，API/SSE 继续独立 Bearer；真实核心端到端尚待验收，未配置生产凭据。

当前只取得定向证据：registry 11 项、实际 Codex roots 67 条断言、管理员监控 7 项及 legacy HTTP 6 项。历史首轮完整集成有 3 项失败；云端续建已修补相关路径，最终同源回归仍待完成，**不能报告全套通过**。设计上限为64会话、共享32个业务执行；每连接8个待完成请求和4 MiB有界输出的传输保护正在实现，仍待背压/取消/超限验收。具体合同与最终同源结果以[验收矩阵](workspace-core-acceptance.md)为准。

历史客户端验证基线 `d9e33d3`：20项协议、11项浏览器检查及一个真实自然语言主动召回/来源/更正样本已有证据，重复召回问题保留。协议直接调用与自然语言样本分别记账；这些结果不认证新核心。新路径首期macOS/Linux，Windows继续支持原stdio，新核心暂不支持。新架构真实客户端、fresh-machine、完整精确提交CI和发布门仍开；本轮未替换部署、改全局配置或安装开机服务。

## dashboard 会话生命周期补丁（2026-10-15）

已在源码补齐 optional dashboard 端口占用后的等待接管；MCP 关闭时取消等待与监听，required 端口失败语义不变。三项隔离 stdio 回归覆盖正常 EOF 后接管、等待者退出无遗留监听和 required 失败。该补丁尚未替换已安装运行时；不代表多客户端实时聚合或进程退出根因已经解决。合同与限制见 [Live Desk](dashboard-live-desk.md#多个-mcp-会话共用端口)。

## 2026-10-15 Live Desk 修复

面板已移除硬编码冲突/写路径/操作链，导航、筛选、持久历史、只读健康检查与刷新控制接入真实数据；提供中英日、稳定选中/焦点、网络失败反馈及移动端详情定位。统计只代表当前进程缓冲，历史窗口最多 100 条。代码按 HTML/CSS/i18n/纯状态/网络/渲染/交互拆分。详见 [Live Desk 合同与隔离验证](dashboard-live-desk.md)及[本轮验收证据](acceptance-dashboard-2026-10-15.md)。仓库更新不等于部署进程更新；用户服务未由本次修复重启。

## 2026-10-15 体验与客户端验收

此前体验复测工具及实时网页已纳入仓库；实际 Codex app-server 在独立 HOME/mock 库完成 3 进程、2 次重连、17 次 MCP 工具调用、0 模型轮次。历史真实 90 轮的人工复核有 4 个问题轮次。此后历史桌面自然语言样本证据见顶部当前切片；修复后的付费复测和 fresh-machine 仍未验证。详见[本轮验收记录](acceptance-2026-10-15.md)。下方带日期段落保留历史证据。

## 2026-10-13 验收工具更新

三协议验收器已覆盖 Chat、Responses、Anthropic；新增请求/输入/输出限制与失败用量记录。回环 fixture 与真实调用分层，未知价格不声明美元硬上限；真实模型与用户客户端仍待验收。候选包增加更正重放、跨 namespace 直接查询/历史隔离及恢复后的历史/证据比对。当前步骤和证据口径见[本轮验收记录](acceptance-2026-10-13.md)。


更新：2026-10-10。当前定位：**local-first technical MVP，尚未正式发布 binary package**。历史运行时实现基线为 `9ba0050d3ffb9228b02e2d0895b4e76d11ead989`；本轮新增源代码绑定的本地 portable package 构建/解包验证，Windows wrapper 行为测试和修复，以及 schema7 全局 self-model 追加版本。包可在本地生成，不代表人工发布批准。

## 已实现

| 能力 | 当前合同与证据入口 |
| --- | --- |
| 原 standalone stdio 的32个工具 | [按用途索引](tool-reference.md)，以运行时 `tools/list` 的 schema 为准；新核心25允许/7拒绝属于顶部进行中切片 |
| Scoped Event / Claim / Episode / Reflection 浏览、lookup、修订与证据关系 | 显式 namespace、scope-first 查询、不跨范围扩大；[工作流](runnable-memory-workflow.md) |
| 原子 Claim 纠错与持久 request_id 回执 | [更正工作流](runnable-memory-workflow.md)、[反馈候选合同](memory-feedback-experience.md) |
| 持久反馈候选 | 提案、确定性验证、拒绝、事务内再验证与原子提交；来源标签不认证真实性 |
| 派生 FTS5 与短 CJK fallback | 可检查/重建；基于原文再次验证 scope 和字面匹配；不提供语义 embeddings |
| 有界任务上下文 | 完整 Claim/Event 优先、来源关联 Episode、可选诊断；[完整 JSON 字节合同](context-diagnostics.md) |
| Caller-owned 操作预算 | 仅 recall/context/reflection 三条路径；可观察 stop/error receipt；[非持久配额](caller-operation-budget.md) |
| 丰富 Episode 与版本化经验候选 | source-linked、inspect/reject/revise/rollback/activate；候选只供召回，不自动执行 |
| Schema 7 | [记录/观察时间与纳秒排序](temporal-metadata.md)、[独立 Reflection origin/affected scope](reflection-scope-history.md)、规范化来源关系 |
| 全局 self-model 追加版本 | 初始化/迁移真实 baseline、CAS、同范围来源 diff 与显式补偿回滚；[实验性单用户合同](self-model-versions.md) |
| 只读 scoped export | 有界、一致、同范围、无写日志；[交换而非备份](scoped-export.md) |
| 安全本地生命周期 | 显式 init/migrate、只读 doctor、结构读回、排他 writer admission、迁移前备份/恢复演练；[操作手册](database-operations.md) |
| 原生模型协议 | `openai-responses` / `anthropic` 非流式文本 decision 与 reflection proposal；保留 Chat Completions/OpenRouter；[离线合同与限制](provider-contract.md) |
| 内部模块整理 | [SQLite、MCP、自动反思私有模块边界](implementation-module-boundaries.md)，未新增框架或权限 |

新 Claim 有记录时间；历史未知时间保持 null。Reflection 的 origin 与 affected scope 各有职责，后者不是另一项可见性授权。旧 receipt/fingerprint 字节兼容性有迁移回归，不能通过迁移伪造历史或改变已成功请求的含义。

## 本轮交付：原生模型协议

新增显式 `openai-responses` / `[model.openai_responses]` 与 `anthropic` / `[model.anthropic]` 配置，分别使用 Responses 和 Messages 原生请求/响应；原有 Chat Completions/OpenRouter 配置不迁移。共享纯 prompt/领域解析与有界 HTTP 副作用层，拒绝重定向、无自动重试，错误与密钥 Debug 脱敏；只接受完整纯文本结果。未新增工具、streaming、vision、provider 托管状态或第二条 reflection 写路径。

本轮验证范围为配置、协议 fixture、错误/拒绝/截断和实际 MCP stdio 离线路径；具体命令见[测试指南](testing-guide-2026-03-24.md)。本轮未调用付费 API，没有新增 live-certified 声明。下方 schema7 与历史提交测试数字保持当时含义，不作为新增协议代码的通过数量；最终运行结果及精确提交 CI 需单独记录。

本轮本地验证完成：696 项 all-feature Rust 测试通过（0 failed/ignored/filtered），严格 all-target/all-feature Clippy、fmt、33 项状态同步、13 项 Python fixture、40 项包合同测试通过；实际二进制重启/纠错/备份恢复、temporal/scope/export 与固定离线 evaluator 均通过。独立审查发现的 nullable Chat tool_calls、底层隐式重试与 TOML 密钥片段问题已修正并回归。固定 evaluator 仍为 8/10，保留两个不支持的 paraphrase case；不视为真实模型效果。构建缓存空间不足后仅清理可再生 debug cache，使用 debug info 关闭、incremental 关闭的本地验证 profile 重新完整执行；没有跳过测试。精确提交的三平台 CI 结果记录在草稿 PR，不用本地 Linux 结果代替 Windows/macOS。

## 本轮交付：schema7 全局 self-model 版本

本地实现增加 append-only 全局序列、只读同范围 written-component history/diff、可选 expected-version guard 和需显式确认的补偿回滚。旧库迁移只捕获当前投影，不伪造过去的有效时间；漂移 fail closed。新全局写、Claim 效果、来源、版本和 receipt 同事务。新代码的本地测试/独立审查与精确提交 CI 必须分别记录，旧 schema6 数值不替代验证。

本轮源代码本地验证：673 项 all-feature Rust 测试全部通过（0 failed/ignored/filtered），严格 all-target/all-feature Clippy、fmt、独立只读审查、三组实际二进制离线闭环、9 项 Python fixture 与 33 项包合同测试通过。二进制/源码身份和范围见[schema7 阶段结果](plans/2026-10-10-schema7-results.md)。精确提交 CI 与发布仍单独验证，不继承历史 head 绿灯。

## 本轮交付：可追溯包与 Windows wrapper

- [Portable package](portable-packages.md)：从精确 Git commit/tree 构建本机原生二进制，包含校验信息、许可、隔离 mock 配置与使用说明；验证先检查归档和来源，再在无 Rust 的子进程环境完成真实二进制闭环。
- PowerShell wrapper：补齐 literal 路径、仓库根目录相对路径、调用者配置环境恢复及多余参数拒绝；真实平台行为由 `scripts/test-windows-wrapper.py` 在 Windows CI 执行。
- 这些检查是受控本地/CI 安装模拟；M1 真实用户客户端、真实 fresh-machine 十分钟验收、live provider 和人工 release decision 未因此完成。
- Windows 回归按真实原生进程检查参数透传；Python 数据库验证显式关闭连接，避免临时目录清理持有文件锁。Windows 的各 Python 检查分成独立 CI 步骤，后续成功命令不能掩盖前一项失败。
- 任何精确提交是否通过，以该提交 PR checks 为准；下方保留原实现基线，不将旧绿灯作为本轮结果。

## 2026-10-10 包与 Windows 精确提交证据

包/wrapper 阶段 `0ca8bccc64cadb3f1eae9d1a74bad602a1c60bc9` 的 [CI run 38035347484](https://github.com/yooyui/mcp-memory-ledger/actions/runs/38035347484) 三平台全部成功：Linux x86_64 与 macOS ARM64 各 634 项 all-feature Rust 测试，Windows x86_64 为 247 项 native Rust 测试；三平台均通过 13 项 Python fixture、40 项包回归和精确源码构建/无 Rust 解包闭环。Windows PowerShell 7.6.6 实际通过 12 项 wrapper 测试，其中原生参数/环境测试覆盖 20 种组合。

首轮 Windows 检查发现并修复了 doctor 参数绑定、SQLite 连接关闭和测试 PATH 基线问题；各 Python CI 步骤独立传播失败。以上是指定 host 与该提交的证据，不是所有 Windows/macOS 版本、真实 fresh-machine、用户客户端或正式发布认证。Schema7 后续 head 须重新执行自己的 CI。

## 验证证据（明确绑定提交）

实现基线 [9ba0050](https://github.com/CeauYoo/mcp-memory-ledger/commit/9ba0050d3ffb9228b02e2d0895b4e76d11ead989) 的 [CI run 37930021202](https://github.com/yooyui/mcp-memory-ledger/actions/runs/37930021202) 三平台成功：

- Ubuntu / macOS：各 633 项 all-feature Rust 测试。
- Windows：247 项 native Rust 测试，不代表全部 shell wrapper parity。
- 三平台：各 9 项 Python fixture 测试、installed-copy 重启/备份恢复、反馈/经验 evaluator、时间/范围/export 可执行工作流。
- 同源本地：551 项默认 / 633 项 all-feature Rust 测试，无失败或忽略；fmt、严格 Clippy、32 项 status-sync 及 diff 检查通过。

这些是实现基线的证据；后续提交必须单独核验 CI，不能继承旧 head 的绿灯。[测试指南](testing-guide-2026-03-24.md)提供复现入口，[最终 70 项核对](plans/2026-10-09-original-plan-final-reconciliation.md)记录源码身份和逐项边界。

固定离线代理指标 8/10，两个语义改写 miss 原样保留；50 个额外 query variants 单独计数，不能算作 60 项独立任务。测量同时记录延迟退化、额外上下文字节和存储成本；shared-host 合成数据不是生产容量保证。真实模型任务成功率、token 和费用收益尚未测量。详见[评估方法](evaluation-methodology.md)。原始运行输出保留本地；已发布历史证据不改写身份或数值。

## 部分实现 / 仍有限制

- 显式 scoped snapshot 已实现；省略 namespace 的旧接口兼容路径仍未统一隔离。namespace 不是 authn/authz。
- legacy 全局 self-model 治理仍是 experimental。`decide_with_snapshot` 只执行服务端 commitment gate，结果标记 `experimental_non_authoritative`，不是完整 policy verdict。
- identity/commitment 已有有界追加版本、记录边界 effective-time、同范围来源 diff 与显式补偿回滚；迁移前有效时间保持 unknown，不提供回溯调度或跨范围全局快照。
- 诊断是有界且不完整的可观察样本；缺失诊断不证明没有冲突，记录年龄不代表过期。
- 索引损坏可触发字面读取 fallback，但不保证受损 trigger 下写入仍健康；检查与重建是全库工具。
- 所有历史与成功回执 retain-all；没有自动 retention、tombstone 或 compaction。

## 尚未关闭的产品与研究门

1. 原M1冻结故事的完整对齐、新架构真实MCP客户端/自然语言闭环及fresh-machine安装证据；旧部署已有上方有限真实客户端样本，不再视为完全未测。
2. 正式发行包与人工 release approval、实际用户机器支持范围及 rollback note；本地构建器和 CI wrapper 验证属于实现基础。
3. 经单独授权和预算约束的真实同模型效果 / token / 成本实验。
4. 更广泛 self-model 生命周期政策、语义检索，以及需独立需求和权限的 remote/team/autonomy。

当前没有正式 Alpha/Beta/GA 或 production-ready 声明。启用 dashboard 仍只允许 loopback，daemon 仍 observe-only。

## 文档权威与历史

[路线图](roadmap.md)回答先后顺序，[唯一 active plan](plans/README.md)管理执行，[reality gates](product/follow-up-reality-gates.md)约束完成声明。

旧 schema、工具数、验证基线和分支状态已完整保留在[带日期的历史状态](project-status-history-2026-10-09.md)。历史 70 项 baseline、schema5 / schema6 阶段报告都是当时证据；以最终核对解释其增量，不覆盖或美化旧结果。
