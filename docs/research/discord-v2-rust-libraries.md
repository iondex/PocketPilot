# Discord Components V2：Rust 库选择

核验日期：2026-10-20（Asia/Shanghai）。核验包括 crates.io 元数据、实际发布包源码和官方仓库源码；未执行下载包的脚本、构建代码或向真实 Discord 发送试验消息。本文中的可行性是源码与协议层结论，客户端效果仍需集成验收。

## 推荐

PocketPilot 适合保留 **Serenity 0.12.5 的 HTTP、Gateway、授权上下文与代理**，增加 **twilight-model 0.17.1 + twilight-util 0.17.0 的类型和 builders** 来生成 Components V2 请求。使用 Serenity 公开的 `Http::request` / `Http::fire` 和 `Route` 发送序列化后的具名请求类型，不手工操作 `serde_json::Value`。[S1][S2][T1][T2]

这是稳定发行包的组合，不需要依赖未发布的 Serenity `next`，也不需要重写已经验证的 HTTP CONNECT 代理和 Gateway 生命周期。Twilight builders 的输出是其自己的 model，不能直接传给 Serenity `CreateMessage.components`；边界必须是明确的 V2 请求 DTO，而非假设两个库的 builder 类型兼容。[S1][T2]

```toml
serenity = { version = "=0.12.5", default-features = false, features = [
  "builder", "client", "gateway", "http", "model", "rustls_backend",
] }
twilight-model = { version = "=0.17.1", default-features = false }
twilight-util = { version = "=0.17.0", default-features = false, features = ["builder"] }
```

Twilight 0.17 的 MSRV 是 Rust 1.89，因此这条路径至少要求 Rust 1.89；检查当次锁文件的传递依赖后才能宣称完整构建的最低工具链。Serenity 保留 ISC 许可，Twilight 也为 ISC。[S3][T3]

## 实际能力比较

| 库和版本 | 已核实的 V2 类型 / builders | HTTP、交互和 Gateway | 主要代价 |
| --- | --- | --- | --- |
| Serenity 0.12.5，2025-12-20 发布 | 无 `CreateContainer` / `CreateTextDisplay` 等 V2 builders | 完整既有 HTTP/Gateway、defer、ephemeral；公开低级请求入口可发送额外类型 | 单独使用不能类型化构造 V2；与 Twilight 组合可保留现有连接代码 |
| Serenity 官方 `next`，固定 `c84604143ccdf9ffa62d609ed191ab5900c8576d` | `CreateContainer`、`CreateTextDisplay`、`CreateSection`、`CreateMediaGallery`、`CreateFile`、separator、thumbnail 等完整 builders | typed send/edit、interaction initial/edit/followup、ephemeral/defer、Gateway | 未发布；MSRV 1.95；builder 生命周期、token、channel ID、Gateway 和 reqwest 版本均发生变化 |
| Twilight model/http/gateway 0.17.1 + util 0.17.0 | Container/TextDisplay/Section/FileDisplay/Separator/Thumbnail builders；MediaGallery/MediaGalleryItem 为 typed structs，无发布版 Gallery builder | 完整 typed send/edit、interaction response/edit/followup、Gateway；flags 含 `IS_COMPONENTS_V2` | 全 SDK 迁移成本较高；HTTP 的 `.proxy` 是反向代理，不能直接替代现有 CONNECT 代理 |
| discordrs 3.0.0，2026-10-18 发布 | Container/TextDisplay/Section/**MediaGallery**/File 等完整 builders，typed request models | REST 和 Gateway；typed ComponentsV2Message；REST 可注入 reqwest Client | 新大版本仅发布两天；需全 SDK 迁移和重新验证；目前并无使用它替换既有连接层的必要 |
| poise 0.7.1，2026-10-12 发布 | 依赖 Serenity 0.12.5，本身不补齐 V2 model/builders | 更高层命令框架 | 不能单靠升级 poise 获得 V2 |
| serenity-builder 0.3.1 | 发布源码围绕 Serenity 传统 message/embed 转换，无 V2 Container/TextDisplay | 依赖 Serenity | 名称中的 builder 不代表支持 Components V2 |

比较依据为发布包中的 Cargo manifest 和具体类型实现，不依据 README 声称的“支持 Discord API”。[S1][S3][S4][S5][T1][T2][T3][R1][P1][B1]

`serenity` crates.io 最新版本仍是 0.12.5，没有正式发布 0.13。官方 `next` manifest 的 package version 甚至仍写 0.12.5，不能用此字段认定为稳定 API。`master` 分支是旧 0.5.8 代码，也不是应采用的最新版分支。`serenity_self` 是独立 fork，不是官方 Serenity 0.13；`discord-sdk` 则是 Discord Game SDK 的包装，不是 bot REST/Gateway Components V2 库。[S3][S4][O1][O2]

