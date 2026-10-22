# 飞书与 Lark 的共享实现边界

核对日期：2026-10-22。依据为飞书/Lark 官方开发文档、`larksuite` 官方 SDK 与本仓库固定的 OpenLark 0.21.0 实现。下文区分协议依据、当前代码和真实租户验收。

**飞书与 Lark 已共用一套 integration 实现。** 配置分别为 `[feishu]` 和 `[lark]`，可以同时启用。内部 `Region::{Feishu, Lark}` 选择固定官方 API 根地址和平台标识；凭据、令牌客户端、绑定和操作状态分别隔离。两端均已有本地 HTTP、SDK WebSocket 和运行时测试，真实租户的安装、权限、客户端显示及交付仍待验收。

## 已核对的共同点与差异

| 边界 | 飞书 | Lark | 对实现的影响 |
| --- | --- | --- | --- |
| REST 根地址 | `https://open.feishu.cn` | `https://open.larksuite.com` | 共用资源路径与 typed DTO，由固定平台枚举选择根地址 |
| 发送消息 | `POST /open-apis/im/v1/messages` | 相同路径与方法 | 可复用发送、回复、分页与错误映射，具体接口逐项核验 |
| 凭据与身份 | 该应用的 app ID/secret、tenant、open ID | 目标 Lark 应用自己的对应值 | 平台选择不会替换凭据；禁止共享令牌缓存与授权身份 |
| 长连接 | 企业自建应用，SDK WebSocket | 企业自建应用，同类 SDK WebSocket | 复用事件派发与重连逻辑，同时按平台配置发现入口 |
| 卡片 | JSON 2.0 与新版交互回调 | 官方也有 JSON 2.0 与新版交互回调文档 | 可以共享协议模型；具体组件、回调传输及 SDK 版本需实测 |
| 开通与发布 | 飞书后台、租户管理员审核与可用范围 | Lark 后台、目标租户审核与可用范围 | 同一份程序分别部署配置，不把一端发布等同于另一端生效 |

