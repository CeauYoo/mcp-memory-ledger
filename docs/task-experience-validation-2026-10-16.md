# 动作检查、经验反馈与界面切片验证

日期：2026-10-16。基线为 `20d4037`，本文对应后续切片，不能继承基线或历史提交的 CI。未操作用户电脑、真实凭据或付费模型。

## 已实现范围

- T09–T11：受限 apply_patch 参数检查，独立操作策略，最新 active 任务读取，私有观测回执，配置与 CAS 撤回预览。检查不授予权限，不覆盖 shell/hosted tools/宿主未运行等旁路。
- Schema 9：经验采用和结果来源的不可变报告、精确版本/CAS/幂等、当前资格重查、可恢复的 use → Episode → pending 候选组合。原报告与当前资格分别展示，不宣称模型采用或因果效果。
- 原版 owner-only 页面：中英日任务/经验摘要、含任务项目发现、紧凑选择与宿主未核验状态。页面浏览不修改会话绑定；旧 observer 不暴露任务。
- 真实 Codex 验证脚本：固定官方 0.159.2，通过独立 HOME 与合成 Responses fixture 检查 startup/resume 请求及 hook 上下文；仅保存安全摘要。脚本准备与离线单测不等于宿主链路通过。

## 代码修复与审阅

1. macOS / Windows artifact 测试失败源于 fixture 未规范化根目录，而生产入口已有规范化。修复测试并补 Unix 路径别名回归，保留对未规范化根的拒绝；不放宽文件边界。
2. 独立审阅发现事件参数冲突或容量耗尽时，回执事务回滚可能丢失新观察的任务版本。策略和任务版本下限现在各自独立提交，最终回执事务仍重查，增加冲突/满容量/并发回归。
3. 经验记录、派生组合与新 owner 读取接口经过限定范围独立静态审阅；未发现待处理授权阻断。currently_reusable 仅表示当前结构资格，历史失败或不确定结果必须另读，不能当效果证明。
4. 宿主脚本初始化异常会关闭已创建子进程；完成事件绑定本次 thread/turn。四项 hook/请求证据缺一则失败。CI 产物仅扩展单一安全摘要路径，不上传临时 HOME、数据库或原始 RPC/模型请求。

## 本地验证

最终全量 `cargo test --all-features --offline --no-fail-fast`：929 通过、14 失败、0 ignored。14 项失败均在 local_core 测试创建 AF_UNIX listener 时收到 EPERM；没有其他失败。修复后的 lib 92 项与 CI 合同 4 项另行定向通过。严格 Clippy、Node 13 项与宿主脚本离线 Python 9 项通过；旧计划状态同步 33 项通过不代表新计划全部完成。

本云不支持测试所需 AF_UNIX listener，核心端到端测试在创建 socket 时 EPERM；不忽略失败、不禁用沙箱。相应结果交精确提交的三平台 CI。真实 Chrome 和 Codex 宿主链路以 Ubuntu CI 实际结果为准，Windows 新 hook 配置预览仍未支持。

## 后续

模型草稿、中文与可选本地向量检索属于下一切片。真实模型效果、30 个多轮体验场景和用户机器部署另行验收，不由本地机制测试替代。

## Native CI inventory correction

The exact `39c0f05` Ubuntu run found the local-core tool-name expectation missing the three newly registered experience-use tools. The other 14 local-core tests passed; macOS/Windows and later gates were not yet complete at correction time. The follow-up changes only this expected-name fixture and this record, preserving scoped denial assertions and production behavior. The source manifest above remains the historical `39c0f05` source snapshot; it is not a claim that its test hash describes this follow-up. Native verification must be checked against the follow-up commit.
