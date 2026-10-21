# 接入方案

PocketPilot 在一台桥接主机运行，用消息平台接收用户操作，通过本机或 SSH 访问已有 agent runtime。Telegram、Slack、Discord、飞书可以在同一个 `serve` 进程中启用，共享允许连接的执行实例与操作账本。runtime 支持 Codex 原生入口及下面的 Claude Code 独立 server 首版；国际 Lark 及其他 runtime 的方案见[后续支持](support.md)。

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

默认构建包含四个消息平台，无需 features flag。所有构建/检查/测试使用 release profile；`make release` 可顺序执行完整检查、打包和清理。部署进程引用 `dist/pocketpilot`，不要引用会被清理的 `target/`。

配置和 token 只放私有根 TOML，文件权限建议 `0600`。程序不读取 `.env`，不包含其他配置文件。平台凭据、身份或代理更改后重启桥接。相对路径按配置文件所在目录解析；字段和默认值以公开配置示例为准。

`doctor --credentials` 做离线凭据格式检查；`doctor --connect` 只读检查主机与已登记 runtime 入口。Telegram / Discord / 飞书平台探测分别使用 `doctor --telegram`、`doctor --discord`、`doctor --feishu`，不会注册命令或创建 agent 任务。Slack 的 token、owner DM 和 Socket Mode 在 `serve` 启动时验证。

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

server 直接使用目标账号已有的 Claude CLI 模型与认证配置，包括 `~/.claude/settings.json` 中的 Anthropic 兼容接口。已配置 DeepSeek 的账号无需把模型密钥复制到 PocketPilot；远端账号需自行具备对应配置。桥接配置只声明 server 与连接，不重复保存模型 token。

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


## 飞书企业自建应用

