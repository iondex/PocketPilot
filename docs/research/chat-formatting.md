# Slack / Discord / Telegram：Markdown 与 emoji

核验日期：2026-10-20（UTC+8）。本文区分平台能力、仓库实现与待验收事项。格式验证使用合成样本，没有向真实聊天发送试验消息。平台文档会持续更新，尤其不能将普通消息的限制推及平台另设的富消息 API。

## 结论

菜单的共享层应统一 workspace → 项目/目录 → session 的语义、状态和文案；展示层保留平台适配。Agent 正文应先解析为 Markdown 结构，再生成 Slack 或 Discord 内容，不能把同一份原始 Markdown 无差别发送给两端。Unicode emoji 可作为共同基础，但状态必须同时保留文字，不能仅靠圆点颜色区分。

当前菜单和动态标签保持纯文本；Agent 正文共用 `session::markdown`，Slack 输出普通 `rich_text` blocks，Discord 输出支持的 Markdown 子集。两端共享语义，具体排版由客户端决定。[代码：Slack 渲染](../../crates/slack/src/render.rs) · [代码：Discord 发送](../../crates/discord/src/api.rs)

### 富消息的能力上限与选择

Telegram 的 `markdown`、`html`、`blocks` 是同一富消息模型的三种输入编码，不能简单按样式数量排成三档。纯 Markdown 标记便于书写；Rich HTML 提供对应块类型的白名单标签，Rich Markdown 又允许嵌入这些标签。`blocks` 的优势是精确、类型化地控制结构和字段，而不是所有高级块都只接受 JSON。逐字段、客户端呈现的完全等价不能仅凭有对应标签就认定。详见 [Telegram 三种输入比较](telegram-rich-messages.md)。

跨平台应追求共同语义和各端最充分的展示能力，而不是把所有端都限制到最小 Markdown 子集。平台能力、适合 PocketPilot 的选择和当前是否实现应分开记录：

| 平台 | 建议探索的完整展示路径 | 当前实现到目标的主要差距 |
| --- | --- | --- |
| Slack | 结构化 Block Kit：正文 `rich_text`，原生 `table` / `data_table`、图表 `data_visualization`、任务 `plan` / `task_card`，以及可折叠容器和卡片轮播 | 当前只生成普通正文和菜单块；锁定 SDK 已有多种新块 DTO，但公共解析会提前将表格展开为行，须先保留结构 |
| Discord | Components V2 原生布局加 Discord Markdown；传统 embeds / content 按消息用途另行选择 | 当前仍是传统 content/components；锁定 Serenity 0.12.5 未包含 V2 builders，须补齐类型化支持及新消息布局版本 |
| Telegram | `sendRichMessage` + `blocks` 作为程序生成目标；简单正文可用 Rich Markdown，复杂文本也可使用受支持 HTML 标签 | 当前仍用 `sendMessage` HTML；锁定 teloxide-core 0.13.0 未包含 RichMessage DTO，须补齐发送、编辑、topic 和分段适配 |

平台官方能力和消息限制分别见 [Slack 富消息](slack-rich-messages.md)、[Discord 富消息](discord-rich-messages.md)、[Telegram 富消息](telegram-rich-messages.md)。这里的“完整”指各平台可表达的消息结构，不代表任意 HTML/CSS、任意组件可混合，或当前 app 已实机验证全部能力。

接入顺序建议先做 Slack 原生表格，再接 Telegram 富消息，最后评估 Discord V2 布局。公共 Markdown 文档保留表格、单元格格式和对齐等信息，在平台渲染时再选择原生结构或行式降级；不直接信任 Agent 生成的平台 HTML/组件 JSON。角色和阶段提示继续来自 `session::presentation`。每端在入队/首次投递前固定其布局版本和分段结果，继续保留作用域验证、mentions 控制及未知结果不重发的约束。平台允许媒体展示不改变当前附件“用户上传给 Codex”和不导出历史的产品范围。

### 三端能力与当前使用路径

