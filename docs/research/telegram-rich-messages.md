# Telegram 富消息三种输入的表达能力

核验日期：2026-10-20（Asia/Shanghai）。仅查阅 Telegram 官方 Bot API，未发送真实消息或验证具体客户端渲染。

结论：**`blocks` 最适合程序精确构造；不能据此宣称它拥有 HTML 无法表达的全部额外样式。** 三种输入都是同一种富消息的编码方式，官方没有给出逐字段完全等价保证。纯 Markdown 语法覆盖较少，但 Rich Markdown 可以嵌入受支持的 Rich HTML，因此组合使用时远比普通 Markdown 丰富。[1][2][3]

## 三种输入的关系

`InputRichMessage` 明确规定：

> “Exactly one of the fields html, markdown, or blocks must be used.”

三者互斥；共同支持 `is_rtl`、`skip_entity_detection`。`markdown`/`html` 可附带 `media` 数组，通过 `tg://photo?id=…`、`tg://video?id=…`、`tg://document?id=…`、`tg://audio?id=…` 引用上传文件或已有文件。收到的 `RichMessage` 则统一使用 `blocks: Array<RichBlock>` 与可选 `is_rtl`，没有三种输出格式。[2][3][4]

| 输入 | 可表达内容 | 主要代价或边界 |
|---|---|---|
| `markdown` | GFM 风格标题、列表、任务列表、代码、表格、脚注、公式；可混入 Rich HTML 扩展 | 需要处理 Markdown 与 HTML 双重语法；并非所有 HTML 块内部都会继续解析 Markdown |
| `html` | 官方列举的行内标签、24 类输入块所对应的标签、表格属性、媒体和交互按钮 | 仅支持白名单标签和属性；需要正确转义，不是浏览器 HTML/CSS |
| `blocks` | `InputRichBlock` + `RichText`，显式字段控制结构、样式和媒体 | 请求更冗长，需构造类型化对象；不能直接发送现成 Markdown |

官方 Rich Markdown 原文：

> “Rich Markdown is compatible with GitHub Flavored Markdown where possible and can contain arbitrary HTML. Supported rich message HTML tags are parsed as described in Rich HTML style.”

“arbitrary HTML” 不表示支持任意网页样式：Rich HTML 同一节明确写着：

> “Only the tags mentioned above are currently supported.”

因此没有自定义 CSS、JavaScript、任意字体、自由颜色或网页布局的通用承诺。Markdown 在普通块级 HTML 标签内不再解析，例外为 `<details>`、`<tg-collage>`、`<tg-slideshow>`；行内 HTML 中可以嵌套 Markdown。[1]

## 各种样式如何表达

| 能力 | Rich Markdown | Rich HTML | `blocks` / `RichText` |
|---|---|---|---|
| 粗体、斜体、删除线、代码、剧透 | 原生语法 | `<b>`、`<i>`、`<s>`、`<code>`、`<tg-spoiler>` | 对应 RichText 类型 |
| 下划线、上下标 | 需混入 HTML | `<u>`、`<sub>`、`<sup>` | Underline/Subscript/Superscript |
| 高亮 | `==text==` | `<mark>` | Marked |
| 标题、分割线、代码块 | 原生语法 | `<h1>`–`<h6>`、`<hr>`、`<pre><code>` | heading/divider/pre |
| 列表、任务列表 | 原生语法 | `<ul>/<ol>/<li>`、checkbox | list + InputRichBlockListItem |
| 表格基础结构 | 管道表格，可左右/居中对齐 | `<table>/<tr>/<th>/<td>` | table + RichBlockTableCell |
| 表格边框、条纹、紧凑、题注、跨行跨列 | 需混入 HTML | bordered/striped/compact、caption、rowspan/colspan、align/valign | is_bordered/is_striped/is_compact/caption；cell rowspan/colspan/align/valign |
| 普通引用 | `>` | `<blockquote>` | blockquote |
| 可折叠引用、居中引文、署名 | 需混入 HTML | `<blockquote expandable>`、`<aside>`、`<cite>` | expandable_blockquote/pullquote/credit |
| 折叠详情 | 混入 HTML，内部可写 Markdown | `<details><summary>`，可 `open` | details + summary/blocks/is_open |
| 脚注/引用 | `[^id]` 与 `[^id]: …` | `<tg-reference name>`、`<a href="#…">` | RichTextReference/ReferenceLink |
| 行内/独立 LaTeX 公式 | `$…$`、`$$…$$` 或 math 代码块 | `<tg-math>`、`<tg-math-block>` | RichTextMathematicalExpression / mathematical_expression block |
| 页脚、锚点及内部链接 | 需混入 HTML | `<footer>`、`<a name>`、`<a href="#…">` | footer/anchor + RichTextAnchor/AnchorLink |
| 媒体及题注 | `![](url "caption")`；复杂属性混入 HTML | img/video/audio/tg-document，figure/figcaption/cite | photo/video/animation/audio/document/voice_note + caption |
| 拼贴、幻灯片 | 混入 tg-collage/tg-slideshow，内部可写 Markdown | `<tg-collage>`、`<tg-slideshow>` | collage/slideshow |
| 地图 | 需混入 HTML | `<tg-map lat long zoom>` | map：Location、zoom、width、height、caption |
| 行内按钮/按钮行 | 需混入 HTML | `<tg-button>` / `<tg-button-row>` | RichTextButton / buttons |
| 自定义 emoji、动态日期 | `tg://emoji` / `tg://time` 特殊链接 | `<tg-emoji>` / `<tg-time>` | CustomEmoji / DateTime |
| 生成中占位 | 混入 `<tg-thinking>`；仅 draft | `<tg-thinking>`；仅 draft | thinking；仅 draft |

