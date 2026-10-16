# 旧项目事实与可归属历史的显式接入（MVP）

## Schema 8 task-history boundary

Legacy adoption currently rejects a database containing task history because this provenance closure does not yet include task revisions/checkpoints. It does not silently ignore or rewrite that data. Existing registered workspaces and ordinary task operations continue unchanged. Full database backup/restore includes tasks; scoped memory export is not a task backup. See [task ledger](task-ledger.md).


此功能把既有数据库中的一个 `project/legacy` 域绑定到**新** workspace UUID；不是导入器、复制器、清洗器或通用旧库迁移。它不改数据库中的 namespace、事实 ID、来源、历史或 receipt，不修改既有 workspace 的默认 namespace，不调用 provider / reflection，也不写客户端配置。

## 先预览，再显式执行

使用同一现有 schema-current 数据库、私有 registry 和现有项目目录；所有路径必须是绝对路径。下列路径均为示例，不要直接指向未确认的正式数据。

```sh
agent_llm_mm workspace --registry /private/example/registry.json adopt-preview \
  --root /private/example/project --namespace project/legacy \
  --database /private/example/existing.sqlite

agent_llm_mm workspace --registry /private/example/registry.json adopt \
  --root /private/example/project --namespace project/legacy \
  --database /private/example/existing.sqlite
```

`adopt-preview` 在单一只读 SQLite 快照中返回 `inventory.supported`、`blocked_reasons`、已知账本表的全库计数和候选 namespace 计数；不创建 registry / core lock、不迁移、不生成授权 token。非法路径、非法 namespace 或无法读取数据库属于命令错误。合法但不支持的账本返回 `supported: false`，需要读取 JSON，不能只检查退出码。预览可与运行中的核心并存，因此不是后续状态的保证。这里的只读指账本记录与 registry 不变，不承诺文件系统目录项完全不变：SQLite 的只读 WAL 连接可能创建/维护 `-wal`、`-shm` 侧车。预览读取当前已提交但未 checkpoint 的 WAL 数据，不能使用会忽略这些数据的 `immutable=1`。

`adopt` 在执行时重新检查，不相信此前预览。它获取与核心共用的数据库 advisory lock，拒绝活动核心，然后获取 SQLite `BEGIN IMMEDIATE` writer reservation；在**同一连接、同一预约期间**核验 current schema 与完整来源闭包，保持预约直到 registry 原子替换完成。预览之后出现的新数据、并发 SQLite writer、已占用的 root / namespace 或失效 registry 都会使执行拒绝。数据库没有 SQL 写入。registry rename 前失败保留旧 registry；rename 后若目录 fsync 失败，错误会明确提示替换已经可见、持久性未确认，必须先 inspect registry，不能把报错当作未执行而盲重试。锁仅协调配合该协议的程序，不能阻止任意同 OS 用户进程。

执行使用平台核心锁：macOS/Linux 使用 Unix 私有锁，Windows candidate 使用当前用户私有 DACL 与共享锁合同。Windows 执行测试已进入原生 CI，但本轮 Linux 云端结果不代表 Windows 运行验收通过。

## scoped-history-v2 支持与保守阻断

支持既有 Events、Claims、evidence links、legacy `episode_events` 与原样保留的 operation logs / durable receipts。候选域必须至少有一条 Event 或 Claim，所有候选事实 owner 必须为 World；Unknown owner 拒绝。双向 evidence link、共同 episode 的所有成员及 feedback 元数据的结构引用必须完整且属于同一 World/project 域。feedback 检查会涵盖外域入边；不能解析的外域 feedback 也会阻断，因为无法证明没有引用。全库 feedback 盘点限 10,000 行 / 16 MiB，超限显式拒绝，不截断后放行。

在 current schema7 上支持归属明确的 reflections、reflection scopes/evidence、trigger ledger、feedback candidates、experience episodes/sources 以及 experience candidates 的**全部版本**与来源关系。候选域的所有来源与入边组成双向闭包，不能只信任 namespace 标签、当前版本或 JSON。具体检查：

- reflection 必须为 verified / legacy_unambiguous、唯一 origin、实际 effects 与 scopes 一致，JSON evidence 与有序归一化关系一致；所有 claim/event 来源存在。跨域/global effects 不会被重新归类为项目历史。
- experience payload 的 ID/namespace/time/version/status/kind/change/rollback 与关系行一致；episode/event 与 candidate-version/episode 来源集合一致；版本从 1 连续到 current，status/revise/rollback 与历史快照相符。
- feedback candidate 的行与 payload、replacement owner/namespace、target/evidence/commit 引用与 lifecycle 结构一致；committed 引用还须匹配其持久 reflection。此检查不重新证明历史报告的语义真伪，也不把旧 validation/receipt 当成新的授权。
- trigger 的 namespace/type/key/status 结构一致，evidence 与关联 reflection 同样纳入双向来源闭包。

其他项目完整且无关联的历史不会因存在而阻断本项目。缺失端点、Unknown owner、混域来源、归一化与 JSON 不符、缺失版本会对受影响候选域拒绝。unknown reflection、无法解码或含未知字段的 provenance payload 无法证明没有入边，因此仍明确全局拒绝；不会忽略 JSON 后只看关系表。全图盘点最多 100,000 行，所读取的全部字段（包括 IDs、namespace 和 keys）合计最多 16 MiB；先检查大小，再装载，超限拒绝。opaque summaries、identity payload 文本不进入图；其 global mutation 存在性单独检查。

