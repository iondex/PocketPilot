# 后续支持规划

PocketPilot 的扩展目标是让同一套会话体验覆盖更多消息工具和编程运行时。新增消息端复用已有 agent 能力；新增 runtime 复用已有聊天入口、授权和操作账本。

| 方向 | 目标 | 当前边界 |
| --- | --- | --- |
| Lark / 飞书 | 自建 bot 的私人入口、丰富卡片、文本与附件输入 | 待实现；优先消息端，不等待新 runtime |
| Claude Code | 已有独立 server 首版；后续补外部会话索引与结构化问答 | server 管理会话；真实 CLI/模型与权限流程仍需单独验收 |
| Pi Coding Agent | 长驻 RPC 与会话管理适配 | 待实现，需要明确进程所有权、版本和输入调度语义 |
| 公共交互 | 降低平台间重复逻辑，统一错误、恢复与能力呈现 | 按实际相同规则逐步收束，保留原生控件和授权差异 |

当前能力见[接入方案](integrations.md)和[当前架构](architecture.md)。以下均为后续方案；Lark 尚未在项目中实现或完成真实租户联调。

## Lark / 飞书

### 产品入口与会话隔离

**确定方案：企业自建应用开启机器人能力；私聊仅提供导航，原生 session 必须绑定已授权群中的独立 thread。** 出站 WebSocket 接收 `im.message.receive_v1` 和新版 `card.action.trigger`，无需本机公网回调入口。旧版卡片回调不能替代新版长连接事件。[L1] [L2] [L3]

使用顺序为 workspace（执行环境）→ 项目目录 → session → 明确选择已授权群 → 建立根消息和 thread。一个原生 session 对应一个群 thread，所有任务、附件和输出均路由该 thread；不能以同一私聊的“当前会话”作为多会话路由。默认只接收授权用户明确 @bot 的群 thread 输入。

官方话题接口支持群内 `reply_in_thread=true`，返回/事件携带 `thread_id`；绑定保存 `(region, app_id, tenant_key, owner, chat_id, thread_id, root_message_id)`。优先复用已授权普通群，不自动建群或改变群模式。thread 是消息容器，群设置决定内容可见性；它本身不是访问控制边界。[L4]

### 应用、身份与最小权限

应用的可用范围只覆盖安装者或明确指定用户。应用范围不是 PocketPilot 的执行授权：每次菜单、消息和卡片操作仍验证部署域、应用、租户、操作者、聊天和目标绑定，再检查配置允许的 provider/项目。事件的发送者必须是允许的 `user`，不把其他 bot 的消息作为原生运行时输入。[L3]

| 目的 | 首选范围 | 不随首版默认申请的范围 |
| --- | --- | --- |
| 接收私人输入 | `im:message.p2p_msg:readonly` + `im.message.receive_v1` | 通讯录、用户代理信息、读取所有群消息 |
| 机器人发送、回复、更新自己发送的卡片 | `im:message:send_as_bot`；发送、回复及卡片更新接口均把它列为可选权限之一 | 以用户身份发送、全量 `im:message` |
| 必需的群 thread 输入 | `im:message.group_at_msg:readonly`，机器人已入群且允许发言 | `im:message.group_msg`、包含其他 bot 的全群消息权限 |
| 可选输入附件 | 资源接口要求的 `im:message:readonly` 等授权之一；按实际安装域验证其审批与范围 | 为纯文本入口提前申请历史消息读取权限 |

权限依据分别来自接收事件、发送/回复、更新卡片与消息资源下载接口；它们是不同能力，不能用“能收到附件事件”推断“能下载资源”，也不能用发消息权限推断能读所有群消息。国内与国际控制台列出的权限名可能有差异，安装验收须逐项核对，表中不要求同时授予同一行所有候选权限。[L3] [L5] [L6] [L7] [L8]

建议身份模型区分以下 scope：

- `region`：国际 Lark 或国内飞书，决定控制台、REST、WS 发现及 token 的部署域。
- `app_id + tenant_key`：应用与租户组合；校验事件头应用、租户及操作者租户，拒绝不匹配事件。
- `open_id`：当前应用内的用户身份；同一人在不同应用中的值不同，不能跨应用复用。首版使用显式允许的 `open_id`，无需为获取 `user_id` 扩大通讯录权限。[L3] [L10]
- `chat_id`：私聊或明确允许的群；群模式另带 `thread_id + root_message_id`。当前私聊选择也必须包含用户与 revision，不能在全局共享“当前会话”。