| 平台与 API 路径 | 常用格式 | 表格 | PocketPilot 当前使用 |
| --- | --- | --- | --- |
| Slack `text` / `mrkdwn` | 强调、代码、链接、引用；不是标准 Markdown | 没有 GFM 管道表格语法 | 纯文本 fallback 与少量界面文字 |
| Slack `rich_text` block | 结构化强调、代码、链接、引用、嵌套列表 | `rich_text` 本身不包含 table 类型 | Agent 正文 |
| Slack `table` block | 行列、对齐、换行、富文本单元格 | 原生结构化表格，普通 app 可经 `chat.postMessage` 发布 | 尚未使用；当前按列名和值展开 |
| Slack `markdown` block / `markdown_text` | 标准 Markdown 标题、列表、代码、任务等 | 官方列出标准 Markdown 表格转换 | 尚未使用；两种入口的条件和字段互斥需分别处理 |
| Discord bot `content` | 标题、强调、代码、链接、列表、引用、剧透等子集 | 无原生 GFM 表格布局；继续转成可读的列名和值 | Agent 正文；不直接发送管道表格 |
| Telegram `sendMessage` + HTML / MarkdownV2 | 强调、链接、代码、引用、剧透；须专门转义 | 无表格、标准标题或列表结构 | 既有 HTML 渲染；表格按列名和值展开 |
| Telegram `sendRichMessage` + Rich Markdown / Rich HTML / blocks | 标题、列表、任务、折叠、脚注、公式等结构 | 原生表格，最多 20 列 | 尚未使用；需要独立发送、编辑与分段适配 |

依据 Slack [S1][S3][S4][S7][S8]、Discord [D1][D2]、Telegram [T1][T2][T3]。Discord 官方将正文定义为 Markdown 子集，未将 GFM 表格列为受支持的排版；不能因为文档网页自身使用了表格就推断消息也支持表格。

## 1. Slack：需要区分普通文本、结构化 blocks 与标准 Markdown

### 普通 Block Kit：`plain_text` 与 `mrkdwn`

`plain_text` 适合项目名、目录、session 标题、按钮及选项标签，避免用户内容被解释成格式指令。`mrkdwn` 是 Slack 自定义语法，受 Markdown 启发，但不等同于 CommonMark/GitHub Markdown。部分 Block Kit 字段只接受 `plain_text`。[S1][S2]

| 内容 | Slack `mrkdwn` | Discord 消息 |
| --- | --- | --- |
| 粗体 | `*粗体*` | `**粗体**` |
| 斜体 | `_斜体_` | `*斜体*` 或 `_斜体_` |
| 删除线 | `~删除线~` | `~~删除线~~` |
| 行内代码 | 反引号包围 | 反引号包围 |
| 代码块 | 三反引号包围 | 三反引号包围；高亮效果需验收 |
| 命名链接 | 角括号内用 URL 与竖线标签 | `[标签](https://example.com)` |
| 标题 | 普通 `mrkdwn` 没有标准 Markdown 标题语法；菜单可使用独立 header block | 支持 `#`、`##`、`###` 标题 |
| 列表 | 文档建议用换行与普通项目符号模拟 | 支持有序、无序列表 |
| 表格、任务清单 | 不应把 GitHub 表格/任务清单当作普通 `mrkdwn` 的原生结构 | 不应依赖 GitHub 表格/交互任务清单；官方格式指南未将其列为支持项 |

上表的 Slack 规则依据 [S1]；Discord 规则依据 [D1][D2]。这里讨论的是消息正文语法，并非否定 Slack 存在其他结构化 Block Kit 组件。

Slack 链接例子：`<https://example.com|查看说明>`。直接发送 `[查看说明](https://example.com)` 到普通 `mrkdwn` 不能作为命名链接兼容策略。[S1]

### 新的标准 Markdown 路径：不能与 `mrkdwn` 混淆

