# Telegram 私聊与配对

## 1. 访问模型

Telegram 只接收私聊业务，不支持群、supergroup、channel 或在群中的 callback。即使某个已配对用户在群中发送命令，也必须拒绝。无需配置 chat/user 允许列表；真实 sender 与 private chat 身份来自 Telegram 更新，不以 username、转发来源或用户输入的 ID 授权。

配对按 `(bot_id, user_id)` 持久化在自有 SQLite，chat_id 必须是该用户自己的私聊地址。配对后可使用当前配置允许的所有 host/workspace；不同用户各有自己的私聊 topic/状态缓存/绑定，同一 Codex session 的操作仍由同一 actor 串行化。

私聊访问判定、持久 poller、配对账本、CLI 批准/撤销、菜单、topic、历史和输入处理已接通。自动测试覆盖本地 API fixtures；实际手机联调步骤见 [测试入口](../testing.md)。

## 2. CLI 配对

`/start` 流程：

1. 验证消息是用户本人的私聊；群聊静默忽略，不创建配对申请、不泄漏 workspace 信息。
2. 未配对用户只获得一个短期一次性码和“请在运行 PocketPilot 的机器上批准”的提示；此时不能列会话或触发 agent。
3. 管理员在本机执行 `pocketpilot pair approve CODE`。只有 CLI 管理入口可以批准，Telegram 按钮/消息不能自助批准。
4. 成功后再次 /start，复用固定的「管理」topic，在其中显示管理菜单；不创建 Codex session。

当前 CLI：

```sh
cargo run -p app --bin pocketpilot -- pair pending
cargo run -p app --bin pocketpilot -- pair approve CODE
cargo run -p app --bin pocketpilot -- pair list
cargo run -p app --bin pocketpilot -- pair revoke BOT_ID USER_ID
```

申请码采用随机 UUID，只存 SHA-256 hash，10 分钟过期，批准事务会消费申请；再次批准同码失败。新申请替换该 bot/user 的旧码；后续 poller 对重复申请做限速，不能让陌生私聊无限刷提醒。

CLI 使用独立的配对管理数据库连接，不获取服务主锁、不执行 operations/deliveries 的恢复逻辑。服务持锁运行时仍可批准；授权变更不写入 TOML 的用户列表，也不改变任何 Codex session。

撤销配对立即禁用对应本地绑定；后续 dispatch/deliver 重新检查配对状态，未开始的输入/输出不再执行。已经被服务端接受的输入不能撤回，不通过 interrupt 实现授权回滚。

## 3. 私聊中的 Session Topic

每个已配对用户有一个固定的「管理」topic，只用于管理。/start、/workspaces、/sessions、/new、/open 和 /topics 都路由到这里；/help 按话题类型显示说明；一个具体 Codex thread 对应另一个 session topic。管理 topic 通过数据库约束禁止绑定 agent。使用 Telegram 支持的 bot 私聊 topic 模式，不创建群聊作为替代。

getMe 的 has_topics_enabled 必须为 true，当前 SDK 已通过私聊 thread_id 保留测试。未启用时明确提示需开启相应 bot topic 模式；不能声称所有普通私聊 bot 默认都可创建 topic。SDK 必须保留正确的 message_thread_id/thread_id 类型。

```text
PocketPilot 私聊
  管理：工作区 / 会话 / 新建确认 / 提示
  调查测试失败
  登录回归
```

topic 名只保留截短的原始标题，最多 40 个 grapheme，且内部截断同时限制 Unicode 码点数量；不拼工作区、路径、thread 短 ID 或状态字样。原生 emoji icon 单独表达状态，不用改标题表达运行中。/status 返回主机、工作区、cwd、完整 thread/provider/turn ID 和实际运行状态。

未绑定 topic、General 入口或管理 topic 的普通任务文本只获得拒绝提示，不复述正文、不发送给 agent；已绑定 session topic 中的普通文本只发给该 session。全部 Off-topic 文字输入在配对后先归一到管理上下文；命令、按钮和返回/刷新共享同一份管理导航。普通任务文本按管理规则拒绝，不把默认输入猜测成“最近会话”。转发是服务端路由，Bot API 无法强制客户端切换页面。