## 稳定组合的构造接口

下例按照 twilight-util 0.17.0 的真实签名构造。builder 通常实现 `Into<具体 model>`，但不连锁推导为 `Into<Component>`，所以传入容器前显式 `.build()`。[T1][T2]

```rust
use serde::Serialize;
use twilight_model::channel::message::{
    AllowedMentions, Component, MessageFlags,
    component::{ButtonStyle, SelectMenuType},
};
use twilight_util::builder::message::{
    ActionRowBuilder, ButtonBuilder, ContainerBuilder, SelectMenuBuilder,
    SelectMenuOptionBuilder, TextDisplayBuilder,
};

#[derive(Serialize)]
struct MessageBody {
    flags: MessageFlags,
    components: Vec<Component>,
    allowed_mentions: AllowedMentions,
}

fn body() -> MessageBody {
    let button = ButtonBuilder::new(ButtonStyle::Secondary)
        .custom_id("pp:opaque-handle")
        .label("返回项目")
        .build();
    let options = SelectMenuOptionBuilder::new("示例项目", "0")
        .description("项目目录")
        .build();
    let select = SelectMenuBuilder::new("pp:another-handle", SelectMenuType::Text)
        .placeholder("选择项目")
        .min_values(1)
        .max_values(1)
        .option(options)
        .build();
    let container = ContainerBuilder::new()
        .accent_color(Some(0x5865F2))
        .component(TextDisplayBuilder::new("## 📁 选择项目\n项目说明").build())
        .component(ActionRowBuilder::new().component(select).build())
        .component(ActionRowBuilder::new().component(button).build())
        .build();
    MessageBody {
        flags: MessageFlags::IS_COMPONENTS_V2,
        components: vec![container.into()],
        allowed_mentions: AllowedMentions::default(),
    }
}
```

`AllowedMentions::default()` 的 Twilight 文档明确禁止全部 ping。普通 message/create 的 flags 不应带 `EPHEMERAL`；只在 interaction 初始回复或 followup 创建时加入。先 defer 时初始 ACK 已确定 ephemeral 状态，后续编辑保留该状态，不能通过编辑任意更改可见性。[T1][D1]

媒体图库不需要拼 JSON，可直接构造 `component::MediaGallery { id: None, items: Vec<MediaGalleryItem> }`，再包装成 `Component::MediaGallery`；每个 item 包含 `UnfurledMediaItem`、可选说明和 spoiler。`SectionBuilder::new(accessory).component(TextDisplayBuilder::new(text).build()).build()` 支持按钮或 thumbnail accessory。SDK 的 builder 不代表所有嵌套组合均合法，仍需遵守官方组件位置和数量约束。[T1][T2][D2]

### Serenity 的发送边界

```rust
use serenity::http::{Http, LightMethod, Request, Route};
use serenity::model::id::{ChannelId, MessageId};

#[derive(serde::Deserialize)]
struct Receipt {
    id: MessageId,
}

async fn send(http: &Http, channel_id: ChannelId, body: &MessageBody)
    -> serenity::Result<MessageId>
{
    let bytes = serde_json::to_vec(body)?;
    let request = Request::new(Route::ChannelMessages { channel_id }, LightMethod::Post)
        .body(Some(bytes));
    Ok(http.fire::<Receipt>(request).await?.id)
}
```

协议序列化使用 `serde_json` 不等于业务代码操作无类型 JSON。可直接用 `Http::request` 验证 HTTP 成功状态并丢弃编辑响应，或用只包含 `id` 的回执 DTO 读取新消息 ID。保留原调用方的操作账本、超时和未知结果处理；上述入口本身不保证应用级 exactly-once。[S1]

| 操作 | Serenity 0.12.5 `Route` | HTTP 方法 / 路径 |
| --- | --- | --- |
| 频道新消息 | `ChannelMessages { channel_id }` | POST `/channels/{channel_id}/messages` |
| 编辑频道消息 | `ChannelMessage { channel_id, message_id }` | PATCH `/channels/{channel_id}/messages/{message_id}` |
| interaction 初始响应 | `InteractionResponse { interaction_id, token }` | POST `/interactions/{interaction_id}/{token}/callback` |
| 编辑最初响应 | `WebhookOriginalInteractionResponse { application_id, token }` | PATCH `/webhooks/{application_id}/{token}/messages/@original` |
| followup 新消息 | `WebhookFollowupMessages { application_id, token }` | POST `/webhooks/{application_id}/{token}` |
| 编辑 followup | `WebhookFollowupMessage { application_id, token, message_id }` | PATCH `/webhooks/{application_id}/{token}/messages/{message_id}` |

