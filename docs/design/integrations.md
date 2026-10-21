# Integrations 的集中实现与接口

本文记录当前重构的职责和取舍。术语以根目录 CONTEXT.md 为准；agent runtime server 的能力规划另见 agent-runtime-adapters 研究。

## 可行性与收益

三端使用同一配置快照、catalog、会话操作账本和连接协调，已有相同的“执行环境 → 项目 → session”导航与角色展示语义。收进一个 integrations crate 可让 app 只依赖一个入口，消除原来跨平台 fixture 的 crate 级循环依赖，并集中消息能力、长度预算、渲染结果和附件下载契约。

仅做一个转发旧 crate 的 facade，不能收拢这些共同契约；相反，把三个平台都实现为同一个万能 SDK 接口，又会把 ephemeral 回调、Telegram topic、Slack DM/thread、指定用户和 owner 校验压成大量无效参数。选择一个 crate、分平台模块、少量公共 trait。

## 目录与依赖

```text
crates/integrations/src/
  lib.rs             公共入口
  contracts.rs       生命周期、平台身份、附件来源契约
  rendering.rs       渲染请求、能力与可持久化消息段
  navigation.rs      三端共享的目录选择、分页与只读 session 加载
  bindings.rs        绑定清单、缓存状态与观察/解绑规则
  network/           三端共享的 HTTP CONNECT、WS relay、受限下载
  telegram/          Telegram 协议、topic、指定用户、菜单、SQL 与投递
  discord/           Discord 协议、Gateway、V2、菜单、SQL 与投递
  slack/             Slack 协议、Socket Mode、Block Kit、SQL 与投递
```

```text
crates/connections/   Connection、Host、Program、Endpoint 与超时；src/ssh 内实现 OpenSSH
crates/adapters/      Adapter、AdapterFactory、ConnectOptions 与 ServerEndpoint
  src/codex/          Codex DTO、发现、RPC、framing 和 AgentProvider 实现
```

三端始终编译并默认可用，由运行配置决定启用哪些账号，不再使用项目 Cargo features。第三方依赖仍按所需 SDK 功能选择其自身 Cargo features。

主要依赖为 app → integrations → engine → adapters → session，adapters → connections；engine 也直接引用 connections 解析部署配置。Connection 构造本地或 SSH 命令，调用方持有进程生命周期；AdapterFactory 通过通用 ConnectOptions 建连，Adapter 继承 AgentProvider，并提供连接存活与控制能力快照。Codex 的协议帧、握手和 CLI 参数属于 adapters::codex。engine 不依赖 integrations；公共 Integration Context 由 integrations 持有并引用 engine。Claude Code / Pi 的 runtime server 和 driver 仍在规划。

network 只有 integrations 这一类使用方，因此作为其内部公共网络模块。它不是业务词汇中的 Connection。OpenSSH 配置、命令和认证由 connections::ssh 持有；Codex 原生实现由 adapters::codex 持有。原 ssh 与 codex crate 已删除，不保留兼容 facade。

## 公共接口

| 契约 | 统一内容 | 平台保留内容 |
| --- | --- | --- |
| `Integration` | 具名平台身份、能力、注入 Context、运行及退出 | 身份校验、收消息协议、后台任务与平台资源 |
| `Renderer` | 无 I/O 的语义输入、明确格式/预算、可序列化的固定消息段 | Slack Block Kit、Discord V2、Telegram RichMessage 的类型化 DTO |
| `AttachmentSource` | 下载输入文件及类型化错误 | 平台文件身份、URL allowlist、凭据、重定向与预算检查 |

Renderer 的关联 Part 类型保留平台完整表达能力；公共层不持有无约束的 JSON，也不要求所有平台降到最低 Markdown 子集。平台原生 UI builder 可继续表达菜单、按钮、卡片、图库和结构化数据，语义正文经共同 Markdown 文档生成。

角色/阶段来自 session::presentation；表格、对齐、公式和脚注应在文档中保留，直到平台转换。无法原生表达的内容转换为文字或代码，保持内容可读。平台能力与已经启用的渲染格式须一致，不能把官方存在的接口直接当作本应用已支持。

## 授权与投递契约

- Context 只注入已存在的配置、共享连接及 Store；渲染不启动进程、不读取凭据、不执行工具。
- 接收消息后先进行平台 ACK 与身份/作用域检查；写入前重新检查配置、绑定 revision 和时效。
- 原生布局、格式版本、分段内容须在首次写入前固定；重启不重新渲染已冻结的段，不重新发送未知结果。
- 平台 SQL 和状态机仍在所属目录，共享同一数据库并不等于共享平台授权。
- AttachmentSource 只在明确授权的输入路径调用；附件保持用户上传给 agent 的方向，不恢复历史导出。
- Discord V2 布局使用稳定 Twilight builders/model；现有 Serenity HTTP/Gateway 保留代理和账号认证。V2 消息类型与传统消息不得隐式互换。

