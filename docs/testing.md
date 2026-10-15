# Telegram 测试入口

代码提供 `pocketpilot serve`：私聊配对、工作区/会话菜单、topic 绑定、完整历史集合、创建/续轮/steer、状态卡、持久消息合并和配置热更新。当前受测 Codex CLI 为 0.162.1；协议在 build.rs 中动态生成。

## 先准备共享服务

PocketPilot 连接已经运行的 Codex app-server；启动、退出、重连不会启动或重启 agent 服务。Desktop、CLI 和 PocketPilot 必须连接同一个执行实例。共用 ~/.codex 的历史文件不能证明执行实例相同。

已按授权配置 anjie-bsc-arb 的既有共享 daemon 和九个工作区；SSH 控制已通过专用会话验收并开放。直接使用 /workspaces 选择。其他机器请填写实际共享 Unix socket，或已经存在的 loopback WS/WSS endpoint。

本机当前 Desktop 以 stdio 运行，没有可附加的控制 socket/公开 listener；现有本地活跃任务尚不能接入。没有重启 Desktop，也没有用一个共用历史文件的新服务冒充它。支持受测的本地共享入口，仍需本机部署提供这样的入口。

公开字段见根目录 [pocketpilot.example.toml](../pocketpilot.example.toml)。实际配置全部放根目录 pocketpilot.toml，不 include、不读取 .env；Telegram token、密码和 TOTP seed 都内联，真实配置已被 Git 忽略。cwd 使用 daemon 返回的规范绝对路径；同一个 workspace 的显式 worktree 可加入 additional_cwds。

首次只读检查：

```sh
cargo build --workspace --locked
./target/debug/pocketpilot config-check --credentials
./target/debug/pocketpilot doctor --connect --telegram
./target/debug/pocketpilot sessions --workspace WORKSPACE_ID list
```

本机和 SSH 的 Unix proxy 都在字节通道上执行 WebSocket 握手。SSH 读取原有 OpenSSH config，保留 Include、Match、ProxyJump、密钥/agent 和 known_hosts 规则。远端非交互 shell 必须保持 stdout 安静。密码、key passphrase 和 TOTP 通过 OpenSSH askpass responder 读取同一根 TOML；OTP 提示必须匹配明确的 prompt_patterns，未知挑战拒绝回答。

在独立测试会话中确认三端看到同一 active turn、桥接断开后任务继续，再把对应 provider 的 control_verified 设为 true。这个字段是部署确认；只读 doctor 不会自动授予控制资格。未开启时列表、状态、历史照常可用，写入明确拒绝。

## 启动与配对

```sh
./target/debug/pocketpilot serve
```

在 Telegram 私聊 bot 发送 /start，取得一次性码后，在本机另一个终端运行：

```sh
./target/debug/pocketpilot pair approve CODE
```

批准后再次 /start。管理员 CLI 可与服务同时运行，不获取执行账本的服务锁、不恢复执行中操作。其他人必须完成同样的本机批准。群聊输入、群内按钮和错误 private peer 均不能进入业务流程。

已配置 bot 的只读账号检查已通过，私聊 Topics 已开启；初始检查未发送测试消息或消费 updates，正式 serve 启动后才处理私聊请求。其他 bot 需要先在 BotFather 开启私聊 Topics。发现 webhook 时 serve 拒绝启动，不自动删除 webhook。

## 建议测试顺序

1. /workspaces 选择工作区；已有会话可分页选择。新建且尚未进入历史索引的已加载会话也应可见。
2. 进入空闲会话，确认只显示最后用户输入和对应 final；进入运行中会话，确认全部可读 user/agent 文本在一个历史集合内，长内容为单个 Markdown 附件。工具和推理内容不发送。
3. /status 和 /info 对比原端状态；直接发送文本，空闲时续轮，运行中使用带 expectedTurnId 的 steer。等待审批时也可追加 steer，审批和结构化问答继续在原端处理。
4. /new WORKSPACE_ID 任务，点击“创建并发送”后才创建 thread 和首轮；没有停止、取消、重启或审批代答命令。
5. 连续产生几条可见文本，检查合并与私聊共享速率预算；多个 topic 不应各自突破预算。429 按 retry_after 等待。
6. 修改根 TOML 增删允许的 host/workspace 或调节速率；有效配置自动生效，非法配置保留最后有效版本。已撤销配对、已解除映射或已移除 workspace 的排队动作/投递不得继续执行。
7. 停掉并重启 PocketPilot，确认原端任务继续运行，桥接不自动 resume/start/steer，也不重发结果未知的请求。重启前的 pending 消息及轮询故障后补收的输入应显示草稿，点击新的确认按钮前不得进入 agent；解绑后旧草稿按钮失效。断网时状态卡应显示连接未知。
8. /rename 只改平台显示名；/unbind 只解除映射。关闭/删除 topic 后再次添加应创建新容器，原容器不能被换绑到另一个 session。

正常退出使用 Ctrl-C 或 SIGTERM。data_dir、Telegram/Discord token 和 Discord guild 修改需要重启桥接；host/workspace、SSH 认证策略和 delivery 设置支持热更新。日志输出位置/过滤器在启动时装配。

## 已验证与仍需你验证

自动测试包含本地 Bot API + WebSocket fixtures 的配对/topic/历史/创建/续轮/steer 流程，以及丢回执不重复写入、快照/权限撤销、持久批次恢复。另用真实 Codex 0.162.1、独立临时 CODEX_HOME 和本机模拟模型，验证了 TCP WebSocket、Unix proxy 的多客户端 active turn、steer、最后连接退出后继续完成和最终历史读取；没有接触原有用户任务或调用付费模型。

按用户授权，另在远端共享 daemon 上创建专用真实模型探针：active_observed、steer_accepted、断线后仍 active、completed、final_marker_found、resume_accepted 全部通过；Desktop 的只读 API 也观察到相同 active turn。原有用户任务未收到测试输入。开发期间创建的专用探针会话保留供核对。

当前仍需你验证真实 Telegram 私聊 topic 收发和实际操作体验；本地既有 Desktop 接入、真实 OTP/慢服务器故障组合尚未验收，不能宣称 52 项场景全部通过。

未知写入/消息发送不会自动重试；先核对 /status、/history 和原客户端，再决定是否重新发送。历史超过单个附件上限会给出明确提示，不用截断文件冒充完整历史。

## Discord 原型

可选编译和同进程运行，详见 [Discord 原型](design/discord.md)。仅配 Telegram 时无需启用此 feature。Discord 尚未提供真实 token/guild，当前是 mock API/原生协议 fixture 验证，不宣称已连上真实服务器。

## 当前本机桥接

桥接已在本机后台启动，进程号保存在被忽略的 var/data/bridge.pid。停止桥接时仅向该进程发送 SIGTERM；不对 Codex daemon 或其进程组发送信号。日志位于 var/logs，配对 CLI 可同时运行。电脑休眠/退出登录等本机环境变化可能使桥接离线，原生任务生命周期仍由其 daemon 管理。
