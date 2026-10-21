# 上传文件给 Codex

文件附件是用户输入的一部分。三端均不提供历史导出，长回复直接分段显示，不自动生成 `session.md`。

| 平台 | 操作 |
| --- | --- |
| Telegram | 先打开并绑定 session，在该话题发送文件或图片，可带说明文字 |
| Slack | 先选 workspace → 项目 → session，在对应根卡片的 thread 内上传文件，可带文字；主输入框只打开菜单 |
| Discord | 在已绑定 session channel 执行 `/pp-ask`，填写必选 `text`，可附加 `file`；普通 channel 消息不作为任务 |

纯文件输入使用“请查看上传的附件”作为任务。每个文件最多 20 MiB，公共输入最多四个文件；Slack 可在一次消息上传多个，Telegram 每个文件消息独立处理，Discord 当前表单提供一个 file 选项。不读取链接指向的任意外站资源，不解压或执行上传文件。

下载发生在操作者、绑定和控制资格校验之后，使用对应 integration 配置的代理。Telegram 使用私有 file ID，Slack 需要 bot scope `files:read`，Discord 只读取交互解析出的官方 CDN 附件 URL。文件引用保存在私有输入队列，日志不记录下载 URL、token 或文件内容。超限和下载失败会明确报错，不静默只发送说明文字。

文件写入所选 Codex 执行账号的 `.codex/pocketpilot/uploads/`，每次输入使用独立目录，文件名去除路径和控制字符。local_proxy / ssh_proxy 使用配置的现有 Codex control socket 确定该账号的私有目录，不把 AWS 文件路径当成本机路径。没有可确认私有目录的自定义 WebSocket 实例会拒绝附件，需要先配置受测的共享 control socket 入口。

每个文件写入都有持久操作记录；确认返回后，原生输入才携带文件路径。PNG/JPEG/WebP/GIF 图片同时使用 Codex LocalImage 输入；其他文件通过带文件路径的上下文提交，原生执行策略照常决定如何读取。文件中的指令与用户请求分开标注。附件上下文的展示只保留文件名，不自动把私有绝对路径带回聊天记录。

重启或延迟接收的输入仍需新的确认，确认保留附件引用；若文件链接已失效，重新上传。已经进入结果未知的原生写入不会自动重发。上传文件可能被当前或后续 turn 使用，程序不会在投递后立即删除；清理前应确认原生会话不再引用它们。
