# 05 · 资料与研究记录

调研日期：2026-10-15（Asia/Shanghai）。链接可能更新；涉及具体源码行为的结论以固定 commit 为准。

## 1. OpenAI 官方文档

通过官方域名搜索后实际下载并读取页面，不只引用搜索摘要：

- [Codex App Server](https://developers.openai.com/codex/app-server/) / [Markdown 版本](https://developers.openai.com/codex/app-server.md)：transport、initialize、thread/list/read/resume、状态、分页历史、turn/steer、审批、CLI remote。
- [Codex CLI reference](https://developers.openai.com/codex/cli/reference/)：CLI remote/transport 的参考入口。相关可用命令同时用本机二进制 help 核对；不把网页描述当成旧版 CLI 实际支持证明。

官方文档明确说明 app-server/WebSocket 的实验状态；本文没有将其描述为稳定生产公共 API。默认示例 model 名称与本项目无关，PocketPilot 不硬编码文档示例模型。

## 2. 固定源码核对

读取 OpenAI 官方开源仓库 [openai/codex](https://github.com/openai/codex)，固定 commit：

`6fbd7be1518d259e30e6fef48f05f433d2461a20`，commit 时间 `2026-10-15T11:24:07Z`。

| 源码/测试 | 用于核对 |
| --- | --- |
| [CLI main.rs](https://github.com/openai/codex/blob/6fbd7be1518d259e30e6fef48f05f433d2461a20/codex-rs/cli/src/main.rs) | app-server proxy 分支：解析 socket 后转发，不隐式 daemon start |
| [thread_processor.rs](https://github.com/openai/codex/blob/6fbd7be1518d259e30e6fef48f05f433d2461a20/codex-rs/app-server/src/request_processors/thread_processor.rs) | resume_running_thread 与冷恢复分流；配置覆盖处理、历史拼接 |
| [thread_lifecycle.rs](https://github.com/openai/codex/blob/6fbd7be1518d259e30e6fef48f05f433d2461a20/codex-rs/app-server/src/request_processors/thread_lifecycle.rs) | rejoin、订阅附着、pending request replay、无订阅且非 active 后才计时卸载 |
| [turn_processor.rs](https://github.com/openai/codex/blob/6fbd7be1518d259e30e6fef48f05f433d2461a20/codex-rs/app-server/src/request_processors/turn_processor.rs) | expectedTurnId 校验、non-steerable turn、start-or-steer 竞争语义 |
| [thread_state.rs](https://github.com/openai/codex/blob/6fbd7be1518d259e30e6fef48f05f433d2461a20/codex-rs/app-server/src/thread_state.rs) | 连接订阅与活跃快照管理 |
| [thread_resume.rs tests](https://github.com/openai/codex/blob/6fbd7be1518d259e30e6fef48f05f433d2461a20/codex-rs/app-server/tests/suite/v2/thread_resume.rs) | running resume、配置继承、多个 writer 等待验证场景的参考 |
| [thread_unsubscribe.rs tests](https://github.com/openai/codex/blob/6fbd7be1518d259e30e6fef48f05f433d2461a20/codex-rs/app-server/tests/suite/v2/thread_unsubscribe.rs) | idle timeout、订阅消失场景的参考 |
| [turn_steer.rs tests](https://github.com/openai/codex/blob/6fbd7be1518d259e30e6fef48f05f433d2461a20/codex-rs/app-server/tests/suite/v2/turn_steer.rs) | steer 场景后续实测的参考入口 |

**没有编译或运行上游测试**。源码支持设计方向，不证明本机二进制就是该 commit，也不证明桌面私有集成路径具有相同保证。

## 3. Telegram 与 Discord

已实际读取：

- [Telegram Bot API](https://core.telegram.org/bots/api)：getUpdates/offset、24 小时更新保留、至少一周无更新后 update_id 可以随机重建、ForumTopic、message_thread_id、createForumTopic/editForumTopic、标题长度、callback_data、sendMessage 长度、retry_after、topic 权限。
- [Telegram Bot FAQ](https://core.telegram.org/bots/faq)：管理员/privacy mode 收消息行为、long polling 与 webhook 互斥、每 chat/群/总体发送建议限制。
- [Discord Channel resource](https://discord.com/developers/docs/resources/channel)：category/channel 映射、每 category 最多 50 个 channel、命名/parent_id 参考。

Telegram 文档也描述支持 topic 的私聊能力；MVP 选择 forum supergroup，避免把 SDK 对新 Bot API 能力的支持程度变成首版前置条件。Discord privileged intent、interaction 等完整细节在该阶段需要单独查证，当前只是架构预留。

## 4. Rust 生态

- [teloxide](https://docs.rs/teloxide/latest/teloxide/)：Telegram API、long polling、命令、框架与 feature；不假设框架自动满足持久化 inbox。
- [openssh](https://docs.rs/openssh/latest/openssh/)：Unix-only、OpenSSH 配置复用、ControlMaster、认证限制、远程 shell 转义与错误不确定性。
- [sqlx](https://docs.rs/sqlx/latest/sqlx/)：数据库访问与 SQLite 支持参考。
- [crates.io API](https://crates.io/data-access)：逐项读取 `GET /api/v1/crates/{name}` 的版本、downloads、recent_downloads 和更新时间；查询结果见 [crates-snapshot.json](research/crates-snapshot.json)。该文件不是经过编译验证的 lockfile。

研究使用 agent-reach 路由中的 Exa 搜索、GitHub CLI 和通用网页读取方式。环境有 Exa/mcporter 和 gh，但没有 agent-reach 主命令，所以 doctor/check-update 不可执行；没有因此假装做过环境体检或版本更新检查。没有安装或更新技能依赖。

## 5. 本机静态验证记录

1. 确认当前工作目录和远程仓库起初为空，无现有应用实现或仓库约定需要沿用。
2. 查询 PATH CLI `codex --version` 和 app-server、daemon、proxy、generate-json-schema 帮助。
3. 查询桌面内置 binary 的版本。
4. 两份 binary 分别执行 schema 导出，比较实际方法/字段。
5. 读取桌面相关进程元数据，确认远程连接使用 SSH proxy；不保留个人 host 名称。
6. 检查默认控制 socket 是否存在，没有建立真实会话连接。
7. 使用 GitHub API 固定上游 commit 并读取源码。
8. 获取 Telegram/Discord/库文档和 crates.io 元数据。

匿名机器可读记录：[protocol-observations.json](research/protocol-observations.json)。仅保留版本、schema 字段/方法与 hash，未提交原始生成 bundle（避免大体积、许可证和版本噪声；实施时生成/筛选受测 fixtures）。

复现实测字段差异的命令：

```sh
codex --version
codex app-server --help
codex app-server proxy --help
codex app-server generate-json-schema --out /tmp/pocketpilot-schema
```

桌面 binary 路径因安装产品/版本不同而变化，不能写死为所有用户通用路径。协议必须按**实际服务使用的 binary**核对，不能只在 PATH 随便找一个 Codex。

## 6. 明确未验证

- 没有接入真实桌面/远程 app-server 列表、读取用户会话内容或发送消息。
- 没有执行真实 thread/start/resume/steer/interrupt、审批回复或 unsubscribe。
- 没有运行 bot、创建 topic/channel、读取 Telegram 私人消息或安装 bot 权限。
- 没有改变 Codex 配置、开启控制 socket、bootstrap/start/stop/restart daemon。
- 没有完成三端同实例、断线不取消、跨端审批和历史补偿的行为验收。
- 没有为 Claude 宣称具体支持能力。

上述事项被转化为 [实施计划](04-mvp-plan.md) 中的 P0 门禁和验收矩阵。设计完成不等于这些行为已经被证明。