## 代价及验证

一个 crate 会扩大单次平台改动的重编译范围；目录拆分本身不能承诺缩小 binary 或加快 LTO。收益来自依赖单向、公共约束集中、平台实现局部化，以及所有支持端始终经过同一检查。

不在第一轮抽象平台特有的 callback、topic 和 owner/指定用户校验流程，也不让 engine 操作 SDK DTO。后续只有在至少两个平台共享同一行为及错误/恢复语义时，才继续合并该流程。

验证沿用三端 protocol/HTTP fixtures、公共渲染长度和结构测试、owner/指定用户隔离、一次性动作、未知结果保护及重启分段恢复；统一由 make check 和默认全平台 make package 执行。视觉和新 API 的真实投递由用户触发，文档中的平台能力不替代客户端验收。

## 公共导航

`integrations::navigation::Navigator` 统一工作环境、项目与 session 的选择。主机按名称和 ID 排序；项目先按最近活动倒序，再按忽略大小写的名称、目录和稳定 ID 排序。主机/项目的稳定 ID 优先于同名别名，名称、SSH alias、项目短名与路径须唯一；歧义或已移除的父主机明确拒绝，不转到另一执行环境。Telegram 的 `主机/项目` 快捷键也通过同一选择器处理。

三端的浏览和 Telegram 新建流程共用有界分页；过时 offset 超出目录范围时回到首页，每页最多 100 项。原生控件决定更小的页大小：Telegram 主机/项目 8、session 6；Discord 主机/项目 20、session 8；Slack 主机 8、项目 80、session 8。从 session 返回项目列表时，统一定位到该项目当前所在的页。session 页面统一解析项目归属、取得共享运行时并调用已有只读 picker，继续使用其排序、游标快照和操作探针过滤。

公共导航不持有平台凭据或回调句柄，不创建、resume 或续轮。各平台将结果映射成原生控件，保留授权、菜单消息更新、一次性动作、过期确认及本地 SQL；已有持久动作编码继续使用。Telegram 的显式 catalog 刷新属于独立平台流程，不在选择器中隐式触发。

模块封装、平台无关计算及持久收发的第三方库复用范围见 [库复用核验](../research/integration-library-reuse.md)。

菜单动作、资源创建、输入确认与投递回执的后续收束顺序见 [状态转换规划](../planning/integration-state-transitions.md)。该文区分交互句柄复用和实际副作用领取，列出现有三端差异及逐批验收条件。

## 已绑定会话管理

`integrations::bindings` 集中绑定清单、状态、分页、跳转地址、观察偏好与带版本检查的变更。`Repository` 保留三端原生 SQL，`Manager` 接收已通过平台授权的作用域：Telegram bot/私聊、Discord bot/guild/owner、Slack app/team/bot/DM/owner。目标仍是对应 topic、channel 或 thread 绑定；公共模块不接管指定用户、owner 校验或 agent 写权限。

三端首页的“已绑定会话”每页最多显示八个入口，包含执行环境、项目和绑定状态。详情显示上次读取到的会话状态及缓存年龄，不能把缓存当作实时状态；清单不连接运行时，元数据由现有打开与观察路径填充。移除项目、变更 endpoint 等使绑定标为失效，仍允许用户明确解绑。解除绑定的记录保留在清单中，不删除聊天历史或停止原生任务。

“暂停自动观察”只停止后续后台读取、状态卡刷新与新输出发现，任务输入、手动状态与最近交互仍可用。已经开始的观察及排队投递按原账本完成。恢复观察后继续原游标，可能补发暂停期间的变化。暂停偏好使用独立 revision，不修改绑定 revision，现有输入确认仍有效；解绑增加绑定 revision，使旧输入和投递失效。操作在同一数据库事务内复核账号作用域、绑定 revision 和观察 revision，过期或重复管理操作不能覆盖新状态。

平台保留原生表现：Discord 的 `/pp-bindings` 与首页按钮使用私有 select/button 菜单，Slack 使用 App DM 内的 Block Kit，Telegram 的 `/bindings` 路由到管理 topic。三端菜单均有暂停/恢复与单条解绑确认。Discord/Slack 可打开原聊天容器；Telegram bot 私聊 topic 缺少可靠官方直达链接，显示话题说明并提示从话题列表进入，不构造 `t.me/c`。依据见 [导航与预算核验](../research/binding-navigation-links.md)。

本轮仅收束绑定管理；[交互状态转换规划](../planning/integration-state-transitions.md) 中的其他流程仍保留现有实现。

绑定详情与确认页的可用操作由公共 `Entry::controls` 决定；三端仅将这些操作翻译为原生持久句柄。确认页重新检查选中版本和 active 状态；Slack 状态卡的管理按钮与 Discord `/pp-unbind` 共用此流程，不再维护另一套解绑执行路径。
