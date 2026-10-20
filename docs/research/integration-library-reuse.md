# Integrations 导航与库复用边界

核验日期：2026-10-20（Asia/Shanghai）。范围为当前锁定依赖、本地发布包源码、库作者的 Rustdoc 与 Telegram 官方接口；未向真实聊天发送消息，未执行第三方示例或替换依赖。

建议：**三端导航统一到本项目的领域模型和查询流程；继续复用现有 SDK、Tokio 与 SQLx，不为拆分 Service/Store 引入 actor 框架，也不用通用任务队列替换写入账本。** 这里的“复用库”与“抽出公共业务规则”是两件事。

## 问题范围

将库可解决的机制与应用需要维护的领域职责分开评价：

| 范围 | 能复用的库 | 仍由 PocketPilot 定义的内容 |
| --- | --- | --- |
| Service/Store 封装 | Rust 模块可见性、普通 struct/enum；已有 Tokio 任务和 SQLx 事务 | 导航、授权、投递各自拥有的数据与接口；事务边界 |
| 平台无关计算 | 已有 session/engine 规则和 Markdown parser | 主机→项目→session、筛选排序、分页、历史预览、增量筛选与确认条件 |
| 持久收发与恢复 | SQLx 的 SQLite 连接池、事务、绑定参数 | 配置/绑定 revision 复查、一次性动作、固定分段、回执、不重发未知写入 |
| 平台协议与 SDK 适配 | teloxide、slack-morphism、Serenity、Twilight builders | SDK 尚未覆盖的协议薄层、各端身份/作用域映射与平台错误分类 |

这不是以某个库替换全部实现的推荐。库可以承担通用机制，不能直接决定应用授权或工作区语义。

## 当前依赖与已存在的复用

本次核验的 Cargo.lock：Tokio 1.53.3、SQLx 0.8.6、teloxide 0.17.0 / teloxide-core 0.13.0、slack-morphism 2.31.0、Serenity 0.12.5、twilight-model 0.17.1 / twilight-util 0.17.0。以下能力均对应这些版本，不将上游开发分支当成已采用能力。

| 机制 | 已有能力与适用边界 |
| --- | --- |
| Tokio | 有界 mpsc 提供容量和背压，适合需要独立拥有状态的 worker；JoinSet 管理任务集合。仅为组织代码而引入额外 mailbox，会增加消息协议和停机流程，没有必要把每个模块变成 actor。[1][2] |
| SQLx | Transaction 提供 commit/rollback，未结束的事务在 drop 时回滚。可以继续支持领域 repository 的原子状态转换；这不提供跨数据库事务和远端 HTTP 的原子提交。[3] |
| teloxide | Dialogue 是 Storage 与 ChatId 的封装，支持持久 SQLite 对话状态；Dispatcher 负责 Telegram update 路由。这些适合单端对话流程，公开模型没有自动提供本项目的 topic、平台身份、绑定 revision、过期按钮和一次性操作契约。[4][5] |
| Slack Morphism | 已提供 Socket Mode、Web API、Block Kit 与 rich text/table 的类型及 builders。继续让它承担平台 DTO 与协议；菜单选中之后查询哪台主机、哪个项目仍属项目业务。[6] |
| Serenity + Twilight | 现有 Serenity 承担 HTTP/Gateway；Twilight 0.17 builders 提供 Container/TextDisplay 等 V2 类型。无需为了统一菜单更换连接 SDK；两套模型通过具名请求边界组合，不能假定 builder 类型可以互传。[7][8] |

本地 `teloxide-core-0.13.0/src` 中未检出 `RichMessage`、`RichBlock`、`rich_message`、`send_rich_message`，版本化 Requester 列表也未提供对应方法；Telegram 官方已提供 `sendRichMessage`。所以现有 [rich_api.rs](../../crates/integrations/src/telegram/rich_api.rs) 的类型化补充仍有必要。未来若 SDK 覆盖完整接口，应替换这层协议代码；不能因此删除本项目的 Markdown AST→RichMessage 转换与长度/结构预算。[9][10]

