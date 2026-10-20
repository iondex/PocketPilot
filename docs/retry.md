# Codex retry 工具

`retry` 独立运行，持续监控配置允许连接的 Codex 实例，遇到可重试的最终失败后等待至少 60 秒，发送一条内容恰好为“继续”的新输入。新轮次再次出现可重试错误时，重新等待 60 秒。`serve` 不包含此功能。

## 启动

```sh
make check
make package
cargo clean
./dist/pocketpilot --config /absolute/path/pocketpilot.toml retry
```

`retry` 不启动聊天平台连接，也不要求配置 integration。它与 `serve` 共用根配置和 provider/workspace catalog，但使用独立连接、后台任务和 `[service].data_dir/retry/` 下的操作账本与进程锁。可以与 `serve` 同时运行；相同 data_dir 下只能启动一个 retry 进程。`serve` 没有 `--retry` 参数。工具只连接已有 app-server，不启动或停止 Codex daemon，不会通过新建 CLI 执行实例冒充原会话。

provider 仍需完成共享生命周期验证并设为 `control_verified`。主机的 `allowed_workspace_roots` 限定实际 cwd，发送前重新验证；不要求会话已经绑定聊天 topic 或登记为 workspace。

## 错误分析

Codex 的 [官方轮次事件](https://learn.chatgpt.com/docs/app-server#turn-events) 在 `turn/completed` 中区分 completed、interrupted、failed，失败包含 message、codexErrorInfo 和 additionalDetails。当前构建使用 Codex CLI 导出的 DTO；错误分类优先读取 `codexErrorInfo`，仅在缺失或为 `other` 时匹配原生错误 message。不会扫描用户或 agent 正文中的错误词，也不会记录完整错误正文或 additionalDetails。

| 分类 | 结构化 `codexErrorInfo` | 无代码或 `other` 时匹配的消息片段（不区分大小写） |
| --- | --- | --- |
| 并发限制 | 无专用代码匹配 | `Concurrency limit reached`、`concurrent request limit` |
| 服务过载 | `serverOverloaded`、`flexUnavailable` | `Our servers are currently overloaded`、`server is overloaded` |
| 限流 | `rateLimitExceeded` | `rate limit reached`、`rate limit exceeded`、`too many requests` |
| 服务端故障 | `internalServerError` | `internal server error`、`service unavailable`、`bad gateway`、`gateway timeout` |
| 连接/响应流故障 | 下述四种对象代码，按 HTTP 状态码分类 | `stream disconnected before completion`、`error sending request`、`connection reset by peer` |

对象代码包括 `httpConnectionFailed`、`responseStreamConnectionFailed`、`responseStreamDisconnected`、`responseTooManyFailedAttempts`，读取内部 `httpStatusCode`：429 归为限流；500–599 归为服务端故障；408 或缺失/null 归为连接故障；其他状态码不自动续轮。HTTP 状态码不会单独从消息文本提取。

上述可重试错误统一等待至少 60 秒后续轮。结构化代码的对应消息可能随 Codex 版本变化；表中消息片段是工具实际实现的后备匹配规则，不代表代码与消息一一绑定。

不自动续轮的代码包括 `contextWindowExceeded`、`sessionBudgetExceeded`、`usageLimitExceeded`、`unauthorized`、`badRequest`、`sandboxError` 和策略/未知代码。即使命中可重试代码，消息含 `usage limit`、`quota`、`billing`、`context window`、`session budget`、`unauthorized`、`authentication`、`api key`、`permission denied` 或 `credits` 时也不续轮。单独的 `Please try again later` 不触发重试。

服务端连接错误和 PocketPilot 自身 RPC 超时/断连有不同含义：前者必须属于一个确认失败的 Codex 轮次才能触发续轮；后者只重试读取。结构化的永久错误覆盖 message 中可能出现的可重试字样。

## 监控与发送规则

- 启动后分页读取 loaded 与持久化会话，持续发现新会话。监听 thread/turn 生命周期及错误通知，同时每轮扫描后最多等待 5 秒再补扫，防止通知不广播或丢失。最多同时扫描 4 个 provider，更多实例轮转；大量会话或慢连接会延长扫描周期。
- 只对当前已加载、空闲、最新轮次为 failed 的会话续轮。正在运行、等待输入、等待审批、手动中断、已关闭/未加载、归档或已正常结束的会话不会被自动恢复。枚举全部历史不意味着重放旧轮次；只检查最新轮次。
- Codex 的 `willRetry=true` 错误交由其自身恢复；原生轮次仍在运行时不另发输入。最终失败先进入 60 秒等待，重复通知不会重置计时。
- 等待期间用户继续、轮次变化、会话关闭/消失或配置权限变化会取消旧重试。发送前使用 retry 进程的会话锁，重新核对 cwd、控制资格、最新失败轮次和配置快照。原生 `turn/start` 没有针对上一失败轮次的原子比较参数，serve 或其他客户端在最后一次检查后仍可能并发修改会话。
- 每个 provider/thread/失败 turn 使用唯一操作来源，先持久化授权和 prepared 状态，再原子进入 dispatching。每个失败轮次最多发送一次“继续”；它是新一轮输入，不重发原始用户任务。
- 回执超时、断线或进程在写入中退出时，不重发该操作；沿用 `outcome_unknown` 恢复规则。重启工具必须再次显式启用；尚未发送的 prepared 操作重新读取当前失败并等待完整 60 秒，已经发送或结果未知的操作不重放。
- Ctrl-C/SIGTERM 退出监控，只关闭 PocketPilot 的连接。日志记录 provider/thread/turn、状态变化、错误分类、等待、取消与操作结果，不输出会话正文。

没有需要迁移的新表或列；retry 自己的数据库使用现有 schema 的 `operations` 表保存授权、输入身份、回执与未知结果。迁移机器时同时转移 data_dir/retry，避免已发送操作的去重记录丢失。原型账本的初始化和迁移规则仍见 [迁移指南](migration.md)。
