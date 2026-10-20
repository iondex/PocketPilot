# Discord 真实服务器验收

适用于已经创建独立服务器和 bot 的部署。当前实现是 Gateway + guild slash commands；本地 HTTP/WebSocket fixture 不能证明真实权限、客户端呈现或附件传递可用。命令语义见 [Discord 原型](design/discord.md)。

## 安装与准备

1. 在 Discord Developer Portal 为应用使用 Guild Install，授权 `bot` 与 `applications.commands`。在 Bot → Authorization Flow 关闭 Requires OAuth2 Code Grant；PocketPilot 不提供 OAuth 回调服务器。若授权提示 `integration requires code grant`，关闭该开关并保存后再安装。若 bot 已入群但命令授权缺失，重新授权安装。
2. bot 需要 View Channels、Manage Channels、Send Messages、Read Message History、Attach Files。可以在独立测试服务器先授予 Administrator 排查，但最终应以这些权限验收。新建的 PP category 通过覆盖权限对 `@everyone` 隐藏；服务器管理员仍能看到。
3. 可使用权限值 `101392`（View Channels、Manage Channels、Send Messages、Read Message History、Attach Files）生成安装链接，安装到目标测试服务器。本机开启开发者模式，从服务器菜单复制 Server ID，填入私有根配置的 `[discord].guild_id`；`token` 使用 Bot Token，不是 Application ID、Public Key 或 client secret。字段示例只维护在 [公开配置](../pocketpilot.example.toml)。
4. **使用服务器 owner 账号测试。** 程序每次从 Discord 读取 owner 身份；其他 Administrator 也会被拒绝。不需要 Telegram 配对。
5. 本实现只使用 `GUILDS` intent，不开启 Message Content / Server Members / Presence 特权 intents。应用的 Interactions Endpoint URL 应留空，避免把交互切换到 HTTP 回调；也不需要 ngrok。Gateway 与 HTTP 接收交互互斥，见 [官方交互文档](https://docs.discord.com/developers/interactions/receiving-and-responding#receiving-an-interaction)。
6. 选择专用测试项目和会话；agent 必须是已经运行、可共享连接的执行实例。不要在日常工作会话中测试写入或断连。

## 构建、离线检查和启动

```sh
make check
make package
cargo clean
./dist/pocketpilot doctor --credentials --discord
./dist/pocketpilot doctor --connect
./dist/pocketpilot providers list
./dist/pocketpilot workspaces list
./dist/pocketpilot serve
```

`doctor` 默认不联网；`--credentials` 不是 Discord token 有效性探测。查看 `configuration.discord_configured` 和 `discord_runtime_enabled`；`--connect` 检查 Codex 入口。`doctor --discord` 经配置代理只读查询 bot、application、guild 和 owner，输出安装链接；不注册命令、不连接 Gateway。Discord 在 `serve` 启动时再注册`/pp` 与十个 `pp-*` guild commands 并连接 Gateway。

`Discord slash-command prototype ready` 表示 REST 初始化、命令注册及客户端构造已完成；它在 `client.start()` 之前输出，**不代表已收到 Gateway Ready**。`Discord Gateway ready` 才表示收到实际 Ready 事件；最终仍以服务器 owner 执行 `/pp` 并点击菜单获得回执为交互接通证据。

只配置 Discord 时移除未使用的 `[telegram]`，避免占位 token 导致另一 integration 启动失败。与 Telegram 共用时，使用同一个 `serve` 进程；同一 data_dir 不能另起第二个服务。

工作区为空时，先检查主机允许路径与执行实例发现；可使用 `workspaces add --provider PROVIDER_ID --cwd /srv/code/pilot-test --name 测试项目` 明确登记测试目录。provider/workspace 保存在 catalog，不在 TOML 新增旧式数组。

## 第一轮：工作空间 → 项目 → session（只读与映射）

在测试服务器的普通文字频道输入 `/pp`，后续通过下拉框和按钮完成：

1. 选择工作空间（执行环境/主机），例如本机或云端。
2. 选择该环境中的项目目录。项目每页最多 20 项，使用“下一页 / 上一页”，不复制 UUID。菜单只显示面包屑、当前范围和下拉框；目录过长时看末尾即可。
3. 选择会话。会话每页 8 项，标题为主，emoji、状态文字与更新时间帮助区分；分页 cursor 不出现在回复里。
4. 点击“进入会话”跳转到对应 session channel。也可“返回会话列表”，继续在原菜单选择。打开不 resume、不启动 turn。
5. 核对工作空间 → 项目 → session 归属。再次打开相同会话复用原 channel，重复点击已消费的打开控件提示重新打开菜单，不重复创建资源。

`/pp-workspaces` 与 `/pp-open` 也可作为菜单入口。`/pp-projects workspace:环境名称`、`/pp-sessions workspace:环境名称 project:项目短名` 保留为直达入口；参数不完整时先补齐上层选择，不要求手填 thread ID 或 cursor。所有菜单仅调用者可见，翻页、返回和刷新更新原消息，不在频道追加长列表。

在 session channel 执行 `/pp-status`、`/pp-recent`，或用 `/pp-rename title:测试名称` 修改频道显示名。状态与回执显示标题、项目、状态文字，使用 emoji 辅助，不出现原始 thread/turn ID。核对原端没有因浏览、打开或查询新增 turn 或自动 resume。

入场与 `/pp-recent` 只显示最近用户输入和对应 final。渲染后超限的结果分段发送为普通消息，不生成 `session.md`，不支持完整历史导出。尚未载入的会话只有明确发送输入时才会 resume。

## 第二轮：创建、续轮和 steer

先按 [共享执行验证](design/local-desktop.md#部署验证) 确认 provider 对应同一执行实例，停止桥接后用 `providers verify PROVIDER_ID --project PROJECT_ID` 验证专用会话并记录控制确认，再恢复桥接。`doctor --connect` 不会自动授予控制；已有通过验证的 provider 无需再次 trust。

```text
/pp-new workspace:执行环境名称 project:测试项目短名 text:只回复 DISCORD_SMOKE_OK，不运行工具，不修改文件 confirm:true
```

**Discord 的 `text` 是首轮任务，不是标题。** 此命令会创建原生会话、立即发送首轮，再建立 channel；与 Telegram 的 `/new` 不同。核对只创建一个 session、一轮输入和一个 channel。若收到未知结果提示，先在原端核对，不能反复执行 `/pp-new`。

随后在这个 session channel 中执行：

```text
/pp-ask text:只回复 DISCORD_SECOND_OK
```

确认 idle 时产生新 turn；直接在 channel 发送普通文字应不启动任务。active steer 则选一个仍在运行的专用测试会话，在其 channel 用 `/pp-ask` 补充要求，核对原端仍为同一 turn；若期间结束并发生竞争，应提示重新确认，不能自动把过时 steer 改成另一轮。

NotLoaded 会话仅在明确 `/pp-ask` 时 resume。等待审批/用户输入的状态不得被展示为 idle，桥接也不能自动代答审批。

## 第三轮：展示、权限和恢复

| 测试 | 通过条件 |
| --- | --- |
| Markdown | 短合成样本包含标题、粗斜体、引用、列表、代码和链接；表格显示为列名与值，任务显示勾选框；HTML 与剧透标记保持字面含义 |
| 动态标题 | 项目/session 名包含 `* _ ~ < > &` 与 emoji；菜单和状态卡保持文字，不产生额外格式 |
| 菜单与移动端 | 下拉框选择执行环境 → 项目 → session，返回/分页刷新同一条消息；超长标题和 emoji 不超出 native option 限制，不显示 UUID 或 cursor |
| 旧菜单 | 配置或范围变化后旧控件拒绝；过期时提示 /pp；读取失败保留上次菜单并给调用者错误提示 |
| 长输出 | 专用测试会话产生超过 1,800 个 UTF-16 单元的合成文本，收到多条普通消息；中文、代码块内容完整，不出现自动生成的历史文件 |
| 文件上传 | 在已绑定 channel 执行 `/pp-ask file:附件 text:查看此文件`；也可只填 file。Codex 收到执行主机上的文件路径，图片使用原生图片输入；无历史导出命令 |
| mentions | 合成输出包含 `@everyone`、`<@123>` 和 `<@&123>`，不触发用户、角色或全体通知 |
| 非 owner | 普通成员看不到 PP category；非 owner 管理员即便能看见或调用命令，也不能操作会话 |
| 错误上下文 | 私信、其他 guild、未绑定 channel 不得把输入发送到原会话 |
| 桥接退出 | 测试任务运行期间正常退出 PocketPilot，原生任务继续；重启后观察恢复，不补发旧输入 |
| 跨平台 | Telegram 与 Discord 绑定同一测试 thread，用同一进程操作，核对共享状态和串行写入，无串会话 |
| 解绑 | `/pp-unbind` 后不再转发；原生任务仍继续；再次显式打开同一会话复用原 channel |
| 范围撤销 | 移除测试 workspace 或撤销 provider 控制确认后，后续写入拒绝；已接受的任务不被终止 |

观察 worker 每轮至少间隔五秒，guild 文本 outbox 每四秒最多发送一条；API 限速可能进一步延后。状态卡是 channel 内普通状态消息，代码没有 pin 操作，不要求出现在 Discord 的 Pins 列表。

丢写回执、进程退出和平台发送未知结果不得导致自动重发。不要在第一轮验收手工删除 category/channel：原型还没有资源核对 CLI 或删除后自动替换。资源创建进入 unknown 时，先检查 Discord 和本地账本，不能删除账本来强行重建。

## 常见卡点

| 现象 | 优先检查 |
| --- | --- |
| 完全没有 `/pp` 或 `pp-*` 命令 | guild ID、Guild Install、`applications.commands`、初始化报错；命令是 guild 级，不是等待全局传播；更新后可重开 Discord 客户端 |
| ready 日志出现，但命令显示应用未响应 | Gateway 网络、Interactions Endpoint 是否非空、进程是否退出；不能只看初始化日志 |
| 只能看到 workspace，无法 ask/new | provider 是否已完成控制确认；命令是否在正确 session channel；是否是 owner |
| 命令有回执但 channel/附件失败 | bot 的 Manage Channels / View Channels / Send Messages / Attach Files，以及 category 覆盖权限 |
| workspaces 列表为空 | catalog 是否已发现实例、cwd 是否位于允许根路径、是否登记测试项目 |
| 资源创建或输入提示结果未知 | 先核对原会话/Discord，不自动重试写入或资源创建 |

私有 `.env.discord` 可作为人工交付凭据的临时文件；PocketPilot 运行时不读取它。实际配置始终是根 TOML 的 `[discord]`，包含 `token`、`guild_id` 和可选 `proxy`。如需代理，可填 `http://192.0.2.10:7890`；空值直连。

## 本地回归

```sh
cargo test --release -p integrations discord:: --locked
cargo test --release -p integrations telegram::runtime::tests::discord --locked
```

第二条命令覆盖 Telegram/Discord 共用连接、操作锁、回执以及 Discord mock HTTP 流程。真实 Gateway、账号权限与客户端体验仍由上述服务器流程验收。