当前官方还列出独立 `markdown` block，支持标准 Markdown 粗体、标题、列表、表格、任务清单、带语言代码块；图片语法转换为超链接，所有标题级别渲染为相同大小。单个 payload 中全部 markdown blocks 累计上限 12,000 字符。其 Usage info 明确提到使用平台 AI features 的 app；本文没有证明当前普通 DM app 具备该路径的使用条件，不应据此直接替换现有消息格式。[S3]

`chat.postMessage` 也列出 `markdown_text`，上限 12,000 字符，与 `text` / `blocks` 互斥。这说明 Slack 已有标准 Markdown API 路径，不能再概括成“Slack 完全不支持标准 Markdown”；但是否适合当前菜单与按钮组合，仍需独立核验，不能把该字段混入现有 `text + blocks` 请求。[S4]

### 普通 bot 可用的结构化文本与原生表格

`rich_text` 是结构化 Block Kit 数据，不接收一段 Markdown 源码。官方优先推荐其用于用户定义的格式化文本；元素包括 section、list、preformatted 和 quote，表格不属于其中一种元素。当前按 Markdown AST 生成 `rich_text` 的路径适合保留，并可精确控制链接和 mention 元素。[S8]

`table` 是另一个独立 block。官方明确普通 app 可在 `chat.postMessage` 的 `blocks` 或 `attachments` 中提供 table；该页没有要求 platform AI features 或专属 AI app 身份。单元格可为 `raw_text`、`raw_number` 或 `rich_text`，后者支持强调、emoji、超链接等。上限为 100 行、每行 20 格；一张表的所有单元格累计不超过 10,000 字符，一条消息中全部表格单元格的累计值也不超过 10,000 字符。可配置列对齐和自动换行。当前文档没有说明“一条消息只能有一张表”，不能沿用旧限制替代现行规则。[S7]

因此，Slack 表格可通过普通 `rich_text` 加独立 `table` blocks 实现，不必先切换到标准 Markdown API。实现时需要在公共解析层保留表格、表头与对齐信息，让 Slack 选择原生表格，Discord 和 Telegram 普通消息选择行式降级；同时重新核对 SDK 类型、分段预算、fallback、消息更新及持久投递 payload，不能仅把展开后的文本重新拼回表格。[S7][S8]

### 转义、通知与长度

- Slack 使用 `&`、`<`、`>` 作为控制字符；作为普通内容时分别转义为 `&amp;`、`&lt;`、`&gt;`。不要把整段内容做通用 HTML 实体编码。[S1]
- `parse=none` 禁用自动 URL 解析，**不会关闭 `mrkdwn`**；关闭顶层标记解析需显式 `mrkdwn=false`。`verbatim=true` 也仍会处理手写格式和手写特殊解析串，不能当作完整的 mention 防护。[S1][S2][S4]
- section 的 `text` 上限 3,000 字符；`chat.postMessage.text` 建议控制在 4,000 字符，超过 40,000 字符会被截断。不同 block 有自己的上限，不能用顶层上限统一判断。[S4][S5]
- 使用 blocks 时，顶层 `text` 仍承担屏幕阅读器等 fallback；官方格式指南也区分桌面和移动通知的文字提取行为。因此可见 block 与 fallback 的语义应一致，不应只优化可见部分。[S1][S4]

## 2. Discord：较接近常见 Markdown，但仍是子集

Discord 官方将正文描述为 Markdown 子集。除了上表列出的格式，官方帮助还列出下划线、块引用、较小字号的 subtext，以及 `||剧透||`；这些都不是 Slack `mrkdwn` 的通用能力。代码块的语言高亮与各客户端的具体呈现需独立验收。[D1][D2]

Message 正文文档将 `content` 上限列为 2,000 字符，但本次读取的官方 OpenAPI 将 `MessageCreateRequest.content` 列为 4,000，webhook 仍为 2,000。主源存在差异，PocketPilot 继续保持渲染后 1,800 的保守预算，不据此扩大投递长度；不能把用户订阅权益直接当作 bot API 额度。Embed 有独立字段限制及总计 6,000 字符限制。Components V2 的单组件、整消息布局和互斥规则见 [Discord 专项研究](discord-rich-messages.md)。[D3]

