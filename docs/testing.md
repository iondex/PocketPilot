# 部署与验收

本文使用通用主机与项目示例。实际 SSH alias、账号、地址、工作区、令牌及认证信息只保存在私有配置与本地状态中。

## 运行前提

- 安装仓库指定 Rust 工具链、当前支持的 Codex CLI 和系统 OpenSSH。
- 准备已运行且可共享连接的 Codex app-server。Desktop、CLI remote 模式和 PocketPilot 必须连接同一执行实例；共用历史目录不足以证明共享。
- 从根目录 [配置示例](../pocketpilot.example.toml) 创建私有 `pocketpilot.toml`，限制文件权限，填写允许的主机、项目根路径和 bot token。不使用 `.env` 或 include。
- Telegram bot 开启私聊 Topics；在 BotFather 关闭用户自行创建话题，保留 bot 创建话题的能力。已有 webhook 与 polling 冲突时由部署者选择接收方式，程序不自动修改。

共享入口和本地 Desktop 的限制见 [接入说明](design/local-desktop.md)。PocketPilot 不负责开启系统 SSH 服务或启动、升级、重启原生 daemon。

## 构建与启动

```sh
cargo build -p app --release --no-default-features --locked
./target/release/pocketpilot doctor --credentials
./target/release/pocketpilot doctor --connect --telegram
./target/release/pocketpilot serve
```

`doctor` 默认离线检查根配置和 catalog，并在 `configuration` 中报告平台配置及 Discord 编译功能；`--credentials` 额外检查已配置的凭据，Telegram 检查 token 格式，Discord 的非空 token 与非零 guild ID 由配置加载检查。离线检查不能证明 token 有效或权限足够；`--connect` 才连接 agent，`--telegram` 才探测 Telegram。Discord 暂无专项联网诊断，真实测试见 [Discord 验收](testing-discord.md)。

诊断成功只证明对应探测可用，不自动授予控制权限。新 provider 默认只读；验证同一 active turn、多客户端输入和断线后继续运行后，再用 CLI 为该实例确认控制权限。诊断输出可能包含本地部署信息，分享前应脱敏。

SSH 使用有效 OpenSSH config，支持 ProxyJump、ProxyCommand、agent 和 known_hosts。远端非交互 shell 的 stdout 应保持安静。TOTP 按验证码挑战文案匹配，可兼容通用账号上下文前缀，规则不要硬编码用户名或 IP；未知挑战不回答。配置与认证边界见 [连接与配置](design/connections-and-config.md)。

## 私聊配对

1. 在 Telegram 私聊 bot 发送 `/start`，取得短期申请码。
2. 在运行 PocketPilot 的机器上执行 `pocketpilot pair approve CODE`。
3. 再次 `/start`，进入固定管理 topic。群聊、未配对用户和错误私聊身份不得进入会话操作。
4. 管理 topic 使用按钮和命令导航，普通任务文本只得到提示；选中会话后去对应 session topic 输入任务。

配对可通过 CLI 查询和撤销。不要在公开文档或问题附件中记录真实申请码、平台用户 ID、聊天 ID 或原始配置。

## 功能验收

所有写入测试只使用专用测试会话和话题；不向已有工作会话注入探针。