官方 SDK 明确区分 `https://open.larksuite.com` 和 `https://open.feishu.cn`，并提供独立 OAuth 域名。**部署选择**是国际与国内分别配置和保管应用凭据、token 缓存、身份、绑定及去重账本；不通过换域名复用另一部署的配置，不自动跨域重试。官方两站的消息/卡片资料均有相应能力说明，但文档内容并非总是同步，两个域要分别验收。[L2] [L3] [L11]

### 长连接、回调和重复输入

官方长连接由应用使用 App ID / App Secret 主动建立；无需公网 IP 或域名。长连接回调要在 **3 秒内处理完成**；每应用最多 50 个连接，多客户端是随机一个接收的集群模式，不是广播。新版卡片回调同样要求 3 秒内响应，可返回 toast、卡片更新或保持内容。官方 Python 样例同时注册 `im.message.receive_v1` 和 `card.action.trigger` 并交给 WS client，证明这两种订阅可走同一长连接机制。[L1] [L2] [L12]

**实现选择**：首版每个已配置应用只运行一个事件接收 owner。WS handler 只做大小限制、解析、scope/新鲜度校验、持久化入队和快速响应；SSH 建连、附件下载和原生运行时操作在后台消费，不阻塞 3 秒回调。只有成功持久化才确认“已收到”，不能把 ACK 文案写成“已执行”。持久化失败返回明确失败，不能先确认后丢任务。WS 中断或多实例恢复也不自动产生执行授权。

官方消息事件明确要求用 **`message_id` 去重，不要依赖 `event_id`**。消息收件唯一键采用部署域、应用、租户与 `message_id`，保留来源 chat/thread；重复事件可重新 ACK，但不能再次投递原生输入。卡片回调另记录事件标识及已消费的动作 nonce；同一按钮多次点击、回调重投、菜单版本变化均只能消费一次授权。[L3]

发送和回复接口提供 `uuid`，官方承诺相同值在 **1 小时内至多成功发送/回复一条消息**；这不是任意时长的 exactly-once，也不是原生运行时幂等保证。[L5] [L6] PocketPilot 仍应在写前冻结 payload 和 operation ID，记录 `claimed → sent/failed/unknown`。超时、断线或丢失写回执进入 `unknown`，**不自动重发**消息创建、会话创建、resume、续轮或 steer；可用只读查询辅助核对，但查不到不能证明未执行。用户的新操作需新鲜授权并说明不确定状态。

所有 callback payload 都是外部输入。`value` 中的 session、路径或操作名不能直接作为权限；服务端 nonce 查回持久化动作，再复查用户、chat、绑定 revision、provider endpoint 和项目。长连接的传输鉴权也不能替代这些业务校验。若将来选择公网 webhook，才加入对应协议的签名、解密和 challenge；首版不为了旧卡片协议引入公网服务。

### 输入文件与资源预算

用户输入文件使用 **获取消息中的资源文件**：`GET /open-apis/im/v1/messages/{message_id}/resources/{file_key}?type=file|image`，以该部署的 `tenant_access_token` 鉴权。机器人必须和消息在同一会话，`file_key` 必须属于这个 `message_id`。这不是仅凭 `file_key` 下载机器人自己上传资源的接口；不能从附件事件提取任意 URL 并带 token 抓取。[L8]

飞书官方资源接口规定：文件不使用 Range 时仅支持小于 **100 MB**；100 MB 及以上文件需分片，单次不超过 **32 MB**；图片不支持 Range，仅可完整下载小于 100 MB 的资源。接口不支持表情包，保密消息/防泄密群、不可见消息、合并转发子消息和卡片等也存在拒绝边界。国际部署须重新核对对应资源接口，不能把国内文档的 Range 能力当成已经跨域验证。[L8]

**实现选择**：首版不做超大文件 Range 拼接；设置低于平台上限的单文件/单操作总字节、数量、超时、并发和临时磁盘预算，读取过程中计数并中止超额，不只相信 `Content-Length`。先校验用户、绑定、输入动作和时效，后下载。下载走 `AttachmentSource`；限制目标域与重定向，不把 token 发到不受信任主机；文件名仅作显示，远端路径由既有附件流程生成。拒绝无法证明归属的资源，并清理失败的本地下载暂存；已上传到 runtime 的文件可能被后续轮次引用，按其保留策略处理。文件可下载不等于 provider 支持该模态，模型输入能力仍由原生 adapter 决定。

