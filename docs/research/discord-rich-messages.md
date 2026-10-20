# Discord：最丰富的消息格式与 PocketPilot 选型

核验日期：2026-10-20，Asia/Shanghai。本文是能力研究，没有向真实聊天发送验证消息，没有修改运行配置。官方站点本次返回 403；正文证据来自 Discord 官方 `discord-api-docs` 与 `discord-api-spec` 仓库的当前主分支，链接同时保留公开文档入口。

## 结论

**Components V2 是 Discord bot 当前最丰富的原生消息布局方案**：Markdown 文本、彩色容器、分隔、图文并排、媒体图库、文件与交互控件可以按允许的结构组合。但它没有原生表格、数学公式或 Mermaid 图表组件，也不接受任意 HTML/CSS。单纯把正文放进 V2，不会获得比 Discord Markdown 更多的文字语法。[D1][D2][D3]

PocketPilot 可优先把私有 workspace → 项目 → session 菜单升级为 V2；会话正文仍适合 Markdown，状态摘要可用 V2 Container，长文继续分段。两类消息可各自选择格式，不需要整个平台一次性切换。[D1][D2]

## 三条发送路径

| 路径 | 能力 | 可组合内容 | 适用场景 |
| --- | --- | --- | --- |
| 普通 `content` | Discord Markdown 子集、emoji、链接、代码、引用等 | 传统 embeds、附件、最多 5 个 Action Rows | 连续会话正文，兼容当前实现 |
| `embeds` | 标题、说明、颜色、作者、字段、图片、缩略图、页脚、时间 | 普通 `content` 与传统按钮/选择器 | 紧凑状态卡、结果摘要；不能当作无限长文档 |
| Components V2 | Text Display、Section、Container、Separator、Thumbnail、Media Gallery、File、按钮/选择器 | 允许的组件树；不再使用传统 `content`/`embeds` | 更精细的菜单、状态卡和图文结果 |

普通消息与 embed 能混合，但 V2 与传统正文/embeds 是同条消息的互斥路径。传统组件仍继续支持，官方推荐新功能评估 V2。[D1][D2][D4][D5]

## Components V2 能做什么

| 组件 | 具体能力与边界 |
| --- | --- |
| Text Display | Markdown、mention 与 emoji；语义接近原来的 `content`，可出现多个并控制顺序 |
| Section | 1–3 个 Text Display，与一个 Button 或 Thumbnail accessory 并排；不是任意列数的网格 |
| Container | 将文本、Section、Action Row、图库、分隔符、文件分组，支持 RGB 侧边强调色与 spoiler；允许的子组件不包括另一个 Container |
| Separator | 可选分隔线及小/大间距 |
| Thumbnail | Section 旁的小图；支持 GIF/WebP 等动画图片，当前不支持视频 |
| Media Gallery | 每个图库 1–10 项，图片/视频等媒体、说明与 spoiler；可引用直接资源 URL 或上传附件 |
| File | 显示实际上传的文件，只接受 `attachment://filename`，不能拿普通网页链接冒充文件 |
| Action Row | 一行最多 5 个按钮，或一个 select；字符串 select 最多 25 项 |
| Modal | 是独立交互弹窗，可有文字输入、上传、选择器及新 checkbox/radio；这些控件不是都能放进聊天消息 |

以上组件的父子结构和用途来自完整组件枚举及各组件章节；“支持更多布局”不等于任意 HTML、嵌套卡片或交互网页。[D1][D6]

V2 通过 `IS_COMPONENTS_V2 = 1 << 15` **逐消息启用**，普通 bot 可在 Create Message、webhook、interaction response/followup 使用。公开 guild 消息和 ephemeral 交互消息都可用；私密性来自 `EPHEMERAL`，不是 Container。没有证据表明需要 Discord Nitro 或新增 privileged intent 才能启用这些发送组件。[D1][D2][D6]

## 互斥和编辑生命周期

