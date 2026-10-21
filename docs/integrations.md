# 接入方案

PocketPilot 在一台桥接主机运行，用消息平台接收用户操作，通过本机或 SSH 访问已有 agent runtime。Telegram、Slack、Discord 可以在同一个 `serve` 进程中启用，共享允许连接的执行实例与操作账本。runtime 支持 Codex 原生入口及下面的 Claude Code 独立 server 首版；Lark / 飞书及其他 runtime 的方案见[后续支持](support.md)。

## 构建与启动

准备仓库指定 Rust 工具链、系统 OpenSSH、Codex CLI 和已运行的 app-server。复制根 [pocketpilot.example.toml](../pocketpilot.example.toml) 为私有 `pocketpilot.toml`，填写主机和要启用的 integration；未使用的平台段落直接省略。

```sh
make check
make package
cargo clean
./dist/pocketpilot doctor --credentials
./dist/pocketpilot doctor --connect
./dist/pocketpilot serve
```

默认构建包含三个已实现的消息平台，无需 features flag。所有构建/检查/测试使用 release profile；`make release` 可顺序执行完整检查、打包和清理。部署进程引用 `dist/pocketpilot`，不要引用会被清理的 `target/`。

配置和 token 只放私有根 TOML，文件权限建议 `0600`。程序不读取 `.env`，不包含其他配置文件。平台凭据、身份或代理更改后重启桥接。相对路径按配置文件所在目录解析；字段和默认值以公开配置示例为准。

`doctor --credentials` 做离线凭据格式检查；`doctor --connect` 只读检查主机与已登记 runtime 入口。Telegram / Discord 平台探测分别使用 `doctor --telegram`、`doctor --discord`，不会注册命令或创建 agent 任务。Slack 的 token、owner DM 和 Socket Mode 在 `serve` 启动时验证。

## 执行环境与控制资格

UI 中先选 workspace，即执行主机，再选该主机下的项目目录，最后选 session。主机允许范围和 SSH alias 来自 TOML；provider 和项目保存在 catalog，不在 TOML 里维护旧式 provider/workspace 数组。

```sh
./dist/pocketpilot hosts list
./dist/pocketpilot providers list
./dist/pocketpilot workspaces list
./dist/pocketpilot workspaces add --provider PROVIDER_ID --cwd /srv/code/project --name 示例项目
```

本机与远程都需连接已有、可共享的 app-server。Desktop、CLI 和 PocketPilot 使用同一历史目录，不代表它们连接了同一执行实例。只有 stdio、没有共享入口的原生进程不能直接附加；不要用另起 app-server 或自动 resume 代替共享验证。

新发现的 provider 默认只读。停止桥接后，使用专用项目执行：

```sh
./dist/pocketpilot providers verify PROVIDER_ID --project PROJECT_ID
```

该命令会创建专用会话、提交验证任务和附件，检查多连接共享 active turn、steer、断开客户端后任务继续及最终读取；成功后启用控制。再恢复 `serve`。失败不自动重发未知写入，也不停止原生任务。已有外部验证结果时可用 `providers trust PROVIDER_ID` 人工登记，`--revoke` 撤销；普通连接诊断不授予控制权。

SSH 使用系统配置、密钥/agent、known_hosts、ProxyJump / ProxyCommand 与认证链。需要 TOTP 时仅在私有 TOML 中配置受限挑战匹配。不要关闭主机密钥校验，也不要自动停止原生 daemon。

## Telegram

### 安装

1. 在 BotFather 创建 bot，取得 token，填入 `[telegram].token`。`default_locale` 可设为 `zh-CN`、`en` 或 `auto`。
2. 开启 bot 私聊 Topics 功能。为固定管理入口，关闭用户自行创建新 thread 的权限；程序会诊断这些能力，不代替管理员修改 BotFather 设置。
3. 仅源码 `integrations::telegram::OWNER_USER_ID` 指定的账号可在本人私聊使用。更换指定用户需修改常量并重新构建；不使用配对申请或 CLI 批准。群聊和其他账号静默拒绝。
4. 指定用户发送 `/start`，进入固定“管理”topic。