`Route` 自动带 Discord API 前缀，不能自行重复加入。初始响应的 body 还需 `type` 和 `data` envelope；Twilight 已提供 `http::interaction::{InteractionResponse, InteractionResponseType}` 以及 `InteractionResponseDataBuilder`。[S1][T1][T2]

Serenity 0.12.5 的 `Message.components` 并非支持 V2 类型：其 `deserialize_components` visitor 只保留顶层 `type=1` action rows，**主动跳过其他顶层组件**。因此 V2 container 不会使整个 Message 或 ComponentInteraction 的 message 字段解码失败，但其子组件结构不可从旧 Message 模型读回。直接依靠交互事件的 `custom_id` / values 和持久化的 opaque action handle 处理菜单，授权仍可读取 message author、flags、channel/guild/user。[S2]

旧 `ActionRowComponent` 对不支持的子组件会报错，所以不能把 V2 组件塞进传统顶层 action row，并期待旧反序列化器容错。把 V2 原生组件放入其正确的 container / section 位置，并使用真实 V2 布局样本验证 Gateway 事件边界。[S2][D2]

## 其他路线的迁移边界

### Serenity next

已检查的 revision 为 `c84604143ccdf9ffa62d609ed191ab5900c8576d`。其 API 包括 `CreateComponent::Container(CreateContainer::new(...))`、`CreateContainerComponent`、`CreateSectionComponent` 和 `CreateSectionAccessory`。`CreateMessage`、`EditMessage`、`EditInteractionResponse`、`CreateInteractionResponseMessage`、`CreateInteractionResponseFollowup` 均支持 V2 `components` 和 flags。[S4]

但它不只是增加几个 builder：多数 builder 带 `<'a>` 并用 `Cow`；`Vec` 参数通常需要 `.into()`，跨异步/持久边界使用 `.into_owned()` 或 `<'static>`；HttpBuilder 接收经过解析的 `Token`；测试 fake token 也必须符合三段格式；reqwest 要求 `>=0.13`，现有 0.12 Client 不能直接注入；`client` feature 已移除，ClientBuilder 移入 gateway 模块；channel ID 也被进一步细分。Gateway `ws_url` 从共享 mutex 变为 `Arc<str>`，现有 relay 注入代码须随之调整。因此固定 git rev 虽可复现，仍要承担未发布 API 和较广迁移范围。[S4]

### Twilight 全量迁移

Twilight 的模型、builders、HTTP 和 Gateway 均是分别发布的库。HTTP `ClientBuilder::build` 内部固定创建 connector；0.17.1 没有公开注入自定义 hyper client / connector 的 builder 方法。`.proxy(host, use_http)` 明确用于 Twilight HTTP 反向代理，不是通用 HTTP CONNECT 代理，不能直接传当前部署的传统代理地址。[T3][T4]

Gateway 可用 `ConfigBuilder::new(token, intents).proxy_url(relay_endpoint).build()` 选择初始 websocket 地址；但 shard 的连接路径优先使用收到 READY 后保存的 `resume_gateway_url`，其次才是配置的 proxy URL。仅修改 `proxy_url` 不能证明恢复连接仍经 relay；`resume_url` builder 只影响带既有 session 的首次恢复。这条路线需要额外解决代理与恢复连接行为。[T5]

typed deferred response 为 `InteractionResponse { kind: DeferredChannelMessageWithSource, data: Some(InteractionResponseDataBuilder::new().flags(MessageFlags::EPHEMERAL).build()) }`；组件 ACK 可用 `DeferredUpdateMessage` 且 data 为 None；之后通过 `client.interaction(application_id).update_response(token).flags(IS_COMPONENTS_V2).components(Some(&components))` 编辑。普通发送、编辑与 followup 也接受 typed components/flags；部分 API 注释仍只列旧 flags，需结合真实字段序列化和当前 Discord 协议判断。[T1][T2][T4][D1]

### discordrs

3.0.0 发布源码确有完整 V2 builders，并非只在 README 宣称支持。`ComponentsV2Message`、`ContainerBuilder`、`TextDisplayBuilder`、`SectionBuilder`、`MediaGalleryBuilder`、`FileBuilder`、typed requests 和 Gateway 均可查到实现。REST 基于 reqwest 0.12，`RestClientBuilder::proxy` 和 `use_client` 提供传统代理或自建 Client 接入方式。[R1]

