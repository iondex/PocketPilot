# 飞书与 Lark 的共享实现边界

核对日期：2026-10-22。依据为飞书/Lark 官方开发文档及 `larksuite` 官方 SDK；下文将官方事实与项目建议分开。

**结论：适合共用 integration 的协议与业务实现，并以枚举选择区域；仅替换域名不足以完成接入。** 凭据、租户、用户、绑定和操作状态必须独立，目标区域的事件、卡片和权限仍需验证。当前 PocketPilot 仅实现国内飞书；国际 Lark 尚未实现，也未完成真实账号验收。本文是后续结构设计依据，不是已支持 Lark 的说明。

## 已核对的共同点与差异

| 边界 | 国内飞书 | 国际 Lark | 对实现的影响 |
| --- | --- | --- | --- |
| REST 根地址 | `https://open.feishu.cn` | `https://open.larksuite.com` | 共用资源路径与 typed DTO，由固定区域枚举选择根地址 |
| 发送消息 | `POST /open-apis/im/v1/messages` | 相同路径与方法 | 可复用发送、回复、分页与错误映射，具体接口逐项核验 |
| 凭据与身份 | 该应用的 app ID/secret、tenant、open ID | 目标 Lark 应用自己的对应值 | 区域不是凭据替换器；禁止共享令牌缓存与授权身份 |
| 长连接 | 企业自建应用，SDK WebSocket | 企业自建应用，同类 SDK WebSocket | 复用事件派发与重连逻辑，同时按区域配置发现入口 |
| 卡片 | JSON 2.0 与新版交互回调 | 官方也有 JSON 2.0 与新版交互回调文档 | 可以共享协议模型；具体组件、回调传输及 SDK 版本需实测 |
| 开通与发布 | 飞书后台、租户管理员审核与可用范围 | Lark 后台、目标租户审核与可用范围 | 同一份程序分别部署配置，不把一端发布等同于另一端生效 |

