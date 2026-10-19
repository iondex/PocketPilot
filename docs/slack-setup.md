# Slack 安装与测试

按以下步骤准备自建内部 app。PocketPilot 使用本机 Socket Mode，不需要公网地址、Request URL、ngrok 或单独的 HTTP 服务。当前开发实现按本指南验收，真实 Slack 验收完成前不视为生产接入。

## 你需要完成的步骤

已有 app、token 和 ID 时无需重建应用：在现有 app 的 **App Manifest** 中按下方文件核对并补齐配置，然后重新安装到 workspace 以应用新增 scopes；继续使用该 app 的 ID 与凭据。

1. 打开 [Slack Apps](https://api.slack.com/apps)，选择 **Create New App → From a manifest**，选择测试 workspace（team），粘贴 [slack-manifest.json](slack-manifest.json) 的完整内容并创建。
2. 在 **Basic Information → App-Level Tokens → Generate Token and Scopes** 创建 token，添加 `connections:write`，保存生成的 `xapp-…`。
3. 在 **OAuth & Permissions → Install to Workspace** 安装并批准权限，保存 **Bot User OAuth Token** `xoxb-…`。如果 workspace 要求管理员审批，由管理员批准。以后修改 scopes 后需要重新安装。
4. 在 Basic Information 复制 **App ID**（`A…`）。从 workspace 网页地址取得 team ID（`T…`）；个人资料的更多菜单中选择 **Copy member ID**，取得你自己的用户 ID（`U…`）。首版只允许该用户控制会话，不自动把所有 Slack 管理员当成操作者。
5. 创建专用的私有测试频道，在频道成员/Integrations 中添加 PocketPilot bot，复制 channel ID（`C…`）。不要使用 Slack Connect 共享频道。每个项目使用一个专用频道，所有 session thread 继承该频道的成员可见性。
6. 将两个 token 和上述 ID 填入 Git 忽略的根 `pocketpilot.toml` 的 `[slack]`，字段见 [唯一配置示例](../pocketpilot.example.toml)。不要把真实 token 发到聊天、issue 或命令行参数中，也不使用 `.env`。
7. 准备允许路径内的专用测试项目和可共享的 Codex 执行实例。先做只读检查；确认共享实例及其生命周期后，再用 `providers trust PROVIDER_ID` 授予该实例控制能力。

Manifest 已开启 Socket Mode、Interactivity、`/pp`、公开/私有频道消息事件及对应权限。保持 Socket Mode 开启；无需填写 Event Subscriptions 或 Interactivity Request URL。App-level token 需单独创建，manifest 不包含秘密。

## 构建与启动

```sh
cargo build --release -p app --features slack --locked
./target/release/pocketpilot doctor --credentials
./target/release/pocketpilot doctor --connect
./target/release/pocketpilot serve
```

配置多平台时使用同一个启用了对应 features 的 `serve` 进程，例如 `--features discord,slack`。不需要 Telegram 的部署应移除示例中的 `[telegram]` 占位配置。平台账号、token、用户/频道范围及 data_dir 改动需要重启桥接。

## 在 Slack 中验收

1. 在专用频道主输入框执行 `/pp`，选择项目与已有会话；确认生成根卡片，打开没有发起原生 turn 或 resume。
2. 再次打开同一会话，确认复用原 thread；用根卡片查看状态、最近交互与历史。
3. 使用明确的新建流程创建空会话，然后在其 thread 内发送“只回复 SLACK_SMOKE_OK，不运行工具，不修改文件”。核对原端只收到一次输入。
4. 在仍 active 的专用会话中补充文字，核对同一 turn 的 steer；消息编辑、bot 回复、未绑定 thread 和频道主消息不得变为 agent 输入。
5. 验证长输出作为单个 Markdown 文件、中文和代码完整、输出中的 `<!channel>` 等不会触发提及。
6. 正常退出 PocketPilot，原生任务应继续；重启后不得重发旧输入。恢复或过期输入需要新确认，结果未知时先在原端核对。
7. 解除绑定后原生会话继续运行；其他用户即使能看到频道，也不能控制该会话。

自定义 slash command 不能在线程内调用；`/pp` 只用于频道入口，会话内通过普通回复和卡片按钮操作。不要给 bot 输入真实工作任务，直到专用测试流程通过。

## 准备完成的检查单

- App 已安装，Socket Mode 已开启，Interactivity 已启用。
- `xapp` 拥有 `connections:write`，`xoxb` 来自同一个 app 的目标 team 安装。
- App/team/user/channel ID 已填写，bot 已加入目标非共享频道。
- 专用项目在允许根路径内；已登记 provider/workspace。
- token 只保存在私有根 TOML；可以只告知“配置已填好”继续联调，无需发送 token。