REST 地址与同路径消息接口有直接文档证据：[飞书发送消息](https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1/message/create.md)、[Lark 发送消息](https://open.larksuite.com/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1/message/create.md)。官方 Node SDK 直接提供 `Domain.Feishu`/`Domain.Lark`；Go SDK 也定义两端 base URL。这说明共享传输和模型是官方 SDK 已采用的方案，但不能据此推断所有产品 API 的能力完全一致。[Node SDK](https://github.com/larksuite/node-sdk/blob/main/README.md)、[Go SDK client](https://github.com/larksuite/oapi-sdk-go/blob/v3_main/client.go)

## 必须隔离的身份与状态

Lark 官方说明不同租户下应用的数据隔离；`user_id` 属于租户，`open_id` 属于应用，`union_id` 属于应用开发商范围。消息接口也明确同一个人在不同应用中的 Open ID 不同。因此不能使用显示名、邮箱或相似的 `ou_`/`oc_` 前缀证明两端是同一身份，也不能复制另一应用的 owner ID 作为授权身份。项目按平台和应用共同隔离状态，不依赖两端 ID 字符串必然不同的假设。[Lark 用户身份](https://open.larksuite.com/document/home/user-identity-introduction/introduction.md)、[Lark 消息接收者类型](https://open.larksuite.com/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1/message/create.md)

令牌代表应用或用户对租户资源的访问权限。两端应分别创建应用、取得凭据并配置租户、owner 与允许群组。令牌缓存、消息去重、观察 cursor、绑定、操作账本、一次性动作 nonce 均需要包含完整应用作用域；跨平台同名资源不能合并。[飞书访问凭证](https://open.feishu.cn/document/server-docs/api-call-guide/calling-process/get-access-token.md)、[Lark tenant token](https://open.larksuite.com/document/server-docs/getting-started/api-access-token/auth-v3/tenant_access_token_internal.md)

当前状态作用域为 `platform:app_id:tenant_key:owner_open_id`。保留既有 `feishu:...` 命名空间和序列化变体，Lark 使用独立 `lark:...` 前缀以及 `Scope::Lark`、`Target::Lark`。共享现有 `feishu_*` SQL 表，每次读取和状态转换校验完整 namespace，无需修改 schema 或重写飞书记录。平台内生成的 client ID 使用 `pp:feishu:...` 或 `pp:lark:...` 前缀，观察与操作账本也保持各自作用域。

## 长连接与卡片的实际边界

两端长连接文档均说明：只支持企业自建应用，消息处理须在三秒内完成；多个同应用客户端按集群方式分配事件，而非广播。官方 Lark 示例为 WebSocket client 显式设置 `WithDomain(lark.LarkBaseUrl)`。因此不能只改 REST client，继续让 Lark 的 WebSocket 发现走飞书地址，也不能同时启动多个消费者并假定每个都会收到所有事件。[飞书长连接](https://open.feishu.cn/document/server-docs/event-subscription-guide/event-subscription-configure-/request-url-configuration-case.md)、[Lark 长连接](https://open.larksuite.com/document/ukTMukTMukTM/uYDNxYjL2QTM24iN0EjN/event-subscription-configure-/use-websocket.md)、[Go WebSocket client](https://github.com/larksuite/oapi-sdk-go/blob/v3_main/ws/client.go)

应由目标平台的官方发现接口返回 WebSocket 地址；不要对飞书 WS host 做字符串替换，不引入自定义 relay。当前快速验证、持久化入队后再处理业务的架构可以共用。回调重复推送仍须通过目标应用作用域内的事件 ID 和持久化状态去重。

两端均文档化卡片 JSON `schema: "2.0"`，并注明客户端版本要求。Lark 文档额外注明暂不支持新版飞书卡片搭建工具；这属于编辑工具的区别，不能推导为 Lark 不支持 JSON 2.0。项目自己生成 JSON 的渲染逻辑因此有明确的共享基础。[飞书卡片结构](https://open.feishu.cn/document/feishu-cards/card-json-v2-structure.md)、[Lark 卡片结构](https://open.larksuite.com/document/uAjLw4CM/ukzMukzMukzM/feishu-cards/card-json-v2-structure.md)

新版交互回调同样使用 `card.action.trigger`、schema 2.0 和应用/租户/操作者字段。但是核对时，Lark 回调页仍写着 SDK 暂不支持新版回调，而飞书页写着已支持；两页还将推送方式列为 Webhook。因此，需要同时检查固定 SDK 的实现和目标租户行为。本仓库的本地 WebSocket 测试已通过真实 SDK 验证 `card.action.trigger` 派发、持久化入队及带回调数据的 protobuf ACK；这证明了当前 SDK 路径可执行，不能代替真实租户实际推送该事件的验收。Lark 页还提示卡片场景可能下发 `open.larksuite.com` 以外域名，不能把“选择 REST 根地址”误写成所有网络主机都只有该域名。[飞书新版回调](https://open.feishu.cn/document/feishu-cards/card-callback-communication.md)、[Lark 新版回调](https://open.larksuite.com/document/uAjLw4CM/ukzMukzMukzM/feishu-cards/card-callback-communication.md)

## 权限、应用发布与直连策略

两端分别在自己的开发者后台创建和配置应用。企业自建应用的发布范围为本企业；商店应用属于另一个上架和审核流程。权限、机器人可用范围、事件订阅以及应用版本是否已发布，都要在目标租户确认。开通平台读取权限不扩大 PocketPilot 本地执行授权。[飞书自建应用流程](https://open.feishu.cn/document/home/introduction-to-custom-app-development/self-built-application-development-process.md)、[Lark 自建应用流程](https://open.larksuite.com/document/home/introduction-to-custom-app-development/self-built-application-development-process.md)、[Lark API 权限](https://open.larksuite.com/document/server-docs/getting-started/overview-of-app-scopes.md)

**本项目要求飞书与 Lark 必须且只能直连，均不允许代理配置。** 这是项目部署策略。配置拒绝 `proxy` 字段，包括空字符串；HTTP、WebSocket 与附件路径均不继承环境代理，不添加任意 base URL、WS host override 或 relay 作为“区域支持”。需要联网权限的部署应直接允许官方端点与协议要求的必要资源域名，而不是把拒绝代理误解为拒绝官方发现地址。

## 当前共享结构

全部业务代码位于 `crates/integrations/src/feishu/`。`config::Region` 是平台选择边界：app 装配时从配置表确定平台，随后将其传入 API、运行时和 Store。配置不开放 `region`、`base_url`、`proxy` 或 WebSocket 地址覆盖字段。两个 SDK 客户端分别创建，REST 和 WS 发现使用同一个客户端配置；WS 使用官方发现结果及上游 `LarkWsClient::open`。

事件 DTO、卡片渲染、导航、附件、绑定、观察、输入编排、授权检查和数据库操作共用。`MessageFormat::FeishuCard` 是共享卡片格式的内部名称，两端能力声明一致。账号或允许群组变化均计入重启指纹，避免旧实例继续沿用更新前的授权。只读诊断分别使用 `doctor --feishu` 和 `doctor --lark`；未显式指定时，普通离线诊断不请求这两个平台。

本次接入没有增加 vendor patch。现有 OpenLark 补丁限于有界分片重组、连接日志脱敏和入站活动刷新存活时间；事件派发与卡片 ACK 沿用上游实现。原因和退出条件见[依赖维护](dependencies.md)。协议模型与状态路由均由项目代码处理。

## 本地测试与验收边界

`make check` 使用合成身份、SQLite 临时数据库、本地 HTTP 服务和本地 WebSocket 对端验证以下行为：

- 两个配置段可同时装配；固定 API 根地址和能力声明正确；拒绝代理、地址覆盖及未知配置字段。
- 相同外部应用、租户、用户、群和事件 ID 不造成跨平台去重、绑定、nonce、观察或操作账本串用；既有飞书 namespace 和序列化保持兼容。
- 共享运行时在两端都执行 owner/tenant 校验、允许群组及 thread 限制、卡片持久化入队、任务提交与结构化问答。
- REST 客户端的令牌缓存互相独立；消息、卡片和附件继续走同一套已测试 API 实现。
- 真实 SDK 通过 HTTP 发现本地 WebSocket，接收 protobuf 事件并 ACK；重复消息只持久化一次，卡片回调 ACK 在持久化后携带响应数据。
- 独立测试子进程设置不可达的 HTTP/HTTPS/ALL 代理，清空 NO_PROXY，发现、WS 连接和事件处理仍能直连完成。子进程环境不会影响并行测试。

本地协议测试不访问真实租户。部署验收仍需分别使用两端自己的企业自建应用，确认权限开通与应用发布、私聊导航、群话题任务、文件/图片交付、客户端卡片与按钮、事件重投去重、断线重连及升级后既有飞书状态。不能据本地 fixture 宣称所有卡片组件、所有产品 API 或目标租户权限均已验证；真实验收结果应按版本记录在[发布检查](releasing.md)中。