首版沿用现有附件产品边界：文件由用户上传给 agent，不增加历史导出或自动生成 `session.md`。超长结果使用有界富文本分段，不能把合并转发接口当作尚未发送的原生历史生成器。

### 尽可能丰富的正文与卡片

**推荐选择**：可读正文优先 `post` 或 Card JSON 2.0 原生组件；导航、会话目标、状态与动作使用 Card JSON 2.0。不能把 Lark 降成纯文本通道，也不能把未经处理的 agent 原文当可信卡片 Markdown/HTML。

| 内容 | 官方能力 | PocketPilot 渲染方向 |
| --- | --- | --- |
| 普通回复与长文 | `post` 支持文本、链接、图片、视频、`code_block`、`md`；`md` 独占段落，回读会转换成其他标签。[L13] | 语义分段，保留代码语言与链接；文本、代码、图片分别编码，分批发送时不破坏语法 |
| 标题、引用、列表、代码、表格 | Card JSON 2.0 的 `markdown` 组件支持标题、引用、代码、表格等；当前专门的 2.0 文档称支持除 `HTMLBlock` 外的标准 Markdown 与部分 HTML。[L14] | 从公共 Markdown 解析结果生成受控子集；代码内容保持字面值，链接 scheme 受限，不让正文注入 `at`、人员控件或动作 |
| 表格 | 卡片 Markdown 表格五行后分页，每个富文本组件最多四个表格；原生 table 组件可设置列宽、对齐。[L14] | 小表使用原生能力，宽/长表按可读预算分段，必要时转换为列名与值的分段文本；不静默丢列 |
| 会话导航与操作 | JSON 2.0 共享卡片、布局及交互组件；最多 200 个元素/组件，客户端版本门槛为 7.20。[L15] | 展示目标、运行状态、观察时间、绑定管理和明确动作；过量菜单分页，旧客户端提供独立文本命令/消息降级 |
| 数学、脚注、特殊扩展 | 上述标准 Markdown 声明不等于已保证全部 GFM/LaTeX/脚注扩展 | 未经对应组件和客户端验证的格式用可读文本；不声明 Native 能力 |

国内目前的 `post` 文档推荐 `md` 使用 CommonMark 0.31 + GFM，国际文档也有 `md`，但不能仅凭 tag 同名假设扩展完全一致；首批格式验收应按两种部署建立 fixture。卡片 2.0 的专门组件文档优先于仍含旧限制的 1.0 富文本介绍。[L13] [L14]

发送/回复接口的上限是文本 **150 KB**、卡片和 `post` **30 KB**；模板展开和样式也计入大小。渲染预算按最终 UTF-8 payload 与元素数计算并留余量，不把字符数当字节数；超限先语义切分，复杂布局降级为可读文本。[L5] [L6]

卡片更新有三条不同边界，必须分别建模：

- 回调即时响应：3 秒；可以只发 toast 并异步处理。[L1]
- 回调 token 延时更新：token 有效 **30 分钟，最多更新 2 次**，不是长期状态凭据。[L1]
- 按 `message_id` 更新已发送卡片：仅限 **14 天内**的卡片，更新前后均须共享卡片配置 `update_multi=true`；JSON 2.0 目前只支持共享卡片，不能设计群内每用户独享的私密菜单。[L7] [L15]

因此群卡片只含群可见信息；私人目标选择放在私聊。超期、撤回或删除的卡片不得被当作永久可更新面板：提示用户打开新的目录卡片，按显式操作建立新 panel/message 映射并废弃旧 nonce。会话绑定与 `thread_id/root_message_id` 独立于可替换的状态面板；根卡片过期不应重新创建 thread 或丢失旧路由。后台观察可以更新尚可更新的面板，但不能为了更新标题而偷偷建新群、新会话或反复发送目录。

### Rust 实现与现有模块复用

