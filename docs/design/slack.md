# Slack 私聊接入

PocketPilot 是用户自建的内部 Slack App，只接受配置 owner 与该 App 的一对一私聊。安装、原地升级和 CLI manifest 更新见 [安装指南](../slack-setup.md)，公开配置见 [示例](../../pocketpilot.example.toml)。代码具备本地协议、账本和跨平台回归；真实 App 需完成权限升级后按指南验收。

## 入口与会话映射

| 概念 | 映射 |
| --- | --- |
| Slack team / app / bot | 固定安装边界，不随 token 变更静默继承其他身份 |
| 配置 owner | 唯一操作者，其他管理员和用户不自动获权 |
| owner DM | 启动时以 bot token 调用 `conversations.open(users=[owner])` 得到唯一 D… 会话 |
| UI workspace | 配置 Host，即本机/AWS 等执行环境，作为第一层入口 |
| PocketPilot 项目 | 已选环境下的目录，对应 engine 持久化 Workspace，同一个 DM 支持多个项目 |
| 原生 session | DM 中的一张根卡片及其 thread |
| 用户任务 | owner 在已绑定 thread 内发送的新文字 |
| 主输入框文字 | 打开项目菜单，不作为任何原生会话的输入 |

不需要 channel ID、频道邀请或 slash command。普通/私有频道、群聊和其他 DM 全部拒绝。后端同时核对 app、team、owner、DM ID；消息事件还必须标记 `channel_type=im`。交互按钮保存 channel/owner/root/binding/revision 的一次性 nonce，modal 延续原 nonce 上下文，不能用另一会话或另一用户的按钮替代授权。

## 用户路径

1. 在 Apps → PocketPilot → Messages 发送“菜单”或普通文字，获得工作空间选择。
2. 先选择 workspace，再从该环境的项目下拉框选择目录，最后选择已有 session；打开只读，发送/复用对应根卡片，不 resume。
3. 在卡片 thread 内回复任务。idle 时 start；active 时携带 expectedTurnId steer；NotLoaded 仅因明确用户输入 resume。
4. 新建按钮打开 modal，填写标题并确认，创建命名空会话。第一条任务在 thread 内另发。
5. 状态、最近交互和解除绑定均通过根卡片按钮。解绑保留原生任务、会话和 Slack 历史。

每个 session 绑定持久保存项目、provider/thread、cwd、endpoint 指纹和 revision。项目属于每个 session 绑定，不再存在“一 DM 一项目”的锁定。再次打开相同原生会话复用原 thread，不会更换绑定身份。

## 权限与连接

运行配置只有 app_token、bot_token、app_id、team_id、owner_user_id 和可选 proxy。app-level token 使用 `connections:write`；bot scopes 为 `chat:write`、`im:history`、`im:write`、`files:read`。Messages Tab 启用且可写，订阅 `message.im`；Socket Mode 和 Interactivity 开启。

`Api::connect` 通过 auth.test 核对 team/bot 身份，再取得 owner DM。它不发送欢迎消息、不创建任务。后续所有 API 输出均限定这个 DM，无需频繁查询频道成员资格。owner/app/team/token/proxy 改动要求重启，候选热重载整份拒绝。

Web API 使用 slack-morphism 的类型化 DTO、Hyper/rustls 及显式 CONNECT connector。Socket Mode 使用同一代理，通过有界 WebSocket 传输与具名 envelope/ack 调用 SDK payload DTO；连接、帧大小、心跳和退避均有上限。未配置 proxy 时直连并忽略环境代理；失败不回退直连，目标 TLS 证书正常验证。长输出和入站附件下载也走该配置。

## 持久接收与输出

Socket 回调只做身份检查和有界持久化，收到业务事件后以 `(namespace,event_id)` 去重，成功持久接收再 ACK。任务正文不写入 tracing；连接 URL、token、response_url 和完整帧也不记录。SQLite 接收失败不确认已解析的事件；未知 payload 只提取 envelope ID，避免无效事件反复投递。

worker 执行慢 SSH / agent 操作。交互和 modal 保持三秒预算；耗时执行不阻塞 ACK。消息编辑、删除、自身输出、bot 消息、空文字和未绑定 thread 均不构成任务。在已绑定 thread 中，文件与可选文字一起提交；纯文件使用“请查看上传的附件”作为请求。DM 主输入框附件不选择默认会话。附件在授权后下载并写入所选 Codex 主机的私有附件目录；图片作为原生图片输入，文档以文件路径上下文传入。

