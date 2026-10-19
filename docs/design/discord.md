# Discord 原型

此版本是可运行的可选原型，尚未完成真实 guild 验收。它复用 Codex provider、连接缓存、每会话锁和操作账本；Telegram 和 Discord 同时控制同一会话时，在同一进程内共享锁，原生服务端仍负责跨 Desktop/CLI 的并发条件检查。已有 bot 和测试服务器时，按 [Discord 验收](../testing-discord.md) 从只读到写入逐步测试。

## 配置和启动

在用户自建服务器安装自己的 bot，授权 bot 和 applications.commands。需要 View Channels、Manage Channels、Send Messages、Read Message History、Attach Files；使用用户计划的 Administrator bot 也能工作。原型使用 GUILDS intent 和 slash commands，不需要 Message Content intent。

在根 pocketpilot.toml 中填写 [discord].token、guild_id 和可选 proxy，token 不进入 .env、日志或示例。然后运行：

```sh
make package FEATURES=discord,slack
cargo clean
./dist/pocketpilot doctor --credentials --discord
./dist/pocketpilot serve
```

可以只配 Discord，也可以与 Telegram 同进程启动。未启用编译 feature 却填写 Discord 配置会明确报错；没有两个并行服务争用同一个账本的部署方式。host/workspace 沿用同一根配置，平台 token/guild 和 data_dir 改动需要重启桥接。

启动只 upsert 自己的 pp-* guild commands，不批量删除其他命令。应用 ID 由 Discord 官方账号接口取得。服务器 owner 在请求执行前重新校验；默认命令权限是 Administrator，但其他管理员不能通过原型的 owner 校验。群外、私信、其他 guild 和非 owner 都不能进入控制流程。

## 交互

| 命令 | 行为 |
| --- | --- |
| /pp-workspaces | 选择工作空间（本机/AWS 等执行环境） |
| /pp-projects workspace:环境名称或ID | 查看该环境内的项目目录和短名 |
| /pp-sessions workspace:环境 project:项目短名 | 分页列出状态、短标题和 thread ID；下一页传返回的 cursor |
| /pp-open workspace:环境 project:项目短名 thread:ID | 为已有会话建立 category/channel 映射；不 resume 或发送输入 |
| /pp-new workspace:环境 project:项目短名 text:内容 confirm:true | 明确创建新会话并发首轮，每步单独持久回执 |
| /pp-ask text:内容 | 在绑定 channel 内续轮；NotLoaded 显式 resume，active 使用 expectedTurnId steer |
| /pp-status | 只读刷新状态 |
| /pp-history | 一个消息/附件查看全部可读 user/agent 文本 |
| /pp-rename title:显示名 | 只修改 Discord channel 名 |
| /pp-unbind | 只解除映射，原生会话继续运行 |

命令先收到 ephemeral ACK，再处理慢 SSH。操作回执仅发送给调用者，实际会话历史与输出进入其 session channel。新 category 对 @everyone 隐藏，owner 和 bot 有显式访问权限；已有服务器管理员仍遵循 Discord 自身权限规则。

用户先选 workspace（执行环境/主机），再选项目目录，最后选 session。目录选择、归属校验与排序复用 engine::navigation；平台只负责命令呈现。

一个项目一个 PP category，一个 session 一个短名 channel；短名尾部保留 thread 标识。channel 内固定状态卡展示状态、短标题和 thread ID。进入 active 会话先投递一个完整历史集合，idle 显示最后用户输入与对应 final；正文通过公共 Markdown 解析输出 Discord 支持的子集，菜单和动态标题按字面转义。渲染后超过 1,800 个 UTF-16 单元或代码反引号无法保真表示时，发送保留原文的单个 Markdown 附件，原型附件预算 8 MiB，超限明确报错、不截断冒充完整历史。所有自动消息关闭 mentions，agent 输出中的 @everyone 不会触发提醒。

## 持久化与生命周期

平台 ID 按字符串保存，避免 Discord snowflake 的有符号整数边界。discord_categories、discord_provisions 保存资源创建前的 dispatching 和返回 ID；未知结果不再自动创建，避免重复 category/channel。原型尚无资源核对 CLI：未知创建需管理员先检查 Discord 和本地账本，不能直接反复执行命令。

discord_bindings 保存 bot/guild/channel、owner、workspace/instance/thread、cwd 和 endpoint 指纹；数据库禁止改绑身份。解绑保留记录，同会话恢复到原 channel；更换会话必须创建另一个 channel。workspace 范围或 endpoint 改变时冻结旧映射。删除 category/channel 后的自动替换暂未实现，需人工核对。

agent 输入使用与 Telegram 相同的 operations 状态机：Received → Prepared → Dispatching → Accepted/Rejected/OutcomeUnknown。丢回执、进程退出和重新收到同一 interaction 都不会再次发送未知输入。原型没有持久离线命令 inbox：未收到 ACK 的命令由用户重新明确发起；已经进入写账本的请求按记录核对。

观察使用只读轮询、独立绑定 worker 和每实例历史读取限额。一次观察中变更的文本合为一个集合；持久 outbox 与观察游标同事务提交。后台每轮至少间隔五秒；guild 文本 outbox 每四秒最多发一条，再由 Serenity 执行 API bucket/429 限速。该节流是原型固定值，Telegram 的可配置 burst/私聊预算保持独立。

输出发送前重新检查配置、owner、绑定 revision 与 endpoint。发送失败或回执未知只标记 unknown，不自动重发。状态卡首次发送前保存 unknown 墓碑；首次回执丢失不反复创建卡，/pp-status 仍可查询。更新已知状态卡可安全重试同一内容。

服务退出关闭 gateway shards 和本机连接；后台没有 resume、interrupt、unsubscribe、审批代答、daemon restart 或 stop。状态未知不会显示为空闲。原有会话模型、cwd、sandbox 和 approval 设置不由续轮/steer 覆盖。

## 验证与下一阶段

当前 mock HTTP + 类型化 WebSocket fixture 覆盖 category/channel 创建、完整历史过滤、只读进入、steer、同 interaction 去重、丢回执不重发、owner 拒绝、解绑不停止任务；另有持久资源/输出恢复与容器不可改绑测试。

真实 gateway、权限、上传和断网行为需要在目标服务器单独验收。下一阶段再增加 select/button 菜单、普通 channel 文本输入、完整离线 inbox、资源核对 CLI、删除 channel 后替换与可配置合并预算。当前用 slash 明确发消息，避免把普通聊天误当 agent 输入。

只读接入诊断使用 `doctor --discord`；Gateway 的实际 Ready 事件会记录 `Discord Gateway ready`，与 REST 初始化日志区分。