**选型**：[OpenLark](https://github.com/foxzool/openlark) 的拆包版 `openlark-client` / `openlark-core` **0.21.0** 是社区 Rust SDK，不是官方 SDK；使用 auth/websocket 最小依赖，不引入旧单体 `open-lark 0.14`。已核对发布源码的 typed 消息/新版卡片回调及 ACK，尚未接入本项目编译或真实租户测试。[L16]

启用前须对 client 做窄修补：分片数量、单消息/合计字节、未完成消息数和 TTL 均有界；去除带认证参数的 WS URL 日志；心跳以有效入站活动刷新。宿主独立禁用 SDK 传输日志，SDK 错误只映射为脱敏类型。SDK 没有自动重连，由现有 integration 生命周期实现有界退避；HTTP 禁用隐式代理、重定向和自动重试，WS 代理复用 `network::Relay` 并加明确 connector seam，不能配置了代理却直连。REST 使用 core 的 typed Transport 和具名 DTO，不把 SDK 的 Value 传入业务层。

对选定版本补上目标域、token 刷新、分片预算、心跳/重连、背压及脱敏的编译与故障验收，通过后封装为 `integrations` 内部依赖；不通过则比较维护必要 REST/WS 子集的成本，复用现成 HTTP/TLS/WebSocket/protobuf 库，并以官方 SDK wire 行为作参考。官方 Python WS 实现处理二进制 protobuf、心跳与分片，不是简单 JSON 文本 read loop。[L17] 默认继续纯 Rust；Python/Go sidecar 只能作为另行评估的部署选择，不能默认为必需依赖。

拟新增 `crates/integrations/src/lark/`，将 API DTO、配置、WS 接收、入站输入、附件、渲染、投递、观察、菜单和平台 SQL 留在该目录。复用现有公共层，按必要能力扩展具名枚举，而非复制另一平台的所有实现：

- `Integration`：运行生命周期、公共上下文和平台 capabilities；目前 `Platform` 仅有 Telegram/Slack/Discord，Lark 仍需正式实现。
- `Renderer`：纯渲染、无 I/O、在发送前冻结可序列化的 native parts；为 Lark 增加独立格式和真实预算，不能借用 Slack/Telegram 的 capability 声明。
- `AttachmentSource` 与既有附件暂存/远端传递：把消息资源来源封装在平台边界内，下载前先授权。
- `navigation`：复用执行环境 → 项目 → 会话分页、provider 与 cwd 核验；Lark 卡片只负责原生布局与动作编码。
- `bindings`：复用清单、暂停/恢复观察、解除绑定确认、revision 和 session cache；增加有域/应用/租户/用户/chat/thread 的 `Scope`/`Target`，实现 Lark 自己的 repository，SQL 不外溢到 app。

应用身份、允许列表、回调 nonce、消息收件账本、冻结投递、平台消息映射与观察 cursor 持久化后才能启用写操作。默认后台只读；接入不增加 interrupt、stop、审批代答等控制，不用重新发一次输入来恢复断线。

### 分阶段范围与验收

| 阶段 | 范围 | 进入下一阶段前的门槛 |
| --- | --- | --- |
| L0：能力原型 | 单个自建应用、单部署域、单授权用户；官方样例或 Rust 候选只验证传输 | 消息与新版卡片回调均通过出站 WS；3 秒响应有证据；安装权限明确；不调用生产原生运行时 |
| L1：群会话入口 | 私聊目录 → workspace → 项目 → session；每个 session 在已授权群建立独立 thread，任务在该 thread 提交 | 陈旧卡片、并发切换目标、重复 `message_id`、重复点击、越权租户/用户/chat、重连和重启均不串会话；写结果 unknown 不重发 |
| L2：完整内容 | `post`/JSON 2.0 渲染、输入附件、长输出分段、有界观察和状态卡更新 | 代码/表格/中文/转义/UTF-8 大小、文件归属与超额中止、过期卡片、权限撤销、token 轮换和限流均通过；不能下载或不能渲染时明确降级 |
| L3：群 thread 隔离验证 | 从首版必需：用户明确指定已有群，根消息 + thread，默认只处理 @bot 输入；此阶段扩大故障验证 | 两个原生会话隔离、错误 thread/root 拒绝、群成员可见性、移除 bot、删除根消息、跨用户卡片均通过；证明无需全群读取也能完成宣称流程 |
| L4：第二部署域 | 国际 Lark / 国内飞书另一套独立配置 | 独立应用凭据、权限、WS callback、附件和渲染全流程验证；未通过的一域保持未支持状态 |

待真实原型核实：候选 Rust 库对新版回调响应的实际覆盖、目标域 post 扩展和最新资源下载差异、指定群内 @bot 的 thread 路由，以及长期根消息被撤回后的用户恢复流程。任何未知都阻止相应 capability 升级；完成文档与阅读官方样例不等于“已支持 Lark”。

### Lark 一手来源

- [L1：新版卡片回传交互、3 秒响应与延时 token](https://open.feishu.cn/document/feishu-cards/card-callback-communication)
- [L2：飞书使用长连接接收回调](https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/event-subscription-guide/callback-subscription/configure-callback-request-address)；[国际 Lark 对应文档](https://open.larksuite.com/document/uAjLw4CM/ukTMukTMukTM/event-subscription-guide/callback-subscription/configure-callback-request-address)
- [L3：接收消息事件、权限与 message_id 去重](https://open.feishu.cn/document/server-docs/im-v1/message/events/receive)；[国际 Lark 对应事件](https://open.larksuite.com/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1/message/events/receive)
- [L4：话题概述、thread ID 和群类型](https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1/message/thread-introduction)
- [L5：发送消息、权限、大小与 uuid](https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1/message/create)
- [L6：回复消息、reply_in_thread 与 uuid](https://open.feishu.cn/document/server-docs/im-v1/message/reply)
- [L7：更新已发送卡片、共享模式与 14 天限制](https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1/message/patch)
- [L8：获取消息中的资源文件、权限与 Range 限制](https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1/message-resource/get)
- [L10：用户身份概述、open_id 的应用范围](https://open.feishu.cn/document/home/user-identity-introduction/introduction)
- [L11：官方 Python SDK 的飞书/Lark 域名定义](https://github.com/larksuite/oapi-sdk-python/blob/0d9749a4728446a0ffa2e8374b37b5e1ce1cfd9c/lark_oapi/core/const.py)
- [L12：官方 Python SDK WS 事件和卡片样例](https://github.com/larksuite/oapi-sdk-python/blob/0d9749a4728446a0ffa2e8374b37b5e1ce1cfd9c/samples/ws/sample.py)
- [L13：飞书发送消息内容结构、post/md](https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/im-v1/message/create_json)；[国际 Lark 消息内容结构](https://open.larksuite.com/document/uAjLw4CM/ukTMukTMukTM/im-v1/message/create_json)
- [L14：Card JSON 2.0 富文本组件](https://open.feishu.cn/document/uAjLw4CM/ukzMukzMukzM/feishu-cards/card-json-v2-components/content-components/rich-text)；[国际 Lark 对应组件](https://open.larksuite.com/document/uAjLw4CM/ukzMukzMukzM/feishu-cards/card-json-v2-components/content-components/rich-text)
- [L15：Card JSON 2.0 结构、共享卡片和元素限制](https://open.feishu.cn/document/feishu-cards/card-json-v2-structure)
- [L16：OpenLark 维护者仓库](https://github.com/foxzool/openlark)；[crate 元数据](https://crates.io/crates/openlark-client)（社区候选的一手描述，不是官方平台保证）
- [L17：官方 Python SDK WS client 实现](https://github.com/larksuite/oapi-sdk-python/blob/0d9749a4728446a0ffa2e8374b37b5e1ce1cfd9c/lark_oapi/ws/client.py)

[L1]: https://open.feishu.cn/document/feishu-cards/card-callback-communication
[L2]: https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/event-subscription-guide/callback-subscription/configure-callback-request-address
[L3]: https://open.feishu.cn/document/server-docs/im-v1/message/events/receive
[L4]: https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1/message/thread-introduction
[L5]: https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1/message/create
[L6]: https://open.feishu.cn/document/server-docs/im-v1/message/reply
[L7]: https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1/message/patch
[L8]: https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1/message-resource/get
[L10]: https://open.feishu.cn/document/home/user-identity-introduction/introduction
[L11]: https://github.com/larksuite/oapi-sdk-python/blob/0d9749a4728446a0ffa2e8374b37b5e1ce1cfd9c/lark_oapi/core/const.py
[L12]: https://github.com/larksuite/oapi-sdk-python/blob/0d9749a4728446a0ffa2e8374b37b5e1ce1cfd9c/samples/ws/sample.py
[L13]: https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/im-v1/message/create_json
[L14]: https://open.feishu.cn/document/uAjLw4CM/ukzMukzMukzM/feishu-cards/card-json-v2-components/content-components/rich-text
[L15]: https://open.feishu.cn/document/feishu-cards/card-json-v2-structure
[L16]: https://github.com/foxzool/openlark
[L17]: https://github.com/larksuite/oapi-sdk-python/blob/0d9749a4728446a0ffa2e8374b37b5e1ce1cfd9c/lark_oapi/ws/client.py

## Claude Code 与 Pi 的 runtime 适配

### 运行时服务的职责

Codex 已有 app-server，继续接入原生服务。其他 agent 可能需要独立的 runtime server，负责会话目录、状态和事件、受控文件暂存、写操作账本与长驻进程。聊天连接通过通用 `AdapterFactory` / `AgentProvider` 契约访问它，不将 CLI 参数或进程管理泄露到 integration。

服务进程与连接生命周期分开：桥接退出、SSH 断开或一个客户端关闭，不应结束执行任务。服务可以拥有自己创建的 agent 进程；已有原生终端进程是否可附着必须单独证明。能读 transcript 不代表知道实时状态，也不代表获得控制权。

`RuntimeKind` 对应 Codex 原生入口和 Claude Code server 首版；`pi` 仍只有枚举与安装协议的扩展位置，没有可运行 adapter。后续按以下能力逐项协商，而非要求各 runtime 假装与 Codex 完全等价：

| 能力 | 要求 |
| --- | --- |
| 会话目录与读取 | 按项目筛选、稳定分页、只读历史；不能为了浏览而启动或 resume agent |
| 状态与观察 | 区分活跃、空闲、需用户操作、断线和未知；快照、事件缺口及缓存时间有明确定义 |
| 所有权与控制 | 分清可发现、可观察、可控制和可实时附着；只控制已授权并可证明归属的实例 |
| 输入与调度 | 空闲续轮、运行中追加、排队下一轮分别表达；不得用 interrupt 模拟 steer |
| 回执与重连 | 接受输入与完成执行分开；同一操作去重，回执未知不重发 |
| 文件与图片 | 有界暂存与模态能力声明；不靠工具调用伪造文件上传或扩大项目权限 |

### Claude Code

首版已采用纯 Rust `pocketpilot-agent-server` 管理 Claude stream-json 进程，不要求额外 Node/Python SDK sidecar。当前能力与安装方式见[接入方案](integrations.md#claude-code-独立-runtime-server)。只控制 server 自己管理的会话，不宣称能附着任意终端；官方 SDK 的 subprocess 协议仍需随 CLI 版本复核，本机核对版本为 CLI 2.1.304 / SDK 0.3.304。

后续补充只读外部 transcript 索引、状态事件覆盖、结构化问题和真实端到端验证。活跃轮次 steer 未证明，不能将排队输入当成即时 steer。工具权限请求目前保持 pending，不自动批准或拒绝；原生提问的回答接口按下面的公共模型单独交付。

参考：[官方 TypeScript SDK](https://github.com/anthropics/claude-agent-sdk-typescript)、[官方 Python SDK](https://github.com/anthropics/claude-agent-sdk-python)。

### Pi Coding Agent

目标为 Pi Coding Agent，先确认所部署包和仓库对应的版本。评估以独立服务持有长驻 RPC 进程，通过其原生会话与事件接口映射公共模型。普通交互终端不自动等同于可共享的 RPC server。

重点区分 `steer` 与 `follow_up` 的实际交付时机，以及会话切换是否可能被扩展取消、历史分支如何还原、进程重启后如何确认任务状态。只读目录不能用可能修改会话文件的恢复操作代替；没有通用文件接口时，由 runtime server 提供受限暂存，不用 shell 任务模拟上传。

参考：[Pi Coding Agent](https://github.com/badlogic/pi-mono/tree/main/packages/coding-agent)。包名或仓库迁移时重新核对部署版本和协议，不沿用旧版本的能力假设。

### ACP 的位置

ACP 可作为兼容入口或 runtime driver 的一种协议选择，不作为内部能力上限。选择时核对实际实现声明的会话列表、恢复、历史、事件和权限能力；不能把“支持 ACP”直接解释为纯只读观察、多客户端共享写入或任意活跃终端附着。公共操作账本、目标验证与 unknown 语义仍由 PocketPilot 保持。

参考：[Agent Client Protocol](https://agentclientprotocol.com/)、[Rust SDK](https://github.com/agentclientprotocol/rust-sdk)。

## 交付顺序

1. 完成 Lark 的单应用、单部署域、单授权用户传输原型，确定 Rust 实现与最小权限。
2. 交付 Lark 私人目录与授权群 thread：一个 session 一个 thread，首版即为必需；输入表单、富文本和附件复用公共层，Codex 完成导航到原生回执的闭环。
3. 从首版采用明确授权的群 thread，再独立完成另一部署域；逐项开放已验证能力。
4. 在已交付的 server 生命周期和能力契约上扩展 Claude Code 的观察/问答覆盖，并交付 Pi 的只读观察及受控输入，不让聊天入口重复实现 agent 协议。
5. 持续将已经相同的导航、确认、错误和恢复规则放入公共模块；平台身份、ACK 和原生容器管理保留各自实现。

每一阶段都以授权隔离、重复事件、过期操作、断线后任务存活、未知结果不重发及真实客户端展示为交付门槛。完成后把“已支持”行为更新到当前架构与接入方案，后续支持文档只保留尚未完成的内容。


## Agent 提问与用户回答（规划，尚未实现）

聊天入口应能展示 agent 的问题，并将用户答案送回原始请求。此功能与工具权限审批分开；问题按钮不会批准 Bash、文件操作或修改权限规则。普通消息输入继续使用现有授权、绑定、文件和操作账本。

### 两条回复路径

| 场景 | 展示与回复 | 原生操作 |
| --- | --- | --- |
| 普通 agent 文本提问 | 保留正文 Markdown；用户直接在绑定 thread/topic/channel 回复 | Codex 活跃轮次使用带 `expectedTurnId` 的 steer，空闲轮次使用 start；Claude 空闲时发送下一轮 user input |
| Codex 结构化提问 | 按问题顺序显示标题、选项说明与允许的自由输入；提交前显示待提交答案 | 对原始 JSON-RPC request ID 返回 `ToolRequestUserInputResponse`；不新建 turn，不以 steer 冒充答案 |
| Claude `AskUserQuestion` | 保留原问题、单选/多选和自由输入；答案提交到原始 tool request | 对匹配的 `control_request.request_id` 返回 `control_response`，保留原工具输入并补充 answers |
| 其他工具权限请求 | 仅展示“等待原生端权限处理”状态 | 保持 pending，不批准、不拒绝、不发送 interrupt |

Claude 的活跃输入不能因传入 stream-json 就宣称为 steer。当前 server 对活跃轮次返回不支持；普通文本回答在 agent 已结束轮次时通过 user input 继续。以后只有证实原生插入时机及前置轮次约束后，才开放真正 steer；不会偷偷排队下一轮或中断现有执行。

### 公共模型和模块边界

`session` 定义具名 `QuestionSet`、`Question`、`Option`、`AnswerDraft`、`AnswerSubmission` 与 `QuestionState`，描述顺序、稳定 ID、单选/多选、可选自由输入和是否阻塞。运行时能力单独声明结构化问题的读取、回答、多选与自由输入支持；不将 `control_verified` 当成所有能力都存在。

问题的逻辑身份至少包括 provider/instance、session、turn、item/tool-use、原生 request ID、连接 epoch 和问题 revision。问题正文与答案是用户数据，不能放进日志或按钮 value；原生请求数据只在 adapter/server 内保存为有界的具名 DTO，integration 只拿不透明 question handle。Claude 无稳定 question ID 时生成本请求内 ID，同时保存到原完整问题文本的映射。

`adapters` 负责捕获原生请求、能力协商和原样返回协议响应。独立 server 持有 Claude 的 stdin 与 pending request，因此聊天断开不撤销原生问题。`engine` 负责问题仓库、有效性核验与提交账本。`integrations` 公共层负责选择、修改、确认和提交的相同状态转换；各平台仅负责组件、回调身份和输入路由，不复制 Codex/Claude 的协议。

初版接口建议为 `pending_questions(session)`、`question_changes()` 和显式 `answer_question(submission)`，与 `execute(Mutation)` 的普通输入分开。只读观察不持有自动回答路径。提交必须在持久化授权后，经实例锁/会话锁重新检查目标、revision、epoch、当前原生 pending 状态和允许回答的用户。

### 原生协议映射

Codex：已核对本机 `codex-cli 0.162.1` 生成的 JSON schema。方法是实验性 `item/tool/requestUserInput`；params 包含 `threadId`、`turnId`、`itemId`、`isBlocking` 和 questions，单题有 `id`、`header`、`question`、`options`、`isOther`、`isSecret`。不要从旧 `autoResolutionMs` 自行推算默认答案或自动回答期限。响应形状为 `answers: {question_id: {answers: [string, ...]}}`，通过原连接返回原 request ID。构建时继续使用生成 DTO；升级 CLI 后复核实验性字段，并保留 initialize 的实验能力协商。只能回答投递到本连接且仍可核验的原生 request；不能把其他 Desktop/CLI 连接上的 request ID 复制过来接管问题。

Claude：按官方 Agent SDK 的 `can_use_tool` / `AskUserQuestion` 边界实现。保存原 questions、`request_id` 与 `tool_use_id`；成功响应为 `control_response.response = {subtype: success, request_id, response: {behavior: allow, updatedInput: ...}}`。这里的 allow 只准用于已经核验为 AskUserQuestion 的原请求；不能给任意 `can_use_tool` 共用一个“允许”函数。`updatedInput` 保留原 questions，answers 以完整问题文本为 key、所选 label/自由文本为 value；当前 SDK 为 string map，多选按该版本的约定序列化，不能擅改为数组或用展示序号代替 label。不发送 `updatedPermissions`。

Claude CLI 2.1.304 / SDK 0.3.304 的 initialize 响应还提供 `pending_permission_requests` / `pending_user_dialog_requests`。恢复时应合并快照和 live/replayed request，以原 request ID 去重；字段缺失不能当成空列表。继承自旧 worker 的请求未必出现在列表中，不能从本地 pending 为空推断原生端没有等待项；未知继承请求仍需只读核验，不能重建猜测的 request ID。

Claude 的 `control_cancel_request` 立即使 pending 及未提交草稿失效。Codex 的请求取消、连接替换、原生轮次结束或原始 request 不再 pending 时也要废弃旧入口；事件缺口只能触发只读重新核验，不能触发默认回答。仅在原生协议证明非阻塞问题仍有效时保留其回答入口，不能单凭 turn 已结束推断所有问题有效或失效。

### 聊天交互

| 平台 | 选项与回答入口 | 路由边界 |
| --- | --- | --- |
| Slack | Block Kit 单选/多选组件；Modal 收集自由文本和多题；thread 中保留状态摘要 | App、team、owner、绑定 thread 与 question revision |
| Discord | Components V2 展示；select/menu 选择；Modal 输入自由文本；多页只保留草稿，不提前发送 | guild、owner、绑定 channel、question revision；不向用户展示 raw ID |
| Telegram | Inline keyboard 选择；多选有“确认”按钮；自由文本回复指定问题消息 | 固定允许 user ID、群 topic、问题消息；无关联文本继续走普通输入 |
| Lark / 飞书 | JSON 2.0 卡片单选/多选与 form；超过 input 长度上限时使用带明确问题引用的 thread 文本 | 域、app、tenant、owner、允许群、root/thread、revision |

全平台显示“❓ Agent 提问”、问题序号、选项说明、草稿/已提交/已取消状态；回答成功后关闭旧控件，避免多次点击。多题完成并确认后才一次性回传。只存在一个 pending 问题也不能把所有普通文本自动截走：自由文本回答必须来自专用表单、明确 reply-to 或显式“回答此问题”上下文；没有这个上下文的文本按普通输入处理。群内卡片是群可见信息，不能用按钮的用户校验假装内容私密；`isSecret` 问题不提供群内填写，初版提示回原生端处理。

### 可靠性与验收顺序

1. 先补 adapter 的请求/响应读写边界和 fake-runtime fixtures：结构化请求、取消、重复 request、错误 session、旧 epoch、事件缺口；不启动真实模型。
2. 引入问题状态机：pending → draft → submitting → answered；取消进入 cancelled，原生状态无法核实进入 unavailable，发送后丢回执进入 unknown。草稿修改不发送原生响应；取消/失效后不能复活。
3. 以提交 ID 记录答案 digest、目标 revision 和 dispatch intent。一次提交最多写一次；unknown 不重发。重连后先查询可恢复 pending，绝不将旧 JSON-RPC ID 回到新连接；server 重启不能仅凭数据库中的旧 pending 发送答案。
4. 用同一组公共行为测试覆盖四个平台的越权点击、多选修改、自由输入、重复提交、陈旧菜单和并发取消；另验证 native wire 返回原 ID、原 questions 未丢字段、其他工具权限没有被回答。
5. 最后逐个平台做真实测试，确认富文本、交互响应期限及长题分段，再将对应能力由“规划”升为“已支持”。没有凭据的平台保持离线验证状态。

参考：[Codex app-server 协议](https://developers.openai.com/codex/app-server)、[Claude Agent SDK 用户输入](https://platform.claude.com/docs/en/agent-sdk/user-input)、[Claude Agent SDK Python 实现](https://github.com/anthropics/claude-agent-sdk-python)。