`allowed_mentions` 控制哪些用户、角色或 everyone 提及产生通知；官方给出 `parse: []` 作为不解析提及的方式。它解决通知问题，并不把消息正文变成纯文本，也不会阻止星号、链接、引用等 Markdown 被渲染。项目名、目录、session 标题仍需上下文相关的转义。[D3]

## 3. Telegram：普通消息有限制，新的 Rich Messages 是独立路径

当前使用的普通 `sendMessage` 支持 `parse_mode=HTML`、MarkdownV2 或显式 entities，文字上限为解析后 4,096 字符。官方格式列表包括粗体、斜体、下划线、删除线、剧透、引用、链接、行内代码和代码块；HTML 是白名单标签而非浏览器 HTML，不能发送 `<table>`、`<h1>` 或 `<ul>` 期望获得对应结构。标题可降级为粗体，列表和表格用可读文本表达。[T1]

MarkdownV2 不是 CommonMark：普通文本中 `` _ * [ ] ( ) ~ ` > # + - = | { } . ! `` 等字符必须按官方规则转义，code/pre 和 URL 还有各自规则。继续从解析结构输出 HTML，可避免将 Agent 原文直接当作可信 HTML 或 MarkdownV2。[T1]

但 Telegram 已在 Bot API 10.1（2026-06-11）引入 Rich Messages，10.2 又补充显式输入 blocks。`sendRichMessage` 使用 `InputRichMessage`，其中 `markdown`、`html`、`blocks` 必须三选一；Rich Markdown 与 Rich HTML 支持标题、列表、表格、任务、折叠内容、脚注和公式，不能与普通 `sendMessage.parse_mode` 混为一谈。当前文档限制为文本 32,768 个 UTF-8 字符、500 blocks、嵌套深度 16、表格最多 20 列。`skip_entity_detection=true` 可关闭自动检测 URL、username mention、hashtag 等实体。[T2][T3][T4]

普通 bot 可调用 `sendRichMessage`；文档另述代表 business account 发送时，账号本身也必须能发 rich messages，含媒体的 block 还要求聊天媒体权限。当前仓库尚未采用该 API。升级需要独立处理发送、编辑、SDK DTO、客户端验收、链接/实体策略、分段和持久投递，不应把 Rich HTML 白名单扩展直接套在当前 `parse_mode=HTML` 上。[T2][T3]

## 4. Emoji：Unicode 为默认，平台自定义能力分开处理

| 类型 | Slack | Discord | PocketPilot 建议 |
| --- | --- | --- | --- |
| Unicode | 可直接发送，读取 API 结果时可能已转换为 `:shortcode:` | 标准 emoji 使用 Unicode | 默认采用，避免需要额外配置 |
| 标准 shortcode | Slack 使用冒号形式，例如 `:tada:` | API reference 以 Unicode 描述标准 emoji；不能假设任意 Slack shortcode 跨平台兼容 | 共享层不要存平台 shortcode |
| 自定义 emoji | team 内定义；`emoji.list` 返回名称、URL 或 alias | `<:NAME:ID>`；动画为 `<a:NAME:ID>` | 不作为导航与状态的必要依赖 |
| 跨 workspace/server | 自定义名称依赖当前 team | 外部 emoji 受可用性与权限约束 | 不为装饰扩大 bot 权限 |

Slack 行为依据 [S1][S6]；Discord 格式与权限依据 [D1][D4][D5]。Slack text object 的 `emoji` 字段只用于 `plain_text`，控制 Unicode 与冒号形式的处理，不是启用 Markdown 的开关。[S2]

Discord 另有 **application-owned emoji**：由应用自己的 Bot token 或 Developer Portal 管理，最多 2,000 个，只能由所属 app 使用；使用这类 emoji 不要求 `USE_EXTERNAL_EMOJIS`。这与跨 guild 使用自定义 emoji 不同，不能把“自定义 emoji”一概归结为必须增加外部 emoji 权限。当前 PocketPilot 没有采用这条路径，也没有必要为基础状态图标引入资源管理。[D4][D5]

Discord 文档说明其桌面/Android 与 iOS 采用不同 emoji 图形资源，因此相同 Unicode 不保证相同视觉外观。[D1] 对跨端菜单，应依靠文字与布局建立层级，emoji 只辅助扫描。

推荐规则：

- workspace、项目和 session 标题保持干净；消息首行可用一个角色图标辅助分辨来源，正文不自动给每行加装饰 emoji，也不改写用户自带的 emoji。
- 状态最多一个图标，后接明确文字，例如 `运行中`、`等待确认`、`等待输入`、`连接异常`；`未加载`、`未知` 不应被误解成成功或失败。
- 操作按钮优先使用动词，如“选择”“打开”“返回项目”；避免连续彩色图标挤占 Slack 按钮和移动端空间。
- Slack 读取到的 Unicode emoji 可能已经是 shortcode；跨平台转发时可对普通文本节点做受控归一化，未知/自定义 shortcode 保留。不给 code、URL、用户原文执行全局替换；不为展示 Unicode emoji 新增 Slack `emoji:read` 或 Discord 外部 emoji 权限。只有未来确实需要列举自定义 emoji 时，再单独评估最小权限。[S1][S6][D5]

以上为产品建议，并非平台强制规则。

## 5. 当前实现

| 层次 | 行为 |
| --- | --- |
| 公共解析 | `session::markdown` 使用锁定的 `pulldown-cmark`；共享 Telegram 既有中文粗体与 phase label 兼容处理，不复用 Telegram HTML |
| 公共边界 | 输入最多 8 MiB、深度 64、解析事件 8,192；过度结构降级为原文字面文本；仅接受无 URL 凭据的 http/https/mailto 链接 |
| 表格与任务 | 表格转换为“列名：值”的行，保留单元格内的链接与强调；任务标记显示为 Unicode 勾选框 |
| Slack 正文 | 标题显示粗体 `rich_text_section`；强调、引用、代码和列表使用普通 `rich_text` 元素；不生成 User/Broadcast/Channel mention 元素；尚未使用原生 table block |
| Slack fallback | 从文档生成可读纯文本，转义 `& < >`；post/update 显式 `mrkdwn=false`、`link_names=false`，post 禁用自动 URL 解析与 unfurl |
| Discord 正文 | 生成标题、强调、引用、列表、链接和代码子集；原文中的 Discord 专属提及/剧透语法作为文字，所有发送、编辑和交互保留 `allowed_mentions` 禁用通知 |
| 菜单与状态 | 动态标题、目录、选项保持字面意义；共享 `RuntimeStatus::text_in` 提供不带图标的状态，Telegram 原有带图标入口保留 |
| 文字与长度 | Slack 保守预算 2,800、Discord 渲染后 1,800、Telegram 3,500，按 UTF-16 计数；公共文档按结构分段，每段独立渲染，复杂语法降级为字面文本，不生成 `session.md` |
| 历史与附件 | 三端只保留最近交互，不提供历史导出入口；独立文件附件用于用户上传给 Codex，文件本身与聊天文字分开处理 |
| 持久兼容 | 旧 Slack outbox 没有 format 字段时默认为纯文本；新正文明确标记 Markdown；发送与未知结果保护延用原账本 |

代码依据：[公共解析](../../crates/session/src/markdown.rs)、[Slack 适配](../../crates/slack/src/markdown.rs)、[Slack API](../../crates/slack/src/api.rs)、[Discord 适配](../../crates/discord/src/render.rs)、[Discord Gateway](../../crates/discord/src/gateway.rs)。预算是项目自己的保守阈值，不是平台上限。Unicode emoji 保留原文，自定义 shortcode 不跨平台猜测转换，不增加 emoji 权限。

## 6. 验证与后续范围

本地测试覆盖中文标点粗体、嵌套强调、HTML 字面文本、安全链接、代码、任务清单、表格降级、emoji/组合字符、UTF-16 长度，附件下载预算、原生文件写入，以及 SDK 请求的通知保护、旧 outbox 默认值和分段恢复。Telegram 既有 HTML 渲染行为保留并纳入全 workspace 回归。Slack 的 `blocks.validate` 已接受覆盖 section、链接、有序列表、引用及带语言代码块的合成 payload；该只读校验不等同于桌面/移动端视觉验收。

长文字以多个普通消息连续显示。投递前固定各段内容并持久保存；每段回执后记录下一个段号，重启从未发送段继续。当前段回执未知则停留在 outcome_unknown，不重发已确认段或未知段。段间复用原平台的发送预算和绑定/权限检查。桌面/移动端显示和实际通知仍需按 [Slack 操作指南](../slack-setup.md)、[Discord 测试指南](../testing-discord.md) 由用户触发验收。

第三方库的维护、许可和适配范围见 [选型核验](markdown-libraries.md)。

本次已重新读取 Slack Markdown、Table、Rich text、`chat.postMessage`，Discord API Reference / Message Resource，以及 Telegram Bot API 当前官方页面。Discord 的帮助中心格式指南 [D2] 本次复取返回 403，保留既有官方引用；不把“官方没有列出表格”写成“本次实机测试过所有客户端均不支持”。原生 Slack 表格与 Telegram Rich Messages 尚未通过当前 app 的请求或客户端验收，本文核验的是官方接口能力。

Discord 若后续需要更完整的结构化菜单，可单独评估 Components V2：Text Display 支持 Markdown、mentions 和 emoji，并受 `allowed_mentions` 控制。但启用消息 flag `IS_COMPONENTS_V2`（`1 << 15`）后不能撤销，该消息不再使用传统 `content` / `embeds`，附件也需要相应 File 组件。它不是给当前纯文字消息直接加一个开关的无成本升级，不应与本轮通用正文解析绑在一起。[D6]

Components V2 仍为后续选项；当前实现没有新增权限，客户端视觉验收应单独记录。

## 官方来源

- [S1：Slack Formatting message text](https://docs.slack.dev/messaging/formatting-message-text/)
- [S2：Slack Text object](https://docs.slack.dev/reference/block-kit/composition-objects/text-object/)
- [S3：Slack Markdown block](https://docs.slack.dev/reference/block-kit/blocks/markdown-block/)
- [S4：Slack chat.postMessage](https://docs.slack.dev/reference/methods/chat.postMessage/)
- [S5：Slack Section block](https://docs.slack.dev/reference/block-kit/blocks/section-block/)
- [S6：Slack emoji.list](https://docs.slack.dev/reference/methods/emoji.list/)
- [S7：Slack Table block](https://docs.slack.dev/reference/block-kit/blocks/table-block/)
- [S8：Slack Rich text block](https://docs.slack.dev/reference/block-kit/blocks/rich-text-block/)
- [D1：Discord Reference — Message Formatting](https://docs.discord.com/developers/reference#message-formatting)
- [D2：Discord Markdown Text 101](https://support.discord.com/hc/en-us/articles/210298617-Markdown-Text-101-Chat-Formatting-Bold-Italic-Underline)
- [D3：Discord Message Resource](https://docs.discord.com/developers/resources/message)
- [D4：Discord Emoji Resource](https://docs.discord.com/developers/resources/emoji)
- [D5：Discord Permissions](https://docs.discord.com/developers/topics/permissions)
- [D6：Discord Components Reference](https://docs.discord.com/developers/components/reference)
- [T1：Telegram Bot API — Formatting options](https://core.telegram.org/bots/api#formatting-options)
- [T2：Telegram Bot API — Rich Message Formatting Options](https://core.telegram.org/bots/api#rich-message-formatting-options)
- [T3：Telegram Bot API — InputRichMessage / sendRichMessage](https://core.telegram.org/bots/api#inputrichmessage)
- [T4：Telegram Bot API — Recent changes](https://core.telegram.org/bots/api#recent-changes)
