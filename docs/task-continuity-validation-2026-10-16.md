# 任务续接首批验证记录

日期：2026-10-16。本文及对应源码摘要是首批提交 `20d4037` 的历史记录，后续切片另行记录。基础提交为规划提交 `9656fe8`；本文记录首批新增源码的云端验证，不继承基线 CI，也不代表用户机器已部署。最终三平台运行及候选包结果须绑定发布后的精确提交。

## 交付范围

- Schema 8 任务修订、稳定要求条目、候选隔离、checkpoint、原子幂等与 CAS。
- 6 个任务工具及 CLI；standalone 38 个工具、核心 31 个范围工具和 7 个全局拒绝入口。
- 必载要求不静默截断，task/checkpoint 快照与 optional recall 跨读分开标注。
- 原生 Codex adapter、显式默认任务选择、生命周期输出、版本回退拒绝、独立状态与回执。
- 有界本地产物点时核验；不访问远程 URI、不提升旧验证结论。
- owner-only 原界面只读任务摘要，中英日和真实状态；旧无凭据 observer 不暴露任务。

## 本轮结果

| 检查 | 结果 |
|---|---|
| Rust 全量 `cargo test --all-features --offline --no-fail-fast` | 878 通过，14 失败，0 ignored；14 项全部在创建 AF_UNIX listener 时遭云端 `EPERM`，不是全套通过 |
| 严格 all-target/all-feature Clippy | 通过 |
| 格式检查与 diff whitespace | 通过 |
| Node 前端合同 | 11 通过 |
| Python portable package / local core verifier / Windows verifier / experience / memory evaluation | 分别 47 / 17 / 14 / 36 / 13 通过，共 127 |
| 既有计划状态同步 | 33 项通过；不表示新任务全部完成 |
| 独立静态增量审查 | 已完成限定范围，未发现未处理代码阻断项；不是安全认证 |
| 真实 Chrome 任务界面 | 本云受 socket `EPERM` 阻断，新增场景交精确提交 Ubuntu CI |
| 真实 Codex 0.159.2 | 公开 schema 与 hooks/list 解析成功；生命周期注入受 sandbox 启动阻断，且 hook 未 trusted，未算通过 |

14 项 Unix 核心运行测试含新增“adapter → bridge → core”无模型端到端场景，必须在支持 IPC 的真实运行器验证。未把环境失败改成 ignored 或伪造通过。

首轮发现并已修正：schema 7 测试降级构造遗留 schema 8 对象、backup 测试误将新 current version 当作未来版本、demo shell wrapper 忽略 `CARGO_TARGET_DIR`。离线体验脚本约定固定 binary 路径，本地将同源已编译二进制复制至忽略的测试目录后复测；未修改其凭据输入与真实运行边界。

## 审查与修复重点

- 必载快照与整体非原子可选召回诊断正确区分，避免适配器拒绝全部正常上下文。
- stdout 部分写入失败不再追加第二份 JSON。
- 持久 task/checkpoint 的 ID、版本、范围、来源边与 payload 重新核验；损坏数据报告存储故障，不责怪查询参数。
- 同 session 的 fetch、输出和回执由既有 OS 锁串行化。
- 暂停或不完整的新版本也提高已绑定会话的版本下限，不能随后接受更旧 active 状态。
- Unix 文件打开使用 `O_NOFOLLOW | O_NONBLOCK`，配合 handle 与前后元数据检查，避免最终 symlink 与 FIFO 替换阻塞；读取总预算按实际字节累计。

## 尚未闭合

T01–T08 为首批实现，不是全部产品验收完成：没有可信用户事件导入；source 和 verification 是调用方报告。真实宿主接受和模型采用未证明，Windows 新 hook shell 配置预览未验证。运行中主动推送、动作检查和经验应用反馈属于后续切片。adapter DB 不在主库备份中；产物观察仅限支持的本地文件与点时身份，不提供抗同 OS 用户竞态保证。

对应合同：[任务账本](task-ledger.md)、[Codex 适配](codex-task-context-adapter.md)、[界面](dashboard-live-desk.md)。源码文件摘要见[首批源码清单](task-continuity-source-manifest.sha256)。原始云端运行日志未打包成用户诊断或上传用户数据。