本接入使用**国内飞书**开发平台 [open.feishu.cn/app](https://open.feishu.cn/app)，REST 固定为 `https://open.feishu.cn`，配置段为 `[feishu]`。不使用国际 Lark 应用、群自定义 Webhook 机器人或公网回调服务器。认证、token 缓存和 WebSocket/卡片 ACK 复用 OpenLark 0.21.0；没有第二个 Python/Node 消息 sidecar。

当前代码提供私聊目录、执行环境 → 项目 → session、命名空会话创建、已授权群中的独立 session 话题、文字/图片/文件输入、状态和最近历史、增量观察及公共绑定管理。已覆盖离线协议与隔离测试；真实应用的授权、客户端显示和事件投递仍需安装后验收。

### 1. 创建应用并开启机器人

1. 用目标企业账号进入开发平台，选择**创建企业自建应用**，填写名称、图标和描述。
2. 在“应用能力 / 添加应用能力”中开启**机器人**。不要创建只有 Webhook 地址的群自定义机器人。
3. 设置应用可用范围，至少包含你本人及后续添加机器人的操作者。PocketPilot 另行限制 `owner_open_id`，群内其他成员不能因此获得原生任务控制权。
4. 在“凭证与基础信息”取得 App ID 和 App Secret。Secret 只写入本机私有配置，不贴到聊天或提交到 Git。

### 2. 申请权限

推荐本项目的完整附件模式；控制台若提供 `im:message.group_msg:readonly`，优先使用该只读项，否则申请同页列出的 `im:message.group_msg`。

| 权限 | 用途 |
| --- | --- |
| `im:message:send_as_bot` | 以机器人身份发送根卡片、回复话题和更新自己的状态卡片 |
| `im:message.p2p_msg:readonly` | 接收你发给机器人的私聊目录请求 |
| `im:message.group_msg:readonly` 或 `im:message.group_msg` | 接收群里的用户消息，包含没有 @ 的独立文件和图片；这是较宽的群读取权限 |
| `im:message:readonly` | 下载属于入站消息的图片和文件资源 |

若准备通过“获取企业信息”取得 tenant_key，可在基础版本一并临时申请 `tenant:tenant:readonly`，配置完成后移除。

只测试文字时，可以用 `im:message.group_at_msg:readonly` 替代全用户群消息权限，并暂不申请资源下载权限；这时**无法保证收到没有 @ 的独立附件**。全用户群消息权限已覆盖 @bot 输入，不必再重复申请 `group_at_msg`。不用申请 `include_bot`、通讯录、历史导出或用户身份发消息权限。新版卡片回调和获取机器人自身信息没有单独的权限要求。

即使平台推送了全群用户消息，代码也只处理指定企业、指定 owner、已配置群及已绑定 root/thread 的输入；普通群消息、其他人的消息和其他机器人的消息不进入 agent。文字与富文本任务必须明确 @本机器人；文件/图片允许 owner 在绑定话题中独立上传。

### 3. 先发布基础版本，取得所需 ID

先在“版本管理与发布”发布一个包含机器人能力、权限和可用范围的基础版本，等待管理员审核。程序启动会读取机器人身份；飞书要求机器人能力已发布，不能把所有步骤都留到长连接订阅之后。

- **owner_open_id**：在本应用上下文的 API 调试台打开“发送消息”，把 `receive_id_type` 选为 `open_id`，使用“快速复制 open_id”选择本人并复制。只复制即可，不需要发送消息，也不需要通讯录权限。它通常以 `ou_` 开头；不要填 `user_id`。
- **群 chat_id**：在飞书客户端 7.60+ 的目标内部测试群中，打开群设置并复制群 ID，通常以 `oc_` 开头。群名只作显示，不作为路由身份。
- **tenant_key**：使用已有可信事件中的 `header.tenant_key`，或在 API 调试台一次性调用“获取企业信息” `GET /open-apis/tenant/v2/tenant/query`，复制 `data.tenant.tenant_key`。后一方法需临时申请 `tenant:tenant:readonly`；完成初始化后可移除该权限并再次发布。企业编号 `display_id`、App ID、企业域名都不是 tenant_key。

不要为初始化开放匿名模型输入。PocketPilot 不提供“第一个发消息的人自动成为 owner”的绑定方式。

### 4. 写入私有配置并保持程序在线

在根 `pocketpilot.toml` 增加以下段落；不使用的平台段可省略。程序不自动读取 `.env.feishu` 或其他 `.env` 文件。

```toml
[feishu]
app_id = "cli_REPLACE_IN_PRIVATE_CONFIG"
app_secret = "REPLACE_IN_PRIVATE_CONFIG"
tenant_key = "REPLACE_WITH_TENANT_KEY"
owner_open_id = "ou_REPLACE_WITH_OWNER_OPEN_ID"
proxy = ""

[[feishu.groups]]
chat_id = "oc_REPLACE_WITH_ALLOWED_GROUP_ID"
name = "开发测试群"
```

`groups` 至少配置一个已明确授权的群，可以重复添加表项配置多个群；`name` 是菜单中的友好名称。Secret 和身份字段更改后需要重启桥接。空代理不使用系统代理环境变量；如需代理，填写 `http://代理主机:端口`，HTTP 与 WebSocket 均走这一配置。

```sh
chmod 600 pocketpilot.toml
make check
make package
cargo clean
./dist/pocketpilot doctor --credentials
./dist/pocketpilot doctor --feishu
./dist/pocketpilot serve
```

`doctor --feishu` 只获取机器人身份，不发聊天消息、不注册任务；先确保这一步成功。长连接保存时 `serve` 必须保持运行。若已有 systemd 管理的实例，更新配置并重启该桥接实例即可，不要再启动第二个 `serve` 竞争数据目录锁，也不需要停止 Codex/Claude 执行服务。

### 5. 配置事件与新版卡片回调，再发布

在开发平台“开发配置 → 事件与回调”中分别完成：

1. **事件配置**：选择“使用长连接接收事件”，保持本机程序在线后保存；添加 **接收消息 v2.0**，事件标识 `im.message.receive_v1`。
2. **回调配置**：选择长连接接收回调并保存；添加新版 **卡片回传交互**，标识 `card.action.trigger`。不要选择旧版回调或重复添加两个版本。
3. 创建并发布新版本，等待管理员审核，使事件、回调和权限变更生效。若临时增加过企业信息读取权限，可在收尾时移除并一起发布。

同一应用保持一个活动的 PocketPilot 接收实例；不要与使用同一凭据的官方示例 WS 程序同时接收，平台会随机分配事件而非广播。

长连接只需要 App ID / App Secret，不需要公网域名、HTTP 回调 URL 或内网穿透；不把 HTTP 回调的 Verification Token / Encrypt Key 填入本项目配置。平台要求回调快速响应，代码先持久化入队再 ACK，SSH、模型和附件下载不占用回调的等待时间。

采用两次发布是为了适配“启动先读取已发布的机器人身份”和“保存长连接前程序必须在线”的顺序，并非每个飞书应用都必须固定发布两次。

### 6. 加群并验证使用流程

1. 在目标测试群的群设置 → 群机器人中，手动添加已发布的 PocketPilot 应用机器人，确认它能发言；程序不创建群、改群模式或添加成员。
2. 用配置的 owner 私聊机器人，发送 `/start` 或任意文字打开目录。
3. 选择 workspace（执行主机）→ 项目 → session；也可用“新建会话”表单创建命名空会话，名称不会作为模型任务发送。
4. 选择已配置群。程序发送根状态卡片，并以 `reply_in_thread=true` 打开独立话题，保存对应 thread ID。普通群即可使用，无需改成“话题群”。
5. 在该话题中 **@PocketPilot + 任务文字**；混合文字和图片的富文本同样需要 @。不要用群主聊天流或普通引用回复代替话题回复。
6. 在同一话题上传独立文件/图片，不必再补一条 @ 消息；先确认完整附件模式的群消息和资源权限已批准。单条富文本最多四个附件，单文件上限 20 MiB；Claude 图片另受其 adapter 的 5 MiB 限制。
7. `@PocketPilot /status` 只读查看状态；`@PocketPilot /history` 显示最近交互。结果采用 Card JSON 2.0，以角色 emoji、Markdown、代码和表格分段展示；不会生成 `session.md` 或历史导出文件。
8. 私聊“已绑定会话”可查看、刷新、暂停/恢复观察，或确认解绑。解绑保留原生会话、任务、群话题及消息。

输入控制仍取决于 provider 的既有验证资格；飞书安装不会自动授予 runtime 控制权。Codex 支持原生 start/steer，Claude 当前仅支持空闲续轮。首次进入只展示最近交互，读取历史仅为建立增量 cursor，不意味着发送完整历史。

### 常见配置问题与边界

- **保存长连接提示客户端未连接**：先确认基础版本已发布、`doctor --feishu` 成功、`serve` 在线，且国内 App 的凭据未混入 Lark 国际应用。
- **文字能收，独立文件收不到**：检查是否只有 `group_at_msg` 权限；无 @ 附件需要全用户群消息事件权限。收到事件后下载失败，再核对 `im:message:readonly`、机器人是否仍在群中和消息资源是否仍可见。
- **点击没有生效**：确认订阅新版 `card.action.trigger` 并发布；菜单只有指定 owner 在原消息/原聊天点击才有效，操作有效期五分钟，转发、复制、陈旧或重复点击不会执行任务。重新私聊打开目录。
- **有消息但不进入 agent**：检查 owner 的应用内 open_id、tenant_key、群 allowlist 和 root/thread。不存在自动“当前会话”兜底。
- **绑定或写入结果未知**：不会自动重发。先检查群中的根消息或原生会话；核对后可从私聊解绑该记录，再发起新的明确操作。重启不会偷偷再建一个话题或续一轮。
- **旧状态卡片无法更新**：飞书按 message ID 更新卡片有 14 天限制；话题路由仍保留，可用私聊刷新或话题 `/status` 获取新状态，不自动重建根话题。JSON 2.0 卡片要求兼容的飞书客户端（官方最低版本 7.20）。

官方参考：[长连接事件](https://open.feishu.cn/document/ukTMukTMukTM/uYDNxYjL2QTM24iN0EjN/event-subscription-configure-/request-url-configuration-case)、[长连接回调](https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/event-subscription-guide/callback-subscription/configure-callback-request-address)、[接收消息及权限](https://open.feishu.cn/document/server-docs/im-v1/message/events/receive)、[卡片回传交互](https://open.feishu.cn/document/feishu-cards/card-callback-communication)、[话题概述](https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1/message/thread-introduction)、[资源下载](https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1/message-resource/get)、[用户 ID](https://open.feishu.cn/document/home/user-identity-introduction/open-id)、[群 ID](https://open.feishu.cn/document/server-docs/group/chat/chat-id-description)、[企业信息](https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/tenant-v2/tenant/query)。
