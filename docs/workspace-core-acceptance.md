# 本机核心与工作空间验收矩阵

## 2026-10-16 Windows candidate 增量

Windows transport 新增本机命名管道，原生配置输出、核心数据库锁与显式 adopt 共用；协议/会话/退出排空与 Unix 共用。检查 DACL/owner、拒远端和未知权限、bridge 双向同用户身份验证，不防同用户任意进程或特权管理员。新建文件私有创建，不修已有 ACL。原生共享测试与包内二进制烟测已接线；本轮精确提交 Windows CI 结果仍待补充，此处不是通过声明。后文 Windows unsupported 描述均为旧阶段快照，不能作为本轮 candidate 结论。


## 2026-10-16 新功能与证据分层

当前增量已接入，**未提交，待最终同源验证**：workspace unbind/disable/enable；只读 inspect/status 与 client-config TOML；原生 backup/restore 到新路径。接口分别见[生命周期](local-workspace-core.md)、[配置输出](workspace-client-config.md)、[数据库恢复](database-operations.md)。这些操作默认保留账本与回执；registry 独立备份，恢复不自动授予权限或切换部署。

此前提交 `f8eb9ed` 的 [CI run 38709687717](https://github.com/yooyui/mcp-memory-ledger/actions/runs/38709687717) 已全绿：Ubuntu/macOS 各 794 项 Rust 测试、Windows 429 项，以及 Ubuntu 真实 Chromium 核心网页检查。Windows 仍不支持新核心；这些结果不覆盖本次未提交改动。这是此前核心集成的精确平台证据，不是新增生命周期/配置/原生备份的通过结果。独立安全审查与新核心真实客户端自然语言尚未通过，fresh-machine、真实模型效果/费用和人工发布仍开放。

本轮冻结源码的 Linux 云端检查：812 项 Rust 全功能测试去重后 801 通过、11 项 `local_core` 因 AF_UNIX `EPERM` 环境阻断；首轮另 1 项构建缓存路径问题经仅调整忽略的构建目录后重跑通过。all-targets/all-features Clippy、fmt、diff-check、status-sync（33 项）通过。详细原始计数与重跑口径见[测试指南](testing-guide-2026-03-24.md#2026-10-16-新功能的必要验证入口)。这不替代当前增量的精确提交 CI，也不关闭真实客户端或安全审查门。

本轮增加的验收范围：解绑旧路径/停用后旧连接下一请求与 replay 拒绝；enable 有效 root 条件；只读配置检查与 TOML 不变更配置或声称运行健康；current schema 一致快照、恢复新路径、不覆盖与 registry 独立性。此处不重新开启全部文档阶段，也不把 M1 外部门当作这些 M2 实用功能的实现前置条件。

以下 2026-10-15 矩阵与云端 EPERM 记录是历史阶段快照；其“平台 CI 待验收”已由上述 f8eb9ed 更新，但未关闭真实客户端或当前增量门。

历史快照：2026-10-15。该时点总体状态：**进行中，尚未完成新核心/桥验收，未部署**。本页仅记录[唯一 active plan](plans/README.md)的 M1.W1–W4 证据、缺口和退出条件，不另设执行队列。当前正式 Mac 二进制仍为 `d9e33d38c73c66cf7e13bb7e843b7c8283be1fb4`；本轮不替换它，不改全局客户端/API 配置，不安装开机服务，不停止用户正常进程。

## 范围与设计合同

目标是让多个客户端经轻量 stdio 桥连接一个本机核心，由核心统一持有数据库、业务 runtime 和 observer。桥使用管理端固定的 `--workspace-id` 与 `--workspace-root`；核心按私有 registry 精确校验二者。工作空间 ID 不随目录名字变化，符号链接规范化；克隆、worktree 或移动目录必须按明确绑定操作处理，不凭相同 Git remote 自动合并。

每次工具调用重新读取当前授权。默认只允许本项目域；个人 exact namespace 读取和写入分别显式授予。普通工具不能修改 registry，模型提供的 namespace、root、threadId、clientInfo 或 `_meta` 不能扩大权限。读/写动作、嵌套字段、实际来源记录和 receipt replay 均在授权边界内；同一个 namespace 的既有幂等合同保持兼容。

普通 workspace 会话隐藏并拒绝 `build_self_snapshot`、`decide_with_snapshot`、`run_reflection`、`get_self_model_history`、`get_self_model_versions`、`inspect_retrieval_index`、`rebuild_retrieval_index` 七个全局工具；未知工具默认拒绝。保留同范围 `build_task_context`、`get_memory` 与 `supersede_memory` 等常用闭环。核心运行模式关闭普通 ingest 的自动反思，不依赖客户端是否省略 hints。上述限制不是已实现可信 SnapshotHandle 或完整 scoped self-model 的声明。

核心默认不开放监控 HTTP 数据面。共享 observer 的全客户端聚合和管理员 HTTP 权限分别验证；管理员能力只在隔离 fixture 注入合成值，不创建生产凭据或与 workspace 桥共享权限。loopback、只读或前端过滤都不等于认证。现有 standalone stdio dashboard 合同与新核心模式分别说明，不能自动套用。

本轮边界是受控本机启动下防止模型参数越权及误路由；root/ID 不是凭据，不声称能够隔离任意同 OS 用户进程。已能读个人信息也不意味着可以防止调用方复制内容。export 的无持久日志/范围闭包合同保持独立，不机械套用全工具写日志。

## 当前已取得的证据

| 证据 | 结果 | 能证明什么 | 不能证明什么 |
| --- | --- | --- | --- |
| 当前 Codex roots 隔离探针 | **67 条断言通过**；2 组配置、10 个观察点、6 个 MCP 进程、16 次 roots 诊断、0 模型 turn；自有进程已清理 | 本机 `codex-cli 0.162.0-alpha.17.2` experimental app-server 的实际 MCP 能力、启动 cwd、会话移动行为 | 新核心/桥正确性、桌面 UI 项目切换、其他版本/平台、自然语言工具选择 |
| 正式 Mac d9e33d3 协议验收 | **20 项通过**，仅专用合成 namespace | 实际安装二进制上的写入、重连召回、来源、更正/历史及数据范围隔离 | 新会话权限授权、单核心架构或自然语言端到端 |
| 正式 Mac d9e33d3 网页验收 | **11 项通过**，三语言 × 五宽度共15组布局，0 JavaScript 异常 | 实际8787的导航、控件、持久历史更新、断线恢复及布局 | 全客户端实时聚合、管理员认证、新核心网页或长期稳定性 |
| 用户真实自然语言样本 | 主动召回/来源核对及后续更正在一个样本中通过；首轮3次召回含重复 | 该样本中客户端会主动使用已有记忆并执行用户更正 | 每次都调用工具、最多两次规则被严格执行、新 workspace 会话体验或真实模型效果基准 |
| 旧架构端口接管补丁 | 仓库已有3项隔离 stdio 回归证据，未部署 | 正常EOF后存活等待者可接管、等待者退出清理、required失败语义 | 单核心服务、统一实时聚合，或旧正式监听PID的退出原因 |

原始记录仅保存在本机，不把原始用户记忆、客户端轨迹或截图复制到仓库。当前本地证据入口：

- 仓库 `target/workspace-core/client-roots/REPORT.md`、`run-20261015-03/evidence.json` 与 `verification.json`。失败的早期探针尝试保留，67 条断言针对最终第三次记录。
- 部署目录 `verification/live-20261015/REPORT.md` 及其协议、浏览器、自然语言和问题清单摘要；部署目录位于 `<LOCAL_DEPLOYMENT_ROOT>`。
- 仓库 `target/workspace-core/plan-audit.md`、`authorization-audit.md` 是本次只读审计，记录当时源码基线及待实现合同，不是新实现通过证据。

### Codex roots 结论

initialize 未声明 roots；16 次诊断 `roots/list` 都返回空 roots，工具 `_meta` 没有 cwd/root。无固定 MCP cwd 时，新 A/B 会话各自进程继承不同启动目录；固定 MCP cwd 时，两者都在固定目录。更新既有会话 cwd 后，thread readback/通知已变，但 MCP PID/cwd 未变，也没有 roots 变化通知。

因此本期采用显式固定连接绑定。会话移到其他目录时不会自动重绑；应显式维护 registry 并重建连接。不能把启动 cwd、固定部署 cwd 或 threadId 当稳定且会动态更新的项目权限。诊断探针的 roots 请求不是对不支持客户端的日常轮询方案。

## 新实现退出矩阵

“进行中”仅表示代码工作已开展；“待验收”表示所列行为尚无本页认可的完整 fresh evidence。测试文件存在和隔离探针通过不能自动改变状态。每行完成须补源码身份、命令、产物和明确未测边界。

| ID / 对应计划 | 状态 | 必须验证的合同 | 当前依据与缺口 |
| --- | --- | --- | --- |
| W01 / M1.W1 注册与绑定 | **进行中，待验收** | 稳定ID、canonical root、重复注册、symlink、移动后显式绑定、不同clone/worktree默认不同；缺失/歧义/冲突拒绝，不回退user/default | `src/support/workspace_registry/` 与 `tests/workspace_registry.rs` 正在构建；实际客户端根目录能力已有上方独立证据 |
| W02 / M1.W1 个人授权与撤销 | **进行中，待验收** | A/B项目互拒；个人S默认不可读；显式R(S)不带W(S)；显式写与撤销分别测试；旧连接下一请求及旧receipt重放重新鉴权 | registry与会话策略代码在实现；需要完整真实进程闭环，不能只测集合包含关系 |
| W03 / M1.W1 单核心与桥 | **进行中，待验收** | 两客户端共享一个runtime/observer；客户端EOF不关核心；相同RPC id不串响应；单连接取消不影响其他连接；第二核心拒绝；断开不降级直连库 | 新核心/桥需隔离Unix进程证据；旧端口等待补丁不是替代品 |
| W04 / M1.W1 全工具边界 | **进行中，待验收** | tools/list隐藏七工具，直接调用仍拒绝；新增/未知工具默认拒绝；全部允许工具的scope/owner/来源和读写分类正确；拒绝早于存储/模型/回放 | `src/interfaces/mcp/session_policy.rs` 是纯策略切片；需与实际router清单和真实dispatch覆盖一致 |
| W05 / M1.W1 全局旁路与自动反思 | **进行中，待验收** | 普通ingest即使含触发条件也无模型/全局self-model写；snapshot/decision/history/version/index运维不可旁路；嵌套namespace、旧owner默认和ID引用不能扩大范围 | 旧snapshot即使显式scope仍读全局identity/commitments，因此本期明确拒绝，不以字段默认修补宣称安全 |
| W06 / M1.W2 统一可观察性 | **待验收** | 来自两个客户端的允许操作进入同一observer；可信session/workspace/correlation归因；成功召回、失败和拒绝可追踪；不记录原始密钥/正文；export保留无持久日志合同 | 正式OBS-02、OBS-04尚需新源码/回归解决；模型提供namespace不能充当日志身份 |
| W07 / M1.W2 管理员监控 | **待验收，仅隔离合成能力** | 默认无数据HTTP；显式启用无管理员能力须失败关闭；有/无/错误能力覆盖summary/events/detail/history/SSE/status/health；公开静态壳无数据、标签页内存登录/锁定/刷新/同源边界；workspace不能取得管理能力 | 不创建真实token、不开正式管理面；当前旧版只读loopback成功不证明新授权合同 |
| W08 / M1.W2 恢复可靠性 | **待验收** | 隔离核心正常/异常结束、旧socket处理、重连、同key回放、拒绝重复核心、损坏库fail-closed；backup→restore-new-path→readonly verify；原数据/receipt兼容 | 现有生命周期/更正回归可复用；正常EOF与事务注入不是完整crash/power-loss证明 |
| W09 / M1.W3 候选包与平台 | **待验收** | 新核心/桥在无Cargo/Rust运行PATH下执行；macOS/Linux各自精确提交证据；Windows旧stdio编译和原闭环不回归 | 新核心Windows暂不支持；不得把旧三平台包通过重标为三平台核心通过 |
| W10 / M1.W3 用户客户端 | **待验收** | 实际客户端固定workspace连接完成写入→重连→召回→来源→更正→旧/新历史→外域拒绝；个人授予/撤销；自然语言样本另记 | roots探针仅协议；旧d9e33d3自然语言样本不是新核心验收。正式切换需另确认 |
| W11 / M1.W4 文档与支持 | **文档/模板已补，最终核对待完成** | 最小安装/注册说明、workspace移动/撤销、兼容矩阵、数据/日志边界、安全支持与变更说明；不猜责任人/SLA | SECURITY/SUPPORT/CHANGELOG/协作与Issue/PR模板、workspace接入/切换边界已补；CODEOWNERS和私密报告渠道无已确认负责人，不编造支持SLA，不新增服务安装、SBOM或发布动作 |

此前 Mac 隔离证据限定在 `<LOCAL_WORK_ROOT>` 下。本次续建只在助手云端 `<CLOUD_CHECKOUT>` 工作，不访问或修改 Mac；使用隔离数据库、非正式端口和 mock，不访问现有凭据、不发收费请求、不重启或停止正常用户进程。云端 Linux 结果不能替代 Mac/Windows 或正式部署证据。任何scope泄漏、串响应、隐式模型调用、破坏历史/回执或恢复失败均阻断该切片继续扩张。

## 云端续建的定向证据

本次打包实现将 `verify-local-core.py` 及两种桥样例纳入精确源码包资产；Unix 解包验证调用包内验证器，失败阻断候选通过，Windows 明确标记新核心 unsupported 并保留旧 stdio 验证。新增 CI Unix 验证器安全回归入口。云端当前定向结果为 45 项 portable package Python 测试、17 项 local-core verifier Python 测试通过；这不是实际包构建/新核心运行通过。最终 Rust/全量测试数字须绑定最终源码后另记。云端运行环境的 Unix socket 创建被系统以 `EPERM` 拒绝；这不是核心运行通过或路径权限修复结果，不绕过环境限制。实际 Unix 进程/核心/候选闭环须由对应精确提交的 Ubuntu/macOS CI 提供，CI 保持失败即阻断并保存合成验证摘要。

一次全局配置只在客户端启动桥时提供正确项目 cwd 的情况下适用；当前连接不会自动跟随项目移动。个人权限管理已有显式 CLI，不等于网页授权界面。旧 `project/foo` 数据不自动改为 `project/<uuid>`；基础事实可按[显式接入合同](workspace-legacy-adoption.md)预览并 adopt 为新 UUID 对旧域的映射，高级历史继续明确阻断。保留备份，不静默改历史。注册表需要另行备份，`bind` 仍仅新增别名；2026-10-16 已新增显式 `unbind` 解除旧根，以及独立 `disable` / `enable`，不会删除账本数据。

管理员 CLI 与网页登录已接线：公开静态 shell 无运行数据，API/SSE 使用独立 Bearer，凭据只留标签页内存，锁定清空数据，刷新需重登，精确 Host/同源检查拒绝跨站。前端 Node 定向9项通过；真实 Chromium 在本云端启动时也被 Unix socket `EPERM` 阻断，不能把 Node 测试当浏览器实测。对应浏览器脚本 `--binary` 模式已接入 Ubuntu CI，使用真实隔离 Rust core；不带该参数只验证合成 Node API，两者不得混算；正式凭据和管理面未配置。

## 平台与验证级别

| 路径 | macOS | Linux | Windows |
| --- | --- | --- | --- |
| 现有 standalone MCP stdio | 既有实际部署/历史精确CI；新源码仍需回归 | 既有精确CI；新源码仍需回归 | 既有native与PowerShell wrapper证据；继续保持支持范围，不宣称全部GUI客户端 |
| 新Unix核心＋stdio桥 | 实现中；完整隔离/客户端/候选包待验收 | 实现中；实际平台CI/候选包待验收 | **暂不支持**，不能称三平台核心已通过 |
| 新核心管理员网页 | 仅合成能力隔离验收范围，尚未正式配置 | 需对应平台协议证据 | 新核心不支持 |

完成层级依次记录为：源码与合同 → focused回归 → 隔离进程闭环 → 独立审查/完整本地门禁 → 精确提交平台CI → 实际客户端 → 用户批准后的部署。协议直接调用与自然语言端到端分开；当前客户端实测只限定相应版本和环境。没有新机器、长期运行、模型质量或费用证据就保持未测。

## 历史问题与仍开放的门

| 项目 | 当前状态 | 后续归属 |
| --- | --- | --- |
| MCP-01 监听者退出后旧进程不接管 | 旧架构补丁未部署；新核心待验收 | M1.W1/W2；不能只用端口重试宣称单核心完成 |
| OBS-02 成功召回缺历史 | 已确认问题，修复待验收 | M1.W2；顺带按工具合同盘点新增feedback/experience路径 |
| CLIENT-03 重复召回 | 真实样本有3次含重复；未宣称已修复 | M1.W3的提示/工具说明与真实样本；提示规则不是服务端配额 |
| OBS-04 实时仅监听进程 | 旧实现能力边界 | M1.W2统一observer与客户端归因 |
| DIAG-05 旧PID退出原因 | 仍未知 | 新安全生命周期诊断不得倒推历史原因 |
| 原M1.3.1冻结故事 | 新证据覆盖若干子项，整体仍待对齐 | 原要求project/a 10条、project/b 5条；正式本机协议是3事件/3Claim及另1浏览器事件，真实自然语言为个人域，不能静默等同 |
| 原70项有限工程要求 | 大多已由final reconciliation收口 | 复用既有实现；保留可选语义检索、完整认知层等非目标 |
| fresh-machine、真实模型/费用、人工发布 | **未关闭** | 独立外部证据/授权；无Cargo PATH隔离运行不叫新机器验收 |
| 团队云、远程MCP、自治、破坏性生命周期 | **不在本轮范围** | 不因本机授权扩张引入；retain-all继续有效 |
