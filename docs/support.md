# 后续支持规划

PocketPilot 的扩展目标是让同一套会话体验覆盖更多消息工具和编程运行时。新增消息端复用已有 agent 能力；新增 runtime 复用已有聊天入口、授权和操作账本。

| 方向 | 目标 | 当前边界 |
| --- | --- | --- |
| Lark / 飞书 | 自建 bot 的私人入口、丰富卡片、文本与附件输入 | 待实现；优先消息端，不等待新 runtime |
| Claude Code | 发现、观察和明确授权的会话输入 | 待实现，需要验证独立 runtime server 与官方接口 |
| Pi Coding Agent | 长驻 RPC 与会话管理适配 | 待实现，需要明确进程所有权、版本和输入调度语义 |
| 公共交互 | 降低平台间重复逻辑，统一错误、恢复与能力呈现 | 按实际相同规则逐步收束，保留原生控件和授权差异 |

当前能力见[接入方案](integrations.md)和[当前架构](architecture.md)。以下均为后续方案；Lark 尚未在项目中实现或完成真实租户联调。

## Lark / 飞书

### 产品入口与会话隔离

**推荐选择：企业自建应用开启机器人能力，以单人授权私聊作为首个入口，出站 WebSocket 接收消息和新版卡片回调。** 这条路线不需要为本机部署公网回调入口。官方长连接目前仅支持企业自建应用；新版 `card.action.trigger` 支持长连接，旧版卡片回调不支持。消息事件是 `im.message.receive_v1`，需要启用机器人能力并单独订阅。[L1] [L2] [L3]

保留两种交互方案，先完成 A，再按实际需要启用 B：

| 方案 | 建议交互 | 隔离方式与限制 |
| --- | --- | --- |
| A：私人 bot 私聊 | 私聊卡片选择执行环境、项目、会话；清楚展示当前目标，再显式提交输入 | 私聊本身只有一个聊天流。每个绑定保存自己的面板与 revision，任务经带会话 nonce 的卡片输入表单提交；普通文字打开目录。附件先成为待确认输入，再明确选择目标并确认，不依赖“最近会话”。每段输出标注执行环境、项目与会话 |
| B：私聊目录 + 人工指定的会话群 | 私聊管理目录；用户明确选择已配置、已授权的群；每个原生会话绑定一条根消息及其 thread | 以 `(region, app_id, tenant_key, chat_id, thread_id)` 隔离并保存 `root_message_id`；群成员可见输出，授权不能仅靠 thread ID。默认只接收明确 @ 当前机器人的用户输入 |

官方话题文档明确说明在群内使用 `reply_in_thread=true` 创建话题、从响应或消息事件获取 `thread_id`；该 ID 在当前租户内唯一。文档区分旧话题群 `chat_mode=topic` 与普通群的话题消息形式。**这些事实不足以证明应用机器人私聊可提供同样的 thread 容器**，因此 A 不依赖私聊 thread；B 优先使用现有普通群的根消息回复，群模式不由 PocketPilot 自动更改。[L4]

这是私人入口优先的产品选择：不按项目自动建群，不默认申请全群消息权限。多个原生会话在同一个私聊中不会获得原生独立聊天窗口；若用户需要并列查看与持续讨论多个会话，再使用 B。群 thread 是组织讨论的容器，不是访问控制边界；话题及消息历史可见性受群设置影响。[L4] [L5]

### 应用、身份与最小权限

应用的可用范围只覆盖安装者或明确指定用户。应用范围不是 PocketPilot 的执行授权：每次菜单、消息和卡片操作仍验证部署域、应用、租户、操作者、聊天和目标绑定，再检查配置允许的 provider/项目。事件的发送者必须是允许的 `user`，不把其他 bot 的消息作为原生运行时输入。[L3]

| 目的 | 首选范围 | 不随首版默认申请的范围 |
| --- | --- | --- |
| 接收私人输入 | `im:message.p2p_msg:readonly` + `im.message.receive_v1` | 通讯录、用户代理信息、读取所有群消息 |
| 机器人发送、回复、更新自己发送的卡片 | `im:message:send_as_bot`；发送、回复及卡片更新接口均把它列为可选权限之一 | 以用户身份发送、全量 `im:message` |
| 可选群输入 | `im:message.group_at_msg:readonly`，机器人已入群且允许发言 | `im:message.group_msg`、包含其他 bot 的全群消息权限 |
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