### 使用

管理 topic 选择执行环境、项目、session；普通任务只在已绑定 session topic 中发送。`/new` 创建命名空会话，标题不作为首轮任务，第一条任务随后发送。`/status` 查看状态，`/history` 查看最近交互，`/history errors` 查看近期错误。

`/bindings` 或首页“已绑定会话”提供暂停/恢复自动观察与解绑。bot 私聊 topic 没有可靠的官方直达链接，请从聊天的话题列表进入。`/topics` 管理已记录话题；删除话题会删除该平台的聊天记录，原生会话保留。`/language en`、`/language zh-CN` 切换用户界面语言。

状态通过原生 topic icon、角色/阶段 emoji 和 typing 提示表达。Agent 正文使用 RichMessage；管理菜单独立转义。长输出按结构分段，不导出历史文件。

## Slack

### 安装与最小权限

创建自己的内部 Slack App，开启 Bot 和 Socket Mode。仅接受配置 owner 与 App 的一对一 DM；不需要将 bot 邀请到频道，也不需要 channel ID。

| 设置 | 必要值 |
| --- | --- |
| App-Level Token | `connections:write`，保存为 `app_token`（`xapp-…`） |
| Bot OAuth Scopes | `chat:write`、`im:history`、`im:write`、`files:read` |
| Bot User OAuth Token | 保存为 `bot_token`（`xoxb-…`） |
| Event Subscriptions | 订阅 bot event `message.im` |
| Interactivity | 开启，用于按钮、下拉框和 modal |
| App Home | Messages Tab 启用且可写 |
| 安装身份 | `app_id`、`team_id`、`owner_user_id` |

在 Slack 管理页面完成设置并安装到工作区；新增 scopes 后重新安装授权。所需字段填入 `[slack]`，详见根配置示例。运行不需要 App Configuration Access Token 或带额外管理权限的 full token；仓库不再提供安装 manifest。

启动时核对 team/bot 身份，并通过 `conversations.open` 取得 owner DM。其他用户、管理员、频道、群聊和其他 DM 均不能进入控制流程。Socket Mode 使用出站连接，不要求公网回调地址。

### 使用

在 Apps → PocketPilot → Messages 发送“菜单”或普通文字，依次选择执行环境、项目和 session。DM 主输入框只打开菜单，不向最近会话发送任务。进入会话后，在对应根卡片的 **thread 内回复** 才提交任务。

新建按钮打开标题表单，创建命名空会话，首轮任务在 thread 内另发。状态卡的“管理”进入公共绑定详情；暂停/恢复与解绑复用带版本检查的确认流程。

首次进入读取历史以建立完整观察游标，但只展示最后一次用户输入及对应最终回复。再次进入已初始化绑定复用原 thread，不重读完整历史、不重发入场内容；原 thread 已有聊天记录仍可见。后台只投递可见变化。

正文使用 Block Kit，包括 rich_text、header、table；不支持的样式转换为文字。长回复分段投递，DM 内各 thread 共用发送预算。

## Discord

### 安装

1. 在 Developer Portal 创建 bot，使用 Guild Install，将 `bot` 和 `applications.commands` 安装到目标服务器。通过 Gateway 接收交互，Interactions Endpoint URL 留空。
2. Bot → Authorization Flow 关闭 Requires OAuth2 Code Grant；PocketPilot 不提供 OAuth 回调服务器。
3. 授权 View Channels、Manage Channels、Send Messages、Read Message History、Attach Files。运行只使用 `GUILDS` intent，不需要 Message Content、Server Members 或 Presence 特权 intent。
4. 在 `[discord]` 填写 Bot Token 与 `guild_id`。应用 ID 由平台查询，不使用 Public Key 或 client secret 代替 Bot Token。
5. 使用服务器 owner 账号操作；其他 Administrator 也不会自动获权。新建 category 对普通成员隐藏，但服务器管理员仍遵循 Discord 自身权限。

