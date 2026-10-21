# 已绑定会话导航与 Discord V2 预算

核验日期：2026-10-21，UTC。仅查阅一手文档、官方 schema 与上游问题记录；未向真实聊天发送消息。

## Telegram：私聊 topic 不能套用群组链接

Telegram 的官方链接规范把 message links 定义为群组和频道内的消息链接。`t.me/c/<channel>/...` 的 `channel` 必须是 channel/supergroup ID；forum topic links 复用该语法，通过 topic 创建服务消息定位 topic。它没有定义“bot 私聊 ID + topic ID”的链接格式。因此不能把正数用户 ID 填入 `t.me/c`，也不能因为私聊支持 forum topics 就推断群组链接同样适用。[T1]

Bot API 确实支持在 bot 私聊创建 topic，并以 `sendMessage.message_thread_id` 向该 topic 发送消息。这里的参数是**发送目的地**，不是让用户客户端切换 topic 的导航命令。[T2][T3]

| 能力 | 官方支持与实现边界 |
| --- | --- |
| 直接打开 bot 私聊 topic | 本次官方链接规范和 Bot API 未提供可依赖的格式或 `openTopic` 方法；应在公共模型中表示“不提供直达链接” |
| 普通 URL 按钮 | `InlineKeyboardButton.url` 可打开 HTTP 或 `tg://` URL，但并未因此新增私聊 topic URL 语法 |
| callback 按钮 | `callback_data` 为 1–64 字节，触发 bot 回调；可显示会话状态或操作确认，不能声称点击后客户端自动切换到另一 topic |
| 向已有 topic 发提示 | 使用已绑定 `chat_id + message_thread_id` 发送普通消息；这会产生新消息，不是纯跳转操作 |
| 回复已有消息 | `ReplyParameters` 支持指定原消息，也明确处理跨 topic 回复；若已有可验证的普通消息回执，可用原生回复展示上下文，不保证等同于“直接打开 topic”的 URL |
| 转发 | `forwardMessage` 支持目标 topic，但不能转发服务消息或受保护消息，不能依靠转发 topic 创建服务消息实现导航 |

来源：[T3][T4][T5][T6]。普通消息回复与客户端点击效果仍需真实客户端验收；本研究不承诺某一客户端一定提供跨私聊 topic 的跳转入口。

**当前可靠降级：**“已绑定会话”显示工作区、项目、会话标题和 topic 名称，并提示用户从 topic 列表进入；提供状态、最近交互、解绑等经过授权的 callback 操作。已有 topic 继续复用，不为了缺失链接重复创建。若增加“在会话内发送定位提示”，应作为显式发送动作处理，并保留正常回执语义。[T1–T6；本段为项目设计建议]

公共绑定接口应区分“可直接跳转的地址”和“平台内可识别的会话容器说明”，不要强制每个平台都返回 URL。

## Discord：按最终消息计费，不按 Markdown 节点计费

| 预算 | 当前一手证据 |
| --- | --- |
| Text Display 正文 | 官方 OpenAPI `TextDisplayComponentForMessageRequest.content` 为 `minLength: 1`、`maxLength: 4000` |
| 全消息组件 | 官方 Component Reference：最多 40 个组件，包含布局与子组件；2025-04-29 已移除旧的顶层 10 个和 Container 子组件 10 个限制 |
| 所有 Text Display 的合计文字 | 本次官方 Component Reference 和 schema 没有明确独立的合计上限；不能推断可发送 `40 × 4000` 字符 |
| Media Gallery | 一个 Gallery 包含 1–10 个媒体项；图像替代文字最多 1,024 字符 |
| 上传附件 | 官方 OpenAPI `MessageCreateRequest.attachments.maxItems = 10`；这不是“每个 Gallery 都可再上传十个附件” |
| V2 模式 | 必须设置 `IS_COMPONENTS_V2`；传统 `content`、`embeds` 不再工作，附件必须通过组件显式呈现 |
| 提及 | Text Display 遵循消息级 `allowed_mentions`；发送与编辑均须保留禁用自动提及配置 |

来源：[D1][D2][D3]。schema 的 `maxLength` 没有在此处明确给出 UTF-16 计数规则；本地采用 UTF-16 单元作为保守预算应标记为项目决策，而不是援引为官方计量定义。

截至本次核验，官方文档仓库的 [PR #8562](https://github.com/discord/discord-api-docs/pull/8562)（补充 Text Display 4k 文档）仍未合并；确定的 4,000 证据来自官方 OpenAPI，不能用未合并 PR 扩大合计额度。[D3][D4]

**建议实现：**将 3,600 UTF-16 单元作为单条 V2 消息的应用总预算，包含重复角色标题；同一条原生输出的相邻短段按最终格式合并，只有达到最终长度或实际组件预算才切分。Markdown AST 节点数用于解析资源保护，不应直接等同于 Discord 原生组件数。代码块闭合、表格降级后的文本膨胀、Gallery 项数和所有嵌套组件应单独计量。不同角色、轮次、阶段以及已冻结回执的消息段仍应保留边界。[D1–D3；本段为项目设计建议]

上游还有一份关于多字节正文触发内部 processed-size 限制的公开问题报告，涉及 webhook 与普通发送路径；其阈值取决于结构，且并非公开规范，不应硬编码成官方固定额度。本次没有线上复现。它说明“字段长度合规”不保证所有端点都无额外限制，应保留有类型的发送错误与未知回执处理，不能遇错后自动重新发送另一种格式。[D5；后半段为项目建议]

## 一手来源

- [T1：Telegram Links：Message links / Forum topic links](https://core.telegram.org/api/links#message-links)
- [T2：Bot API：createForumTopic](https://core.telegram.org/bots/api#createforumtopic)
- [T3：Bot API：sendMessage](https://core.telegram.org/bots/api#sendmessage)
- [T4：Bot API：InlineKeyboardButton](https://core.telegram.org/bots/api#inlinekeyboardbutton)
- [T5：Bot API：ReplyParameters](https://core.telegram.org/bots/api#replyparameters)
- [T6：Bot API：forwardMessage](https://core.telegram.org/bots/api#forwardmessage)
- [D1：Discord Component Reference：Text Display / Media Gallery / Container](https://docs.discord.com/developers/components/reference)
- [D2：Raised Component Limits，2025-04-29](https://docs.discord.com/developers/change-log#raised-component-limits)
- [D3：Discord 官方 OpenAPI：TextDisplayComponentForMessageRequest / MessageCreateRequest](https://github.com/discord/discord-api-spec/blob/main/specs/openapi.json)
- [D4：官方文档仓库 PR #8562（未合并）](https://github.com/discord/discord-api-docs/pull/8562)
- [D5：官方文档仓库 issue #8554（上游报告，非规范）](https://github.com/discord/discord-api-docs/issues/8554)
