# Slack 最丰富的消息格式

核验日期：2026-10-20（Asia/Shanghai）。仅访问官方公开文档及本机 SDK 源码；没有向真实会话发送测试消息。下述“支持”指官方接口模型，不等同于当前 App 或所有客户端已经实测通过。

## 结论

Slack 的能力上限是**组合式 Block Kit**，不只是 `mrkdwn`，也不只是 `rich_text + table`。当前公开组件还包括富数据表、原生图表、任务卡与计划、卡片轮播、可折叠容器，能够在同一消息里组合文本、媒体和交互。普通消息最多 50 blocks；各组件自己的上限仍要同时满足。[S1][S6–S14]

对 PocketPilot，建议继续保留 AST → 平台结构的接口：正文生成 `rich_text`，标题生成 `header`，普通 Markdown 表格生成 `table`；数据浏览可选 `data_table`，执行步骤可选 `task_card` / `plan`，详细过程可选可折叠 `container`。图表需要显式、可信的结构化数据，不能把任意代码围栏自动当作可执行图表。[S2][S5–S11] 这是实现建议，当前代码尚未接入这些新增组件。

## 不同路径不能混为一谈

| 路径 | 能力与边界 | 建议用途 |
| --- | --- | --- |
| `text` / `mrkdwn` | Slack 自有语法；强调、链接、引用、代码，无标准列表语法；普通 Markdown 管道表格不适用 | 简短界面文字与 fallback |
| `rich_text` | section、list、quote、preformatted，以及有类型的链接、emoji、日期等内联元素；官方称其比 `mrkdwn` 更灵活 | 受控渲染正文，与其他 blocks 组合 |
| 独立 `markdown` block | 接受标准 Markdown；表格、任务清单、语言高亮代码等交由 Slack 转换；图片变链接；各级标题同大小 | 使用 platform AI features 的 App 的候选路径，资格需单独核实 |
| 顶层 `markdown_text` | `chat.postMessage`、`chat.update` 都列出；最多 12,000 字符，**不得与 `text` 或 `blocks` 同用** | 无需混排其他 blocks 时的标准 Markdown 候选 |
| 组合式 Block Kit | 独立标题、表格、数据表、图表、卡片、计划、容器、媒体、按钮、选择菜单等 | 最完整且可控的消息布局 |
| 旧式 `attachments` 字段 | 旧卡片字段可带颜色条、作者、图片等；官方标记 legacy，建议改用 blocks | 兼容既有消息，不作为新正文方向 |

依据 [S2–S5][S19]。独立 `markdown` block 的文档原文是 “This block can be used with apps that use platform AI features”；这既不能简化为“所有 bot 都可用”，也没有证明一定存在额外专属 OAuth scope。其 2025-02-03 公告同样称 “intended for use in apps that use platform AI features”。[S4][S20]

相反，`chat.postMessage.markdown_text` 和 `chat.update.markdown_text` 的方法参考没有列出 AI 专属资格或 `assistant:write` 要求，方法 scope 为 `chat:write`。不能把独立 block 的措辞直接套到这个字段，也不能凭方法字段存在就宣称当前 App 已经验证可用。[S3][S17]

## 文本、标题、代码与任务清单

- `rich_text` 官方：“rich_text is strongly preferred and allows greater flexibility”。列表可设 bullet / ordered、缩进和编号起点；引用有独立 quote 元素。[S2][S5]
- `header` 现在明确列出 `level: 1..4`（H1–H4），标题为纯文本，最多 150 字符。独立 `markdown` block 则仍明确所有标题级别渲染为同一大小；这两种行为不同。[S4][S5]
- `rich_text_preformatted.language` 可指定代码高亮语言，如 python、javascript、json；并非只有独立 `markdown` block 才支持语言高亮。[S5]
- `markdown` block 支持 `- [ ]` / `- [x]` 的任务清单显示；若要真实可交互复选框，应采用 Block Kit `checkboxes` 元素并处理交互，单组最多 10 项。不能将 Agent 写出的任务清单自动升级成控制操作。[S4][S15]
- 本次读取的文本和 blocks 参考未列出 LaTeX / 数学公式或 Mermaid 原生解析接口；不能以普通 Markdown 兼容性推断它们可渲染。图表则有独立原生组件，见下节。[S1][S2][S4][S9]

