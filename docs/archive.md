# 历史归档说明

状态：`current index`

## 当前可直接阅读的历史

- [2026-10-09 整理前实现状态](project-status-history-2026-10-09.md)完整保留各阶段事实与当时未完成项。
- [v4 可用性基线](local-memory-usability.md)保留历史合同；当前教程见[快速开始](quickstart.md)。
- 阶段证据入口见[文档导航](document-map.md)；所有历史数字都绑定原提交，不代表当前 head。

## 归档位置

整理前完整仓库快照保存在本地 Git 分支：

```text
codex/archive/pre-mainline-reset-2026-07-10
```

基线 commit：

```text
48f6eca9db34377d61e646579078e6823ef65c6b
```

该分支保留整理前的完整源码、文档和历史材料。本轮没有推送归档分支，因此它目前只存在于本地仓库。

## 当前分支移出的内容

第一轮主线收束只移出历史和实验材料，不移动核心 runtime、正式 tests 或兼容入口：

- 文档树从 74 个文件收束到 36 个；移出的 40 份文档全部存在于归档分支；
- `docs/superpowers/` 下的历史 specs 与 execution plans；
- 2026-03 至 2026-06 的阶段工作快照和实现对照；
- 原始逐轮对话日志；
- 旧 productization / P1-P2-P3 请求记录；
- 历史 release note 与 demo report；
- 旧仓库发布、改名和 agent-team 审计材料；
- 根目录误提交的 `not-a-sqlite-url` SQLite 文件。

当前分支继续保留：

- 原始讨论的整理稿；
- 当前定位、状态、路线图和唯一 active plan；
- macOS / Windows / MCP 接入与测试文档；
- 仍与当前运行时和数据边界对应的 product contract；
- 全部核心 Rust runtime、测试、脚本与示例。

## 查阅归档

列出归档中的文档：

```zsh
git ls-tree -r --name-only codex/archive/pre-mainline-reset-2026-07-10 docs
```

读取单个文件而不切换分支：

```zsh
git show codex/archive/pre-mainline-reset-2026-07-10:docs/<path>
```

比较当前主线与整理前基线：

```zsh
git diff --name-status codex/archive/pre-mainline-reset-2026-07-10..HEAD
```

如需恢复某个文件，先确认它仍符合当前路线，再执行精确路径恢复；不要整仓 reset：

```zsh
git restore --source=codex/archive/pre-mainline-reset-2026-07-10 -- docs/<path>
```

恢复后仍需更新当前文档入口并重新运行状态同步与测试。

## 归档原则

1. 归档不等于删除历史；Git 分支是可读、可比较、可恢复的证据锚点。
2. 当前分支只保留会影响现在理解、运行、验证或下一步实现的内容。
3. 历史计划中的 checkbox 和“下一步”不再拥有执行权。
4. 从归档恢复内容时，必须先说明它解决哪个当前用户问题。
5. 不为每个实验再创建一个新总路线；实验只在独立、明确的候选分支中存在。
