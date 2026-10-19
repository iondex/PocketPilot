# Slack / Discord：Markdown 与 emoji

核验日期：2026-10-19。本文区分平台能力、仓库当前行为与后续建议；只做调研，没有向真实聊天发送消息，也未修改运行配置。

## 结论

菜单的共享层应统一 workspace → 项目/目录 → session 的语义、状态和文案；展示层保留平台适配。Agent 正文应先解析为 Markdown 结构，再生成 Slack 或 Discord 内容，不能把同一份原始 Markdown 无差别发送给两端。Unicode emoji 可作为共同基础，但状态必须同时保留文字，不能仅靠圆点颜色区分。

当前 Slack 菜单和短正文主要使用 `plain_text`；Discord 短正文直接交给客户端解析。因此两端即使收到相同原文，也不会有相同的排版效果。[代码：Slack 渲染](../../crates/slack/src/render.rs) · [代码：Discord 发送](../../crates/discord/src/api.rs)

## 1. Slack：需要区分三种文本路径

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

### 转义、通知与长度

- Slack 使用 `&`、`<`、`>` 作为控制字符；作为普通内容时分别转义为 `&amp;`、`&lt;`、`&gt;`。不要把整段内容做通用 HTML 实体编码。[S1]
- `parse=none` 禁用自动 URL 解析，**不会关闭 `mrkdwn`**；关闭顶层标记解析需显式 `mrkdwn=false`。`verbatim=true` 也仍会处理手写格式和手写特殊解析串，不能当作完整的 mention 防护。[S1][S2][S4]
- section 的 `text` 上限 3,000 字符；`chat.postMessage.text` 建议控制在 4,000 字符，超过 40,000 字符会被截断。不同 block 有自己的上限，不能用顶层上限统一判断。[S4][S5]
- 使用 blocks 时，顶层 `text` 仍承担屏幕阅读器等 fallback；官方格式指南也区分桌面和移动通知的文字提取行为。因此可见 block 与 fallback 的语义应一致，不应只优化可见部分。[S1][S4]

## 2. Discord：较接近常见 Markdown，但仍是子集

Discord 官方将正文描述为 Markdown 子集。除了上表列出的格式，官方帮助还列出下划线、块引用、较小字号的 subtext，以及 `||剧透||`；这些都不是 Slack `mrkdwn` 的通用能力。代码块的语言高亮与各客户端的具体呈现需独立验收。[D1][D2]

普通 Create Message 的 `content` 上限为 2,000 字符；不能把用户端某些订阅权益视为 bot API 的更大额度。Embed 可以用于结构化展示，但也有独立字段限制及总计 6,000 字符限制，并非无限长 Markdown 容器。[D3]

`allowed_mentions` 控制哪些用户、角色或 everyone 提及产生通知；官方给出 `parse: []` 作为不解析提及的方式。它解决通知问题，并不把消息正文变成纯文本，也不会阻止星号、链接、引用等 Markdown 被渲染。项目名、目录、session 标题仍需上下文相关的转义。[D3]

## 3. Emoji：Unicode 为默认，平台自定义能力分开处理

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

- workspace、项目和 session 标题保持干净；不自动给每行加装饰 emoji，也不改写用户自带的 emoji。
- 状态最多一个图标，后接明确文字，例如 `运行中`、`等待确认`、`等待输入`、`连接异常`；`未加载`、`未知` 不应被误解成成功或失败。
- 操作按钮优先使用动词，如“选择”“打开”“返回项目”；避免连续彩色图标挤占 Slack 按钮和移动端空间。
- Slack 读取到的 Unicode emoji 可能已经是 shortcode；跨平台转发时可对普通文本节点做受控归一化，未知/自定义 shortcode 保留。不给 code、URL、用户原文执行全局替换；不为展示 Unicode emoji 新增 Slack `emoji:read` 或 Discord 外部 emoji 权限。只有未来确实需要列举自定义 emoji 时，再单独评估最小权限。[S1][S6][D5]

以上为产品建议，并非平台强制规则。

## 4. 当前仓库的具体差距

