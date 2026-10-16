# Telegram 测试入口

代码提供 `pocketpilot serve`：私聊配对、工作区/会话菜单、topic 绑定、完整历史集合、创建/续轮/steer、topic emoji、持久消息合并和配置热更新。当前受测 Codex CLI 为 0.162.1；协议在 build.rs 中动态生成。

## 先准备共享服务

PocketPilot 连接已经运行的 Codex app-server；启动、退出、重连不会启动或重启 agent 服务。Desktop、CLI 和 PocketPilot 必须连接同一个执行实例。共用 ~/.codex 的历史文件不能证明执行实例相同。

已按授权配置 anjie-bsc-arb 的既有共享 daemon 和九个工作区；SSH 控制已通过专用会话验收并开放。直接使用 /workspaces 选择。本轮同时登记 anjie-solana-ct 和 aws-work。SSH binary/socket 自动探测；新实例默认只读。

本机当前 Desktop 以 stdio 运行，没有可附加的控制 socket/公开 listener；现有本地活跃任务尚不能接入。没有重启 Desktop，也没有用一个共用历史文件的新服务冒充它。支持受测的本地共享入口，仍需本机部署提供这样的入口。

公开字段见根目录 [pocketpilot.example.toml](../pocketpilot.example.toml)。实际配置全部放根目录 pocketpilot.toml，不 include、不读取 .env；Telegram token、密码和 TOTP seed 都内联，真实配置已被 Git 忽略。cwd 使用 daemon 返回的规范绝对路径；同一个 workspace 的显式 worktree 可加入 additional_cwds。

首次只读检查：

```sh
cargo build --workspace --locked
./target/debug/pocketpilot config-check --credentials
./target/debug/pocketpilot doctor --connect --telegram
./target/debug/pocketpilot providers list
./target/debug/pocketpilot workspaces list
```

本机和 SSH 的 Unix proxy 都在字节通道上执行 WebSocket 握手。SSH 读取原有 OpenSSH config，保留 Include、Match、ProxyJump、密钥/agent 和 known_hosts 规则。远端非交互 shell 必须保持 stdout 安静。密码、key passphrase 和 TOTP 通过 OpenSSH askpass responder 读取同一根 TOML；OTP 提示必须匹配明确的 prompt_patterns，未知挑战拒绝回答。

在独立测试会话中确认三端看到同一 active turn、桥接断开后任务继续，再把对应 provider 的 control_verified 设为 true。这个字段是部署确认；只读 doctor 不会自动授予控制资格。未开启时列表、状态、历史照常可用，写入明确拒绝。

## 启动与配对

```sh
./target/debug/pocketpilot serve
```

在 Telegram 私聊 bot 发送 /start，取得一次性码后，在本机另一个终端运行：

```sh
./target/debug/pocketpilot pair approve CODE
```

批准后再次 /start，进入固定的「管理」topic。这里的普通消息只提示，不发送给 agent；选择已有会话后去对应 session topic 发送任务。管理员 CLI 可与服务同时运行，不获取执行账本的服务锁、不恢复执行中操作。其他人必须完成同样的本机批准。群聊输入、群内按钮和错误 private peer 均不能进入业务流程。

已配置 bot 的只读账号检查已通过，私聊 Topics 已开启；初始检查未发送测试消息或消费 updates，正式 serve 启动后才处理私聊请求。其他 bot 需要先在 BotFather 开启私聊 Topics。发现 webhook 时 serve 拒绝启动，不自动删除 webhook。

## 建议测试顺序

1. 管理 topic 内 /workspaces 选择工作区，回复序号依次选择主机、工作区和会话；回复 0 返回，/refresh 刷新，每次输入发送新菜单，无内联按钮、空白占位或开发探针条目。新建且尚未进入历史索引的已加载会话也应可见。
2. 进入空闲会话，确认只显示对应 agent final；进入运行中会话，确认全部可读 agent 正文在一个集合内，长内容为单个附件。用户输入不回显，无“用户/Agent”标签或空历史占位。工具和推理内容不发送。
3. /status 和 /info 对比原端状态；topic 名只保留原始标题，状态变化更新原生 emoji icon。📁 表示未在该执行实例加载，不是历史读取失败；直接发送文本，空闲时续轮，运行中使用带 expectedTurnId 的 steer。等待审批时也可追加 steer，审批和结构化问答继续在原端处理。
4. /new WORKSPACE_ID 任务，回复 1 确认后才创建 thread 和首轮；没有停止、取消、重启或审批代答命令。
5. 连续产生几条可见文本，检查合并与私聊共享速率预算；多个 topic 不应各自突破预算。429 按 retry_after 等待。
6. 修改根 TOML 增删允许的 host/workspace 或调节速率；有效配置自动生效，非法配置保留最后有效版本。已撤销配对、已解除映射或已移除 workspace 的排队动作/投递不得继续执行。
7. 停掉并重启 PocketPilot，确认原端任务继续运行，桥接不自动 resume/start/steer，也不重发结果未知的请求。重启前的 pending 消息及轮询故障后补收的输入应显示草稿，点击新的确认按钮前不得进入 agent；解绑后旧草稿按钮失效。断网时状态卡应显示连接未知。
8. /rename 只改平台显示名；/unbind 只解除映射。关闭/删除 topic 后再次添加应创建新容器，原容器不能被换绑到另一个 session。