## 原生表格、数据表与图表

| 组件 | 能力 | 主要上限 |
| --- | --- | --- |
| `table` | `raw_text`、`raw_number`、`rich_text` 单元格；富格可带强调、emoji、链接等；逐列 left / center / right 对齐及换行 | 最多 100 行、每行 20 格；单表所有格及每消息所有表格格内容分别最多 10,000 字符 |
| `data_table` | 分页、排序、筛选、富文本、Work Object 等链接，以及 `action_cell` 按钮；第一行为表头 | 2–201 行，包含 1 行表头；1–20 列；每表/每消息表格格内容最多 20,000 字符；`page_size` 1–100，默认 5 |
| `data_visualization` | 原生 line / bar / area / pie，图例、坐标类别及标题 | 每消息最多 2 图；标题最多 50 字符；饼图 1–12 片；其他类型 1–12 series，每 series 1–20 点 |

依据 [S6][S7][S9]。`table` 官方原文：“Apps can programmatically publish messages that include a table by providing a table block in the attachments or blocks fields of a chat.postMessage request.” 当前该页按**每消息累计字符**限制多表，没有写“一条消息只能一张表”；不要沿用旧的一表限制。[S6]

`data_table` 首行不允许 `rich_text`；所有行列数必须一致；整列都是 `raw_number` 时按数字排序，否则默认按字母排序。`action_cell` 仅用于 `data_table`，点击产生普通 `block_actions`；可附纯文本/数字 fallback 供不支持该单元格的客户端显示，复制或分享到其他会话后的按钮禁用。[S7]

`data_visualization` 要求点的 label 与 `axis_config.categories` 对应，series 名称唯一，不允许省略类别数据；这是一套结构化图表 schema，不是 HTML、SVG 或任意 JS 图表执行环境。[S9]

## 卡片、折叠、计划、媒体与交互

| 组件 | 可用表现与限制 | 官方 surface |
| --- | --- | --- |
| `card` | hero 图、图标、标题/副标题各 150 字符、正文/附注各 200 字符、最多 3 按钮 | Messages / Home tabs / Modals |
| `carousel` | 横向滚动的 1–10 张 card | Messages / Home tabs |
| `container` | 组合子 blocks，支持宽度、折叠与默认折叠；最多 10 子 blocks；允许的类型是文档列出的有限集合 | Messages / Home tabs |
| `task_card` | 单步任务标题、状态、rich_text 详情/输出、URL sources；状态 in_progress / complete / error | Messages |
| `plan` | 统一标题下的多个任务卡，最多 50 任务且 task_id 唯一 | Messages |
| `image` | 公网图片 URL 或 Slack 文件，带 alt 文本；支持 png / jpg / jpeg / gif | Messages / Home tabs / Modals |
| `video` | 嵌入视频播放器，亦允许纯音频；额外要求 `links.embed:write`、已声明 unfurl 域名、HTTPS / iframe 等条件 | Messages / Home tabs / Modals |
| `actions` / `context_actions` | 按钮、选择菜单等；actions 最多 25 元素，context_actions 最多 5 个反馈/图标按钮元素 | 按各组件参考适用面使用 |
| `alert` | info / warning / error / success 等提醒；最多 200 字符 | **仅 Modals，不适用于聊天消息** |

