# 数据库初始化、升级与恢复

## Schema 8 task records

The current target adds versioned tasks, requirements, checkpoints and source edges. Existing schema 7 databases require explicit migration with the existing backup/rehearsal workflow; ordinary runtime opening never migrates. Full native DB backup/restore includes task records and durable receipts. Registry and Codex adapter state remain separate files and are not included in DB-only backup. [Task contract](task-ledger.md) · [adapter state](codex-task-context-adapter.md).


当前 schema **8**。所有操作先确认 `AGENT_LLM_MM_CONFIG` 指向的配置，以及实际 `database_url`；使用绝对路径，并将开发/测试库与正式库分开。配置方式见[快速开始](quickstart.md)。

## 二进制数据库命令

```text
agent_llm_mm init
agent_llm_mm migrate
agent_llm_mm doctor --read-only
agent_llm_mm serve
agent_llm_mm backup --db /absolute/path/memory.sqlite --output /absolute/path/backup.sqlite
agent_llm_mm restore --db /absolute/path/backup.sqlite --output /absolute/path/restored.sqlite
```

这里 `agent_llm_mm` 代表你实际构建的绝对 binary 路径（Windows 加 `.exe`）。`backup` / `restore` 只接受显式 `--db` 与 `--output` 文件路径，不读取配置、连接模型或启动服务；其他数据库命令的配置通过环境变量加载。没有 `install` 或 `--config` 参数。省略命令等于 `serve`，省略 doctor flag 等于只读。`doctor --allow-bootstrap` 明确允许改库，不是常规诊断替代品。

| 场景 | 操作与边界 |
| --- | --- |
| 不存在的新库 | `init` 原子争用目标路径，失败保留现场供诊断，不删除可能属于并发进程的数据 |
| 支持的旧库 | 备份后 `migrate`；服务不自动迁移 |
| current 库 | `doctor --read-only` 检查 canonical 结构、迁移 ledger、外键等；只看 version 数字不够 |
| 同版本结构损坏 | fail closed；不要手工提高 version、删表重建或静默“修好”原库 |
| 派生检索损坏 | 检查后显式 `rebuild_retrieval_index`；它不重写原始 ledger facts |

## 安全升级流程

1. 停止客户端/服务写入，确认目标库并准备独立备份位置。
2. 创建备份；保存原配置与旧 binary 以便回退，不在原库上试验恢复。
3. 对旧库运行 `doctor --read-only`，确认报告与预期一致，再显式 `migrate`。
4. 再次只读 doctor；检查版本、结构、行数/来源和关键历史，并用隔离副本验证 replay/查询。
5. 成功后重新连接客户端；失败保留原库、备份与诊断，按恢复流程处理。

迁移自身还会在首次写库前建立 backup anchor 和 restore rehearsal，持有 writer reservation，并在事务中进行 FK/行数/结构 readback。writer 入场 fail-fast，取得 reservation 后最多五秒 busy timeout 允许短暂 reader 释放。内置演练不替代外部备份政策或生产 DR。

Schema5 引入 feedback/experience/derived FTS；schema6 引入时间键、独立 Reflection 范围与证据关系。新字段是 additive，旧 Claim 时间保持 unknown/null。兼容回归保留旧成功回执、候选、反馈和 fingerprint 字节，不凭空增加观察时间或重复旧写入。详见[时间兼容合同](temporal-metadata.md)和[阶段迁移结果](plans/2026-10-09-schema6-results.md)。

Schema7 增加 append-only 全局 self-model 版本。旧库迁移只捕获当时 identity/commitment 投影为 migration baseline0，effective_at 保持 unknown；旧 audit/receipt 字节不重写。初始化 baseline 时间已知。serve、index repair 和普通读取不自动补版本或修复漂移；全局写/版本读发现 head 与投影不一致即 fail closed。详见[版本、回滚和结构合同](self-model-versions.md)。

## 备份与恢复到新路径

优先使用原生命令（Unix / Windows 同一参数合同，Windows 使用实际绝对路径与 `.exe`）：

```text
agent_llm_mm backup --db /absolute/path/memory.sqlite --output /absolute/path/backups/snapshot.sqlite
agent_llm_mm restore --db /absolute/path/backups/snapshot.sqlite --output /absolute/path/restored.sqlite
```