来源：格式语法与全部输入块定义。[1][5] **Thinking、拼贴、幻灯片、地图、按钮、页脚、锚点和公式都不是 `blocks` 独占的样式。** `InputRichBlock` 列出的 24 个输入块均有对应 HTML 标签说明；pullquote 的说明更谨慎，使用 “loosely corresponding”。[5]

真正更明确的是 `blocks` 的字段可控性。例如地图明文列出 `width`/`height`，但 Rich HTML 示例只列 `lat`/`long`/`zoom`，没有这些尺寸属性的文字契约。应将这类能力标成“结构化字段已文档化；文本属性映射未核验”，不能断言 HTML 必定不支持，也不能假设全部字段完全等价。媒体细节也不应视作 `blocks` 独占：文本模式 `media` 数组同样接收完整 `InputMedia*` 对象。[1][4][6]

脚注、公式、任务列表是文本输入的便利语法，非专属能力：结构化输入有相应 reference、mathematical_expression 和 has_checkbox/is_checked 模型。三者表达同类内容时，差别主要在编码与解析。[1][5]

同类内容的最小语法示例（编码方式对照，未实机发送）：

```text
Rich Markdown:
## 计算结果
公式 $x^2$；<u>下划线</u>；说明[^n]
[^n]: 这是脚注。

Rich HTML:
<h2>计算结果</h2>
<p>公式 <tg-math>x^2</tg-math>；<u>下划线</u>；<a href="#n">说明</a></p>
<tg-reference name="n">这是脚注。</tg-reference>
```

`blocks` 则用 heading 与 paragraph 块；段落文本组合 mathematical_expression、underline、reference_link 等 RichText 类型，另附 reference 定义。不能直接把上面的 HTML 放进 `RichText` 字符串并期待再次解析。[1][5]

## 接口、主题和草稿

| 接口 | 行为与限制 |
|---|---|
| `sendRichMessage` | 发送并返回持久的 `Message`；`rich_message` 必填；支持 reply、reply_markup、静默发送及保护内容等参数 |
| `editMessageText` | 已支持 `rich_message`；与 `text` 两者至少有一个；编辑 inline 消息时不支持直接上传新文件或显式按 URL 上传 |
| `sendRichMessageDraft` | 仅目标私人聊天；必填非零 draft_id；同 ID 更新动画、不同 ID 替换；仅临时预览，最终必须 sendRichMessage |

`sendRichMessage` 的 `message_thread_id` 支持论坛超级群，以及启用了 forum topic mode 的 bot 私聊；channel direct-messages chat 则有单独的 `direct_messages_topic_id`，发送到该类聊天时必填。草稿也可带 `message_thread_id`，但其 `chat_id` 文档限定为 “target private chat”，不能推广为群论坛草稿。[7][8][9]

草稿官方原文：

> “the streamed draft is ephemeral and acts as a temporary 30-second preview - once the output is finalized, you must call sendRichMessage with the complete message to persist it in the user's chat.”

草稿不是持久消息，也不是保证交付的最终答复；不支持直接上传新文件或显式按 URL 上传。`thinking` 仅限草稿，不会出现在收到的正式消息中。可选 `can_stop` 会显示停止生成按钮并产生 `stopped_message_generation` update；`keep_on_stop` 只暂留预览，仍需正式发送才能保存。PocketPilot 当前禁止代用户停止原生执行，故该 UI 不能未经业务设计直接启用。[8][10]

