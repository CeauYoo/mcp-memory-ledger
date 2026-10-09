# 数据库初始化、升级与恢复

当前 schema **6**。所有操作先确认 `AGENT_LLM_MM_CONFIG` 指向的配置，以及实际 `database_url`；使用绝对路径，并将开发/测试库与正式库分开。配置方式见[快速开始](quickstart.md)。

## 二进制只有四类命令

```text
agent_llm_mm init
agent_llm_mm migrate
agent_llm_mm doctor --read-only
agent_llm_mm serve
```

这里 `agent_llm_mm` 代表你实际构建的绝对 binary 路径（Windows 加 `.exe`）。没有 `backup`、`restore`、`install` 或 `--config` 参数；配置通过环境变量加载。省略命令等于 `serve`，省略 doctor flag 等于只读。`doctor --allow-bootstrap` 明确允许改库，不是常规诊断替代品。

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

## 备份与恢复到新路径

在支持这些 shell 脚本的 Unix 环境：

```sh
./scripts/backup-sqlite.sh /absolute/path/memory.sqlite /absolute/path/backups
./scripts/restore-sqlite.sh /absolute/path/backups/actual-backup.bak /absolute/path/restored.sqlite
```

备份文件名以脚本实际返回值为准，不照抄占位名称。restore 拒绝已存在的目标文件。恢复后创建指向新路径的独立配置，用同一 binary 做只读 doctor；旧版本备份可能仍需先迁移副本。核验完整性与查询后再手动切换客户端配置，保留原库，避免原地覆盖。

`backup-sqlite.sh` 检测到 `sqlite3` 时用 `.backup`；缺少它时会回退文件复制并提示先停写入者。不能把该 fallback 当作在线一致性备份；尤其不能在写入或存在未合并 WAL 时盲目复制主文件。优先安装可信来源的 sqlite3 或使用 SQLite backup API，并核验恢复结果。

Windows 的当前 native CI 用 Python `sqlite3.Connection.backup` 验证恢复；这不证明 Unix shell helper 在 PowerShell 中可直接运行。可运行[隔离 smoke](runnable-memory-workflow.md)理解流程，但它的合成库演练不是正式库运维命令。Windows 平台支持范围见[开发指南](development-windows.md)。

## 索引与只读导出

- `inspect_retrieval_index {}` 是全数据库只读深检；`doctor --read-only` 同样报告索引健康。
- `rebuild_retrieval_index {}` 是显式全库派生索引重建，没有 namespace 参数。仅在明确需要维护时调用。
- 查询可在缺少索引对象/SQL 错误时降级为字面 fallback；深层 postings 篡改不保证每次查询都能发现，受损触发器也可能阻止写入。
- `export_memory` 是同 scope、有界、一致的只读交换文档，不写日志；不是完整可恢复数据库备份，也不自动脱敏。完整合同见[scoped export](scoped-export.md)。

默认保留全部历史、来源与成功 request_id 回执。删除回执可能破坏重试正确性；本轮没有自动 retention、compaction 或 destructive repair。
