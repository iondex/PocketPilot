# Slack 接入方案

状态：可选 Slack 原型已实现并通过本地编译；真实 workspace 尚待验收。安装操作见 [Slack 安装与测试](../slack-setup.md)，平台依据见 [Slack 边界核对](../research/slack-platform-boundaries.md)。本文同时保留后续演进约束，未通过的真实场景不视为支持。

## 推荐形态

采用用户自建的内部 Slack app，PocketPilot 本机通过 Socket Mode 接收事件、slash commands 和 Block Kit 交互；REST Web API 发送消息、更新卡片和上传文件。无需公网 webhook、反向代理或独立 Node/Python 服务。官方确认 Socket Mode 支持这三类输入，见 [Socket Mode](https://docs.slack.dev/apis/events-api/using-socket-mode/)。

Slack 的 workspace 称为 team；下文“项目”指 PocketPilot workspace，避免两个概念混淆。

| 概念 | 建议映射 |
| --- | --- |
| Slack team | 安装边界；首版固定一个 team、一个本机授权用户 |
| PocketPilot 项目 | 手工准备的 Slack channel，安装者邀请 bot 加入并明确绑定项目 |
| 原生 session | bot 发送的根卡片及其 thread |
| 状态/标题 | 更新根卡片，包含原生 thread ID 与最近观察时间 |
| 输入 | 授权用户在已绑定 thread 内发送普通文字 |
| 输出 | 在同一根消息下回复，`reply_broadcast=false`，避免项目频道被过程输出占满 |

频道可以是私有频道。**Slack thread 继承整个 channel 的可见性，没有独立成员 ACL。** 首版限定普通非共享频道；Slack Connect、外部组织、多用户控制和企业跨 team 安装留到后续独立验收。

相较 Discord 的“每个 session 一个 channel”，Slack 原生 thread 更轻便，也不需要 Manage Channels 权限。首版由用户建频道并邀请 bot，不自动创建或归档频道。

## 用户路径

1. 安装者在项目频道执行一个 `/pp`，返回仅自己可见的项目/会话选择菜单。
2. 选择已有会话后，bot 在项目频道发布根卡片；打开只读，不 resume。若已有映射，返回原 thread 的链接。
3. “新建会话”打开 modal，填写标题并确认；建议沿用 Telegram 的空会话语义，首条任务在 thread 中另发。不要继承 Discord 原型中 `text` 立即触发首轮的含混交互。
4. 在 thread 回复文字，idle 时 start，active 时携带当前 `expectedTurnId` steer；NotLoaded 只因明确用户输入 resume。
5. 根卡片提供“状态 / 最近交互 / 导出历史 / 解除绑定”按钮。解除绑定仅停止桥接，保留 Slack 历史和原生 session。

自定义 slash command **不能在线程内调用**，所以不设计 `/pp-ask` 在线程中识别当前会话。`/pp` 只作为频道入口；根卡片按钮通过 `channel_id` 和根消息 `ts` 定位，thread 内普通消息通过 `thread_ts` 定位。平台来源见 [slash commands](https://docs.slack.dev/interactivity/implementing-slash-commands/)。

首版不需要 App Home；频道菜单够用后，可把跨项目目录放进 Home。Slack 新的 Agent 消息界面也可后续评估，但它涉及不同的事件与会话生命周期，不是启用一个开关就能替换当前项目映射。

## 安装步骤与权限

建议随实现提交一份无秘密 app manifest，安装者按以下步骤完成：

1. 从 manifest 创建内部 app，开启 Socket Mode、Interactivity，注册 `/pp`。
2. 在 Basic Information 创建 app-level token，授予 `connections:write`，得到 `xapp-…`。
3. Install to Workspace，取得 bot OAuth token `xoxb-…`。更改 scopes 后重新安装。
4. 建立专用测试 channel，邀请 bot 加入；在本机明确授权 team、用户及允许使用的 channel，再登记项目映射。
5. 两个 token 放入 Git 忽略的根 `pocketpilot.toml`；沿用现有凭据约定，不使用 `.env` 或命令行明文 token。

| 类型 | 首版权限 / 事件 | 用途 |
| --- | --- | --- |
| App-level token | `connections:write` | 取得短期 WebSocket URL |
| Bot token | `commands` | `/pp` 入口 |
| Bot token | `chat:write` | 根卡片、thread 回复、状态编辑 |
| Bot token | `channels:history` + `message.channels` | 接收 bot 已加入的公开频道内普通 thread 回复 |
| Bot token | `groups:history` + `message.groups` | 接收 bot 已加入的私有频道内普通 thread 回复 |
| Bot token | `files:write` | 单文件完整历史导出/长输出 |

若部署只支持私有频道，可以只保留对应 history scope 和 event。不通过 `app_mention` 承载全部输入：它不能替代 thread 内每条普通回复。首版不需要 user token、`chat:write.public`、`channels:manage` 或全量用户/频道扫描权限。额外使用 `channels:read` / `groups:read` 调用 conversations.info，核对 bot 成员资格、归档与共享标记。`auth.test` 验证安装身份后，应固定 app/bot/team，不能因换 token 静默继承另一安装的映射。

`[slack]` 保存 app_token、bot_token、app_id、team_id、owner_user_id 和 allowed_channel_ids；字段及合成值只维护在 [配置示例](../../pocketpilot.example.toml)。首次打开项目会持久登记频道/项目映射，同一频道不能静默换到其他项目。账号和范围配置改动需要重启。

Socket Mode 当前不能上架公共 Slack Marketplace；对用户自建内部 app 合适。若以后要提供公开多租户产品，另行设计 OAuth 安装、HTTP 接收与分租户凭据存储，见 [官方限制](https://docs.slack.dev/apis/events-api/using-socket-mode/)。

## Rust 模块与现有复用

新增可选 `slack` crate，实现现有 `engine::integrations::Integration`，与 Telegram / Discord 在同一个 `serve` 进程装配。复用 `Context` 的 ConfigFile、Store 和 Connections，以及 `engine::operations::execute_dispatched`；授权、输入时效和 dispatch 原子认领仍由平台负责。

```text
app ── slack::Slack ── engine::integrations::Context
                       ├─ Connections：共享连接、每会话锁
                       ├─ operations：写入回执与 OutcomeUnknown
                       └─ Store：公共账本 + Slack 自有表
```

沿用平铺模块：`config.rs`、`api.rs`、`socket.rs`、`routing.rs`、`commands.rs`、`input.rs`、`binding.rs`、`storage.rs`、`observation.rs`、`delivery.rs`。优先使用 [slack-morphism](https://github.com/abdolence/slack-morphism-rust) 承担类型化 Web API、Socket Mode 生命周期和 Block Kit，平台模块封装其类型，避免手写控制帧、心跳与重连。仍保持单一 Rust 进程，不需要 Axum 或额外 HTTP 服务。

### slack-morphism 的采用边界

核对版本为 `2.31.0`（2026-10-10 发布）。实现时在 Slack crate 启用所需 `hyper` feature、固定版本并由 Cargo.lock 管理；不为客户端接入开启 Axum。它内部使用 Hyper / rustls 和 tokio-tungstenite，不是直接复用本项目 reqwest client；新增传递依赖和二进制体积需在实现时评估。

- **接收回调只做快速本地工作。** 此版本 listener 在业务 callback 返回后构造 ACK，适合先持久 inbox，再成功返回。持久化失败时返回错误，并设置 error handler 的非成功结果以避免确认尚未收下的已解析事件；不得照抄示例中所有错误都返回 OK 的写法。未知/畸形 payload 的解析失败分支会另外 ACK，需要按部署需求记录脱敏错误和可见告警，不能把它当成完整离线 inbox。
- **保留 PocketPilot 的账本。** callback 的成功只是接收成功；不能在其中等待 agent 执行。库不会代替授权、event_id 去重、交互 nonce、会话锁、绑定 revision 或 outbox。多 Socket 连接也不能提供 exactly-once。
- **显式配置限流和重试。** throttler 默认未启用；启用时为 token 绑定 team ID。建议 `max_retries=0`，由持久 outbox 按 `Retry-After` 调度已明确拒绝的请求；未知发送结果不重试。源码中的自动重试针对 RateLimitError，不应误称它默认重试所有失败。
- **使用新版文件流程。** 已有 `files_get_upload_url_external`、`files_upload_via_url`、`files_complete_upload_external`；每阶段的 file ID 和回执仍由 PocketPilot 持久化。
- **约束库日志。** 此版本 WebSocket 实现在 debug/trace 级记录连接 URL 和原始收发正文；接入时须将 `slack_morphism` 传输日志限制为 warn/error，并在应用边界记录脱敏的 ID/状态。不要沿用示例的完整事件 `println!`；需把该 target 纳入现有 telemetry 的强制过滤。

依据：[Socket callbacks 与 ACK 源码](https://github.com/abdolence/slack-morphism-rust/blob/v2.31.0/src/socket_mode/clients_manager_listener.rs)、[限流与重试指南](https://github.com/abdolence/slack-morphism-rust/blob/v2.31.0/docs/src/rate-control-and-retries.md)、[HTTP retry 实现](https://github.com/abdolence/slack-morphism-rust/blob/v2.31.0/src/hyper_tokio/connector.rs)、[WebSocket 实现](https://github.com/abdolence/slack-morphism-rust/blob/v2.31.0/src/hyper_tokio/socket_mode/tungstenite_wss_client.rs)。依赖已加入可选 Slack runtime 并通过本机编译和本地 fixture；尚未连接真实 Slack 验收。

实际平台地址为 `(app_id, team_id, channel_id, root_ts)`。所有 Slack ID 和 `ts` 保持字符串，尤其不能把 `ts` 转为浮点数。绑定同时保存 owner、project/provider/thread、cwd、endpoint 指纹和 revision；一个容器不能被改绑到另一个原生会话。

建议持久表包括频道/项目映射、session bindings、入站 inbox、交互 nonce、outbox 和首次根消息创建记录；SQL 留在 Slack crate，按当前约定维护根 `schema.sql`。采用两平台已验证的领域模型，而不是让 Slack 直接调用 Telegram 的 Service 或 Registry。

## 接收、ACK 与去重

Socket Mode 使用 `xapp` 调用 `apps.connections.open` 获得短期 URL，收到 `hello` 后处理业务；收到 warning / refresh_requested 后重新获取 URL，并有界退避重连。URL 和 payload 中的 `response_url` 都是敏感信息，不入日志。

读事件先做本地校验和有界持久化，再及时 ACK，慢 SSH 和 agent 执行放到 worker；交互 ACK 必须在三秒预算内完成。不能等 agent 结果再 ACK。ACK 只是确认收到，不表示原生操作已成功；若 SQLite 未及时完成，不能声称请求已可靠接收。

打开 modal 的 `views.open` 必须走快速路径：`trigger_id` 三秒过期且只能使用一次，不能等后台 SSH 队列处理。Socket Mode 事件文档没有单列三秒 ACK 超时；三秒是本方案统一采用的接收预算，交互和 trigger 的时限则是平台明确要求。

`envelope_id` 用于传输 ACK，业务事件以 `(team_id, event_id)` 去重；普通消息再记录 channel/message ts，防止交叉订阅或派生事件重复处理。按钮/modal 使用绑定身份及 revision 的一次性 action nonce；不能只用文本哈希去重，否则相同的两次明确输入会被错误合并。

仅接受授权用户在已绑定 root 下的新文字消息；过滤 bot、自身输出、应用消息、非目标 team/channel、未绑定 thread，以及 edit/delete 等 subtype。消息编辑不自动形成新输入，删除 Slack 消息也不等于取消原生任务。纯附件不启动任务，带 caption 时明确按文本处理。

断网补收、过期输入、重启时仍 pending 的写入变为待确认草稿，fresh 确认后重新检查当前绑定和 turn。已经 Dispatching/OutcomeUnknown 的原生操作不自动重发；重启只恢复观察，不自动 resume。

## 输出与历史

Provider 历史是事实来源；Slack 是展示与输入界面。不要轮询 `conversations.replies` 来重建原生 session，避免依赖平台保留策略和历史 API 限速。当前官方对内部自建 app 和商业分发 app 的历史限速不同，不能把其中一类配额写成全平台常量，见 [rate limits](https://docs.slack.dev/apis/web-api/rate-limits/)。

入场发送最近交互短预览；再次打开不重播历史。根卡片仅在状态/标题变化时 `chat.update`，过程文本持久合并后发到 thread，最终结果可单独成条。所有 thread 共享所在 channel 的发送预算，不能按 session 各自发满；HTTP 429 遵守 `Retry-After`，已知未接受的限流请求与未知发送结果分别处理。

Slack 官方建议消息 text 保持在 4000 字符内，超过 40000 可能被截断；Block Kit 各块还有独立上限。初版设更保守展示预算，超长内容导出为一个 `.md` 文件，不能把截断文本标为完整。上传用 `files.getUploadURLExternal → 上传二进制 → files.completeUploadExternal`，带 `channel_id` 和根 `thread_ts`；不使用已淘汰的 `files.upload`。完成步骤最多调用一次，未知结果先核对，见 [消息 API](https://docs.slack.dev/reference/methods/chat.postMessage/) 与 [文件完成 API](https://docs.slack.dev/reference/methods/files.completeUploadExternal/)。

Slack mrkdwn 和通用 Markdown 不同；当前短正文使用类型化 plain_text block，fallback 转义 & / < / >，长正文保留原始 Markdown 附件。尚未实现通用 Markdown 到 mrkdwn 的格式转换。特别是 `<@用户>`、`<!channel>`、`<!here>` 等语法不能直接透传。`parse=none` 本身不能替代 mentions 处理；默认关闭链接及媒体 unfurl。短消息提供无障碍 fallback 文本。

根消息创建、普通消息发送和附件最终分享都有“已成功但回执丢失”的窗口。先记录 dispatch，再存平台回执；未知结果不自动再发。编辑已知 message ts 可以重试同一内容，但不能因此推导创建消息也具有 exactly-once 保证。

## 实现与验收

| 阶段 | 内容 | 门禁 |
| --- | --- | --- |
| 1 | manifest、身份检查、Socket Mode、`/pp` 只读目录 | 真实 team 接通；错误用户/频道拒绝；三秒 ACK 不受 SSH 阻塞 |
| 2 | 选已有会话、根卡片/thread、只读观察 | 打开不 resume；同会话复用 root；长输出与状态可用 |
| 3 | 普通 thread 输入、新建 modal、共享写账本 | idle/active/NotLoaded 路径；重复事件/按钮不重复执行 |
| 4 | 故障、限流、范围变更与重启 | unknown 不重发；恢复草稿再确认；解绑/撤权后停止新投递 |

当前回归包含本地 Web API + SDK Socket fixture、SQLite 和共享 Codex fixture；覆盖的行为以对应测试为准。完整验收还应包括：ACK 后丢业务回执、持久化前崩溃、平台重投、跨连接重复事件、channel 共享限速、bot 回环、旧按钮、root 删除、权限变化和上传三阶段故障。与 Telegram/Discord 同时绑定一个测试 session，验证共享 actor，而非复制一套执行器。

真实验收至少使用一个专用私有频道：安装与邀请 → 目录 → 打开 → 新建 → thread 续轮 → active steer → 附件 → 正常退出桥接 → 原生任务继续 → 重连不重放。Slack Connect 和多用户不是上述结果自动覆盖的场景。

## 当前原型限制

- Socket Mode 保持一个连接，入站业务队列上限 512、出站 pending/dispatching 上限 2048；一个命令 worker，最多八个独立观察 worker。每个频道的所有 thread 共用两秒一次发送预算，尚无可配置 burst 合并策略。
- 快速回调在持久接收后 ACK，按钮消费和入站登记使用同一 SQLite 事务。重连使旧写操作控件失效，积压文字变成待确认草稿；处理中断的请求只提示未确认，不提供自动重发按钮。
- 根卡片的只读控件保留较长有效期；解除绑定和创建使用新的短期确认。过期菜单需从频道 `/pp` 重新打开。
- 根消息创建、附件完成或原生输入结果未知时不自动重试。删除根消息、资源 unknown 和频道改绑尚无核对/替换 CLI，应人工检查；不删除原生会话。
- 文件上传预算 8 MiB。上传中间 file ID 保存在投递记录，完成步骤只调用一次；失败/未知不自动重传。纯附件不会启动任务，带 caption 仅传文本，不读取文件内容。
- 不支持 Slack Connect、跨 team/企业安装、轮换 token 自动刷新、多用户控制、原生 Agent surface、原生取消/审批代答。