正常退出使用 Ctrl-C 或 SIGTERM。data_dir、Telegram/Discord token 和 Discord guild 修改需要重启桥接；host/workspace、SSH 认证策略和 delivery 设置支持热更新。日志输出位置/过滤器在启动时装配。

## 已验证与仍需你验证

自动测试包含本地 Bot API + WebSocket fixtures 的配对/topic/历史/创建/续轮/steer 流程，以及丢回执不重复写入、快照/权限撤销、持久批次恢复。另用真实 Codex 0.162.1、独立临时 CODEX_HOME 和本机模拟模型，验证了 TCP WebSocket、Unix proxy 的多客户端 active turn、steer、最后连接退出后继续完成和最终历史读取；没有接触原有用户任务或调用付费模型。

按用户授权，另在远端共享 daemon 上创建专用真实模型探针：active_observed、steer_accepted、断线后仍 active、completed、final_marker_found、resume_accepted 全部通过；Desktop 的只读 API 也观察到相同 active turn。原有用户任务未收到测试输入。开发期间创建的专用探针会话保留供核对。

当前仍需你验证真实 Telegram 私聊 topic 收发和实际操作体验；本地既有 Desktop 接入、真实 OTP/慢服务器故障组合尚未验收，不能宣称 52 项场景全部通过。

未知写入/消息发送不会自动重试；先核对 /status、/history 和原客户端，再决定是否重新发送。历史超过单个附件上限会给出明确提示，不用截断文件冒充完整历史。

## Discord 原型

可选编译和同进程运行，详见 [Discord 原型](design/discord.md)。仅配 Telegram 时无需启用此 feature。Discord 尚未提供真实 token/guild，当前是 mock API/原生协议 fixture 验证，不宣称已连上真实服务器。

## 当前本机桥接

BotFather 设置已复核：user_topic_creation_enabled=false。按用户准备测试的指示，修复版桥接已重新启动；配对保留，真实管理 topic 与新 session topic 已建立，菜单、历史和输入回执已有成功投递记录。当前手动测试由 macOS launchd 的临时 job local.pocketpilot.manual-test 独立托管，不依赖本次工具进程，也不是登录启动项。进程号记录在忽略的 var/data/bridge.pid，实际存活以 launchctl list local.pocketpilot.manual-test 为准。停止此托管桥接使用 launchctl remove local.pocketpilot.manual-test；仅对 PID 发 SIGTERM 可能被重新拉起。不要对 Codex daemon 或其进程组发信号。

## 本轮最小回归流程

先测试已有工作正常的 SSH anjie-bsc-arb，本地 Desktop 的接入条件单独见 [本地会话说明](design/local-desktop.md)。

1. 在 Off-topic 发送 /start，每条新命令均应在原位置收到前往「管理」的简短提示，再切换到该 topic 查看编号菜单。再次 /start 会在健康的同一管理 topic 新发首页，不创建 Codex session；后续编号也发送新菜单。此前已经产生的 /start topic 不会自动删除。
2. 在管理 topic 发“测试消息归属”，确认回复明确提示未发送给 agent。未绑定入口的普通消息也应进入管理提示，不能发给最近使用过的会话。
3. /start 显示主机编号。回复序号选择 anjie-bsc-arb，再选 workberth 和会话；0 返回，/refresh 刷新。确认 Markdown 正常渲染、没有按钮或空白占位，探针不显示。
4. 添加之前报错的旧会话，仅查看历史和 /status。应无 provider protocol is incompatible，历史是一个集合/附件，没有自动状态气泡。💬 表示空闲或尚未加载，不等于历史不可读。此步骤不向旧会话发送任务。
5. 在管理 topic 使用 /new workberth 手动测试，回复 1 确认。应创建空会话与独立 session topic，标题不自动作为任务发送，并能在 Desktop 的同一 SSH 主机下看到该会话。后续写入测试只用这个专用新会话。
6. 在 session topic 直接发消息，确认只进入该 session。需要验证 steer 时，让这个测试会话输出足够长的无工具文本，在它仍运行时追加一句；应追加到原 turn，不能被中断或重建。空闲时输入则开始下一轮。
7. /status 查看状态，等待任务完成。topic 名应稳定，状态只更新 topic 原生 emoji icon，不发自动状态卡；成功输入不应收到“开始新一轮/已追加”通知或用户消息回显；历史读取错误若发生也不应在正常/连接未知标题之间循环。
8. 可选 /unbind：仅解除测试 topic 映射，原生会话继续存在。验证退出/重启时仅操作 PocketPilot；不结束 Desktop 或远端 daemon。

遇到异常时记录所在 topic、操作命令和大致时间即可；不要重复发送结果未知的任务。

## /start 不再自动产生 Telegram topic 的设置