依据 [S8][S10–S16]。Surface 从官方 `.md` 页头 `available_in_surfaces` 及正文核对，例如 [data table 元数据](https://docs.slack.dev/reference/block-kit/blocks/data-table-block.md)、[图表元数据](https://docs.slack.dev/reference/block-kit/blocks/data-visualization-block.md)、[alert 元数据](https://docs.slack.dev/reference/block-kit/blocks/alert-block.md)。

上述 data table、图表、card、carousel、container、task card 和 plan 的**独立参考页**未声明只能用于流式消息或 AI 专属 App，不应擅自增加这类限制。普通 Block Kit 消息文档也说明：“There are no special OAuth scopes needed to publish blocks beyond those already specified within the methods above.” 但 video 等组件有明确额外条件；“未列出额外条件”仍不等于本机 App/旧客户端已经实测。[S2][S7–S16]

## 普通 bot / owner DM 与 AI 身份

普通 bot 可用 `chat:write` 向自己的 DM 会话发布；已知 DM ID 可直接作为 channel。向两个人之间的私人 DM 发言另有约束，不能把 bot-owner DM 与任意用户之间的 DM 混为一谈。App 的 Messages tab、会话成员身份、workspace 管理策略等也可能影响投递。[S3]

AI 开发指南明确：“Developing and using some AI features require a paid plan”；`assistant:write` 用于 Agent 顶栏入口及 UI 中的 agent 标识等。不能据此推断所有富 blocks、所有 `markdown_text` 或所有 streaming API 都必须付费或具备该 scope。[S18]

## 编辑、流式发送与预算

- 普通消息最多 50 blocks，Home / Modal 最多 100；block 内部及组合组件仍有自己的预算。`postMessage.text` 建议最多 4,000，超过 40,000 会截断；这些限制不能替代表格、section、markdown 等独立限制。[S1][S3]
- `text + blocks` 应提供语义完整的可访问性 fallback。屏幕阅读器默认读取顶层 `text`，而不是所有内部 blocks。继续保持动态名称为纯文本，并只生成受控链接/mention 元素。[S2][S3]
- `chat.update` 支持更新 bot 自己的消息及 blocks；未传 blocks 时，是否保留旧 blocks 取决于有没有传 text，清空必须显式传 `[]`。它不编辑 ephemeral 消息。方法还列出 `block_mismatch` 和 `streaming_state_conflict`，不可假设任意呈现类型可无条件互换、或流式过程中可以普通 update。[S17]
- `chat.startStream` → `chat.appendStream` → `chat.stopStream` 三者的当前方法参考 scope 都是 bot `chat:write`，没有声明 `assistant:write` 是必要条件。start / stop 为 Tier 2，append 为 Tier 4；当前 App 的实际可用性尚未验证。[S21–S23]
- 普通 DM 也需要 `thread_ts`。只在 Slack Code 等“整个 channel 就是 session”的特定场景才允许省略；其他地方省略会报 `invalid_thread_ts`。向 channel 流式发送还需 recipient_user_id / recipient_team_id。[S21]
- `markdown_text` 与 `chunks` 不能同请求并传；append 必须保持 start 选择的模式。`chunks` 支持 markdown_text / task_update / plan_update / blocks；task_update / plan_update 的方法页列出 256 字符 chunk 限制。[S21–S23]
- 流式 `blocks` chunk 最多 50 blocks，超过会丢弃并警告。stop 的独立 `blocks` 字段在结尾渲染，另有 50 个额度；其参考页明确说明可达 **50 + 50 = 100**，这是特殊流式路径，不能推广到普通 postMessage。[S21][S23]
- streaming 过程中 unfurl 禁用；append 可能遇到 stopped_by_user、message_not_in_streaming_state、streaming_mode_mismatch。是否将这些 UI 语义映射为原生 Codex 控制是另一个设计问题，不应随富消息渲染自动启用。[S18][S22]

**官方页面内部有不一致**：AI 开发指南仍说 start / append 不能发送 blocks，但当前方法参考已明列 `chunks: [{type: "blocks", blocks: ...}]`；指南例子的 chunks 字段也与方法参考不完全相同。实现应优先参考当前方法字段定义，并用合成请求做独立验证，不能把教程片段直接作为最终 schema。[S18][S21–S23]

## PocketPilot 的选择顺序

1. **先补原生正文能力**：header 层级、带语言代码块、`table`，保留 plain fallback 和原有安全链接/通知策略；它们解决实际信息损失。
2. **再增强结构化状态**：用 `task_card` / `plan` 表达已知步骤，用 container 折叠过程，用 data_table 展示需要排序/分页的数据；仍使用原授权、动作令牌和投递账本。
3. **图表/媒体按内容选用**：仅从明确数据生成图表；保留表格或文字替代。图片/视频不得变成隐式文件导出或扩大控制权限的入口。
4. **将标准 Markdown / streaming 当作独立候选**：先核实当前 App 资格、编辑、预算、客户端和流式状态恢复；不因新接口可用就丢弃已确认投递、不重试未知写入的保证。

兼容底线是 `rich_text` / 常规 blocks + 纯文本 fallback；超限表格按完整行拆分，旧客户端或不合适的宽表可转“列名：值”。不要为“最丰富”强行给每条回复套卡片，也不要将消息格式探测发往真实用户会话。

## 官方来源

- [S1 Blocks 总览](https://docs.slack.dev/reference/block-kit/blocks/)
- [S2 Formatting message text](https://docs.slack.dev/messaging/formatting-message-text/) · [Rich text](https://docs.slack.dev/reference/block-kit/blocks/rich-text-block/)
- [S3 chat.postMessage](https://docs.slack.dev/reference/methods/chat.postMessage/)
- [S4 Markdown block](https://docs.slack.dev/reference/block-kit/blocks/markdown-block/)
- [S5 Header](https://docs.slack.dev/reference/block-kit/blocks/header-block/) · [List](https://docs.slack.dev/reference/block-kit/block-elements/rich-text-list-element/) · [Preformatted](https://docs.slack.dev/reference/block-kit/block-elements/rich-text-preformatted-element/)
- [S6 Table](https://docs.slack.dev/reference/block-kit/blocks/table-block/)
- [S7 Data table](https://docs.slack.dev/reference/block-kit/blocks/data-table-block/)
- [S8 Task card](https://docs.slack.dev/reference/block-kit/blocks/task-card-block/) · [Plan](https://docs.slack.dev/reference/block-kit/blocks/plan-block/)
- [S9 Data visualization](https://docs.slack.dev/reference/block-kit/blocks/data-visualization-block/)
- [S10 Card](https://docs.slack.dev/reference/block-kit/blocks/card-block/)
- [S11 Carousel](https://docs.slack.dev/reference/block-kit/blocks/carousel-block/) · [Container](https://docs.slack.dev/reference/block-kit/blocks/container-block/)
- [S12 Image](https://docs.slack.dev/reference/block-kit/blocks/image-block/)
- [S13 Video](https://docs.slack.dev/reference/block-kit/blocks/video-block/)
- [S14 Actions](https://docs.slack.dev/reference/block-kit/blocks/actions-block/) · [Context actions](https://docs.slack.dev/reference/block-kit/blocks/context-actions-block/)
- [S15 Checkboxes](https://docs.slack.dev/reference/block-kit/block-elements/checkboxes-element/) · [Select menu](https://docs.slack.dev/reference/block-kit/block-elements/select-menu-element/)
- [S16 Alert](https://docs.slack.dev/reference/block-kit/blocks/alert-block/)
- [S17 chat.update](https://docs.slack.dev/reference/methods/chat.update/)
- [S18 Developing an agent](https://docs.slack.dev/ai/developing-ai-apps/)
- [S19 Legacy secondary attachments](https://docs.slack.dev/legacy/legacy-messaging/legacy-secondary-message-attachments/)
- [S20 Markdown block 公告](https://docs.slack.dev/changelog/2025/02/03/block-kit-markdown/)
- [S21 chat.startStream](https://docs.slack.dev/reference/methods/chat.startStream/)
- [S22 chat.appendStream](https://docs.slack.dev/reference/methods/chat.appendStream/)
- [S23 chat.stopStream](https://docs.slack.dev/reference/methods/chat.stopStream/)