`serve` 启动时注册自身 `/pp` 和 `pp-*` 命令并连接 Gateway。Gateway Ready 日志表示连接已建立；实际菜单与权限由服务器中的操作确认。

### 使用

`/pp` → 执行环境 → 项目 → session。执行环境、项目、会话和绑定入口最多 25 项一页，不超过时完整单页展示。快照过期或游标越界回首页，清除旧翻页记录。菜单仅调用者可见，返回与刷新更新原消息。

会话页的“新建会话”打开原生表单：填写首轮任务 → 核对项目和预览 → 点击“创建并发送任务”。空会话列表也显示入口。表单与预览阶段不连接 agent；确认才创建会话、发送首轮并建立 channel。成功后点击“进入会话”跳转。也可使用显式的 `/pp-new workspace:环境 project:项目 text:任务 confirm:true`。

| 操作 | 入口 |
| --- | --- |
| 提交任务 | 在绑定 channel 执行 `/pp-ask text:任务`；`text` 必填且不能为空，`file` 为可选附件 |
| 状态 / 最近交互 | `/pp-status`、`/pp-recent` |
| 绑定管理 | `/pp-bindings` 或首页按钮 |
| 解绑确认 | `/pp-unbind` |
| 重命名 channel | `/pp-rename title:名称` |

普通 channel 文字不作为 agent 输入。一个项目对应 category，一个 session 对应 channel。创建和重命名 channel 时只移除普通空格，大小写、标点和其他字符保留；应用不另做名称归一化。

正文和菜单使用 Components V2，表格转换为列名与值。过程、最终结果和错误有对应角色/阶段 emoji；短段按最终渲染长度合并，图库每条最多十项，已冻结投递保持原分段。

## 绑定、输入与附件

打开会话默认是只读动作。idle 输入开始新轮次，active 输入使用原生 steer；NotLoaded 只因明确用户输入而 resume。等待审批或输入不会伪装成 idle，PocketPilot 不代答审批。

“已绑定会话”显示执行环境、项目、上次观察状态和缓存年龄，可打开原聊天入口或管理绑定。暂停只影响后续自动观察，手动查询和任务输入仍可用；已开始的读取、排队输出可能完成。解绑保留原生任务和平台记录。

| 平台 | 用户上传文件的位置 |
| --- | --- |
| Telegram | 已绑定 session topic，文件/图片可带说明 |
| Slack | 已绑定 thread，文件可带说明；主输入框不选择默认会话 |
| Discord | `/pp-ask text:说明 file:附件`，目前提供一个 file 选项 |

Telegram/Slack 允许纯文件输入，使用默认说明；Discord 要求 text。公共输入最多四个文件，单文件最多 20 MiB。下载在授权和绑定校验之后进行，仅访问受限的平台资源；程序不解压或执行附件。

文件经单独操作账本上传到目标 Codex 账号的私有附件目录。图片用原生图片输入，其他文件以路径上下文交给 agent；不将私人绝对路径带回展示文本。无法确认目标私有目录的自定义 WebSocket 入口会拒绝附件。链接过期需重新上传，未知写结果不重发。

三端均不提供 session 历史导出，不生成 `session.md`。文件上传是用户给 agent 的输入能力。

## 代理与网络

每个 integration 可独立设置 `proxy`。省略或空字符串表示直连，不读取环境代理；配置代理后失败不自动回退直连。

```toml
proxy = "http://192.0.2.10:7890"
```

当前支持无认证 HTTP CONNECT，并保持 TLS 证书检查。平台 HTTP、WebSocket 和附件下载按同一平台配置路由；SSH 的 ProxyJump / ProxyCommand 属于主机连接配置，是另一层连接。

## 日常维护

使用服务管理器运行一份 `serve`，退出或重启只影响桥接。代理和平台凭据更改需重启；catalog 的项目管理可在运行中进行。日志支持终端/文件与轮转，不记录消息正文、token、OTP seed、认证提示或完整传输帧。