在 BotFather 为此 bot 开启 **Disallow users to create new threads**，保留 Topics 模式。该设置关闭用户建 topic 的能力，bot 仍可创建管理与 session topics。设置开启前，Telegram 客户端可能在 bot 收到 /start 前就自动创建同名 topic，代码无法撤销这一步；不会自动删除这些已有聊天。doctor --telegram 会显示 user_topic_creation_enabled 供核对，应为 false。

## 编号导航补充测试

- 在管理 topic 发普通任务文本：只收到拒绝提示，原文不回显，也不进入 Codex。
- 回复旧菜单或旧按钮：打开/刷新编号菜单，不以长串“按钮已过期”报错打断操作。
- 修改显示名/速率后继续回复序号，菜单仍可用；修改目标 cwd/实例后，旧创建确认不得执行。
- 菜单每次按新输入新发；未知发送结果不自动重试。
- 会话内数字仍是正常 agent 输入，只有管理 topic 将数字解释为导航。
- 无历史 agent 回复时不发占位；/history 可显式查询 agent 历史，CLI 可导出完整角色记录。

## 动态工作区与原型数据库

配置中不再填写 provider/workspace 或 Codex bin/socket。允许的主机、SSH alias、凭据、项目根路径仍在根 TOML。新工作区可在管理 topic 选中主机后用 /add /绝对路径 名称 添加，/remove 名称 只移除应用入口。普通任务文本仍会被管理界面拒绝。

数据库直接使用根 schema.sql；原型阶段不运行 migrations，不清空既有配对/绑定/投递。创建空目录和重开已有状态均有回归覆盖；现有一份 provider 与九个工作区已保留原 ID 导入 catalog。

日常运维保留 serve/config-check/doctor/hosts/pair/providers/workspaces；删除测试专用 probe、sessions、state-init 和重复 telegram-check 命令。共享生命周期测试继续由隔离原生 fixture 覆盖，真实远端测试不写既有用户会话。


## Off-topic 与清空记录回归

1. Off-topic 发 /start 后，管理 topic 应出现新的可见首页；已有旧菜单即使可编辑也不再只编辑它。
2. 在 Off-topic 回复 1，确认使用管理导航选择主机；普通文本只被管理入口拒绝，不能送到任一 agent。
3. 仅清空自己的历史后再次 /start，确认新菜单可见。
4. 明确删除管理话题后 /start，确认只在 Telegram 明确返回原话题不存在后恢复入口。网络错误不应生成重复话题。
5. 服务端可以路由操作，不能替用户强制切换客户端页面；若新菜单仍不可见，需区分 topic 是否存在与客户端列表显示问题。


## 新建空会话、状态 icon、Markdown 与语言回归

- /new WORKSPACE 标题 → 回复 1，仅创建并命名空会话。进入新 topic 不应看到自动任务或空闲状态卡；发送第一条消息后才开始 turn，Desktop 应能看到已补齐的标题。
- topic 仅显示截短原始标题。运行时 icon 为 ⚡️，等待审批 👀、等待输入 ❓，空闲和未加载均为 💬，管理话题独立为 📁；正文不额外发送状态消息。/status 给出主机/工作区/cwd/完整身份/turn。
- 运行中持续出现 typing；停止生成、解绑或离线后停止续期。检查不会为了 typing 发 turn/start/resume/interrupt。
- 每次菜单输入得到新菜单，旧菜单引用不会触发新写入。Off-topic 数字仍进入管理上下文。
- 发送会产生粗体、链接、代码、表格的任务，只在专用测试会话验证格式。表格应为逐项内容；长输出仍是一份原始 Markdown 附件。
- /language en 后 UI 为英文；/language zh-CN 恢复中文。另一个用户偏好独立，标题和 agent 正文不被翻译。


## 话题管理回归

- /start 首页选择「🗂 话题管理」或 /topics，确认只列出本人的已记录话题，/unbind 后的话题仍可管理。
- 仅在专用测试 topic 验证删除：选择序号后先出现聊天记录删除确认，回复 0 不删除；新回复 1 才删除。Codex 原生会话和活跃 turn 继续保留，可从会话列表重新添加。
- 从 Telegram 客户端删除专用测试 topic，再 /topics 或 /refresh，确认失效入口被清理；仅关闭、API 超时或限流不应当成删除。检查请求不覆盖用户改过的名称和图标。
- 模拟删除回执丢失，确认本地转发暂停且重启不重发；旧菜单/离线积压的数字确认要求重新确认。无法删除固定管理 topic，也不能删除另一个私聊的 topic。


## 命令菜单与清除全部

- Telegram 命令建议应有中英文说明，仅作用于私聊；/start 配对权限仍照常核验。/topics 管理话题，/cleartopics 只显示清除确认，不立即删除。
- 用两个专用测试 topic 验证批量清理。确认页显示数量且明确删除全部 Telegram 聊天记录、保留管理入口及 Codex；回复 0 返回，回复 1 才逐项删除。
- 确认页出现后新建第三个测试 topic，再确认旧清理页，第三个必须保留。回执未知/429 时停止本批次，重启不续删，未开始的其他 topic 保留。
- 普通 session 未加载与空闲的原生图标均为 💬；等待/运行/错误仍独立表达，具体加载状态通过 /status 查看。
