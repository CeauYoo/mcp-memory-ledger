# Codex deny-only 动作检查候选

状态：T09–T11 已整合并通过云端定向测试；精确提交三平台与真实宿主验收待完成。真实宿主 blocked/unverified；用户机器未安装。此文不是部署确认。

## 实际范围

原生 Rust `codex-actions` 在匹配的 PreToolUse 中读取当前 session/workspace/task、独立操作者 JSON 策略、任务快照，检查明确支持的 apply_patch 子集全部目标。结果为 pass_through、deny、needs_confirmation 或 unsupported。成功输出空 JSON，只继续宿主原生权限；后三类均输出合法的 Codex deny。不输出 allow/ask/updatedInput，不修改原生沙箱、权限或网络设置。

默认 patch_only 的配置 matcher 是 `^apply_patch$`，不会阻断其他开发工具，也不宣称它们受保护。显式 strict_local 使用 `.*`，未分类工具均被拒绝，会严重限制 shell、MCP、其他工具工作流；不要默认开启。

Codex 官方合同参见 [hooks](https://learn.chatgpt.com/docs/hooks)，2026-10-16 核验。文档/fixture 不能代表真实宿主支持。后续 write_stdin、hosted tools、特殊旁路和已运行进程不受本入口控制；hook 缺失、崩溃或被宿主终止可能放行。因此无法提供 hook-only 的故障必停承诺。

## 普通修订不重新批准

操作者 policy 的 epoch 与 task version 完全独立。默认 task_version_mode=latest_active：expected_task_version 是最小版本，不是每次修订的批准锁。每个动作实时读取最新 active 任务；普通要求修订/checkpoint 不改操作者路径/动作集合，不要求用户编辑 policy。第二次读取发现更高版本时自动按新状态重判；更旧版本拒绝，暂停/取消/完成拒绝新的受检动作。

只有明确选用 pinned 模式时，expected_task_version 才必须完全匹配，变化会要求操作者重新审阅。改变真实路径/操作范围、暂停/恢复策略、过期配置，需要操作者更新 epoch；普通 task.source、active、approval=true 或记忆引用不能完成这一步。

自然语言任务要求会读取但不会自动编译成权限规则。确定性检查只覆盖结构化策略、任务状态、版本和路径；不保证模型已理解或遵守所有新文本要求。策略变更时停止本次动作并要求刷新，不发执行票据。暂停/撤销不停止既有进程，不回滚已发生副作用。

## 一次性设置与日常使用

先按 [上下文适配器](codex-task-context-adapter.md)初始化/绑定任务。显式 workspace-default 可以帮助生命周期选择；动作入口使用已有 session binding，不凭 cwd 猜任务。

`codex-actions policy-preview --state ABS --registry ABS --socket ABS --session-id ID` 输出与当前绑定匹配的策略草案。草案 gate=paused，默认仅 add/update，打印本身不授权。操作者审阅后将 JSON 保存到经确认的非同步私有目录，设置 gate=enabled；Unix 目录 0700/文件 0600。该策略文件必须位于受检工作空间外。已有 policy 修改要增加 epoch。初始化/配置部署仍必须经用户另行确认，工具不会替你安装。

`codex-actions init --receipts ABS` 只创建一个新的私有检查账本，不创建凭据，不安装 hook。现有文件拒绝覆盖。

`codex-actions preview --state ABS --registry ABS --socket ABS --policy ABS --receipts ABS --existing-config ABS --output NEW_ABS_DIR` 一次生成：
- hooks.candidate.json：保留其他配置值的候选；可能重新格式化
- hooks.before.json：精确保留输入字节
- manifest.json：前后 hash、二进制/策略摘要、覆盖限制
- diff.txt：结构化差异
- rollback-plan.json：以候选 hash 为前提的撤回计划

输入须是 hooks JSON；不编辑 TOML。没有原文件时先以显式准备的 `{}` 草稿预览，manifest 的目标是该草稿；此模式不伪装成对不存在全局配置的安装/删除计划。重复 JSON 键、非整数数值等可能无法无损保留的输入拒绝，要求人工合并。输出目录必须全新；失败留下部分候选时不覆盖重试，换新目录。路径含符号链接的输出目录拒绝。程序不检测同步服务归属，预览明确显示未核验。

本批没有 apply/install 命令。部署需另行确认实际目标、路径、原生权限无变化、二进制、宿主版本、hook 信任和候选差异，然后由授权环境执行。hook 定义变更可能需要宿主重新信任，不能绕过。Windows command preview 明确 unsupported，不生成可能错误的 shell 引号。

## 撤回

`codex-actions restore-preview --manifest ABS --current ABS --output NEW_ABS_DIR` 验证原/候选快照 hash，与当前内容做 CAS 比较；相同候选可生成精确原字节的 restore candidate；已是原版则无需变化；后来编辑过则报告冲突，要求三方审阅，绝不覆盖。

命令仅生成计划/候选，不替换 host 配置。hash 不是认证签名；在真正替换前仍需再次核对文件身份、权限和当前 hash，备份并验证。撤回配置不撤回外部动作，不删除任务与经验，不改变原生安全配置。

## 路径与故障限制

- 支持有界、严格 patch grammar；未知形式/不完整 body 整次拒绝。多文件、移动的源/目标全部检查。
- Unix 拒绝路径越界、符号链接、设备路径/特殊文件、硬链接目标和已检测的跨设备目标；Windows checked writes 尚不支持。
- 检查与实际工具写入不原子；挂载/链接可能变化。同设备 bind mount、网络文件系统和同步目录归属未完整验证。不是 race-free confinement。
- 核心离线/坏 JSON、任务读取失败、策略损坏、回执冲突：当 hook 进程仍可返回时明确 deny。hook 自己不运行时无法替宿主保证阻断。
- 回执仅记摘要、决策、版本、时间，outcome 始终 unknown；不收集完整 patch、命令正文或 transcript。
- 同一 tool_use_id 改参数拒绝；重复同参数重新读取状态，不复用成功许可。策略 epoch、同 epoch 摘要、任务版本及观测时钟回退均检查。策略与任务版本下限分别独立提交，后续事件参数冲突或回执容量失败不能抹去已观察的新版本。最多 10000 条事件/策略/会话，满时拒绝新事件，不自动删除防重放记录；维护前需停用入口并另行审阅。
- SQLite 回执是可观察状态，不是凭据；同一 OS 用户仍可改配置、账本或执行器。没有声称不可篡改，也没有生成长期权限。

`codex-actions inspect --receipts ABS --session-id ID` 查看最近 20 条元数据；`codex-actions capabilities` 返回实际限制，不把文档支持写成真实保护。

## 整合验收

源码中包含领域、wire、patch、私有回执、preview/CAS 和候选写入测试。云端已运行整合编译、领域及适配模块单测、严格 Clippy；另补参数冲突与满回执时保留任务版本下限、并发版本不回退的测试。精确提交三平台和固定版本真实 host 故障试验单独记录。未完成这些证据前只能称“候选实现”，不能称“已部署的自动动作保护”。