Telegram 的「允许用户创建 topic」开关开启时，客户端会先为入口消息（包括 /start）创建 topic，再把消息发给 bot，这是平台行为。固定入口需要在 BotFather 开启 **Disallow users to create new threads**，保留 bot 的 Topics 功能。程序通过 getMe.allows_users_to_create_topics 诊断此设置，但 Bot API 不提供本项目可调用的对应设置方法；不会自动删除用户已有 topic。官方说明见 [Bot forums](https://core.telegram.org/api/forum#bot-forums)。

## 4. 按钮导航与绑定

管理界面用 MarkdownV2 展示标题、说明和当前范围，内联按钮承载选择。主机和工作区尽量两列排列，会话和确认项使用单行；分页与返回、首页、刷新有独立按钮。按钮只显示名称和必要 emoji，详细路径留在标题输入、确认页或 /status。动态正文经 SDK 转义，agent 正文使用独立渲染模式。

导航依次选择主机 → 工作区 → 会话，或主机 → 工作区 → 标题 → 创建确认。当前页面、选项和已确认的菜单消息 ID 保存在 SQLite，回调仅包含不透明页面 ID 与选项索引，不携带执行参数。每次选择按 bot/user/chat/topic、当前页面、消息 ID、配对及允许范围复核；参数从本地类型化页面读取。

正常切页编辑同一条菜单消息，不新增一串菜单。确认消息不存在或无法编辑时发送新菜单并更新指针；网络结果未知不自动补发。显式 /start 总是发送新的可见首页，以便清空客户端记录后恢复入口。过时的排队页面在发送前失效，快速点击不能让旧页面覆盖当前页面。

只读按钮没有固定 TTL，重启及显示、日志、速率变化不使浏览永久失效。旧页面按钮恢复当前页面；尚在投递的页面不重复排队。创建、删除和批量删除必须来自当前已投递且未消费的确认页，并受投递时间、进程恢复标记和权限范围约束；过期或恢复后的点击重新显示确认，不执行旧操作。Telegram 回调携带的是原菜单消息时间，确认新鲜度以本地投递回执判断。

管理 topic 普通任务文本被拒绝，仅标题输入页接受会话名称。/new 的标题是元数据，不作为首轮任务发送；支持 HOST/WORKSPACE 或完整工作区 ID，重名时不猜测。异常任务草稿保留单独的确认按钮及原会话/revision 约束。

选择已有会话只读绑定，不 resume。topic 创建有持久化回执，未知结果不自动重新创建；同一原生 thread 重复选择复用已有 topic，容器不跨 thread 换绑。工作区增改通过管理命令或 CLI 写入 catalog。

## 5. 新建、继续和状态

用户选择 workspace 并通过 /new 提供标题后，点击当前确认按钮。thread/start 只创建空会话；输入的标题作为显示名，原生标题为空时单独补写元数据。首轮仅由会话 topic 中新的用户消息启动，标题不自动发送。回执未知不重新创建 thread。空闲续轮只在用户动作内必要时 resume；active 使用 turn/steer + expectedTurnId，不改现有 model/cwd/审批/sandbox。

| 命令 | 行为 |
| --- | --- |
| /status | 只读刷新实际状态 |
| /history | 动态读取最近输入与同轮最终结果；长结果分段显示 |
| /info | 完整主机、路径、thread/provider/turn ID 与 emoji 状态 |
| /ask TEXT | 交给当前 thread；斜杠文本可通过此入口发送 |
| /steer TEXT | 仅追加到捕获的当前 turn，冲突重新确认 |
| /rename TEXT | 修改显示名/topic 名；原生标题为空时补写，已有原生标题保留 |
| /topics | 列出、删除或确认清除全部已记录的会话话题 |
| /refresh | 重新获取当前管理页；话题页也重新检查已知 ID |

没有 stop/cancel/kill/审批答复入口。waitingOnApproval/waitingOnUserInput 仅显示，交由原端处理。未授权、已撤销、旧 nonce、错误 peer、离线积压或过期输入都不得触发新业务操作。

## 6. 历史、合并与速率

Telegram 初次进入会话仅展示最近输入及本轮最终结果、最新进展或错误的短预览；不自动发送完整历史附件。正文过长时明确标为预览，并提示 /history 可查看最近交互；完整历史导出不提供。已绑定且建立观察基线的会话再次打开时保留游标和绑定版本，不重复发送历史，也不跳过尚未投递的新内容。无可展示内容时不发占位消息。内部读取仍用于建立去重基线，原生会话不受影响。

/status 的「未加载」表示当前 app-server 未加载该会话；不是历史丢失或读取失败。该状态仍可只读查看历史，明确发送新输入时才 resume。历史读取失败独立提示且按绑定版本去重，不把已成功读取的运行状态改成“连接未知”。

实时仅投递 agent 正文，不添加“用户/Agent”角色标题或重复用户输入，工具/推理仍过滤；同 session/peer/topic 的短时输出合并。不同用户私聊不能合并，不能将一个用户的历史集合发到另一个 peer。

每个 private chat 的多个 topics 共享 per-chat 预算，叠加 bot 全局限速，429 遵守 retry_after。当前默认每个私聊每分钟 18 条只是保守配置，不是套用群聊限额。成功发送后只记日志，不发送“开始新一轮/已追加/turn ID”等 ACK。不自动发送空闲/运行状态卡；详细信息与错误原因在 tracing 日志中。必要的冲突/未知结果仍用简短提示提醒用户核对。

## 7. 配置和账号变化

根目录 pocketpilot.toml 的 `[telegram].token` 为唯一凭据输入，default_locale 控制缺省语言，不读取 .env、不 include 其他配置。不需要 chat/user ID 配置。bot ID 由 getMe 确认，同 bot token 轮换保留配对；不同 bot 的旧配对、nonce、绑定、状态卡和积压不会自动继承。

用户删除 topic 或阻止 bot 时仅调整本地映射/投递，不停止 agent。关闭话题不等于删除，不能据此清除映射。解绑、撤销配对、退出 PocketPilot 和配置 reload 都不能发 unsubscribe/cancel 或重启服务。


## 8. 清空历史与话题恢复

Telegram 官方说明：删除包含创建 topic 根服务消息的历史时，应将 topic 视为删除。仅清空本地显示可能导致旧菜单不可见，Bot API 的旧消息编辑却仍成功。不能从“缓存 topic ID + edit 成功”推断用户能看到入口。

管理 topic 标题为纯文字「管理」（英文为 Manage），图标独立设置为原生 📁，不在标题中重复 emoji。显式 /start 会验证已有管理 topic，并修正旧版本的标题及图标；明确返回话题不存在则使旧本地入口失效，再创建一个管理 topic。未知网络结果不会触发重建。随后总是发送一条新的首页菜单，后续按钮步骤编辑同一菜单消息。Off-topic 的处理失败提示留在原位置，避免管理入口本身不可用时错误也被送到不可见处。

当前 Bot API 适配器不能直接读取话题列表或强制跳转客户端视图。官方的 View as messages 切换文档明确仅适用于超级群，不应作为 bot 私聊的通用操作方式。话题的客户端可见性仍需实际界面验证。

来源：[Bot forums / managing topics](https://core.telegram.org/api/forum#managing-topics)、[View as messages](https://core.telegram.org/api/forum#view-as-messages)。


## 9. Native emoji、typing 与语言

原生 topic icon 必须使用 getForumTopicIconStickers 返回的 custom emoji ID，不能把任意 Unicode 字符直接当 ID。Api 缓存一次可用图标目录，会话状态映射使用可用的 💬 / ⚡️ / 👀 / ❓ / ❗️ / ⁉️，管理入口独立使用 📁。未加载与空闲统一 💬，仅 /status 区分实际状态。状态、标题与绑定版本持久记录；同状态轮询不重复修改。待发送的状态变更覆盖合并，再经过现有 per-chat/bot 速率控制。修改可能出现 Telegram 服务提示，不能当成无成本动画。

状态图标目录读取失败时退避重试，只有携带真实图标 ID 的更新得到确认才记录图标回执。目录获取等待不阻塞正文队列；限流仍按 Telegram 的 retry_after 处理。元数据与原生执行、正文投递分别记账。

状态更新是没有正文的元数据投递，不产生“空闲”消息。升级时仅删除账本明确记录的旧 bot 状态卡，不删除用户消息或 agent 正文。标题和 icon 分开更新，正常状态切换不改标题。

运行中（不含等待审批/用户输入）建立 typing lease，独立任务每约 3–4 秒续期 sendChatAction。接受新输入后可立即唤醒；不依赖历史读取完成或新文本到达。每次发送重新检查配对、配置范围和绑定版本，解绑/撤销/离线/空闲停止续期，429 尊重 retry_after。停止此任务仅影响 Telegram 提示，绝不控制 agent。

共享 i18n crate 通过 rust-i18n 编译中英文 YAML，使用稳定 ID，调用始终显式传 locale，不修改进程全局 locale。preferences 按 bot/user 保存偏好；默认 zh-CN，可用 /language en 或 /language zh-CN，default_locale=auto 才跟随 Telegram 用户语言。只翻译产品文案，用户标题、路径与 agent 正文不翻译。

Agent Markdown 使用 pulldown-cmark 解析为公共文档，再转换为类型化 Telegram RichMessage blocks。标题、表格、列表、代码、引用、公式和脚注保留原生结构；安全标签白名单转换为样式，未知 HTML 作为文字，链接限制协议。长消息按结构分段并保存固定投递布局，不发送 .md 历史附件；未知网络结果不重发。


## 10. 话题管理与删除同步

首页提供「🗂 话题管理」，也可发送 /topics，路由到管理 topic。分页列出当前私聊中本程序记录的 session topics，包括以前已解绑但尚未删除的 topic；固定管理 topic 不在删除列表中。选择话题按钮只显示确认页，明确说明会删除整个 Telegram topic 及全部聊天记录。用户点击有效确认按钮后才执行 deleteForumTopic；返回可取消。操作只作用于 Telegram 和本地映射，不发 Codex 生命周期请求，原生会话可重新添加为新 topic。

Bot API 没有获取全部 topics 或单个 topic 的读取方法，也没有 forum_topic_deleted 更新。不能发现程序从未记录的 topic，不能保证用户在客户端删除后即时同步。/topics 和 /refresh 会检查当前页最多六个已知 ID：editForumTopic 不指定 name/icon，保留现有元数据；已在真实私聊管理 topic 验证返回 True。明确的 missing-topic 响应才使本地关联和待投递失效；TOPIC_CLOSED 单独处理，超时/限流/不明错误保留记录。发送正文或元数据时发现 topic 不存在也执行同样清理。

删除前在本地事务中复核配对与当前私聊归属，排除管理 topic，并持久登记 dispatching；禁用 binding、递增 revision、退役未开始的输入/输出及草稿按钮。bind 拒绝正在删除、结果未知或已删除的旧 topic。已经通过 dispatch 的原生请求不撤回。删除确认绑定已投递且未消费的当前菜单；延迟/重启后旧确认重新询问。

已确认删除或明确不存在时，topic provisioning 指针退役，后续可以为同一个 Codex thread 创建新 topic。未知结果保留 outcome_unknown 和暂停转发状态，重启不重发删除；只有用户重新选择并确认后才可再尝试。单一根 schema.sql 中维护 topic_removals，无 migration。

依据：[官方方法列表](https://core.telegram.org/bots/api)、[editForumTopic](https://core.telegram.org/bots/api#editforumtopic)、[deleteForumTopic](https://core.telegram.org/bots/api#deleteforumtopic)、[消息服务字段](https://core.telegram.org/bots/api#message)。


## 11. 命令菜单与批量清理

启动时通过 setMyCommands 注册私聊命令，提供 default/zh/en 三套说明。范围仅 AllPrivateChats；命令建议不代表已经配对，也不绕过现有权限检查。注册失败只记录日志，仍可手动输入命令；每套注册有五秒预算。菜单含 start/workspaces/topics/new/status/history/rename/refresh/language/help，没有停止会话命令。

/topics clear 或话题列表中的「🧹 清除全部会话话题」打开独立确认页，显示当前已记录的 topic 数量。确认范围保存为具体 ID 集合，不会把确认后新增的话题一起删除。仅删除本私聊中已记录的 session topics（包括已解绑的），保留固定管理入口及全部原生 Codex 会话。Bot API 无列表接口，程序从未记录的话题不在本次范围内。

批量清理逐项使用与单项删除相同的授权、账本和绑定失效规则，每次删除间隔至少 1.1 秒；任何明确失败、限流或未知结果都停止本批次，不自动重试。返回成功数量与未完成数量，用户检查后可发起新的确认。过期、积压或恢复后的按钮确认重新显示确认页；进程中断不自动继续未完成的删除。


## 12. 表格、显式历史与命令范围

Telegram 正文使用 RichMessage 原生 table，保留表头、单元格样式和列对齐，最多 20 列。超宽表转换为“列名：值”条目，保留链接与强调。分段同时约束文字、嵌套深度和包含嵌套项的块数量，过度嵌套降级为可读文字，内容不截断，不生成历史文件。

/history 每次直接读取当前原生会话，选最近一次用户输入后同一个 turn 中的已确认 final_answer；不采用观察缓存，也不输出 commentary、partial、工具或推理。后续 steer 之后没有最终结果时不沿用更早的最终结果。未完整读取历史时不假定手中的最后一条就是原生最近记录。显式历史包含用户输入及本轮错误；普通实时转发包含 agent 文本和同一原生会话中其他客户端的用户输入（🧑‍💻），精确去重 Telegram 自身输入。过程用 💭，最终输出用 ✅，错误用 ❌；缺 phase 不猜测最终结果。`/history errors` 查看最近十条错误；默认历史只提示较早轮的错误，不混入最新输入的结果。

BotCommandScope 官方仅支持 default、各聊天集合、单聊天及聊天成员等七种范围，没有 message_thread_id。整个私聊共享同一命令菜单；/help 按实际绑定状态显示上下文帮助，不在切换话题时反复覆盖整聊命令。

语言和类型化错误见 [共享 i18n](i18n.md)。官方来源：[BotCommandScope](https://core.telegram.org/bots/api#botcommandscope)。


## 13. 工作区来源与分页

主机下的项目从应用 catalog 读取，不等同于服务器文件系统目录列表。后台从该主机已有共享 Codex 服务的活跃、普通和归档会话 cwd 自动发现；每种会话列表完整读取后续 cursor，按 provider/cwd 去重并限制在允许根路径内。没有会话的新目录仍用 /add 登记。显式 /remove 写入发现排除记录，后台不会把该入口立即加回来；显式 /add 可重新恢复。已有名称、ID 和 worktree 配置不被发现过程覆盖。

普通浏览与 /new 共用工作区分页：按所属会话最大更新时间倒序，无时间记录者最后，再按名称、路径、ID 稳定排序，每页八项，超出时提供「下一页」。显示已登记总数，0 返回上一页或上级；选择项绑定完整 workspace ID，重名不会改变路由。旧 Host/NewHost 导航仍可读取，新翻页使用独立 Action，不引入数据库 migration。

后台每五分钟重新发现，失败后约三十秒重试；每台主机串行扫描，所有主机的后台发现任务最多四个，不阻塞 catalog 快照定期更新。工作区页 /refresh 会执行一次有十二秒等待预算的只读发现，超时显示已有项目并提示可能尚未完整。扫描只调用列表和已加载会话元数据读取，不调用 resume、turn/start 或 steer。目录直接扫描不作为默认自动来源。


## 14. 错误、附件与问题回顾

轮次 error.message 和失败/中断状态进入历史与实时消息；接收到的 error 通知也加入观察，已观察错误按 provider/thread 在 SQLite 留存。willRetry 用 ⚠️ 表示 Codex 重试，不代表桥接重发。每次连接故障只提示一次，成功读取后恢复；详细内部诊断不放进用户消息或日志。

Desktop 标准附件包装在展示时缩为正文与 📎 文件名，不预览图片、不改模型原文。Telegram 上传只转发 caption 文本并提示附件未转交；纯附件不启动任务。所有文本消息禁用链接预览。

完整问题与解决边界见 [Telegram topics 回顾](../research/telegram-topics.md)。


## 15. 正文格式

阶段 emoji 与首行正文之间只留一个空格，实时消息、初始历史和 /history 一致。渲染单独处理阶段标记与正文块结构，保留标题、列表、代码块及平台原生块布局。

GFM 解析后转换为 Telegram RichMessage blocks；普通中文文本的粗体边界提供有限兼容，代码、显式转义和链接保留其语义。平衡的安全行内标签与 details 转换为原生样式和折叠块，其余 HTML 作为文字。原生会话内容不修改；已冻结投递布局不因显示规则更新而自动重写。


诊断日志记录 API 方法、白名单错误类别、耗时、重试时间，以及观察到的消息阶段数量；图标目录获取、匹配和更新分别记录结果。不记录标题、正文、配置秘密、认证提示或原始响应。
