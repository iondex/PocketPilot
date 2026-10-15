# Telegram 私聊与配对

## 1. 访问模型

Telegram 只接收私聊业务，不支持群、supergroup、channel 或在群中的 callback。即使某个已配对用户在群中发送命令，也必须拒绝。无需配置 chat/user 允许列表；真实 sender 与 private chat 身份来自 Telegram 更新，不以 username、转发来源或用户输入的 ID 授权。

配对按 `(bot_id, user_id)` 持久化在自有 SQLite，chat_id 必须是该用户自己的私聊地址。配对后可使用当前配置允许的所有 host/workspace；不同用户各有自己的私聊 topic/状态卡/绑定，同一 Codex session 的操作仍由同一 actor 串行化。

私聊访问判定、持久 poller、配对账本、CLI 批准/撤销、菜单、topic、历史和输入处理已接通。自动测试覆盖本地 API fixtures；实际手机联调步骤见 [测试入口](../testing.md)。

## 2. CLI 配对

`/start` 流程：

1. 验证消息是用户本人的私聊；群聊静默忽略，不创建配对申请、不泄漏 workspace 信息。
2. 未配对用户只获得一个短期一次性码和“请在运行 PocketPilot 的机器上批准”的提示；此时不能列会话或触发 agent。
3. 管理员在本机执行 `pocketpilot pair approve CODE`。只有 CLI 管理入口可以批准，Telegram 按钮/消息不能自助批准。
4. 成功后再次 /start，复用固定的「PocketPilot · 管理」topic，在其中显示管理菜单；不创建 Codex session。

当前 CLI：

```sh
cargo run -p app --bin pocketpilot -- pair pending
cargo run -p app --bin pocketpilot -- pair approve CODE
cargo run -p app --bin pocketpilot -- pair list
cargo run -p app --bin pocketpilot -- pair revoke BOT_ID USER_ID
```

申请码采用随机 UUID，只存 SHA-256 hash，10 分钟过期，批准事务会消费申请；再次批准同码失败。新申请替换该 bot/user 的旧码；后续 poller 对重复申请做限速，不能让陌生私聊无限刷提醒。

CLI 使用独立的配对管理数据库连接，不获取服务主锁、不执行 operations/deliveries 的恢复逻辑。服务持锁运行时仍可批准；授权变更不写入 TOML 的用户列表，也不改变任何 Codex session。

撤销配对立即禁用对应本地绑定；后续 dispatch/deliver 重新检查配对状态，未开始的输入/输出不再执行。已经被服务端接受的输入不能撤回，不通过 interrupt 实现授权回滚。

## 3. 私聊中的 Session Topic

每个已配对用户有一个固定的「PocketPilot · 管理」topic，只用于管理。/start、/workspaces、/sessions、/new、/open 和 /help 都路由到这里；一个具体 Codex thread 对应另一个 session topic。管理 topic 通过数据库约束禁止绑定 agent。使用 Telegram 支持的 bot 私聊 topic 模式，不创建群聊作为替代。

getMe 的 has_topics_enabled 必须为 true，当前 SDK 已通过私聊 thread_id 保留测试。未启用时明确提示需开启相应 bot topic 模式；不能声称所有普通私聊 bot 默认都可创建 topic。SDK 必须保留正确的 message_thread_id/thread_id 类型。

```text
PocketPilot 私聊
  PocketPilot · 管理：工作区 / 会话 / 新建确认 / 提示
  PP · 调查测试失败 · a13f5b
  WEB · 登录回归 · d8e271
```

topic 名只包含稳定短名和短 ID，不包含运行/连接状态。状态显示在一条复用的状态卡中；标题按成功回执缓存，只有名称确实变化时才改名。升级后的旧标题至多同步一次，避免 Telegram 为每次改名生成服务通知。

未绑定 topic、General 入口或管理 topic 的普通消息进入管理提示，明确没有发送给 agent；已绑定 session topic 中的普通文本只发给该 session。首次从未绑定入口进入时提示管理消息去向，不把默认输入猜测成“最近会话”。

