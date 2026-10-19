# Slack：接入与线程边界

本文记录 Slack 官方平台约束及其对 PocketPilot 的设计影响。接入方案见 [Slack 设计](../design/slack.md)。平台事实核验于 2026-10-19；设计建议不代表现有实现或真实 workspace 已验收。

## 普通频道与原生 Agent 界面

**平台事实：**普通频道消息、线程、Block Kit 按钮和原生 Agent 界面是可分别采用的能力。Socket Mode 通过出站 WebSocket 接收 Events API 与交互 payload，无须公网 Request URL；当前不能用于公开 Slack Marketplace 上架。[Socket Mode](https://docs.slack.dev/apis/events-api/using-socket-mode/)

新建 Agent app 使用 `features.agent_view`。`assistant_view` 是旧体验，现有应用暂可保留，新应用不能再选择；从旧体验切换不可逆。新的 Agent 会话出现在 Messages tab 的线程时间线中，不应照搬旧的 Chat / History tab 设计。[Manifest](https://docs.slack.dev/reference/app-manifest/#features) · [迁移指南](https://docs.slack.dev/ai/migrating-to-agent-messaging/)

Agent 能力涉及 `assistant:write`；接收用户输入使用 `message.im`，检测用户打开界面使用 `app_home_opened` 且 `tab="messages"`，新的上下文变化使用 `app_context_changed`。原生停止和标题同步涉及 `agent_session_stopped`、`agent_session_title_changed`；新接入不应默认依赖旧的 `assistant_thread_started` / `assistant_thread_context_changed`。部分 AI 功能要求付费计划；开发者可以申请 sandbox。这些能力并不由启用 Socket Mode 自动提供。[开发 Agent](https://docs.slack.dev/ai/developing-agents/) · [迁移指南](https://docs.slack.dev/ai/migrating-to-agent-messaging/)

**设计建议：**首版采用内部 app、项目频道中的会话根消息和线程，线程内普通文本输入、按钮控制；原生 Agent UX 作为后续独立增量，按 workspace 计划和真实客户端验证。不要为了获得线程而先引入整套 Agent surface。

## Slash command、按钮与线程定位

**平台事实：**开发者自定义 slash command 不能在线程中调用，也不能在所有交互都发生于线程的 split view container 中调用。Slack 内置命令及 Giphy 是例外。[Slash commands](https://docs.slack.dev/interactivity/implementing-slash-commands/)

Message shortcut 携带源消息及频道上下文；global shortcut 不带消息上下文。Message shortcut 要求 `commands` scope。官方 shortcuts 指南还保留“从线程消息发起的 shortcut 不能直接向该线程发布响应”的限制，因此不能将 shortcut 的 `response_url` 当成可靠的线程回复入口。[Shortcuts](https://docs.slack.dev/interactivity/implementing-shortcuts/#limitations) · [Payload](https://docs.slack.dev/reference/interaction-payloads/shortcuts-interaction-payload/)

`chat.postMessage` 支持显式 `thread_ts`，应指向根消息，而不是线程内某条回复的 `ts`。[chat.postMessage](https://docs.slack.dev/reference/methods/chat.postMessage/)

**设计建议：**管理命令只用于频道主输入框；每个 session 根消息提供控制按钮。线程动作根据已存绑定定位；必要时从源消息归一化 `root_ts = message.thread_ts ?? message.ts`，再通过 Web API 显式回复线程。Shortcut 若作为补充入口，需要单独真实验收这条路径，不能仅凭携带源消息就推断响应会回到线程。会话键包含 workspace、channel、root timestamp；不能只存 `thread_ts`。

## ACK 与持久接收

**平台事实：**HTTP Events API 要求三秒内返回 2xx；交互请求也要求三秒内 ACK，否则用户会看到失败提示。按钮、命令、shortcuts、options 和 modal submission 都需要 ACK，modal 可在 ACK 中携带校验错误或视图更新。`trigger_id` 三秒过期且只能使用一次；已经 ACK 不会延长它的有效期。[Events API](https://docs.slack.dev/apis/events-api/#responding) · [交互响应](https://docs.slack.dev/interactivity/handling-user-interaction/) · [Bolt ACK](https://docs.slack.dev/tools/bolt-python/concepts/acknowledge/)

Socket Mode 要求对每个 envelope 使用 `envelope_id` ACK，以便 Slack 决定是否重试。该文档没有单独给出 Socket Mode 事件 ACK 超时数字；不能把 HTTP 头、HTTP 重试时序直接套在 WebSocket 上。交互与 `trigger_id` 的短时约束仍需遵守。[Socket Mode ACK](https://docs.slack.dev/apis/events-api/using-socket-mode/#acknowledge)

**设计建议：**将三秒作为入站处理预算，先做极短的权限检查、幂等登记与持久 inbox 提交，再 ACK；模型、RPC、历史读取和对外发送由 worker 执行。`views.open` 必须走快捷路径，在 `trigger_id` 到期前调用，不能进入普通慢队列。持久写入若无法及时完成，应明确失败并保留可重试语义，不能 ACK 后只依赖内存任务。

SDK 是否在业务 handler 前自动 ACK，必须按实际选用的 SDK/adapter 核实。自动 ACK 能改善超时表现，却可能在进程崩溃时造成“Slack 已确认、任务尚未落盘”；自己掌控 ACK 则必须保证存储延迟不拖过交互时限。这是实现取舍，平台不提供端到端 exactly-once 保证。

## 去重、消息变更与 bot 回环

**平台事实：**Events API 外层 `event_id` 标识逻辑事件，在所有 workspace 中全局唯一；Socket Mode 外层 `envelope_id` 用于传输 ACK，两者用途不同。Slash 和按钮 payload 不是 Events API event callback，不能假设也有 `event_id`。[Events API payload](https://docs.slack.dev/apis/events-api/#receiving-events) · [Socket Mode payload](https://docs.slack.dev/apis/events-api/using-socket-mode/#events)

`message` 有多种 subtype，包括 `bot_message`、`message_changed`、`message_deleted`、`file_share` 和频道服务消息；部分 subtype 为 hidden。编辑、删除事件的结构不同于新消息。[Message events](https://docs.slack.dev/reference/events/message/)

**设计建议：**

- `envelope_id` 负责 ACK，`event_id` 负责事件持久去重；重复事件仍 ACK，但不重复执行。
- 同一用户消息若经多个订阅路径进入，另以 workspace、channel、message `ts` 归一化业务输入，不能只靠不同事件的 `event_id`。
- 按钮用稳定操作 ID 与已存操作状态防止重复执行；不要只用传输 envelope 代表一次业务操作。
- 用户新输入必须显式筛选；忽略 bot 自己、其他 bot、编辑删除和系统消息，不把变更事件再次当任务提交。若允许附带文件的文字消息，显式处理 `file_share`，不要一概丢弃所有 subtype。
- 编辑删除可更新本地展示记录，但不自动撤回或重新执行已经提交的原生任务。

## 线程历史与 token 权限

**平台事实：**当前 `conversations.replies` 官方页面的 Scopes 表，明确为 **Bot token** 列出 `channels:history`、`groups:history`、`im:history`、`mpim:history`；User token 也列出这些 scope。不能继续沿用“bot token 无法读取公开或私有频道线程，必须 user token”的旧结论。读取仍受对应 scope、频道成员资格和 token 所见范围约束。[conversations.replies](https://docs.slack.dev/reference/methods/conversations.replies/)

核验注意：页面 Markdown 阅读器会漏掉 Scopes 表，本结论同时检查了官方页面原始 HTML。这里证明的是当前文档支持范围，没有用真实 token 验收任意 workspace。

该方法对于 Marketplace 与内部自建 app 为 Tier 3；自 2025-05-29 起新建且在 Marketplace 之外商业分发的 app / installation，适用每分钟一次、每页最多 15 条的限制。该方法页面当前说明已有 Marketplace 外安装不受新增限制；不应把不同分发模式概括成统一额度。[方法 Usage info](https://docs.slack.dev/reference/methods/conversations.replies/#usage-info)

**设计建议：**PocketPilot 原生 provider 历史仍是任务和结果的权威来源，Slack 只保存映射、输入账本与投递回执，不轮询 Slack 线程重建原生会话。若后续需要补取 Slack 消息，以对应频道 scope 的 bot token 开始验证，不因旧限制额外引入 user token 与用户 OAuth。

## Rust SDK：slack-morphism

**库事实（同日核验）：**`slack-morphism` 是社区 SDK，当前 GitHub release 与 docs.rs 都为 **2.31.0**，发布于 2026-10-10；许可证 Apache-2.0，仓库未归档。它覆盖 Web API、Events API、Socket Mode 与类型化 Block Kit。这些证据表明近期仍有发布，不构成未来维护承诺。[Release](https://github.com/abdolence/slack-morphism-rust/releases/tag/v2.31.0) · [docs.rs](https://docs.rs/slack-morphism/2.31.0/slack_morphism/) · [Cargo.toml](https://github.com/abdolence/slack-morphism-rust/blob/v2.31.0/Cargo.toml)

- 默认 feature 为空；`hyper` 启用 Hyper 传输、Socket Mode 所需依赖和 `rustls-native-certs`。后者使用 native roots 与 ring，默认 Hyper connector 调用 `with_native_roots()`；无需为了 Socket Mode 启用 `axum`。[Features](https://github.com/abdolence/slack-morphism-rust/blob/v2.31.0/Cargo.toml) · [Connector](https://github.com/abdolence/slack-morphism-rust/blob/v2.31.0/src/hyper_tokio/connector.rs)
- 2.31.0 使用 Tokio 1、Hyper 1、hyper-rustls 0.27 和 tokio-tungstenite 0.30。PocketPilot 当前为 reqwest 0.12 与 tokio-tungstenite 0.28；可共存，但会新增 SDK 的 Hyper connector 并预计保留两套 tungstenite 版本。不能把同为 rustls/native roots 等同于复用同一 HTTP/WebSocket client。[SDK dependencies](https://github.com/abdolence/slack-morphism-rust/blob/v2.31.0/Cargo.toml) · [项目依赖](../../Cargo.toml)
- 库未声明 `rust-version`，其 CI 使用浮动 `stable`，未证明兼容 PocketPilot 声明的 Rust 1.94。edition 2021 也不能代替 MSRV 声明；正式采纳前要用项目工具链实际解析和编译依赖。[CI](https://github.com/abdolence/slack-morphism-rust/blob/v2.31.0/.github/workflows/tests.yml)
- 当前文件 API 已包含 `get_upload_url_external`、`files_upload_via_url`、`files_complete_upload_external`，对应新的上传 URL → 上传字节 → 完成发布流程；旧 `files_upload` 标为 deprecated。[Files API source](https://github.com/abdolence/slack-morphism-rust/blob/v2.31.0/src/api/files.rs)
- Socket Mode 示例将 app-level token 包装为 `SlackApiToken` 并传入 `listen_for(&app_token)`，与 bot Web API token 分开使用；库有 `apps_connections_open` 方法。[Socket example](https://github.com/abdolence/slack-morphism-rust/blob/v2.31.0/examples/socket_mode.rs) · [Apps API](https://github.com/abdolence/slack-morphism-rust/blob/v2.31.0/src/api/apps.rs)

**设计建议：**它适合作为候选 SDK，以可选 Slack crate 包装类型与传输，应用层继续拥有权限、绑定和持久账本；实际选择还需核验 ACK 控制点、写请求重试和取消语义。本文仅核验 API 与依赖声明，未添加依赖、编译 SDK 或连接 workspace。