## 内容样式与交互界面

`RichMessageButton` 支持 URL、callback_data、Web App、login、inline query、copy_text 和 disabled；按钮样式可为 danger/success/primary/link，颜色由客户端主题决定，link 仅允许 callback 按钮。按钮文案仅可含普通文本、自定义 emoji、动态日期；单行 buttons block 为 1–8 个按钮，支持左/中/右对齐。[11][12]

这类按钮与 `reply_markup` 是交互能力，不代表消息正文支持任意 UI。Web App 仅限用户与 bot 私聊，不支持代 business account 发送；其他按钮类型也各有场景限制。`callback_data` 为 1–64 bytes，不能直接塞完整操作上下文；仍应由服务端保存授权范围、过期时间和一次性状态。[11]

## 预算与可用性

官方富消息统一上限：[1]

- 文本 32,768 个 **UTF-8 characters**，包含自定义 emoji 的 alternative text 与公式源码；文档未定义成 UTF-8 bytes，不要与 32,768 字节混淆。
- 500 个块，包含嵌套块、列表项、表格行、引用和 details；不能只数顶层块。
- 格式和块嵌套最多 16 层。
- 媒体附件合计最多 50 个。
- 表格最多 20 列；单元格仅行内格式。

基础发送权限包含 rich messages；消息含媒体时还需相应媒体发送权限。代 business account 发送仅当对应用户有权发送富消息。普通 bot 的 sendRichMessage 参数说明没有 Premium、付费资格或私聊专属限制；不能将 draft 的私聊限制套到正式消息，也不能据此保证所有旧版客户端正确渲染。[7][13]

核验到的官方页面版本为 Bot API 10.3（2026-08-24）：10.1 引入富消息，10.2 增加 outgoing blocks/media，10.3 增加按钮、紧凑表格、可折叠引用、文件块等。该页面没有提供各客户端最低版本、旧客户端回退行为或服务器资格矩阵；这些均待实测，不能声称已验证。[14]

## PocketPilot 建议

采用共享 Markdown AST → 平台类型化渲染；Telegram 新路径优先 `InputRichMessage.blocks`，保持用户文本为数据，显式构造表格、列表、详情及公式，避免把 Agent 原文当可信 HTML。保留现有发送账本与分段确认语义；32,768 字符并不解除 500 块、16 层、20 列等结构预算。

先支持稳定正式消息的发送/编辑；再单独设计非持久 draft，明确最终发送、断线、未知回执和预算。仅靠把旧 sendMessage 的 parse_mode 改成 HTML/Markdown 无法启用 rich_message。本仓库 Cargo.lock 当前为 teloxide 0.17 / teloxide-core 0.13；在相应已安装 SDK 源码中未检出 RichMessage/RichBlock/send_rich_message/rich_message，落地需升级 SDK 或增加类型化 DTO/请求路径。

## 官方来源

[1] [Rich Message Formatting Options](https://core.telegram.org/bots/api#rich-message-formatting-options)

[2] [InputRichMessage](https://core.telegram.org/bots/api#inputrichmessage)

[3] [RichMessage](https://core.telegram.org/bots/api#richmessage)

[4] [InputRichMessageMedia](https://core.telegram.org/bots/api#inputrichmessagemedia)

[5] [InputRichBlock](https://core.telegram.org/bots/api#inputrichblock)、[RichText](https://core.telegram.org/bots/api#richtext)、[RichBlockTableCell](https://core.telegram.org/bots/api#richblocktablecell)

[6] [InputRichBlockMap](https://core.telegram.org/bots/api#inputrichblockmap)

[7] [sendRichMessage](https://core.telegram.org/bots/api#sendrichmessage)

[8] [sendRichMessageDraft](https://core.telegram.org/bots/api#sendrichmessagedraft)

[9] [editMessageText](https://core.telegram.org/bots/api#editmessagetext)

[10] [InputRichBlockThinking](https://core.telegram.org/bots/api#inputrichblockthinking)

[11] [RichMessageButton](https://core.telegram.org/bots/api#richmessagebutton)

[12] [InputRichBlockButtons](https://core.telegram.org/bots/api#inputrichblockbuttons)

[13] [ChatPermissions](https://core.telegram.org/bots/api#chatpermissions)

[14] [Recent changes](https://core.telegram.org/bots/api#recent-changes)