| 位置 | 当前行为 | 影响 |
| --- | --- | --- |
| Slack `render::message` | 非空短正文生成 section `plain_text`；超过 2,800 个 Rust `char` 标记为文件 | Agent 的粗体、链接、代码块短消息不会按 Markdown 排版；这是实现选择，并非 Slack 无此能力 |
| Slack `Api::content` | 顶层 fallback 对 `& < >` 转义；post 设置 `parse=none`；未显式关闭顶层 `mrkdwn` | 可见正文与通知/阅读器的格式处理路径不同，需覆盖格式符号与 mention 场景；现有代码不足以认定“双重转义” |
| Slack `menus` | header、context、行内按钮、项目下拉框，标题/选项主要是 `plain_text` | 菜单应继续使用结构化组件；正文排版不必连带把所有标签改成 `mrkdwn` |
| Slack observation | 增量正文直接拼接；首次摘要最多取 1,200 个 Rust `char` | 未来启用格式渲染时，不能沿用任意字符位置截断，否则可能切断代码块或链接 |
| Discord `send_in` / Gateway 响应 | 短正文原样作为 `content`；超过 1,800 个 Rust `char` 改为原文 `session.md`；超过 8 MiB 拒绝 | 常见 Markdown 多数可显示，但动态标题也可能触发格式；长文缺少内联摘要 |
| Discord API / Gateway | 普通发送、编辑、interaction 响应均禁用用户、角色、everyone 和回复用户通知 | 应保留此防护；新增格式适配不能引入隐式 mention |
| 共享 `RuntimeStatus::label_in` | 状态与 Unicode 彩色圆点绑定 | Slack session 菜单另有不带圆点的状态映射，根卡片仍走共享带圆点版本，策略分散 |
| 共享 `session::delivery` | 组合角色、摘要与原文；未提供两平台正文格式适配 | 应共享语义与解析结果，由平台适配输出；不要共享已按某平台转义的字符串 |

代码依据：[Slack render](../../crates/slack/src/render.rs)、[Slack API](../../crates/slack/src/api.rs)、[Slack menus](../../crates/slack/src/menus.rs)、[Slack observation](../../crates/slack/src/observation.rs)、[Discord API](../../crates/discord/src/api.rs)、[Discord Gateway](../../crates/discord/src/gateway.rs)、[session model](../../crates/session/src/model.rs)、[session delivery](../../crates/session/src/delivery.rs)。2,800 / 1,800 是本项目保守阈值，**不是平台官方上限**；Rust `char` 数也不等于用户看到的字形数。

## 5. 后续实施顺序

1. **统一语义与状态。**共享层提供状态文案 ID、状态类别与菜单层级；平台只决定是否显示图标、使用哪个组件。保留 workspace → 项目/目录 → session，不把 Slack 的 workspace/team 与执行环境混为一谈。
2. **正文走统一解析、分别渲染。**沿用仓库已有 `pulldown-cmark`，输出平台无关的段落、强调、代码、链接、列表结构；复用 Telegram 解析经验，但不把 Telegram HTML 当公共格式。Slack 生成合法 `mrkdwn` 或结构化 blocks；Discord 生成其 Markdown 子集。纯文本标签与 Agent 正文使用不同入口。
3. **完整保留原文，改善长文阅读。**先展示简短摘要与关键结果，再附完整 `session.md`；按段落/代码结构处理长度与降级。Markdown 表格在窄屏可降级为逐项字段，复杂内容保留文件。不要截断后丢失正文，也不要为试用新 Markdown block 先扩大 Slack 权限。
4. **验证实际阅读场景。**用合成样本检查标题中的 `* _ ~ < > &`、URL、中文、emoji/组合字符、列表、三反引号及嵌套格式；覆盖所有投递路径、编辑、通知 fallback、附件及桌面/移动端。确认看似 mention 的文本不会触发通知，且导出文件保持原文。代码测试与真实客户端视觉验收分别记录。

Discord 若后续需要更完整的结构化菜单，可单独评估 Components V2：Text Display 支持 Markdown、mentions 和 emoji，并受 `allowed_mentions` 控制。但启用消息 flag `IS_COMPONENTS_V2`（`1 << 15`）后不能撤销，该消息不再使用传统 `content` / `embeds`，附件也需要相应 File 组件。它不是给当前纯文字消息直接加一个开关的无成本升级，不应与本轮通用正文解析绑在一起。[D6]

以上为待实施建议。本文没有新增权限、发送试验消息或声称客户端视觉验收已完成。

## 官方来源

- [S1：Slack Formatting message text](https://docs.slack.dev/messaging/formatting-message-text/)
- [S2：Slack Text object](https://docs.slack.dev/reference/block-kit/composition-objects/text-object/)
- [S3：Slack Markdown block](https://docs.slack.dev/reference/block-kit/blocks/markdown-block/)
- [S4：Slack chat.postMessage](https://docs.slack.dev/reference/methods/chat.postMessage/)
- [S5：Slack Section block](https://docs.slack.dev/reference/block-kit/blocks/section-block/)
- [S6：Slack emoji.list](https://docs.slack.dev/reference/methods/emoji.list/)
- [D1：Discord Reference — Message Formatting](https://docs.discord.com/developers/reference#message-formatting)
- [D2：Discord Markdown Text 101](https://support.discord.com/hc/en-us/articles/210298617-Markdown-Text-101-Chat-Formatting-Bold-Italic-Underline)
- [D3：Discord Message Resource](https://docs.discord.com/developers/resources/message)
- [D4：Discord Emoji Resource](https://docs.discord.com/developers/resources/emoji)
- [D5：Discord Permissions](https://docs.discord.com/developers/topics/permissions)
- [D6：Discord Components Reference](https://docs.discord.com/developers/components/reference)