- SQLite online backup API 从只读源连接获取一致快照，包括尚未 checkpoint 的已提交 WAL 数据；不裸复制在线主库，也不对源执行迁移或 checkpoint。写入持续繁忙时最多等待约 60 秒后失败。
- 目标父目录必须已存在，目标文件及其 `-wal` / `-shm` / `-journal` 必须不存在；拒绝 symlink 路径、`..` 路径组件和覆盖。私有 staging 经 SQLite `integrity_check` 和现有只读 doctor 的 current-schema、canonical 结构、migration ledger、外键与 self-model 检查后，才以原子 no-clobber hard link 发布为单个 SQLite 文件。目标文件系统必须支持 hard link；不支持时安全失败，不降级为覆盖 rename。
- 当前仅接受此 binary 的 current schema **8**，不把同版本损坏结构或仅有 version 数字的文件当作有效备份。旧版/未来版库拒绝；用匹配版本 binary 恢复到独立副本，再另行显式迁移，不自动升级原库。相同 schema 但不兼容 canonical 结构也拒绝。
- 返回 JSON 包含输出绝对路径、schema 版本和 SHA-256，供记录与后续核对；不建立远端副本、调度任务或自动验证外部提供的 checksum。备份包含完整记忆数据，应保存在受控目录，不是脱敏 support bundle。
- 普通失败删除本次私有 staging，不删除既有目标或任何源文件。强制终止/掉电可能留下 `.agent-sqlite-backup-*` 私有目录；它不是成功产物。只有成功返回的目标才完成所有校验；在只读核验前不要手动使用残留临时文件。该实现不承诺掉电持久性或生产 DR。
- 恢复不修改配置、不自动切换客户端、不激活 workspace。恢复后另建指向新路径的配置，运行 `doctor --read-only`，核验记忆查询，再手动切换客户端，保留原库。

### Workspace registry 是独立管理状态

原生命令当前是 **数据库单文件快照**，JSON 明确返回 `registry_included: false`。不包含 workspace registry、配置、API key 或日志。Registry 保存 workspace stable ID 与权限映射，数据库备份本身不能重建这些映射。需要完整 workspace 恢复时，应停止 registry 管理写入，再单独安全保存实际 registry 文件（包括 stable IDs 与权限）；恢复时由管理员显式审阅和配置，不能把恢复数据库视为已授权/已激活 workspace。本轮没有跨数据库与 registry 的原子 manifest bundle，也不声称并发文件复制可提供一致 registry 快照。

旧 Unix `scripts/backup-sqlite.sh` / `restore-sqlite.sh` 仍是独立 helper。缺少 `sqlite3` 时旧 backup helper 会回退文件复制，必须先停写入者；不要将该 fallback 当作在线备份。原生命令没有这一 fallback，也不依赖 shell 或 sqlite3 CLI。跨平台实现不等于 Windows/macOS 真实 fresh-machine 证据；相关发布门继续开放。

## 索引与只读导出

- `inspect_retrieval_index {}` 是全数据库只读深检；`doctor --read-only` 同样报告索引健康。
- `rebuild_retrieval_index {}` 是显式全库派生索引重建，没有 namespace 参数。仅在明确需要维护时调用。
- 查询可在缺少索引对象/SQL 错误时降级为字面 fallback；深层 postings 篡改不保证每次查询都能发现，受损触发器也可能阻止写入。
- `export_memory` 是同 scope、有界、一致的只读交换文档，不写日志；不是完整可恢复数据库备份，也不自动脱敏。完整合同见[scoped export](scoped-export.md)。

默认保留全部历史、来源与成功 request_id 回执。删除回执可能破坏重试正确性；本轮没有自动 retention、compaction 或 destructive repair。

Schema7 的 self-model 补偿回滚不是 schema downgrade。schema6 binary 不能打开 schema7 库；若需回退旧 binary，恢复迁移前 backup 到新路径，用匹配旧 binary 只读核验后手动切换。旧 backup 不包含迁移后的新写入，不承诺无损逆转或原地降级。

## Database lifecycle file privacy

On Unix, `init` reserves a new database atomically with owner-only mode 0600 (further restricted by the caller's umask). Migration anchors are built in a private 0700 staging directory, synchronized, then published with an atomic no-clobber hard link. Filesystems without hard-link support fail safely before migration. Existing database permissions are not changed. Rehearsal copies and SQLite sidecars stay inside a private temporary directory, removed on both success and ordinary failure; process termination can leave a private directory for manual cleanup.

Windows lifecycle creation uses the current-user-only ACL helpers shared with the local core; Unix mode bits are not a Windows security guarantee. Native Windows execution remains a separate validation requirement. Existing parent-directory ACLs and existing databases are never silently rewritten.

Lifecycle path guards use SQLx's decoded filename, so percent-encoded URLs cannot reserve one path and migrate another. Internal rehearsal URLs escape spaces, `%`, `?` and `#`; in-memory URLs (including `mode=memory`) are rejected. Unix backup publication also synchronizes the parent directory before any original-database migration; a sync failure leaves the backup available and aborts migration.
Nested `file:` SQLite URI filenames are also refused: allowing SQLite to reinterpret another URI after SQLx decoding would make filesystem guards ambiguous. Pass an ordinary file-backed `sqlite://` URL instead.
