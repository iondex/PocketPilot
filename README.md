# PocketPilot

Rust 实现的 Codex 会话桥接器，在本机启动，通过 Telegram 私聊 topic 管理本地/SSH app-server 会话。提供可选 Discord 原型；Lark 和 Claude shim 保留后续规划。

**Telegram 功能链路已接通，SSH 共享实例控制已验证，等待你测试真实私聊交互。** 包括 CLI 配对、工作区与会话菜单、topic 绑定、历史集合、创建/续轮/steer、topic emoji、持久收发与消息合并、OpenSSH/TOTP 和根配置热更新。具体部署状态与验证边界见 [测试入口](docs/testing.md) 和 [验证记录](docs/research/implementation-checks.md)。

## 运行

需要 stable Rust（声明最低 1.94）和当前受测 Codex CLI。官方协议由 build.rs 动态生成到 Cargo OUT_DIR，不提交 schema、生成代码或版本快照。

普通设置和凭据都在根目录 pocketpilot.toml；[公开示例](pocketpilot.example.toml) 不含秘密。配置不 include、不读取 .env，没有 Telegram chat/user 允许列表。真实配置、数据库和日志已被 Git 忽略。

```sh
cargo build --workspace --locked
./target/debug/pocketpilot config-check --credentials
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

批准后再次 /start，在固定的「管理」topic 中选择主机、工作区和会话。管理界面使用 Markdown 编号导航，每次输入后新发菜单；工作区每页 8 项并显示登记总数，0 返回上一页/上级。项目来自手工登记和 Codex 活跃、普通及归档会话的工作目录，按完整 provider/cwd 去重。后台定期补齐；在工作区页 /refresh 会重新扫描会话元数据。普通任务文本会被拒绝；只有明确进入新建流程后，下一条文字才作为会话标题。

/new 按「主机 → 工作区 → 标题 → 确认」创建空会话；也可用 /new HOST/WORKSPACE 标题 或 /new WORKSPACE_ID 标题 快捷创建。同名工作区需指定主机或完整 ID，不会默认挑选一个。首条任务由用户在新话题中发送，活跃会话的输入直接 steer。

/history 动态读取原生会话，返回最近一次用户输入及同轮最终结果，过滤中间说明；尚无最终结果时明确提示。/status 查看实际状态与完整详情。/topics 管理单个话题，也包含「清除全部会话话题」；/topics clear 可直接进入批量清理确认。删除 Telegram 话题与聊天记录时保留 Codex 会话和固定管理入口。Bot API 无法枚举全部话题，仅管理已记录的话题。

私聊命令列表注册中英文说明。Telegram 不支持按 topic 分配不同的命令列表，/help 则按管理话题或会话话题显示对应说明。/unbind 与 /cleartopics 已移除。

共享语言资源与类型化错误见 [i18n 设计](docs/design/i18n.md)。Logo 生成用 [这份 prompt](docs/design/logo-prompt.md)。

Telegram 会话只展示 agent 正文，不回显用户输入，不加角色标签或“开始新一轮”通知；topic emoji 表达状态，运行时持续续期“正在输入”，不自动发送空闲状态卡。空闲首次显示对应 final，活跃历史为单个 agent 文本集合，超长用一个附件；内部完整历史模型保留全部角色。工具和推理不转发。短时输出进入持久批次，同一私聊的所有 topics 共享速率预算。

## 约定

- 业务写入仅创建、用户发起的 resume、续轮和带 expectedTurnId 的 steer；没有停止、取消、审批代答或 agent 服务管理接口。
- 启停、崩溃、重连和配置 reload 只改变桥接连接，不自动恢复或重发 agent 输入；结果未知时先核对。
- 默认只读。provider 的 control_verified 是针对该共享执行实例的部署确认；doctor --connect 本身只检查连接。
- SSH 使用系统 OpenSSH 的有效配置、known_hosts、心跳和认证链；TOTP seed 保留在根 TOML，通过受限 askpass responder 回答已配置的挑战。ProxyJump 和 ProxyCommand 都继承原生配置。
- host/认证/速率由根配置热更新；provider 和 workspace 由 SQLite catalog 动态管理，支持在线新增/移除。Codex binary/socket 自动探测；原容器不能换绑其他 session。
- crate 名称为 ssh、session、codex、engine、telegram、discord、app；源码直接平铺在各 crate 的 src/ 下，binary 仍叫 pocketpilot。

[文档目录](docs/README.md) · [实施计划](docs/planning/implementation.md) · [验收清单](docs/planning/acceptance.md) · [协议生成](docs/design/protocol-generation.md)

## Discord 原型

```sh
cargo build -p app --features discord --locked
./target/debug/pocketpilot serve
```

在同一根配置增加 [discord] 的 token 与 guild_id；可以和 Telegram 同进程运行，共享连接/会话锁/写入账本。一个 workspace 一个 category，一个 session 一个 channel，owner 使用 /pp-workspaces、/pp-sessions、/pp-open、/pp-new 和 /pp-ask。完整范围、安装权限与限制见 [Discord 原型](docs/design/discord.md)。当前未提供真实 Discord 凭据，未做真实 guild 联调。

## 动态目录与日志

provider 是一个现有 agent 执行实例，workspace 是它的项目目录。它们保存在应用 SQLite，不写入 TOML；允许连接的 SSH aliases 和项目根路径仍由配置限定。主机选择页支持 /add /绝对路径 可选名称 和 /remove 名称，普通任务文本只在会话 topic 接收。

```sh
./target/release/pocketpilot doctor --connect --telegram
./target/release/pocketpilot providers list
./target/release/pocketpilot workspaces list
./target/release/pocketpilot workspaces add --provider PROVIDER_ID --cwd /srv/code/project
```

新发现的实例保持只读。完成共享生命周期验证后，可用 providers trust PROVIDER_ID 记录部署确认，--revoke 撤销。测试专用 probe、sessions、state-init、telegram-check CLI 已移除，日常诊断合并到 doctor。

tracing 可同时输出到终端和日志文件，支持 daily/hourly 轮转与 retained_files 数量上限。稳定 0.1 前直接维护 [schema.sql](schema.sql)。开发风格见 [CONTRIBUTING.md](CONTRIBUTING.md)，构建体积分析见 [构建说明](docs/build.md)。

```sh
cargo build -p app --release --no-default-features --locked
./target/release/pocketpilot serve
```

## Telegram 展示与语言

菜单状态只显示 emoji。topic 使用原生 emoji icon：⚡️ 运行、💬 空闲、👀 等待审批、❓ 等待输入、📁 未加载、❗️ 异常、⁉️ 未知。仅状态变化时更新，并由投递队列合并、限速。topic 标题仅显示最多 40 个可见字符的原始标题，不拼主机/路径/ID；详细身份信息由 /status 返回。

新建时填写的内容只作为标题，不发送给 agent，也不启动一轮。原生标题为空时，通过官方 thread/name/set 补写；已有原生标题不覆盖，后台观察/重连不补写。Agent Markdown 使用 pulldown-cmark 解析后转换为 Telegram HTML，表格转为逐项内容；超长仍发送原始 Markdown 附件。

中英文 UI 使用 rust-i18n，偏好按 bot/user 独立持久化。/language zh-CN 切换中文，/language en 切换英文；默认中文，telegram.default_locale 可选 zh-CN、en 或 auto。语言资源编译进 binary，运行时仍只读取根 TOML。
