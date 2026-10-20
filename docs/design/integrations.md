# Integrations 的集中实现与接口

本文记录当前重构的职责和取舍。术语以根目录 CONTEXT.md 为准；agent runtime server 的能力规划另见 agent-runtime-adapters 研究。

## 可行性与收益

三端使用同一配置快照、catalog、会话操作账本和连接协调，已有相同的“执行环境 → 项目 → session”导航与角色展示语义。收进一个 integrations crate 可让 app 只依赖一个入口，消除原来跨平台 fixture 的 crate 级循环依赖，并集中消息能力、长度预算、渲染结果和附件下载契约。

仅做一个转发旧 crate 的 facade，不能收拢这些共同契约；相反，把三个平台都实现为同一个万能 SDK 接口，又会把 ephemeral 回调、Telegram topic、Slack DM/thread、配对和 owner 校验压成大量无效参数。选择一个 crate、分平台模块、少量公共 trait。

## 目录与依赖

```text
crates/integrations/src/
  lib.rs             公共入口
  contracts.rs       生命周期、平台身份、附件来源契约
  rendering.rs       渲染请求、能力与可持久化消息段
  network/           三端共享的 HTTP CONNECT、WS relay、受限下载
  telegram/          Telegram 协议、topic、配对、菜单、SQL 与投递
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

不在第一轮抽象平台特有的 callback、topic 和 owner/pairing 流程，也不让 engine 操作 SDK DTO。后续只有在至少两个平台共享同一行为及错误/恢复语义时，才继续合并该流程。

验证沿用三端 protocol/HTTP fixtures、公共渲染长度和结构测试、owner/配对隔离、一次性动作、未知结果保护及重启分段恢复；统一由 make check 和默认全平台 make package 执行。视觉和新 API 的真实投递由用户触发，文档中的平台能力不替代客户端验收。