| 范围 | 核对行为 |
| --- | --- |
| 管理导航 | 主机 → 工作区 → 会话；点击按钮选择，原消息编辑切页；返回、首页、刷新和翻页按钮有效，工作区按钮可并排，没有空白占位 |
| 工作区发现 | 完整读取活跃、普通和归档会话的目录元数据，遵守允许根路径；无会话的新目录可 `/add`；`/remove` 后不自动恢复 |
| 排序与分页 | 工作区按所属会话最大更新时间倒序；会话跨已登记目录汇总排序；翻页期间新活动不造成重复或跳项 |
| 创建 | `/new` 选择主机、工作区、标题并确认；只创建命名空会话，首条任务由用户另发；重名工作区要求明确定位 |
| 续轮与 steer | 空闲输入启动新轮，活跃输入带当前 turn ID 追加；冲突要求新确认，不自动换轮；不覆盖执行策略 |
| 共享状态 | Desktop、CLI 和 Telegram 观察同一 active turn；`/status` 返回实际状态，等待审批时不替用户回答审批 |
| 历史 | 进入长会话只显示短预览，不导出文件；再次打开不重播。`/history` 查看最近输入和同轮 final；`/history errors` 列近期错误；`/history export` 主动导出全部可见历史 |
| 输入来源 | 同一原生会话的外部输入以 🧑‍💻 展示；Telegram 自身输入精确去重；其他会话内容不得串入 |
| 阶段与格式 | 💭 过程、✅ final、💬 阶段未知、❌ 最终失败；可恢复重试不发消息；emoji 后空格接首行；粗体、链接、代码及表格正常显示 |
| 附件 | 标准附件包装缩为正文与文件名，不显示绝对路径或图片；Telegram 上传只转发 caption 文字并提示限制，纯附件不启动任务 |
| 标题与状态 | topic 仅保留截短原始标题，管理 icon 为 📁，空闲/未加载为 💬；状态变化更新 icon，运行中持续 typing |
| 限速 | 短时输出持久合并，同一私聊的全部 topics 共享预算；429 遵守 retry_after；普通长输出或主动历史查询可用单个 Markdown 附件，入场预览不产生文件 |
| 语言 | 中英文菜单和类型化错误按用户偏好显示；标题、路径、用户输入及 agent 正文不翻译 |
| 话题管理 | `/topics` 单项删除或批量确认；只影响已记录的 Telegram topics，保留管理入口和全部原生会话 |

## 故障与生命周期验收

- 停止、重启或断开 PocketPilot，确认原生任务继续，桥接不自动 resume、续轮、steer 或取消任务。
- 原生写入、资源创建或平台投递结果未知时，不自动重发。先核对原端状态，再由用户执行新操作。
- 重启前积压、轮询故障后补收或过期的输入转为待确认草稿；创建和删除的旧按钮只恢复确认，不执行旧操作，也不能作用于新绑定或新范围。只读菜单点击可恢复当前页面。
- 修改允许主机、认证策略和工作区范围后，尚未开始的执行和投递重新授权；非法配置保留最后有效快照。
- 配对撤销、工作区移除或话题失效后，未开始的输入/输出不得继续投递。已被原生接受的请求不撤回。
- 连接故障提示去重，恢复后重新读取状态和历史；历史暂不可读不等于原生任务已停止。
- 关闭、删除、超时和限流分别处理；不把未知话题状态当成删除。批量删除遇不确定结果停下，重启不续删。
- TOTP 断连恢复只断开测试专用连接，不停止 daemon 或影响其他客户端。

## 自动检查与验证边界

开发检查命令见 [开发约定](../CONTRIBUTING.md)。自动测试包括类型化协议、SQLite、平台 HTTP/WebSocket fixtures，以及隔离原生服务的多客户端生命周期实验。真实验证只对被明确授权的部署和测试会话成立，不推断所有版本、网络、认证组合都可用。

完整验收标准见 [验收清单](planning/acceptance.md)，证据范围见 [验证概览](research/implementation-checks.md)。真实客户端呈现、生产网络故障及未覆盖认证组合仍需部署者验证。

## 运维与反馈

使用服务管理器托管时，由部署者保存其 job/service 标识和本地运行记录。停止桥接应操作对应服务，避免守护进程重新拉起；绝不向 Codex daemon 或其进程组发信号。运行日志按根配置轮转，日志设置及账号级配置的重启要求见连接文档。

反馈只需说明操作步骤、现象和大致时间；截图、日志和诊断结果先移除账号、地址、主机别名、工作区名称、消息正文和认证信息。不要附带完整私有配置。

菜单编辑验收：连续选择主机、工作区和会话时应编辑同一菜单消息；已删除菜单的明确编辑失败可重新发送，未知编辑结果不补发。/start 始终提供新入口，便于清空本地聊天记录后恢复可见菜单。快速点击旧按钮不得使过时页面覆盖当前页面或重复创建/删除。

Slack 自建 app、manifest、token 与 owner 私聊验收见 [Slack 安装与测试](slack-setup.md)。`doctor --credentials` 检查 xapp/xoxb 前缀；真实 token、权限和 Socket Mode 连接由启动与专用测试确认。
