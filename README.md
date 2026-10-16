# PocketPilot

Rust 实现的 Codex 会话桥接器，在本机启动，通过 Telegram 私聊 topic 管理本地/SSH app-server 会话。提供可选 Discord 原型；Lark 和 Claude shim 保留后续规划。

**Telegram 功能链路已接通，SSH 共享实例控制已验证，等待你测试真实私聊交互。** 包括 CLI 配对、工作区与会话菜单、topic 绑定、历史集合、创建/续轮/steer、状态卡、持久收发与消息合并、OpenSSH/TOTP 和根配置热更新。具体部署状态与验证边界见 [测试入口](docs/testing.md) 和 [验证记录](docs/research/implementation-checks.md)。

## 运行

需要 stable Rust（声明最低 1.94）和当前受测 Codex CLI。官方协议由 build.rs 动态生成到 Cargo OUT_DIR，不提交 schema、生成代码或版本快照。

普通设置和凭据都在根目录 pocketpilot.toml；[公开示例](pocketpilot.example.toml) 不含秘密。配置不 include、不读取 .env，没有 Telegram chat/user 允许列表。真实配置、数据库和日志已被 Git 忽略。

```sh
cargo build --workspace --locked
./target/debug/pocketpilot config-check --credentials
./target/debug/pocketpilot telegram-check
./target/debug/pocketpilot doctor --connect
./target/debug/pocketpilot serve
```

只连接已经运行的 app-server，缺少 endpoint 时明确失败。Desktop、CLI 和 PocketPilot 必须连接同一执行实例；相同历史目录不能证明共享。部署前先读 [测试入口](docs/testing.md)。

## Telegram

只接受用户本人的私聊。发送 /start 取得申请码，然后在本机另一个终端批准：

```sh
./target/debug/pocketpilot pair approve CODE
./target/debug/pocketpilot pair pending
./target/debug/pocketpilot pair list
./target/debug/pocketpilot pair revoke BOT_ID USER_ID
```

批准后再次 /start，在固定的「PocketPilot · 管理」topic 中选择工作区和已有会话。管理界面使用 Markdown 编号导航：回复序号依次选择主机、工作区和会话，0 返回，/refresh 刷新；普通任务文本会被拒绝。新建使用 /new WORKSPACE_ID 任务，回复 1 确认后创建；已绑定的会话 topic 内直接发送文本即可续轮或 steer。topic 名保持稳定，运行状态显示在固定状态卡。/status、/history、/info、/rename 和 /unbind 均可使用。

Telegram 会话只展示 agent 正文，不回显用户输入，不加角色标签或“开始新一轮”通知；emoji 状态卡保持简洁。空闲首次显示对应 final，活跃历史为单个 agent 文本集合，超长用一个附件；CLI 完整导出保留全部角色。工具和推理不转发。短时输出进入持久批次，同一私聊的所有 topics 共享速率预算。

## 约定

- 业务写入仅创建、用户发起的 resume、续轮和带 expectedTurnId 的 steer；没有停止、取消、审批代答或 agent 服务管理接口。
- 启停、崩溃、重连和配置 reload 只改变桥接连接，不自动恢复或重发 agent 输入；结果未知时先核对。
- 默认只读。provider 的 control_verified 是针对该共享执行实例的部署确认；doctor --connect 本身只检查连接。
- SSH 使用系统 OpenSSH 的有效配置、known_hosts、心跳和认证链；TOTP seed 保留在根 TOML，通过受限 askpass responder 回答已配置的挑战。
- host/workspace/速率支持热更新。执行地址改变须使用新的 provider ID；原平台容器不能换绑其他 session。
- crate 名称为 session、codex、telegram、discord、app；源码直接平铺在各 crate 的 src/ 下，binary 仍叫 pocketpilot。

[文档目录](docs/README.md) · [实施计划](docs/planning/implementation.md) · [验收清单](docs/planning/acceptance.md) · [协议生成](docs/design/protocol-generation.md)

## Discord 原型

```sh
cargo build -p app --features discord --locked
./target/debug/pocketpilot serve
```

在同一根配置增加 [discord] 的 token 与 guild_id；可以和 Telegram 同进程运行，共享连接/会话锁/写入账本。一个 workspace 一个 category，一个 session 一个 channel，owner 使用 /pp-workspaces、/pp-sessions、/pp-open、/pp-new 和 /pp-ask。完整范围、安装权限与限制见 [Discord 原型](docs/design/discord.md)。当前未提供真实 Discord 凭据，未做真实 guild 联调。