## 导航应共享什么

统一的是“执行环境/主机→该环境下的项目目录→项目内 session”，不是 Slack workspace 或 Discord guild。SDK 的 dialogue、command router、pagination helper 均不拥有这一跨端语义。

适合集中实现：层级和查询输入、可见项目筛选、确定性排序、分页窗口、会话标题与状态、返回/刷新目标。让平台消费具名条目及动作，分别构造 Telegram keyboard、Slack Block Kit 或 Discord components。数据加载、授权复查和过期处理要有明确边界，不能只统一显示文案。

继续留在平台：Telegram topic/配对/数字选择；Slack DM/thread 和 Socket ACK；Discord guild/owner、交互 ACK/ephemeral。持久化回调或按钮时使用的现有格式不能在一次源码整理中无说明地变更；如改变须提供读旧格式或显式失效策略。

项目内的纯函数及小型状态枚举足够表达当前导航。只有业务确实出现大量层级状态、共享 entry/exit 动作，且普通枚举已难维护时，再评价状态机框架。

## actor / 状态机库能否解决封装

| 候选 | 已核验能力 | 对当前代码的判断 |
| --- | --- | --- |
| xtra 0.6.0 | Actor/Handler 与 Mailbox，支持 bounded mailbox。[11] | 可替代自己编写的 actor 消息循环，但不会替代数据库 schema、授权检查或远端回执。当前 Tokio 已提供所需任务机制；先用模块所有权和小接口封装 Service/Store，暂不增加依赖。 |
| statig 0.4.1 | 分层状态机、state-local storage、async handler/action，以及可选 serde。[12] | 可表达复杂向导的状态/事件，但序列化不等于持久事务、按钮有效期或一次性授权。当前三层导航使用 enum 与纯函数更直接；暂不引入宏和额外状态机模型。 |
| teloxide Dialogue | 按 ChatId 获取、更新、退出对话，可选持久 Storage。[4][5] | 能用于简单 Telegram 填表向导，但与三端公共导航不处于同一层；默认 ChatId 粒度也不能直接区分同一私聊内的多个 topic。可以自定义存储/上下文，但此时仍需实现本项目的组合身份和事务约束。 |

上述建议是针对当前代码的设计判断，不是断言这些库不能扩展或不能实现持久系统。将一个大 Service 拆成几个各自拥有状态的模块，本身不需要运行时框架；若没有新增并发需求，不应为了文件分组增加异步消息跳转。

## Apalis 能否替换持久收发

Apalis 1.0.0 与 apalis-sqlite 1.0.0 已提供 SQLite 任务持久化、调度、优先级、可配置失败重试和失联 worker 的 orphan recovery。SQLite 后端文档明确写出：失败任务可重新入队，持有任务的 worker 死亡时任务自动重新入队。[13][14] 旧 `apalis-sql` 0.7.4 也公开 `retry` / `reenqueue_orphaned`，不能将旧包路径当成 1.0 的 SQLite 后端。[15]

这类重新入队有利于最终完成可重试任务；PocketPilot 的外部写入采用另一种约束：请求超时、断线或丢失回执时，结果记为 `outcome_unknown`，不自动重发。一个本地任务唯一键只能阻止重复入队，不能证明远端没有收到上一次 HTTP/RPC 写入。

现有三端恢复代码都将已 dispatching 的投递转成 outcome_unknown，见 [Telegram storage](../../crates/integrations/src/telegram/storage.rs)、[Slack storage](../../crates/integrations/src/slack/storage.rs)、[Discord storage](../../crates/integrations/src/discord/storage.rs)。另外还会在发送前复查配置、配对/owner、绑定 revision 和有效期，并沿固定分段保存回执。通用队列不知道这些规则。

因此，**不采用 Apalis 直接替换当前 outbox 或 agent 写入账本**。即使关闭失败重试，仍需逐项设计 orphan recovery、写入前授权与结果未知状态，不能据此宣称无重复副作用。这里也没有保证“绝不丢失”：请求可能未到达远端，但因为无法判定而需要用户核对。

