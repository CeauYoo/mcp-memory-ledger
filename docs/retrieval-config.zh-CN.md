# 主机检索配置（MVP）

默认任务续接使用 `literal`，本地 embedding 默认关闭。以下最小配置放在服务端 TOML 中，启用中文分词与有界增强召回：

```toml
[retrieval]
task_context_mode = "enhanced"
optional_deadline_ms = 2000
```

`task_context_mode` 支持 `literal`、`enhanced`、`hybrid`。可选召回总期限为 100–5000 毫秒，默认 2000。可选检索失败或超时保留已加载的完整强制要求；若无法完成最新版本复核，不能把旧快照当作可继续执行的授权。结果中的排名与适用性标签也不授予执行权限。

MCP 工具参数不能改写主机配置、端点、命名空间白名单或期限。显式 hybrid recall 需要主机启用本地 embedding；工具调用本身不能启用网络访问。本地模型配置、隐私边界与验证限制见 [local-embeddings.md](local-embeddings.md)。

可选别名仅适用于精确项目命名空间，进行单跳、有界扩展：

```toml
[retrieval.aliases."project/my-project"]
"数据库" = ["存储"]
```

测试：`cargo test --test task_resume_enhanced`。测试只使用临时数据库与合成输入，不连接真实模型或使用凭据。
