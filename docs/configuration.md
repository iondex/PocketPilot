# 配置说明

配置入口是一个 TOML 文件，默认读取当前目录的 `pocketpilot.toml`，也可以用全局 `--config /absolute/path/pocketpilot.toml` 指定。以[公开示例](../pocketpilot.example.toml)为起点；字段类型与完整默认值见 `app::config`、`engine::config` 和各平台的 `config` 模块。

## 配置与状态的边界

| 内容 | 保存位置 | 修改方式 |
| --- | --- | --- |
| 主机、允许目录、平台凭据与身份 | 私有 TOML | 编辑后重新诊断；凭据和身份更改需重启 |
| 已发现 provider、控制资格、项目目录 | data_dir 中的 SQLite catalog | CLI 或平台导航 |
| 平台绑定、输入操作、观察游标与投递回执 | data_dir 中的 SQLite 账本 | 由应用持久化，不手工编辑 |
| SSH 密钥、agent、known_hosts 与跳板配置 | 执行账号的 OpenSSH 环境 | 使用系统 SSH 工具维护 |
| 模型账号和模型配置 | 原生 Codex / Claude 账号环境 | 在原生 runtime 中管理 |

程序不展开 `.env`、`$TOKEN` 或其他配置 include；token 应直接保存在权限受限的 TOML 中。相对文件路径按配置文件所在目录解析，远程项目和程序路径属于目标账号。配置中的未知字段会报错，不静默忽略。

## 全局设置

| 表或字段 | 默认值 / 作用 |
| --- | --- |
| `language` | `zh-CN`，也支持 `en`；CLI 和后台文案使用的语言 |
| `[service].data_dir` | `var/data`，绑定、catalog、操作和投递状态 |
| `[logging].format` | `json`；也支持 `text` |
| `[logging].filter` | `info`，tracing 过滤指令；传输日志仍受独立脱敏过滤器约束 |
| `[logging].directory` | 省略时仅终端；配置后写轮转日志 |
| `[logging].rotation` | `daily`，也支持 `hourly` |
| `[logging].retained_files` | `14` |
| `[logging].console` | `true`，有文件日志时仍写终端 |
| `[config_reload]` | 默认启用，debounce 500 ms，周期扫描 30 秒 |
| `[delivery]` | 合并窗口、发送预算、历史预览等；默认值见公开示例 |

热更新发布完整快照并重新验证允许范围。平台凭据、owner、代理和其他要求重启的配置不在运行中替换；看到重启提示后正常退出并重新启动。不要依靠热更新切换控制身份。

## 平台身份与网络

未配置的平台不启动，启用任意一个即可。各平台身份独立，不从另一个平台继承 owner。

| 平台表 | 必填身份和凭据 | 网络 |
| --- | --- | --- |
| `[telegram]` | `token`、正整数 `owner_user_id` | 可选显式无认证 HTTP CONNECT 代理 |
| `[discord]` | `token`、非零 `guild_id`；owner 由服务器查询 | 可选显式代理 |
| `[slack]` | `app_token`、`bot_token`、`app_id`、`team_id`、`owner_user_id` | 可选显式代理 |
| `[feishu]` | `app_id`、`app_secret`、`tenant_key`、`owner_open_id` 和至少一个 `groups` | **固定直连，禁止 `proxy` 字段，包括空字符串** |

国际 Lark 尚未开放配置；后续接入也必须直连，不开放代理参数。Telegram / Discord / Slack 省略 `proxy` 或设为空字符串时直连，不继承环境代理；配置代理后失败不回退直连。飞书 HTTP 显式禁用环境代理，WebSocket 使用 SDK 的直接连接入口。

Telegram 仅接受 owner 在本人私聊内的消息和回调，不能配置群聊或用户 allowlist。`default_locale` 支持 `zh-CN`、`en`、`auto`；用户可通过 `/language` 独立覆盖。数字 ID 必须大于零且能表示为正的私聊 ID，缺失时配置失败。

飞书最多允许 32 个不重复群，每个群填写 `chat_id` 和显示用 `name`。tenant、owner、群与绑定 root/thread 都会核对；仅在平台中取得更大读取权限不会扩大本地执行权限。完整 scopes 和发布步骤见[飞书安装](integrations.md#飞书企业自建应用)。

## 执行主机

每个 `[[hosts]]` 都需要稳定 `id`、显示 `name` 和 `transport`。`allowed_workspace_roots` 是执行账号允许访问的项目根目录，使用目标机绝对路径。

| transport | 配置与用途 |
| --- | --- |
| `local_proxy` | 当前账号的本地 runtime 连接 |
| `ssh_proxy` | 必填 `ssh_alias`，可选 `ssh_config`；继承 OpenSSH 的密钥、known_hosts、跳板与认证 |
| `websocket` | 显式 `endpoint`；自定义入口不能确认目标私有附件目录时拒绝文件输入 |

`[hosts.timeouts]` 控制连接、认证、RPC 读和写回执等待等。提高超时不会改变“未知写入不重发”的规则。TOTP 仅在 `[hosts.authentication.totp]` 中私有配置；挑战匹配不包含实际账号或地址，认证提示和验证码不进入日志。

`[[hosts.runtime_servers]]` 是显式的 Claude server 安装声明：`kind`、桥接主机上的 `artifact`、目标 `directory` 和 `agent_binary`。只有 `hosts bootstrap HOST_ID` 才安装并启动，后台发现和 doctor 不执行安装。详见[独立 runtime server](integrations.md#claude-code-独立-runtime-server)。

## 从旧部署升级

本次开源准备包含两个配置变化：

1. Telegram 不再内置个人账号常量，必须添加 `[telegram].owner_user_id`，然后重启。更换 owner 后旧私聊的队列输出和绑定不能继续使用。
2. 飞书移除了 `proxy`。删除已有 `[feishu].proxy`，包括 `proxy = ""`；确认机器能直连后重启。

这些变更不修改原生会话，也不自动删除 SQLite 状态。原型数据库的升级和备份规则见[发布流程](releasing.md)。