这是后续全量 SDK 替换的可评估备选，但“刚发布 3.0.0”不代表已在本项目充分验证。当前需求无需为图库 builder 单独切换整套 SDK；Twilight typed MediaGallery 已能表达相同协议结构。[R1][T1]

## 主源

- [S1] [Serenity 0.12.5 发布包源码：HTTP Request / Client / Route](https://docs.rs/crate/serenity/0.12.5/source/src/http/)。发布包 git revision 为 `1809beb0fc24f3942c500058ad4fa47e6a97d3f9`。
- [S2] [Serenity 0.12.5 message components 容错反序列化](https://github.com/serenity-rs/serenity/blob/1809beb0fc24f3942c500058ad4fa47e6a97d3f9/src/model/utils.rs#L483)；[旧子组件模型](https://github.com/serenity-rs/serenity/blob/1809beb0fc24f3942c500058ad4fa47e6a97d3f9/src/model/application/component.rs)。
- [S3] [crates.io Serenity 版本元数据](https://crates.io/api/v1/crates/serenity)；[Serenity 0.12.5 Cargo manifest](https://docs.rs/crate/serenity/0.12.5/source/Cargo.toml)。
- [S4] [Serenity next 固定 revision](https://github.com/serenity-rs/serenity/tree/c84604143ccdf9ffa62d609ed191ab5900c8576d)；[V2 builders](https://github.com/serenity-rs/serenity/blob/c84604143ccdf9ffa62d609ed191ab5900c8576d/src/builder/create_components.rs)；[Cargo manifest](https://github.com/serenity-rs/serenity/blob/c84604143ccdf9ffa62d609ed191ab5900c8576d/Cargo.toml)。
- [S5] [官方 Serenity 分支列表](https://api.github.com/repos/serenity-rs/serenity/branches?per_page=100)。
- [T1] [twilight-model 0.17.1 发布源码：message component](https://docs.rs/crate/twilight-model/0.17.1/source/src/channel/message/component/)；[interaction response types](https://docs.rs/crate/twilight-model/0.17.1/source/src/http/interaction.rs)。
- [T2] [twilight-util 0.17.0 发布源码：message builders](https://docs.rs/crate/twilight-util/0.17.0/source/src/builder/message/)；[InteractionResponseDataBuilder](https://docs.rs/crate/twilight-util/0.17.0/source/src/builder/interaction_response_data.rs)。发布包 revision 为 `007889d01731d75a0dadcee106795de21235acb4`。
- [T3] [twilight-util 0.17.0 manifest](https://docs.rs/crate/twilight-util/0.17.0/source/Cargo.toml)；[crates.io model](https://crates.io/api/v1/crates/twilight-model)、[util](https://crates.io/api/v1/crates/twilight-util)、[http](https://crates.io/api/v1/crates/twilight-http)、[gateway](https://crates.io/api/v1/crates/twilight-gateway)。
- [T4] [twilight-http 0.17.1 ClientBuilder / connector](https://docs.rs/crate/twilight-http/0.17.1/source/src/client/)；[HTTP request types](https://docs.rs/crate/twilight-http/0.17.1/source/src/request/)。
- [T5] [twilight-gateway 0.17.1 config](https://docs.rs/crate/twilight-gateway/0.17.1/source/src/config.rs)；[shard 连接与恢复](https://docs.rs/crate/twilight-gateway/0.17.1/source/src/shard.rs)。
- [R1] [discordrs 3.0.0 发布源码](https://docs.rs/crate/discordrs/3.0.0/source/src/)；[manifest](https://docs.rs/crate/discordrs/3.0.0/source/Cargo.toml)；[crates.io 元数据](https://crates.io/api/v1/crates/discordrs)。发布包 revision 为 `84e6721034ec7edbb687b35a2fa2e0f97abed750`。
- [P1] [poise 0.7.1 manifest，Serenity 依赖和 MSRV](https://docs.rs/crate/poise/0.7.1/source/Cargo.toml)。
- [B1] [serenity-builder 0.3.1 发布源码](https://docs.rs/crate/serenity-builder/0.3.1/source/src/)。
- [O1] [serenity_self 元数据](https://crates.io/api/v1/crates/serenity_self)。
- [O2] [discord-sdk 的 Game SDK 定位](https://github.com/EmbarkStudios/discord-sdk)。
- [D1] [Discord interaction responses](https://docs.discord.com/developers/interactions/receiving-and-responding)。
- [D2] [Discord Components reference](https://docs.discord.com/developers/components/reference)；[V2 overview](https://docs.discord.com/developers/components/overview)。