所有菜单、卡片、错误和正文在 DM 中以普通持久消息发送，不使用 chat.postEphemeral。过程和结果投递到相应 thread，根卡片仅在状态/标题变化时更新。菜单、状态和错误使用 plain_text block；正文通过公共 Markdown 解析生成 rich_text、header 和原生 table block。fallback 是可读纯文本，转义 & / < / > 并显式关闭顶层 mrkdwn 与 link_names；不生成可执行 mention 元素。长输出按结构拆分成普通富文本消息，不生成历史文件；历史导出入口已移除。每段与已确认段号持久保存，未知段不重发。Slack 正文保守限制为 2,800 个 UTF-16 单元或 50 个富文本元素。格式边界见 [Markdown 说明](../research/chat-formatting.md)。

上传方向与限制见 [附件说明](../attachments.md)。

各 thread 共用该 DM 的两秒发送预算，429 依据 Retry-After 调度。发送、文件完成或原生写入的回执丢失时保留 OutcomeUnknown，不自动重发。恢复/过期输入成为待确认草稿，新的确认重新核对绑定和当前 turn；重启恢复只读观察，不自动 resume。

## 模块与状态

integrations::slack 实现 Integrations::Integration，复用 ConfigFile、Store、RuntimeRegistry、共享会话 actor 和公共操作账本。平台 SQL 留在 integrations/slack，schema 维护在根 schema.sql。

slack_bindings 保存 DM session 映射；slack_provisions 防止重复根消息创建；slack_inbox / slack_actions / slack_outbox 保持接收、确认与投递状态。slack_channel_budget 的 channel 字段是 Slack API 使用的 DM 会话 ID，与普通频道模式无关。

新 schema 不创建 slack_channels 项目频道表，也不需要 outbox recipient 字段。旧数据库中的额外表/列不自动删除；旧频道绑定因 DM 校验不可使用，不静默改绑，用户在 DM 中重新选择 session。账号/endpoint 指纹仍不可改变。

本地回归覆盖 owner DM 解析、拒绝非 DM 结果、非 owner/错误 DM/频道拒绝、主输入框只开菜单、thread 路由、多项目绑定、普通消息输出、持久接收后 ACK、重复事件、代理和恢复确认。真实验收还需安装权限、Messages Tab、modal、新建/续轮/steer、文件上传、长回复分段和退出后原生任务继续运行。

平台依据：[App Home](https://docs.slack.dev/surfaces/app-home/)、[message.im](https://docs.slack.dev/reference/events/message.im/)、[conversations.open](https://docs.slack.dev/reference/methods/conversations.open/)、[Socket Mode](https://docs.slack.dev/apis/events-api/using-socket-mode/)。

## 公共导航与原生呈现

integrations::navigation 集中维护 Host → 项目目录的归属、ID/短名/目录查找、同名歧义拒绝、排序及有界分页，Slack 和 Discord 共用此模块。项目内 session 分页继续复用 engine::runtimes::list_picker，不复制 provider 查询或执行逻辑。

Slack 的 menus.rs 只装配 Block Kit：工作空间采用同行按钮，项目采用可搜索的原生下拉框，会话标题和打开按钮同一行。翻页、返回、选择更新原菜单；菜单消息 ID 与 session root 分离并随 durable action 保存，modal 提交也保留来源。会话卡片的控制按钮合并在一行，action_id 仍各自唯一，长期会话输出继续留在对应 thread。

## 已绑定会话

在 App DM 首页选择“已绑定会话”，可查看清单、打开原 thread、暂停/恢复自动观察并确认解绑。清单显示带缓存年龄的上次状态，暂停后仍可发送任务和手动查询；已开始的观察与排队输出可继续完成。完整语义见 [公共绑定管理](integrations.md#已绑定会话管理)。

## 入场历史与观察游标

首次打开 session 会读取完整历史，用于建立包含所有已见条目的观察游标；入场只展示**最后一次用户输入及对应最终回复**，不显示此前轮次或过程消息。首次观察与入场共用 `slack/history.rs` 中的选择和基线生成逻辑，后台随后只投递可见变化。初始投递与游标同事务提交，并同时复核旧游标和 `history_ready`，过时的初始观察不能覆盖已经建立的基线。

再次进入已初始化的现有绑定会复用原 Slack thread，并保留游标，不再全量读取历史，也不重发入场历史；此前已发出的聊天消息仍在 thread 内。配置没有“入场显示完整历史”的开关。状态卡的“管理”按钮进入统一绑定详情页，暂停与解绑沿用公共版本检查；显式重新进入可更新卡片控件，但不会恢复完整历史导出。