baseline 之后的 global self-model versions **仍不支持**，因为无法在此合同内证明全局历史的项目隔离；不会声称完成通用旧库迁移。全局 self-model baseline、当前 identity/commitments 保持原样。未知扩展表、view 或非规范 trigger 也阻断；已知 temporal/self-model/retrieval triggers 必须匹配规范定义，派生检索索引保持原样。没有删除、重写或过滤历史来取得支持结论。

operation logs / receipts 是不变的历史记录，不作为新授权依据。opaque 日志文本不被解释成结构来源。此首片不会把其内容过滤、重写或重签，后续工具仍走正常 scope 校验和 receipt hash/replay 合同。

## Registry 与会话合同

- v1 registry 继续要求 `project/<workspace UUID>`，缺少 `active` 仍按 `true` 读取。
- 首次成功 adopt 把 registry 升为 v2，保留全部原条目，只添加一个新 UUID → 旧 project namespace 映射；只认识 v1 的旧二进制会拒绝 v2，不会默默误路由。
- v2 全表 project namespace 唯一。root、namespace 不得已有归属；不接受 self / world / user 域、空 project 域、空白/控制字符或任何 UUID 形态的旧 project 域。UUID 域为普通 workspace 注册保留。
- 不提供已有 workspace remap 命令；注册已使用 root 不会将旧域切换进来。
- 握手固定可信 `SessionGrant.project_namespace`；每次请求同时复核 workspace ID 与 namespace。管理文件被修改映射时，旧连接下一次读、写、tools/list 和 receipt replay 失败关闭，必须重建连接。
- 零 root 注册仍合法但不可连接；解绑不改变 active；启用仍需要有效绑定目录。原个人显式授权合同不变。

## 操作边界

采用本功能前，应先单独备份既有数据库与 registry。数据库备份/恢复不包含 registry；恢复时必须保留配对的 v2 registry。预览和执行仅证明给定数据库在该快照/预约时刻满足此闭包合同，registry 不绑定数据库文件身份；启动 core 时仍必须明确选择这份账本。

adopt 结束后，旧 standalone 客户端仍可能继续向相同旧 namespace 写入。此命令不会结束进程、修改其配置或自动切换客户端；管理员须显式停止/重新配置旧写入客户端，再按新 UUID/root 连接核心。数据库 writer reservation 只覆盖执行期间，不提供永久隔离。正式部署、真实客户端切换与用户数据验收仍是单独步骤。

## 代码验证

使用合成临时库运行 `cargo test --test workspace_adoption`，以及 `cargo test --lib interfaces::local_core::session::tests`。覆盖只读预览、复杂多项目历史的正向接入、全部 candidate 版本、原数据/receipt 字节不变、双向跨域来源和缺失/失配历史、未知 owner/未知归属/全局 self-model 拒绝、资源上限、锁冲突、预览后新增入边和旧连接映射失效。平台运行结果与全量回归须按实际执行记录，不由这些合同推断。

### 2026-10-16 助手 Linux 云端验证记录

- 本片代码基于 `a3c488bbe84d143c522b1f78ee5b6d17b8abd364`；未访问用户 Mac、真实账本或 provider 凭据。
- 同源 `cargo test --offline --all-features --no-fail-fast`：814 passed，11 failed。11 项全部为 `local_core` 的 AF_UNIX fixture 启动被执行器拒绝（`Operation not permitted`），不能报告完整集成全绿；由后续精确提交的 Ubuntu/macOS CI 补足。
- 旧 demo wrapper 默认 `target/debug` 路径在共享 target 下曾找不到已构建 binary；仅在 gitignored 测试目录建立指向共享构建目录的链接后，harness 2/2 通过，产品脚本与断言未改。
- 吸收上一批 Windows registry 路径词法等价/锁争用修补后，`cargo test --offline --all-features --lib --test workspace_registry --test workspace_adoption --test workspace_client_config`：73/73（lib 43、registry 14、adoption 11、client-config 5）通过。新增 Windows 专用端到端测试需 Windows CI，本机只验证了平台无关词法/重试合同测试。
- 最终串行 `cargo clippy --offline --all-targets --all-features -- -D warnings`、`cargo fmt --check`、`git diff --check` 通过。曾因两个 checkout 共享相同 crate 构建目录发生缓存元数据碰撞；停止并发、重建当前 crate 后通过，未为缓存错误修改产品逻辑。
- status-sync：33 项已完成计划与现实门匹配。以上不代表正式部署、用户旧库可接入、真实客户端切换或 Windows 运行通过。

### 2026-10-16 scoped-history-v2 增量验证

- 基于 `ef27d4358d721a1186592bdcd0ca8b2b3346c73e`，仅助手 Linux 云端合成临时库；不访问用户 Mac、真实账本、模型或凭据，不推送或替换部署。
- `cargo test --offline --all-features --lib --test experience_workflow --test workspace_adoption --test workspace_registry`：92/92 通过（43 lib、14 experience、21 adoption、14 registry）。复杂正例包括两个互不关联项目、四个经验候选版本与 rollback；拒绝例覆盖结构/JSON 两路入边、历史缺口、未知/混域/缺失来源、反思与 committed feedback 关联、动态 SQLite 类型、未知数据库程序及资源边界。
- 串行 `cargo clippy --offline --all-targets --all-features -- -D warnings`、`cargo fmt --check`、`git diff --check`、33 项 status-sync 通过。测试已为 Windows execution 与私有合成目录准备，但 Linux 通过不代表 Windows 运行通过；完整整合同源回归与精确提交跨平台 CI 另行记录。
- 后续 global self-model 版本仍明确拒绝；此增量不宣称通用旧库迁移、真实客户端自然语言验收、fresh-machine、安全认证或发布完成。