- V2 标志一旦设置不能撤销；后续编辑仍须走组件消息路径。[D1][D4]
- `content`、`embeds`、`poll`、`sticker_ids` 等传统字段不能与 V2 正文并存；从旧消息转换时必须按对应编辑端点清空不兼容字段，不能只加 flag。[D1][D4][D5]
- 上传附件不会自动出现在 V2 消息下面，必须用 File、Media Gallery 或 Thumbnail 等组件显式展示。[D1]
- interaction 必须在 3 秒内应答，token 可用 15 分钟。采用 deferred response 时，初次 defer 仅可带 `EPHEMERAL`；再通过 Edit Original Response 设置 V2 与组件，适合 PocketPilot 当前的先确认再查目录流程。[D6]
- 每个组件点击都是新 interaction；不能把初次 slash command 的 token 当成永久菜单会话。普通 channel 消息可用 bot 消息编辑端点持续更新，ephemeral 应使用其交互 webhook 编辑路径。[D4][D6]
- 已有消息的 ephemeral 状态不能通过编辑改变。未知发送结果、分段恢复和动作一次性消费仍需保留项目现有账本语义。[D6；最后一句为项目建议]

官方当前 webhook 文档还存在一处需验收的边界：它的 V2 互斥注释列入 `files[n]`，但组件文档明确支持上传文件与 `attachment://`。因此不能据这句注释宣称“V2 不能上传附件”；具体 multipart 发送、初次回调和 followup 的附件组合应按端点分别验证。[D1][D5][D6]

## 预算与长文本

| 项目 | 当前官方证据 |
| --- | --- |
| 传统 `content` | Message 文档写 2,000 字符；官方 OpenAPI `MessageCreateRequest.content` 却写 4,000，而 webhook schema 仍为 2,000。存在主源不一致，PocketPilot 应继续按已支持的 2,000 内保守分段，不宣称 bot 已普遍支持 4,000 |
| Embed | 每消息最多 10 个；所有 embed 的标题、说明、字段、页脚、作者名总计最多 6,000 字符；description 4,096，title 256，fields 最多 25，field value 1,024 |
| V2 组件 | 整条消息最多 40 个组件，含嵌套组件；旧版“顶层最多 10/Container 子组件最多 10”已于 2025-04-29 撤除，不应沿用 |
| Text Display | 官方 OpenAPI 单组件 `content.maxLength = 4000`；本次组件文档未明确所有 Text Display 合计文字上限，不推断 40 × 4,000 都可发送 |
| Select | 最多 25 选项；label/value/description 各最多 100；placeholder 最多 150 |
| Button | label 最多 80，custom_id 1–100；显示宽度仍受客户端影响 |
| 媒体/附件 | 每图库 1–10 项；官方 OpenAPI 一条消息附件数组最多 10；图像替代文本最多 1,024 |
| 文件大小 | 默认每文件 20 MiB，2026-09-03 从 10 MiB 上调；interaction 提供 `attachment_size_limit`，须读取实际值 |
| 整体请求 | Create Message 最大请求体 25 MiB，不能因每文件 20 MiB 就假定可一次发送十个 20 MiB 文件 |

来源：[D1][D4][D7][D8][D9]。字段限制不能简单相加；选型不是把允许的组件数全部塞满。SDK、本地 UTF-16 预算与客户端真实计数差异也应在实现时验收。

## 表格、代码、数学和图表

- **表格：**官方定义正文为 Markdown 子集；当前完整 message component 枚举没有 Table，Embed inline fields 也只是字段布局，不具备表头/行列/对齐语义。不能把官方文档网页上的 Markdown 表格误当作聊天能力。默认按“列名：值”逐行保留信息；紧凑短表可选等宽代码块，但中文/emoji 对齐和手机横向可读性不可靠。[D1][D3][D4]
- **代码：**保留 fenced code 与语言信息，Text Display 官方示例明确支持 code blocks；当前代码高亮的语言集合和手机一致性未通过本次官方 API 文档得到保证，不作为可靠语义编码。[D1]
- **公式与 Mermaid：**当前组件类型和 Markdown API 未列出原生数学/图表渲染。LaTeX、Mermaid 源码默认作为代码或文字；若未来要视觉图表，可生成图片经 Gallery/embed 展示，同时保留可读文字说明。这是应用侧生成，不是 Discord 直接解析公式/图表。[D1][D3]
- **外部富预览：**2026 年官方新增 Component Embeds，可为可抓取网页定义只读 link preview；它是网页元数据+爬取流程，不等于普通 bot `rich` embed 可以任意嵌入 V2，也不适合托管私密会话内容。[D10；最后一项为项目建议]

## PocketPilot 建议