REST 地址与同路径消息接口有直接文档证据：[飞书发送消息](https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1/message/create.md)、[Lark 发送消息](https://open.larksuite.com/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1/message/create.md)。官方 Node SDK 直接提供 `Domain.Feishu`/`Domain.Lark`；Go SDK 也定义两端 base URL。这说明共享传输和模型是官方 SDK 已采用的方案，但不能据此推断所有产品 API 的能力完全一致。[Node SDK](https://github.com/larksuite/node-sdk/blob/main/README.md)、[Go SDK client](https://github.com/larksuite/oapi-sdk-go/blob/v3_main/client.go)

## 必须隔离的身份与状态

Lark 官方说明不同租户下应用的数据隔离；`user_id` 属于租户，`open_id` 属于应用，`union_id` 属于应用开发商范围。消息接口也明确同一个人在不同应用中的 Open ID 不同。因此不能使用显示名、邮箱或相似的 `ou_`/`oc_` 前缀证明两端是同一身份，也不能复制国内 owner ID 当作国际授权。这里的“不要跨区域复用”是基于身份作用域作出的工程建议；不是宣称官方承诺两区域 ID 在字符串层面绝无重叠。[Lark 用户身份](https://open.larksuite.com/document/home/user-identity-introduction/introduction.md)、[Lark 消息接收者类型](https://open.larksuite.com/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1/message/create.md)

令牌代表应用或用户对租户资源的访问权限。两端应分别创建应用、取得凭据并配置租户、owner 与允许群组。令牌缓存、消息去重、观察 cursor、绑定、操作账本、一次性动作 nonce 均需要包含完整应用作用域；跨区域同名资源不能合并。[飞书访问凭证](https://open.feishu.cn/document/server-docs/api-call-guide/calling-process/get-access-token.md)、[Lark tenant token](https://open.larksuite.com/document/server-docs/getting-started/api-access-token/auth-v3/tenant_access_token_internal.md)

建议作用域为 `region + app_id + tenant_key + owner_open_id`。保留已有 `feishu:...` 命名空间，未来 Lark 使用独立 `lark:...` 前缀；不要因重构直接重写已有飞书记录。平台标识、菜单入口和投递目标也应保留区域，防止配置正确而状态路由串端。

## 长连接与卡片的实际边界

两端长连接文档均说明：只支持企业自建应用，消息处理须在三秒内完成；多个同应用客户端按集群方式分配事件，而非广播。官方 Lark 示例为 WebSocket client 显式设置 `WithDomain(lark.LarkBaseUrl)`。因此不能只改 REST client，继续让 WebSocket 发现走国内默认地址，也不能同时启动多个消费者并假定每个都会收到所有事件。[飞书长连接](https://open.feishu.cn/document/server-docs/event-subscription-guide/event-subscription-configure-/request-url-configuration-case.md)、[Lark 长连接](https://open.larksuite.com/document/ukTMukTMukTM/uYDNxYjL2QTM24iN0EjN/event-subscription-configure-/use-websocket.md)、[Go WebSocket client](https://github.com/larksuite/oapi-sdk-go/blob/v3_main/ws/client.go)

应由目标区域的官方发现接口返回 WebSocket 地址；不要把国内 WS host 做字符串替换，不引入自定义 relay。当前快速验证、持久化入队后再处理业务的架构可以共用。回调重复推送仍须通过目标应用作用域内的事件 ID 和持久化状态去重。

两端均文档化卡片 JSON `schema: "2.0"`，并注明客户端版本要求。Lark 文档额外注明暂不支持新版飞书卡片搭建工具；这属于编辑工具的区别，不能推导为 Lark 不支持 JSON 2.0。项目自己生成 JSON 的渲染逻辑因此有明确的共享基础。[飞书卡片结构](https://open.feishu.cn/document/feishu-cards/card-json-v2-structure.md)、[Lark 卡片结构](https://open.larksuite.com/document/uAjLw4CM/ukzMukzMukzM/feishu-cards/card-json-v2-structure.md)

新版交互回调同样使用 `card.action.trigger`、schema 2.0 和应用/租户/操作者字段。但是核对时，Lark 回调页仍写着 SDK 暂不支持新版回调，而飞书页写着已支持；两页还将推送方式列为 Webhook。因此，这些页面不足以证明本仓库所用 Rust OpenLark 的国际长连接卡片 ACK 已经可用，不能省略针对实际 SDK 版本的协议检查和真机验证。Lark 页还提示卡片场景可能下发 `open.larksuite.com` 以外域名，不能把“选择 REST 区域”误写成所有网络主机都只有该域名。[飞书新版回调](https://open.feishu.cn/document/feishu-cards/card-callback-communication.md)、[Lark 新版回调](https://open.larksuite.com/document/uAjLw4CM/ukzMukzMukzM/feishu-cards/card-callback-communication.md)

## 权限、应用发布与直连策略

两端分别在自己的开发者后台创建和配置应用。企业自建应用的发布范围为本企业；商店应用属于另一个上架和审核流程。权限、机器人可用范围、事件订阅以及应用版本是否已发布，都要在目标租户确认。开通平台读取权限不扩大 PocketPilot 本地执行授权。[飞书自建应用流程](https://open.feishu.cn/document/home/introduction-to-custom-app-development/self-built-application-development-process.md)、[Lark 自建应用流程](https://open.larksuite.com/document/home/introduction-to-custom-app-development/self-built-application-development-process.md)、[Lark API 权限](https://open.larksuite.com/document/server-docs/getting-started/overview-of-app-scopes.md)

**本项目要求飞书与 Lark 必须且只能直连，均不允许代理配置。** 这是用户明确要求的部署策略，不是官方平台限制。未来配置仍拒绝 `proxy` 字段，包括空字符串；HTTP、WebSocket 与附件路径均不继承环境代理，不添加任意 base URL、WS host override 或 relay 作为“区域支持”。需要联网权限的部署应直接允许官方端点与协议要求的必要资源域名，而不是把拒绝代理误解为拒绝官方发现地址。

## 建议的共享结构与验收

建议保留一套 integration 实现，在内部使用封闭的 `Region::{Feishu, Lark}` 提供显示名、状态前缀和固定官方 base URL。事件 DTO、卡片渲染、导航、绑定、观察、输入编排、授权检查和数据库操作共用；客户端与凭据实例按区域分别创建。实现可继续位于现有 `feishu` 目录，待入口与命名确实妨碍理解时再统一改名，不复制一份 `lark` 业务目录。

接入 Lark 前需要验证区域分别影响 REST 与 WS 发现、相同外部 ID 在不同区域不会碰撞、各类代理字段被拒绝，以及消息接收、发送、附件、卡片回调和 ACK 的实际协议。最后以独立 Lark 应用与租户完成真实接收、菜单交互、持久化去重、断线重连和既有飞书状态回归，再更新支持列表。在这些步骤完成前，应继续将国际 Lark 标为未实现/未验收。