重启后的旧输入不会自动重放。结果未知时先核对原端，不反复创建会话、channel 或 thread。升级和迁移前备份私有配置与完整 data_dir，保留去重和回执记录；操作方法见[状态数据与迁移](architecture.md#状态数据与迁移)。

独立自动续轮工具通过 `./dist/pocketpilot retry` 明确启用，可不配置聊天平台。它与 `serve` 分开运行，使用 data_dir/retry 下独立状态；错误选择、60 秒等待、资格复查和结果未知规则见[retry 架构](architecture.md#独立-retry-工具)。


## Claude Code 独立 runtime server

此入口只管理 PocketPilot 自己创建的 Claude session，不接管已打开的任意 Claude 终端。CLI 在目标机由目标账号正常安装并登录；不将 token 打包或随二进制上传。已静态核对 CLI **2.1.304** 和对应 SDK **0.3.304** 的 stream-json 类型；离线测试不调用真实模型。遇到需权限审批的工具时保持等待，聊天端的结构化问答与权限处理尚未开放。

桥接主机准备 musl 目标与 C toolchain（Debian/Ubuntu 为 `musl-tools`），再打包：

```sh
rustup target add x86_64-unknown-linux-musl
make package
make package-agent-server
```

`dist/pocketpilot-agent-server-x86_64-unknown-linux-musl` 是独立静态 server；它不包含 Claude CLI。aarch64 可在配置好对应交叉 C toolchain 后用 `RUNTIME_TARGET=aarch64-unknown-linux-musl` 重新打包。各目标都保存在同一个 target 根的 release 目录，不创建 debug 构建；保留 dist 后执行 `cargo clean`。

在目标 host 下增加显式 server 配置。路径是示例，请使用目标账号真实存在且已授权的项目根；`artifact` 相对本机配置文件解析，`agent_binary` 是目标机 CLI 的路径，不是桥接主机路径。

```toml
[[hosts.runtime_servers]]
kind = "claude_code"
artifact = "dist/pocketpilot-agent-server-x86_64-unknown-linux-musl"
directory = "~/.pocketpilot"
agent_binary = "/home/operator/.local/bin/claude"
```

执行安装；local 与 SSH 使用相同命令，SSH 仍走既有密钥、known_hosts 和 TOTP 认证链：

```sh
./dist/pocketpilot hosts bootstrap HOST_ID
./dist/pocketpilot providers list
./dist/pocketpilot workspaces add --provider PROVIDER_ID --cwd /srv/code/project --name 示例项目
./dist/pocketpilot doctor --connect
```

安装会核对 Linux 架构、传输 hash 和可执行文件，在 `~/.pocketpilot/bin/agent-server-HASH` 保存版本化二进制，并从 `~/.pocketpilot/claude-code/` 提供服务。目标需有 `sh`、`flock`、`sha256sum`、`nohup`。已有 socket 时只复用原入口，不更新/重启活跃 server 或任何 Claude 任务。后台扫描、doctor 与普通重连均不会安装或启动 daemon。

新 provider 默认只读。Codex 的 `providers verify` 专用 steer 探针不适用于 Claude；它会拒绝非 Codex provider。用隔离环境验证本账号的 CLI 握手、空闲输入、断线后继续执行及回执后，再通过 `providers trust PROVIDER_ID` 明确登记控制资格。开启后仍不支持活跃轮次 steer；空闲会话可续轮。未知写入不能通过重复点击或自动重连重发。

附件沿用四文件、单文件 20 MiB 的入口上限；Claude 图片另限单图 5 MiB，并只接受 PNG/JPEG/GIF/WebP。文件记录归属 session，不能用其他会话的上传路径。历史只包含 server 管理的已观察输入/主 assistant 文本，不导出文件，不生成 `session.md`。server 异常退出后状态标为未加载/未知，不自动 resume 原生执行。