1. **私有菜单优先 V2：**一个 Container，第一行角色/导航图标和标题，下一行 workspace › 项目面包屑，Separator，String Select，最后紧凑的上一页/下一页/返回/刷新 Action Row；不输出 raw ID。保持现有 opaque action token、owner/guild/channel/config/expiry 校验及 session 打开动作的一次性消费。
2. **会话正文保持清晰：**首行角色 emoji 与阶段标签，正文用 Markdown。状态摘要可用带状态色的 Container；长代码/长结果继续结构分段，每段保留角色语义。不应为了追求卡片而把每段正文都包成多层容器。
3. **表格按能力降级：**共同 AST 保留表头/列对齐，Discord 选择行式数据；不因为 Slack/Telegram 支持表格就给 Discord 发送不可读管道表格。不自动生成 `session.md`，不恢复历史导出。
4. **预算属于发送模型：**持久 payload 必须记录 classic/V2、组件、段落与预算；编辑只修改对应模式，不能在失败后偷偷换一条格式重新发送，避免未知结果重放。
5. **先补 SDK 缺口：**仓库锁定 serenity 0.12.5，当前 `ui::Screen` 只含 `Vec<CreateActionRow>`，本地该 SDK 未包含 V2 的 `CreateContainer`/`CreateTextDisplay` 等 builder。平台支持不代表当前依赖可直接发送；应评估升级 SDK或增加边界明确的 typed DTO，不能在业务层随意拼 JSON。此项未做源码修改。

以上是实施建议，不是本次已经上线的能力。

## 提及保护与验收

Text Display 的可通知 mention 遵循消息级 `allowed_mentions`。传统 channel 消息缺省解析 users/roles/everyone，interaction/webhook 缺省也会解析 users；创建、编辑、followup **每次**都显式发送 `parse: []`、`replied_user: false`，不能依赖初次发送的配置自动延续到编辑。格式转义仍要单独处理，mention 保护不是 Markdown 转义。[D1][D4][D5]

实施后需要本地 typed payload 测试与用户触发的桌面/移动验收：私有菜单更新、分页、过期、宽短标题、中文/emoji、代码、表格降级、状态变化、长文分段及重启恢复。媒体另验 multipart、附件保留/删除和实际客户端呈现；本次研究没有发消息，因此未声称这些视觉效果已经验证。

## 一手来源

- [D1：Component Reference](https://docs.discord.com/developers/components/reference) · [官方源文件](https://github.com/discord/discord-api-docs/blob/main/developers/components/reference.mdx)
- [D2：Using Message Components](https://docs.discord.com/developers/components/using-message-components) · [官方源文件](https://github.com/discord/discord-api-docs/blob/main/developers/components/using-message-components.mdx)
- [D3：Reference / Message Formatting](https://docs.discord.com/developers/reference#message-formatting) · [官方源文件](https://github.com/discord/discord-api-docs/blob/main/developers/reference.mdx)
- [D4：Message Resource：Create/Edit Message、Embed Limits、Allowed Mentions](https://docs.discord.com/developers/resources/message) · [官方源文件](https://github.com/discord/discord-api-docs/blob/main/developers/resources/message.mdx)
- [D5：Webhook Resource](https://docs.discord.com/developers/resources/webhook) · [官方源文件](https://github.com/discord/discord-api-docs/blob/main/developers/resources/webhook.mdx)
- [D6：Receiving and Responding to Interactions](https://docs.discord.com/developers/interactions/receiving-and-responding) · [官方源文件](https://github.com/discord/discord-api-docs/blob/main/developers/interactions/receiving-and-responding.mdx)
- [D7：Raised Component Limits，2025-04-29](https://docs.discord.com/developers/change-log#raised-component-limits) · [官方源文件](https://github.com/discord/discord-api-docs/blob/main/developers/change-log.mdx)
- [D8：官方 OpenAPI：TextDisplayComponentForMessageRequest、MessageCreateRequest、IncomingWebhookRequestPartial](https://github.com/discord/discord-api-spec/blob/main/specs/openapi.json)
- [D9：Uploading Files](https://docs.discord.com/developers/reference#uploading-files) · [Default File Upload Limit Increase，2026-09-03](https://docs.discord.com/developers/change-log#default-file-upload-limit-increase)
- [D10：Component Embeds](https://docs.discord.com/developers/link-previews/component-embeds) · [官方源文件](https://github.com/discord/discord-api-docs/blob/main/developers/link-previews/component-embeds.mdx)
