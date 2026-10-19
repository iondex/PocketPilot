# Slack 私聊安装与升级

PocketPilot 仅支持与配置 owner 的一对一私聊。无需创建频道、邀请 bot、填写 channel ID 或使用 `/pp`。一个私聊管理多个项目，每个原生会话对应一张根卡片和一个 thread；主输入框里的文字只打开项目菜单，不自动提交给 agent。

本机通过 Socket Mode 出站连接 Slack，无需公网地址、Request URL、ngrok 或单独的 HTTP 服务。已有 App 可以原地升级，不必新建 App 或更换 App ID。

## 1. 更新现有 App（网页方式）

1. 打开 [Slack Apps](https://api.slack.com/apps)，选择现有 PocketPilot。
2. 进入 **App Manifest**，先保存现有 manifest 的私有备份，再参考仓库的 [slack-manifest.json](slack-manifest.json) 更新配置。如果原 App 有自定义图标、描述等信息，保留这些字段。
3. 在 **App Home → Show Tabs** 开启 **Messages Tab**，允许用户发送消息。Manifest 对应 `features.app_home.messages_tab_enabled=true`、`messages_tab_read_only_enabled=false`。不需要 Home Tab。
4. 在 **OAuth & Permissions → Bot Token Scopes** 保留/添加以下四项：

   | Scope | 用途 |
   | --- | --- |
   | `chat:write` | 发送根卡片、菜单、会话回复，更新状态 |
   | `im:history` | 接收授权用户发给 App 的消息 |
   | `im:write` | 启动时通过 `conversations.open` 确认 owner 的唯一 DM |
   | `files:write` | 导出完整历史和长输出 |

5. 删除旧频道权限 `channels:read`、`groups:read`、`channels:history`、`groups:history`；移除 `/pp` slash command 及其 `commands` 权限。
6. 在 **Event Subscriptions → Subscribe to bot events** 只保留 `message.im`，删除 `message.channels`、`message.groups`。保持 **Socket Mode** 和 **Interactivity** 开启，无需 Request URL。
7. 点击 **OAuth & Permissions → Reinstall to Workspace**，批准新增权限；如需 workspace 管理员审批，请由管理员完成。保存 manifest 不等于授予新权限。
8. 核对安装后的 **Bot User OAuth Token**。若 Slack 显示新 `xoxb-…`，在本机私有 TOML 中替换；不要把 token 发到聊天或提交 Git。
9. 在 **Basic Information → App-Level Tokens** 确认现有 `xapp-…` 仍有 `connections:write`。原 token 有此权限即可复用，不需重建。

新建 App 时，在 Slack Apps 选择 **Create New App → From a manifest**，导入仓库 manifest，再执行上述安装和 app-level token 步骤。

## 2. 用 Slack CLI 更新现有 App

可以使用 [Slack CLI 的 `slack api`](https://docs.slack.dev/tools/slack-cli/reference/commands/slack_api/) 调用 manifest API，无需把 Rust 项目改造成 Slack 托管项目，也不要使用 `slack deploy` 启动 PocketPilot。

先安装包含 `slack api` 的当前 [Slack CLI](https://docs.slack.dev/tools/slack-cli/)，执行 `slack version`、`slack api --help` 核对。仅有 CLI 登录或运行用的 `xapp`/`xoxb` 不代表有管理现有 App 的权限。即使 `xapp` 勾选了名为 `app_configurations:write` 的 scope，manifest API 仍可能拒绝它并返回 `not_allowed_token_type`；这不是继续增加 bot scopes 能解决的问题。以下方法使用 **App Configuration Access Token**，需要 `app_configurations:read` / `app_configurations:write`，且能管理目标 App；在 Slack Apps 的配置 token 页面生成，不要将它加入 PocketPilot 运行配置。

以下命令在 Bash 中执行。token 通过隐藏输入进入环境，不放进命令行参数；不要启用 `set -x`、`slack --verbose`，也不要把私有导出加入 Git。

```bash
cd /absolute/path/PocketPilot
umask 077
mkdir -p config/private
SLACK_APP_ID=A000000001  # 替换为现有 PocketPilot 的 App ID
read -r -s -p 'App Configuration Access Token: ' SLACK_USER_TOKEN
printf '\n'
export SLACK_USER_TOKEN
# 如需代理，在此 shell 为 CLI 设置 HTTPS_PROXY/HTTP_PROXY。
slack api apps.manifest.export app_id="$SLACK_APP_ID" > config/private/slack-manifest.before.json
```

先确认导出结果 `ok=true`，再保留原 App 的自定义字段，仅替换 PocketPilot 的消息入口、权限和事件：

```bash
python3 - <<'PY'
import json
from pathlib import Path
response = json.loads(Path('config/private/slack-manifest.before.json').read_text())
if not response.get('ok'):
    raise SystemExit('Manifest export failed; check management token and App ID.')
manifest = response['manifest']
features = manifest.setdefault('features', {})
features.pop('slash_commands', None)
features['app_home'] = {
    'home_tab_enabled': False,
    'messages_tab_enabled': True,
    'messages_tab_read_only_enabled': False,
}
manifest.setdefault('oauth_config', {})['scopes'] = {
    'bot': ['chat:write', 'im:history', 'im:write', 'files:write'],
}
settings = manifest.setdefault('settings', {})
settings['socket_mode_enabled'] = True
settings['interactivity'] = {'is_enabled': True}
settings['event_subscriptions'] = {'bot_events': ['message.im']}
Path('config/private/slack-manifest.dm.json').write_text(
    json.dumps(manifest, ensure_ascii=False, indent=2) + '\n'
)
PY
```

查看 `config/private/slack-manifest.dm.json`，确认目标仍为原 App，且自定义字段符合预期。随后验证并更新：

```bash
slack api apps.manifest.validate manifest="$(cat config/private/slack-manifest.dm.json)"
# 上一步 ok=true 后执行：
slack api apps.manifest.update app_id="$SLACK_APP_ID" manifest="$(cat config/private/slack-manifest.dm.json)"
unset SLACK_USER_TOKEN
```

每次都检查响应 `ok`，不要只看进程退出码。[apps.manifest.update](https://docs.slack.dev/reference/methods/apps.manifest.update/) 会替换整个 manifest，因此这里先导出并修改；不直接用模板覆盖现有 App。若 token 失效、管理权限不足或 API 返回不确定结果，先在 App 页面核对实际配置，不自动重发更新。

**CLI 更新后仍需回到 OAuth & Permissions 执行 Reinstall to Workspace 并批准新增权限。** 这一步通常需要浏览器或管理员交互，现有 bot token 无法自行授予权限。若 CLI 报 `missing_scope` / `invalid_auth`，检查管理 token；运行时报缺少 `im:write` / `im:history`，检查重新安装是否完成。

## 必要 token 与最小权限

| Token | 运行时需要 | 最小权限 / 用途 |
| --- | --- | --- |
| App-Level Token（`xapp-…`） | 是，放根 TOML 的 `app_token` | 只需 `connections:write`，用于 Socket Mode |
| Bot User OAuth Token（`xoxb-…`） | 是，放根 TOML 的 `bot_token` | manifest 中的四项 bot scopes |
| App Configuration Access Token | 否，不放运行 TOML | 仅升级时导出、校验和更新 manifest |
| Telegram / Discord token | 对应平台启用时需要 | 与 Slack token 相互独立 |

不需要给运行用 xapp 增加 `app_configurations:write` 或 `authorizations:read`。在 **Basic Information → App-Level Tokens** 生成仅包含 `connections:write` 的 token，替换根 TOML 的 app_token 并验证后，再在 Slack 页面撤销旧 full token；已撤销的旧 token 无法恢复。App-Level Tokens 不属于 manifest 配置，这次 manifest 更新不会自动修改或撤销它们。

新增 bot scopes 后需 Reinstall to Workspace；若 `conversations.open` 返回 `missing_scope` 并同时列出 channels/groups/mpim/im 的 write scopes，那是不同会话类型的备选权限，本项目只需要 `im:write`。不要把列出的全部频道/群聊权限加回去。

## 3. 精简本机配置

在 Git 忽略的根 `pocketpilot.toml` 中配置：

```toml
[slack]
app_token = "xapp-REPLACE_IN_PRIVATE_CONFIG"
bot_token = "xoxb-REPLACE_IN_PRIVATE_CONFIG"
app_id = "A000000001"
team_id = "T000000001"
owner_user_id = "U000000001"
proxy = ""
```

App ID 来自 Basic Information；team ID 来自 workspace 地址；owner ID 在个人资料菜单选择 **Copy member ID**。只允许这个用户操作，不自动授权其他管理员。App/team/owner 保留为明确安装和操作者边界。

删除旧的 `allowed_channel_ids`，新版本会拒绝该废弃配置。无需填写 DM ID；程序使用 `conversations.open(users=[owner_user_id])` 从 Slack 获取它。`proxy` 省略或为空时直连，不继承代理环境变量；如需代理，填写 `http://192.0.2.10:7890`，运行配置只支持无认证 HTTP CONNECT。

配置、凭据或 owner 更改后重启 PocketPilot。程序不读取 `.env`，上述 CLI 管理 token 也不参与运行。

## 4. 构建、检查和启动

```sh
make check
make package FEATURES=discord,slack
cargo clean
./dist/pocketpilot doctor --credentials
./dist/pocketpilot doctor --connect
./dist/pocketpilot --config /absolute/path/PocketPilot/pocketpilot.toml serve
```

只需要 Telegram 和 Slack 时可使用 `FEATURES=slack`。多平台共用一个 `serve` 进程和执行账本。不要同时启动两份桥接。需要长期运行时，将服务管理器的可执行文件指向 `dist/pocketpilot`，不要引用会被清理的 `target/`。

`doctor --credentials` 只做离线格式检查；`doctor --connect` 诊断 Codex 主机连接。Slack token、owner DM 与 Socket Mode 身份在 `serve` 启动时验证；失败提示不包含 token 或完整响应。成功后才处理消息和投递输出。

## 5. 在 Slack 中使用和验收

1. 在 Slack 左侧 **Apps** 找到 PocketPilot，打开 **Messages**。若不能输入，检查 Messages Tab 是否启用且可写，并核对 App 已重新安装。
2. 发送“菜单”“开始”或任意普通文字。主输入框里的文字只打开工作空间选择菜单，不会作为 Codex 任务发送。
3. 先选择 workspace（本机/AWS 等执行环境），再在下拉框选择该环境的项目/目录，最后选择已有会话。PocketPilot 发送该会话的根卡片，并给出打开 thread 的链接；打开只读，不会自动 resume。
4. 在对应根卡片的 **thread 内回复** 才提交任务。可用专用测试会话发送“只回复 SLACK_DM_OK，不运行工具，不修改文件”，核对原端只收到一次。
5. 切换项目或环境时使用菜单的返回按钮，或在 DM 主输入框再次发送“菜单”。导航和分页更新原消息，不连续追加菜单；会话卡片的操作按钮集中排列。不同项目的会话使用各自根卡片/thread；不会把主输入框当作最近一次会话的输入。
6. 根卡片提供状态、最近交互、完整历史和解除绑定。再次打开同一会话复用原 thread，不重播历史；解绑仅解除桥接，不终止原生任务。
7. 新建会话通过按钮打开标题输入框，确认后创建空会话，第一条任务仍需在 thread 内另发。创建/续轮前按 [测试指南](testing.md) 验证共享实例生命周期，再由 CLI 执行 `providers trust PROVIDER_ID` 登记控制资格。
8. 验证另一用户、普通/私有频道、群聊和另一个 DM 均不能控制或接收输出。消息编辑、bot 回复和未绑定 thread 不会发起任务；纯附件不启动任务，带 caption 时仅发送文字。
9. 重启后旧输入不得自动重发；恢复/过期输入需新的确认，Dispatching/OutcomeUnknown 先核对原端结果。退出 PocketPilot 后，原生任务应继续运行。

此版本不再读写旧的 `slack_channels` 项目频道映射。已有数据库中的旧表/字段不会自动删除，旧频道绑定也不会改绑到 DM；运行时只接受 Slack 返回的 owner DM，旧频道收发会被拒绝。保留数据备份，在 DM 中重新选择会话即可建立新的私聊绑定。

平台依据：[App Home / Messages](https://docs.slack.dev/surfaces/app-home/)、[message.im](https://docs.slack.dev/reference/events/message.im/)、[conversations.open](https://docs.slack.dev/reference/methods/conversations.open/)。
