# 资料与研究记录

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

Telegram 文档描述支持 bot 私聊 topics；MVP 已改为仅私聊，需验证 bot 的 topic 模式及当前 SDK 支持，不创建群作为替代。Discord privileged intent、interaction 等完整细节在该阶段需要单独查证，当前只是架构预留。

## 4. Rust 生态

- [teloxide](https://docs.rs/teloxide/latest/teloxide/)：Telegram API、long polling、命令、框架与 feature；不假设框架自动满足持久化 inbox。
- [openssh](https://docs.rs/openssh/latest/openssh/)：Unix-only、OpenSSH 配置复用、ControlMaster、认证限制、远程 shell 转义与错误不确定性。
- [sqlx](https://docs.rs/sqlx/latest/sqlx/)：数据库访问与 SQLite 支持参考。
- [crates.io API](https://crates.io/data-access)：逐项读取 `GET /api/v1/crates/{name}` 的版本、downloads、recent_downloads 和更新时间；历史查询的版本与下载量保留在实施计划表格；当前实际依赖以仓库 Cargo.lock 为准。

资料通过搜索、上游源码和官方页面核对。引用记录保留来源及版本边界，不以工具运行环境或单次抓取结果代替接口保证。

## 5. 本机静态验证记录

1. 确认仓库约定、目标功能和部署前提。
2. 查询 PATH CLI `codex --version` 和 app-server、daemon、proxy、generate-json-schema 帮助。
3. 查询桌面内置 binary 的版本。
4. 两份 binary 分别执行 schema 导出，比较实际方法/字段。
5. 读取桌面相关进程元数据，确认远程连接使用 SSH proxy；实际部署标识不进入公开记录。
6. 检查默认控制 socket 是否存在，没有建立真实会话连接。
7. 使用 GitHub API 固定上游 commit 并读取源码。
8. 获取 Telegram/Discord/库文档和 crates.io 元数据。

文档 JSON 快照已移除。官方 schema、只读投影和 Rust DTO 由 build.rs 动态生成到 OUT_DIR，见 [生成说明](../design/protocol-generation.md)；版本、行为边界仍由本文与验收报告说明。

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

上述事项被转化为 [实施计划](../planning/implementation.md) 中的 P0 门禁和验收矩阵。设计完成不等于这些行为已经被证明。

## 7. 补充需求的资料与静态核对

新增资料对应部署配置、历史投递与后续扩展设计；固定版本和审阅范围保留在本节的源码链接与后续扩展文档，不再维护 JSON 元数据副本。

### Codex 与 SSH 配置

- 实际读取 [OpenAI Remote connections](https://developers.openai.com/codex/remote-connections.md)：桌面发现 SSH alias、通过远程登录 shell 管理 app-server；不由此断言存在公开的本地多主机聚合 RPC。
- 本机 `man ssh`：-G 评估 Host/Match 后输出配置并退出。`man ssh_config`：Include、Match、PreferredAuthentications、KbdInteractiveAuthentication 的语义。只用 `ssh -G -F /dev/null localhost` 验证无配置网络连接的输出，**没有读取真实 SSH 文件或尝试 OTP 登录**。
- [russh client 源码](https://github.com/warp-tech/russh/blob/e5d80a07dc554480b6ea3b2e14adfc190407b627/russh/src/client/mod.rs)：keyboard-interactive start/respond、partial_success、keepalive/inactivity 配置。
- [totp-rs 文档](https://docs.rs/totp-rs/latest/totp_rs/)：TOTP 算法与 secret 类型；只是库能力核对，没有读取实际 seed 或生成真实 OTP。

### Lark

- 实际完整读取 [Thread Introduction](https://open.larksuite.com/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1/message/thread-introduction)：thread_id、reply_in_thread、group_message_type=thread、按 thread 分页历史与新旧群类型差异。
- [官方 Python SDK WS sample](https://github.com/larksuite/oapi-sdk-python/blob/0d9749a4728446a0ffa2e8374b37b5e1ce1cfd9c/samples/ws/sample.py)：消息事件、新版 card.action.trigger 与 ws.Client 的组合。
- [官方 WS client](https://github.com/larksuite/oapi-sdk-python/blob/0d9749a4728446a0ffa2e8374b37b5e1ce1cfd9c/lark_oapi/ws/client.py)：心跳、帧组包、事件分发；[reply model](https://github.com/larksuite/oapi-sdk-python/blob/0d9749a4728446a0ffa2e8374b37b5e1ce1cfd9c/lark_oapi/api/im/v1/model/reply_message_request_body.py)：reply_in_thread、uuid 字段。
- 未完整核实的 REST、WS 与 callback 细节不作为保证；uuid 幂等窗口、地区/版本回调差异、卡片更新期限及配额需在目标组合中另行验证。

### Claude 与 shim 候选

- 实际读取 [Claude Code headless](https://code.claude.com/docs/en/headless)、[CLI reference](https://code.claude.com/docs/en/cli-reference)、[Sessions](https://code.claude.com/docs/en/sessions)：stream-json、resume/replay user、SIGTERM/EOF 与 transcript 格式限制。
- 实际读取 [Streaming vs single mode](https://code.claude.com/docs/en/agent-sdk/streaming-vs-single-mode)：长驻 streaming input、消息排队；不将排队解释为已验证 Codex steer。
- [codex-acp-gateway README](https://github.com/mmonad/codex-acp-gateway/blob/81db00d32207c3333cd05ab6463099ae21fc0d46/README.md)、[acp_proxy.rs](https://github.com/mmonad/codex-acp-gateway/blob/81db00d32207c3333cd05ab6463099ae21fc0d46/src/acp_proxy.rs)、[lib.rs](https://github.com/mmonad/codex-acp-gateway/blob/81db00d32207c3333cd05ab6463099ae21fc0d46/src/lib.rs)：直接 Rust 协议转换候选，断线 EOF/kill 与退出循环的限制。
- [Claude ACP adapter](https://github.com/agentclientprotocol/claude-agent-acp/tree/c0af592d252e6bd072b53407f618a9dd4cd02797)：README 与 package.json，确认 TypeScript/官方 Agent SDK 后端；没有执行它或核对其全部进程清理代码。
- [ACP Rust SDK](https://github.com/agentclientprotocol/rust-sdk/tree/f5c0eef0972bbda8e75a151aeb088642911b3568)：官方 ACP Rust 支持，协议与 Codex wire 有区别。
- [claude-agent-sdk-rs subprocess](https://github.com/tyrchen/claude-agent-sdk-rs/blob/2af170209806aaa4f3d90651463ced80626fc870/src/internal/transport/subprocess.rs)：Drop/start_kill 和 stdin shutdown；[另一个 Rust SDK](https://github.com/Wally869/claude_agent_sdk_rust/tree/c716c96787f4a3d08650cbb925f08d4f77327beb) 只核对 README/元数据。

候选 SDK 的“production-ready”是作者自述，不作为 PocketPilot 结论；许可证、版本、协议语义与生命周期需采用前审查。crate 研究数据见实施计划表格，构建依赖版本以 Cargo.lock 为准。未安装、编译或运行这些候选，未升级 CLI，未创建 Lark 资源。