**选型候选**：[open-lark](https://github.com/foxzool/open-lark) 是社区 Rust SDK，不是官方 SDK。其当前 README 自称支持长连接、卡片和消息，同时仍列出 WebSocket 完善、更多事件处理器待办；不能据此宣称具体新版回调已经完整实现。本规划不锁未经编译验证的版本，也不引用 stars/下载量作为成熟保证。[L16]

先审查候选的目标域设置、事件/卡片响应、token 刷新、protobuf 分片、心跳/重连、限流、背压、秘密脱敏和许可证。通过后封装为 `integrations` 内部依赖；不通过则比较维护必要 REST/WS 子集的成本，复用现成 HTTP/TLS/WebSocket/protobuf 库，并以官方 SDK wire 行为作参考。官方 Python WS 实现处理二进制 protobuf、心跳与分片，不是简单 JSON 文本 read loop。[L17] 默认继续纯 Rust；Python/Go sidecar 只能作为另行评估的部署选择，不能默认为必需依赖。

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
| L1：私人入口 | 私聊目录、带目标的卡片输入表单、文本任务；复用绑定清单和现有受控会话动作 | 陈旧卡片、并发切换目标、重复 `message_id`、重复点击、越权租户/用户/chat、重连和重启均不串会话；写结果 unknown 不重发 |
| L2：完整内容 | `post`/JSON 2.0 渲染、输入附件、长输出分段、有界观察和状态卡更新 | 代码/表格/中文/转义/UTF-8 大小、文件归属与超额中止、过期卡片、权限撤销、token 轮换和限流均通过；不能下载或不能渲染时明确降级 |
| L3：可选群 thread | 用户明确指定已有群，根消息 + thread，默认只处理 @bot 输入 | 两个原生会话隔离、错误 thread/root 拒绝、群成员可见性、移除 bot、删除根消息、跨用户卡片均通过；证明无需全群读取也能完成宣称流程 |
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
- [L16：open-lark 维护者仓库](https://github.com/foxzool/open-lark)；[crate 元数据](https://crates.io/crates/open-lark)（社区候选的一手描述，不是官方平台保证）
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
[L16]: https://github.com/foxzool/open-lark
[L17]: https://github.com/larksuite/oapi-sdk-python/blob/0d9749a4728446a0ffa2e8374b37b5e1ce1cfd9c/lark_oapi/ws/client.py

## Claude Code 与 Pi 的 runtime 适配

### 运行时服务的职责

Codex 已有 app-server，继续接入原生服务。其他 agent 可能需要独立的 runtime server，负责会话目录、状态和事件、受控文件暂存、写操作账本与长驻进程。聊天连接通过通用 `AdapterFactory` / `AgentProvider` 契约访问它，不将 CLI 参数或进程管理泄露到 integration。

服务进程与连接生命周期分开：桥接退出、SSH 断开或一个客户端关闭，不应结束执行任务。服务可以拥有自己创建的 agent 进程；已有原生终端进程是否可附着必须单独证明。能读 transcript 不代表知道实时状态，也不代表获得控制权。

现有代码的 `RuntimeKind` 已列出 `claude_code` 和 `pi`，但没有对应可运行 adapter；枚举值不构成支持承诺。后续按以下能力逐项协商，而非要求各 runtime 假装与 Codex 完全等价：

| 能力 | 要求 |
| --- | --- |
| 会话目录与读取 | 按项目筛选、稳定分页、只读历史；不能为了浏览而启动或 resume agent |
| 状态与观察 | 区分活跃、空闲、需用户操作、断线和未知；快照、事件缺口及缓存时间有明确定义 |
| 所有权与控制 | 分清可发现、可观察、可控制和可实时附着；只控制已授权并可证明归属的实例 |
| 输入与调度 | 空闲续轮、运行中追加、排队下一轮分别表达；不得用 interrupt 模拟 steer |
| 回执与重连 | 接受输入与完成执行分开；同一操作去重，回执未知不重发 |
| 文件与图片 | 有界暂存与模态能力声明；不靠工具调用伪造文件上传或扩大项目权限 |

### Claude Code

优先评估官方 Agent SDK 提供的会话访问、流式输入和事件接口。Rust driver 与官方 SDK sidecar 均作为选型候选；采用 sidecar 前明确额外运行依赖、版本锁定和 IPC 契约，不将 CLI 的未文档化行为当成永久接口。

先交付只读目录、状态来源与历史，再支持服务自己管理的会话创建和空闲续轮。对外部活跃终端的接管、运行中追加、权限请求和断线恢复分别验证。无法证明实时状态或真正 steer 的场景，应显示能力缺失或提供明确的“下一轮输入”，不调用模型生成一个状态摘要来代替读取。

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
2. 交付 Lark 私人目录、带目标的输入表单、富文本和附件；Codex 作为现有 runtime 完成从导航到原生回执的闭环。
3. 按实际需要增加明确配置的群 thread，再独立完成另一部署域；逐项开放已验证能力。
4. 在 runtime server 的生命周期和能力契约稳定后，分别交付 Claude Code、Pi 的只读观察及受控输入，不让聊天入口重复实现 agent 协议。
5. 持续将已经相同的导航、确认、错误和恢复规则放入公共模块；平台身份、ACK 和原生容器管理保留各自实现。

每一阶段都以授权隔离、重复事件、过期操作、断线后任务存活、未知结果不重发及真实客户端展示为交付门槛。完成后把“已支持”行为更新到当前架构与接入方案，后续支持文档只保留尚未完成的内容。
