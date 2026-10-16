# 工作空间检查与客户端配置生成

这是本机 core/bridge technical MVP 的只读管理入口。不会连接核心、打开账本、启动服务、注册项目、改变权限或写入客户端配置。Windows candidate 可生成桥配置；绝对 `--socket` 文件路径映射为本机命名管道，不代表核心已经启动或该 candidate 已通过原生验证；原 `serve` 不受影响。

使用已配置核心的同一 registry 和 socket 绝对路径，不猜测另一个状态目录。binary 路径自动取当前运行的可执行文件；请使用准备长期保留的安装路径运行生成器。

```sh
agent_llm_mm workspace --help
agent_llm_mm workspace --registry /absolute/local-state/workspaces.json status
agent_llm_mm workspace --registry /absolute/local-state/workspaces.json inspect
agent_llm_mm workspace --registry /absolute/local-state/workspaces.json inspect --id UUID
agent_llm_mm workspace --registry /absolute/local-state/workspaces.json inspect --root /absolute/project
```

`status` 默认输出全局摘要；`inspect` 默认精确匹配当前目录，不向上搜索。两者可指定 `--id` 或 `--root`，不能同时指定。已移动或不存在的目录应以稳定 ID 检查。输出稳定 ID、项目 namespace、active、bound、各根有效性与原因、配置可用性、已配置与当前有效的个人读写授权。`active` 与 `bound` 独立：移除最后一个根不等于停用；停用也不抹掉授权。没有有效根、未绑定、已停用均不可生成可用桥配置。

`core_runtime` 始终明确标记未检查。配置可用不表示 socket 正在监听、核心使用同一注册表、数据库可用或客户端已接入；这些运行状态须另外显式验证。

## 固定项目 TOML

从精确注册且可用的项目目录运行：

```sh
agent_llm_mm workspace --registry /absolute/local-state/workspaces.json client-config --socket /absolute/local-state/ipc/core.sock --mode fixed
```

命令将完整 `[mcp_servers.memory-ledger]` TOML 打印到标准输出，包含当前 binary、socket、自动查出的 ID 和 root。可以 `--root /absolute/project` 明确选择；也可以 `--id UUID` 选择该注册项的第一个有效根（按注册顺序）。多个根指向同一个稳定项目身份。路径中的空格、引号和 Unicode 通过 TOML 序列化安全编码，不使用 shell 拼接。输出不包含模型配置、API key 或管理员 token。

将输出审查后再自行合并到客户端配置。命令不会覆盖已有 `[mcp_servers.memory-ledger]`、修改客户端信任或重连已有进程。切换项目时需生成正确固定配置并重建连接。

## 全局 registry-cwd TOML

```sh
agent_llm_mm workspace --registry /absolute/local-state/workspaces.json client-config --socket /absolute/local-state/ipc/core.sock --mode global
```

生成前也验证当前 cwd 是精确注册且可用的 root；不允许 `--id`/`--root` 隐藏选择。输出只固定 binary、socket 与 registry，桥每次启动按当时 cwd 精确解析身份。客户端必须正确传入项目目录；固定 cwd、安装目录、未知子目录、未注册目录均不能依靠它自动得到项目权限。客户端不能正确传入 cwd 时使用 fixed 模式。连接建立后身份不随目录变化，必须重连。

状态检查和配置生成不构成权限授予。停用、解绑、授权修改仍使用独立显式管理命令，见[核心合同](local-workspace-core.md)。验证：`cargo test --test workspace_client_config`。