Telegram 的「允许用户创建 topic」开关开启时，客户端会先为入口消息（包括 /start）创建 topic，再把消息发给 bot，这是平台行为。固定入口需要在 BotFather 开启 **Disallow users to create new threads**，保留 bot 的 Topics 功能。程序通过 getMe.allows_users_to_create_topics 诊断此设置，但 Bot API 不提供本项目可调用的对应设置方法；不会自动删除用户已有 topic。官方说明见 [Bot forums](https://core.telegram.org/api/forum#bot-forums)。

## 4. 菜单与绑定

`/workspaces` 在管理 topic 中以两列按钮显示实际工作区。选择、下一页和返回都在原菜单消息中编辑，不为每次点击新发一张菜单；按钮只按实际条目生成，不添加空白占位。会话页过滤由本地专用探针创建回执明确识别的开发 session，不根据标题猜测，不远端归档/删除它们。按精确 cwd 和显式关联 worktree 分页查询，不能把同目录历史视作同一个执行实例。

选择已有会话先显示完整信息与状态新鲜度；进入和绑定只读，不 resume。topic 创建用 provisioning saga，保存返回 ID；结果未知不再次自动创建。重复绑定返回原 topic。同一 bot 私聊中的容器解绑后只可回连原 thread，更换 session 必须新容器。不同 bot 的私聊按 account 隔离。

工作区增改通过人工编辑根目录单一 TOML 和自动热更新；配对用户可使用允许根路径下的 workspace，不可从聊天指定任意 SSH 命令、binary 或参数。

## 5. 新建、继续和状态

用户选择 workspace 并提供首条任务后，明确“创建并发送”。thread/start 与首条 turn 分别记账；回执未知不重新创建 thread。空闲续轮只在用户动作内必要时 resume；active 使用 turn/steer + expectedTurnId，不改现有 model/cwd/审批/sandbox。

| 命令 | 行为 |
| --- | --- |
| /status | 只读刷新实际状态 |
| /history | 一个历史集合，超限一条附件 |
| /info | 完整标题、workspace、host、thread、观察时间 |
| /ask TEXT | 交给当前 thread；斜杠文本可通过此入口发送 |
| /steer TEXT | 仅追加到捕获的当前 turn，冲突重新确认 |
| /rename TEXT | 只改本地显示名/topic 名，不写原 thread 标题 |
| /unbind | 只解除平台映射，不停止会话 |

没有 stop/cancel/kill/审批答复入口。waitingOnApproval/waitingOnUserInput 仅显示，交由原端处理。未授权、已撤销、旧 nonce、错误 peer、离线积压或过期输入都不得触发新业务操作。

## 6. 历史、合并与速率

新绑定空闲会话显示最后用户输入和对应 final；进入 active 会话输出全部可读历史的单条集合，超长一条 Markdown 附件。历史只读，不向 agent 发总结任务。配对/授权、完整性、异常 partial、投递屏障和未知结果规则见 [历史与投递](history-and-delivery.md)。

「待继续」表示当前 app-server 未加载该会话；不是历史丢失或读取失败。该状态仍可只读查看历史，明确发送新输入时才 resume。历史读取失败独立提示且按绑定版本去重，不把已成功读取的运行状态改成“连接未知”。

实时可见 user/agent 文本保留 item 边界，工具/推理过滤；同 session/peer/topic 的短时输出合并。不同用户私聊不能合并，不能将一个用户的历史集合发到另一个 peer。

每个 private chat 的多个 topics 共享 per-chat 预算，叠加 bot 全局限速，429 遵守 retry_after。当前默认每个私聊每分钟 18 条只是保守配置，不是套用群聊限额。用户的轻量 ACK 优先更新状态卡，避免额外消息。

## 7. 配置和账号变化

根目录 pocketpilot.toml 的 `[telegram].token` 为唯一凭据输入，不读取 .env、不 include 其他配置。不需要 chat/user ID 配置。bot ID 由 getMe 确认，同 bot token 轮换保留配对；不同 bot 的旧配对、nonce、绑定、状态卡和积压不会自动继承。

用户关闭/删除 topic 或阻止 bot 时仅调整本地映射/投递，不停止 agent。解绑、撤销配对、退出 PocketPilot 和配置 reload 都不能发 unsubscribe/cancel 或重启服务。