若以后出现大量彼此独立、可安全重复的只读任务，例如允许重复的元数据刷新，Apalis 可单独评价。当前为减少现有代码量引入它，会增加队列 schema、worker 生命周期和依赖，未见足以抵消适配成本的收益。此处仅评估队列选型，不修改独立 retry 工具或其授权行为。

## 后续收束顺序

1. 先统一导航模型、项目/会话查询、排序分页与三端行为测试，让 SDK 只负责原生控件呈现。
2. 再按真实拥有的数据和事务边界收拢导航、投递的模块接口，减少跨文件访问同一个大 Service/Store。
3. 核对三端历史预览、增量筛选与确认规则；一致的纯计算复用 session/engine 或 integrations 公共模块，平台 SQL/授权继续留在所属端。
4. 只有实际出现 SDK 覆盖缺口或新的任务规模需求，再独立升级/替换库，并验证平台协议和恢复语义。

不承诺减少固定比例行数；成功标准是同一规则只有一个实现、接口表达完整上下文、错误/恢复语义不变，以及 fixtures 能覆盖隔离、过期操作与未知结果不重发。

## 一手来源

[1] [Tokio 1.53.3 mpsc](https://docs.rs/tokio/1.53.3/tokio/sync/mpsc/index.html)

[2] [Tokio 1.53.3 JoinSet](https://docs.rs/tokio/1.53.3/tokio/task/struct.JoinSet.html)

[3] [SQLx 0.8.6 Transaction](https://docs.rs/sqlx/0.8.6/sqlx/struct.Transaction.html)

[4] [teloxide 0.17.0 dialogue](https://docs.rs/teloxide/0.17.0/teloxide/dispatching/dialogue/index.html)

[5] [teloxide 0.17.0 Dialogue](https://docs.rs/teloxide/0.17.0/teloxide/dispatching/dialogue/struct.Dialogue.html)、[Dispatcher](https://docs.rs/teloxide/0.17.0/teloxide/dispatching/struct.Dispatcher.html)

[6] [Slack Morphism 2.31.0](https://docs.rs/slack-morphism/2.31.0/slack_morphism/)，并核对该发布包的 `src/models/blocks/kit.rs` 与 `src/socket_mode/listener.rs`。

[7] [Serenity 0.12.5 Http](https://docs.rs/serenity/0.12.5/serenity/http/struct.Http.html)，并核对发布包 `src/http/client.rs` 的 `request` / `fire`。

[8] [Twilight util 0.17.0 message builders](https://docs.rs/twilight-util/0.17.0/twilight_util/builder/message/index.html)、[ContainerBuilder](https://docs.rs/twilight-util/0.17.0/twilight_util/builder/message/struct.ContainerBuilder.html)、[TextDisplayBuilder](https://docs.rs/twilight-util/0.17.0/twilight_util/builder/message/struct.TextDisplayBuilder.html)

[9] [teloxide-core 0.13.0 Requester](https://docs.rs/teloxide-core/0.13.0/teloxide_core/requests/trait.Requester.html)，并全文检索本地相同版本发布包源码。

[10] [Telegram Bot API sendRichMessage](https://core.telegram.org/bots/api#sendrichmessage)

[11] [xtra 0.6.0](https://docs.rs/xtra/0.6.0/xtra/)、[Mailbox](https://docs.rs/xtra/0.6.0/xtra/struct.Mailbox.html)

[12] [statig 0.4.1](https://docs.rs/statig/0.4.1/statig/)

[13] [Apalis 1.0.0](https://docs.rs/apalis/1.0.0/apalis/)

[14] [apalis-sqlite 1.0.0](https://docs.rs/apalis-sqlite/1.0.0/apalis_sqlite/)

[15] [旧 apalis-sql 0.7.4 SqliteStorage](https://docs.rs/apalis-sql/0.7.4/apalis_sql/sqlite/struct.SqliteStorage.html)
