# 实现与验证记录

日期：2026-10-16。Telegram MVP 已具备可运行功能链路，SSH 共享实例控制已通过专用会话验收；真实 Telegram 交互等待用户测试。Discord 是可选、可运行的 slash-command 原型。

## 实现边界

- 平铺源码：ssh、session、codex、engine、telegram、discord、app。普通设置、允许的主机及所有秘密保留在根 TOML；provider/workspace 由 SQLite catalog 动态管理。
- build.rs 动态导出官方 schema 到 OUT_DIR，typify 生成 DTO，无提交快照。业务 RPC 使用生成类型，RawValue 仅在 framing 边界保存不透明 payload。
- 固定只读方法与四类写入；后台无 resume/停止/审批代答。当前官方 legacy 与 paginated 历史模式均按类型路由。
- 本地/SSH proxy 的字节隧道执行 WebSocket 握手；loaded/list 覆盖尚未进入历史索引的活跃任务，精确 cwd/worktree 分页。
- Telegram 持久 inbox/offset、CLI 私聊配对、菜单/topic、完整历史集合、状态卡、操作回执、持久合并与限速 outbox。
- 系统 OpenSSH config、keepalive/backoff、受限 askpass/TOTP；单根配置快照热更新。SQLite 服务锁、账号隔离、容器墓碑和 unknown 不重放。
- Discord 复用共享连接、会话锁、操作账本；独立 category/channel provenance 与输出账本。原型范围见 [说明](../design/discord.md)。

## 真实 SSH 证据

用户指定 SSH 目标后，只读确认 Desktop 和 CLI proxy 使用既有远端 daemon，运行版本 0.161.0；PocketPilot 使用同一共享 socket。没有升级、重启或停止该 daemon。

专用新会话探针验证：创建与首轮均收到回执；另一个连接读取同一 active turn；steer 收到对应 turn 回执；桥接全部客户端退出后重新只读连接仍 active；原任务完成并在最终文本中保留固定标记；随后仅对该测试会话显式 resume 成功。Desktop 的只读 API 同时观察到相同 active turn。现有用户任务只读查看，未收到探针输入。

调试发现原生服务接受输入后短暂暴露空 rollout，thread/read 返回 -32603。修复在 framing 层识别这个已知只读错误，最多五秒重读；不会重试任何写入。日志只记录方法、错误码和固定诊断分类，不记录原生错误文本、输入或 token。

已授权配置中的该 SSH provider 已开启 control_verified；九个工作区已保留原 ID 存入应用 catalog，公开示例不包含个人 endpoint/凭据。测试产生的专用会话留在原生服务供核对，未主动归档或停止。

## 自动与隔离验证

全功能测试共 82 项通过。完整命令为 cargo fmt --all -- --check、cargo clippy --workspace --all-targets --all-features --locked -- -D warnings、cargo test --workspace --all-features --locked。

行为测试覆盖私聊配对/topic/历史/创建/续轮/steer、重复与丢回执、按 peer 顺序 dispatch、撤销授权/配置与绑定校验、批次重启/固定截止、bot 私聊账号隔离；Discord mock 覆盖 owner 授权、category/channel、共享 provider 与重复/未知写不重发、解绑不结束任务。构建与测试没有真实聊天平台写入。

本机真实 Codex 0.162.1 和 0.161.0 均使用独立临时 CODEX_HOME、模拟模型运行 TCP WS/Unix proxy 生命周期实验：多客户端 active、steer、最后客户端断开后仍运行、最终历史均通过。这些 fixture 不读取用户凭据，也不调用付费模型。

Telegram getMe/getWebhookInfo 已通过：私聊 Topics 已开启，无 webhook；不会自行删除 webhook。正式服务启动后才消费 updates 和处理用户请求。真实交互证据将在用户测试后补充。

## 未验收事项

本机现有 Desktop 0.162.0-alpha.17.2 的服务采用 stdio，缺少公开共享入口；独立 native fixture 不能证明既有本地任务可接入。未重启 Desktop。真实 TOTP、ProxyJump、ProxyCommand 与专用连接断连恢复已通过；慢服务器长时故障、新 UX 的真实 Telegram 交互和 Discord 真实 guild 仍需后续验证。

不把 mock 或单台 SSH 探针记作 A01–A52 全部通过。依赖仍有 proc-macro-error2 的上游 future-incompat 提示，项目 lint 与测试检查独立记录。

## 正式轮询启动检查

实际启动发现 teloxide 默认 HTTP timeout 为 17 秒，短于 30 秒 getUpdates 长轮询。现使用明确配置的 50 秒 HTTP/总请求预算，并加入 18 秒延迟响应行为测试。桥接进程已独立运行，正式轮询健康以自有 polling_cursors.last_poll_at 推进核对，不额外启动第二个 getUpdates 消费者。

GitHub Actions 在任何构建步骤开始前被平台拒绝；官方 annotation 为账户近期付款失败或 spending limit 需提高。此状态不是编译/测试失败，需仓库账户管理员处理 Billing & plans 后重新运行。当前验证结果来自本机完整检查。

## 恢复与轮次竞争补充

补充测试证明：重启时仍 pending 的消息保存原始正文/时间并标为待确认；旧 nonce 作废；本次进程启动前的消息与 poll 故障后补收消息不会直接发给 agent。草稿按钮绑定 instance/thread/映射 revision，解绑后旧确认不能执行。

原生 turn/steer 明确返回“无 active turn/expected turn 不匹配”时归类 Conflict，保留未接受的文本为新确认草稿，不自动转 turn/start。结果未知仍不重发，也不会当作明确冲突重试。完整 A01–A52 当前证明范围见 [逐项审计](acceptance-audit.md)。

## 真实用户测试后的修复

用户报告 /start 自动出现新 Telegram topic、工作区按钮过长、探针会话混入列表、历史未加载文案、协议错误及标题循环。桥接已按用户请求停止，未自动重启；现有 Codex daemon 和会话未收到停止信号。

复现：元数据成功而历史失败时，原有 observer 把同一 topic 在正常标题和“连接未知”之间切换。回归测试在修复前捕获三次重复改名。实际旧会话读取失败的字段是 TurnError.codexErrorInfo：typify 把开放 anyOf 生成为 flatten struct，无法解码官方字符串错误。类型生成修复后同一真实旧会话只读历史查询成功；没有 resume 或发送输入，临时诊断正文不提交。

修复：历史失败不覆盖独立观察的运行状态，历史错误提示按绑定版本去重；状态移入固定卡，topic 稳定短名按回执缓存；历史 NotLoaded 文案改为“待继续”。工作区两列按钮，选择/分页/返回在原消息中编辑，只渲染实际条目；专用探针由本地 create 回执过滤，不按内容猜测，不归档原生 thread。固定管理 topic 负责默认入口、菜单、新建确认和未绑定输入提示，数据库禁止它关联 agent。

Telegram 官方文档证实：允许用户建 topic 时，客户端会在发送 topic-less 消息之前自动创建 topic。要消除此 /start topic，需在 BotFather 开启 Disallow users to create new threads；getMe 的 allows_users_to_create_topics 用于只读诊断，不由 PocketPilot 擅自改变 BotFather 设置或删除已有聊天。

新增模拟测试覆盖重复历史失败不反复改名、管理/session 隔离、默认输入不发 agent、原消息编辑保留按钮、探针过滤、旧队列标题/按钮恢复，以及管理 topic 创建丢回执不重试。上述新交互仍待用户显式重启后实测。

## 设置确认与第二轮用户测试

用户已开启禁止用户新建 topic，telegram-check 返回 user_topic_creation_enabled=false。按“准备测试流程”的指示已启动修复版桥接，实际管理 topic、新会话 topic、历史、菜单及输入回执已有成功投递记录；初始两次旧投递出现拒绝，不据此宣称所有旧队列都成功。新管理/topic 交互仍按测试流程核对。

本地 Desktop 再次只读确认仍为 stdio，未发现具名 Unix/TCP listener；本机 SSH 端口不可连接。没有重启 Desktop、启动本地 daemon、打开 Remote Login 或改动本机 SSH 配置。共享本机方案的事实与待验证项见 [本地会话说明](../design/local-desktop.md)。

## 编号管理与安静会话 UX

按新反馈将 Telegram 管理界面改为持久 MarkdownV2 编号导航：主机 → 工作区 → 会话，0 返回，/refresh 刷新。没有导航按钮 TTL；只读菜单可跨恢复及非授权配置变更使用。创建确认仍受当前 scope、freshness、菜单回执、单次选择、用户配对检查保护。旧按钮回到编号首页，普通管理任务文本拒绝且不复述。

会话状态加 emoji，卡片仅保留标题/状态；正常输入只记详细日志，没有独立 ACK。实时与初始 Telegram 历史只显示 agent 正文，用户 item 仍推进观察游标以免反复处理，不回显或加角色标签。空 agent 历史不入发送队列；内部模型/Discord 的完整角色历史不因 Telegram 展示需求而删改。

新增行为测试覆盖持久菜单恢复、非授权变更、旧回复、未投递菜单、过期数字确认、scope 改变、无按钮的 MarkdownV2 请求、缺失菜单消息的已知拒绝后替换、安静输入/用户 echo 抑制与无空历史。


## 模块重构、动态目录与三台主机验收

SSH/TOTP 已独立为 ssh crate；全部平台 API、配对/身份、交互、运行任务及 SQL 分别归 telegram/discord。engine 统一共享连接、会话串行锁、操作结果与 Integration 生命周期，app 仅保留根配置/CLI/日志/装配/inventory。Service 按职责拆为平铺文件。Serde 的 yes() 已替换为 default_pairing_required，含义仍是旧队列缺字段时默认要求配对。

provider/workspace 从 TOML 移入 SQLite，现有一个实例和九个工作区保留 ID；新实例默认只读。根配置仅保留允许的主机、认证、目录范围和运行设置。catalog 约两秒刷新，CAS 发布避免并发 reload 恢复已撤销范围；已测试 CLI catalog 管理与运行服务共存、重复添加幂等、换 endpoint 不继承控制、删 host 后旧快照不能恢复，以及管理 /add、/remove 不发 agent 请求。

三台配置主机 anjie-bsc-arb、anjie-solana-ct、aws-work 均经正式 binary 完成 SSH 登录、自动 binary/socket 发现、Codex initialize 和只读会话列表读取，每台返回首批 10 条会话。保持 bsc 原有控制确认；solana、aws 新实例保持只读，没有对既有用户任务发送测试输入。

aws-work 使用原生 ProxyJump；额外用临时独立 SSH 配置验证同一目标的 ProxyCommand。真实 Google Authenticator 导出只离线解析并将 TOTP seed 安装到私有根配置；成功认证、强制关闭专用 SSH 测试子进程、重新认证连接均通过。没有停止远端 daemon，没有断开用户 Desktop/CLI 的连接。真实 URI/seed/验证码未加入 fixture 或日志。

原型阶段已用根 schema.sql 替代全部 migration 文件与迁移器，幂等创建缺失表、索引和触发器，不擦除既有记录。配对注册表和服务均复用同一 schema 初始化。稳定 0.1 前直接维护 schema；旧数据需要改列时明确处理，不自动重置状态。

tracing 已补充主机/provider 建连、重连退避、授权操作 dispatch/回执/失败、目录变更和 schema 错误码事件。日志默认可同时输出终端与文件，daily/hourly 轮转，retained_files 数量限制；传输 target 仍被独立脱敏过滤。测试专用 probe、sessions、state-init 与重复 telegram-check CLI 已移除。


## 本轮发布构建（macOS arm64）

构建命令：cargo build -p app --release --no-default-features --locked。产物 target/release/pocketpilot，10,192,144 字节（10.19 MB / 9.72 MiB），较本轮默认 release 基线 19,512,000 字节缩小 47.76%。保留 panic unwind、TLS、SQLite、tracing 和生命周期保护。详细来源见 [构建说明](../build.md)。

SHA-256：`6d33716c7547bc16eb8c3211629024243b5960cd07f5f0acdbf4389bc5c5c9ff`。

最终 fmt、全 feature Clippy（-D warnings）、无默认 feature 的 app Clippy、82 项测试及 release 构建均通过。配置检查确认 3 hosts / 3 providers / 9 workspaces。新版 release 桥接已启动并收到 Telegram ready 与共享服务连接事件，后台输出只保留受轮转管理的 tracing 文件。临时认证导出、秘密备份及测试 harness 已移除；真实根配置保持 0600 且 Git ignored。


## Off-topic /start 没有可见回复的修复

真实记录确认命令已处理，菜单也已成功编辑到管理 topic；原位置的跳转提示却使用 management-route:TOPIC 作为永久去重键，后续 /start 不再收到原位置提示。改为按真实输入 event key 去重，每条新命令分别回复，重投相同输入仍只投递一次；管理菜单继续复用同一条消息。新增入口接收、管理路由和投递成功日志，只记录 ID、目标 topic 与是否编辑，不记录正文。

新增完整 SDK/mock 回归先复现“两次 /start 仅一次原位置回复”，修复后确认两次都收到提示、只创建一个管理 topic、复用菜单且没有任何 agent 请求。Telegram 46 项测试、全 feature Clippy 均通过。手动测试改用 launchd 临时 job 脱离工具进程，不安装登录启动项。

修复版已替换并重启独立托管进程；仅为用户实际已发出的最近一条 Off-topic /start 补入缺失的 UI 跳转提示，未修改或重发 agent 输入。真实 Telegram 投递回执确认该提示作为新消息发送至 topic 0，后台轮询继续成功。当前修复构建为 10,192,656 字节，SHA-256：`45a432a9116d1545903d5e18b25554b91496931238bbc078d26e0e151270fffd`；上一节构建数值对应前一版重构检查点。


## Off-topic 统一管理与清空历史后的恢复

新增配对后 Off-topic 文字入口归一，序号/命令/普通文本共用管理上下文，普通任务仍拒绝。显式 /start 新发管理首页，避免只编辑用户已清空但 bot 仍能编辑的旧消息；编号步骤继续编辑。已有话题仅做同名验证，明确不存在时恢复，未知网络结果不另建话题。入口出错时在 Off-topic 原位置反馈。

新增回归先复现 Off-topic 序号不推进菜单和清空本地历史后只编辑旧消息的问题；修复后验证统一导航、普通文本不进入 agent、全新首页、明确删除恢复、未知检查不重复创建。Telegram 50 项测试及全 feature Clippy 通过。官方文档确认 topic 根服务消息被删除可导致整个 topic 删除，View as messages 切换仅适用于超级群；不宣称 Bot API 能强制用户界面跳转。

修复版已重新构建并重启 launchd 托管进程。向原管理 topic 新发菜单后，真实回执中的 requested_topic 与 returned_topic 相同，发送为新消息而非编辑；说明该服务端 topic 此时仍存在，不能再把本地缓存作为唯一证据。当前 binary 10,193,136 字节，SHA-256：`7a4660b1ae2ca58c405188f80e75daa42352dd66d3fa438e35612ec34efb3d81`。客户端话题列表显示需由用户确认。


## Emoji 展示、空会话、Markdown 与中英文 UI

按新反馈，菜单每次输入均新发 MarkdownV2 消息，状态只用 emoji；管理入口标题为纯文字「管理」，原生 📁 图标独立设置，避免 emoji 重复。session topic 仅保留最多 40 个 grapheme 的原始标题，不拼工作区/路径/ID。原生图标通过 Telegram 允许的 custom emoji catalog 映射，只有状态或标题变化才投递，待投递状态覆盖合并并复用既有速率限制。自动状态气泡已移除；旧状态卡只按本地拥有权指针清理。/status 返回详细主机、工作区、cwd、thread/provider/turn 与运行状态。

创建参数改为标题：只发 thread/start，不再把标题作为首条 turn/start。新会话等待用户在其 topic 发第一条消息。原生 name 为空时，在新鲜且验证 cwd/权限的显式创建/打开/重命名动作中调用官方 thread/name/set；已有标题保留，后台/重连不改标题。生成根已扩展为官方 ThreadSetNameParams/Response，无提交 schema。真实隔离 Codex fixture 已验证空创建、命名与历史读取均不联系模型，再继续原有多客户端/断线生命周期场景。

typing 使用独立、可失效的 lease，接受输入后立即唤醒，每约 3–4 秒续期，历史读取慢或没有新文本也继续；等待审批/输入、空闲、离线、解绑或撤销范围时停止。真实私聊 topic 的 sendChatAction 已返回成功，mock 回归验证多次续期及解绑后不继续发送。

Agent Markdown 由 pulldown-cmark 解析 GFM，转 Telegram 安全 HTML；表格转逐项字段。HTML/链接转义及协议过滤，明确格式拒绝才降级纯文本，超长仍是一份原始 Markdown。完整 HTTP fixture 验证 bold/link/table 与 parse_mode=HTML，并确认 session 没有自动状态气泡。

rust-i18n 4 提供编译期中英文 YAML，显式 locale 参数避免全局语言竞争；preferences 按 bot/user 持久化。缺省中文，default_locale=auto 才采用 Telegram 语言，/language en 和 /language zh-CN 可独立切换。测试覆盖跨用户隔离、持久偏好优先于客户端语言变化和任务正文不参与 UI 翻译。

已对用户截图唯一标识的 ideas 会话做一次仅空标题的维护：确认原生无 name 后补写并回读一致，没有启动 turn；未修改其他 ideas 会话。新版启动后，真实 topic 缓存已显示原始标题、💬 状态，并退役旧状态卡。其余 agent 执行策略、共享 daemon 与已有任务生命周期均保留。

本轮全 feature 96 项测试、fmt、Clippy -D warnings 通过；无默认 feature 的 app 构建与隔离原生 fixture 均通过。旧消息可用 /history 按当前渲染重新查看。原生标题空值检查与 set API 无跨客户端 CAS，接口限制见架构文档。


## 重复管理图标与话题管理

截图中的重复来自原生图标和标题中的 emoji 叠加。管理标题改为纯文字「管理」/Manage，创建时显式选择原生 📁；/start 验证时同步修正旧标题及图标。真实已有管理 topic 已通过 editForumTopic 更新成功，未创建新话题或发送 agent 输入。

官方方法/类型目录核对没有 topics 列表、单项读取或删除服务事件。真实私聊上不带 name/icon 的 editForumTopic 返回成功，作为保留元数据的已知 ID 检查；不把超时/限流或 TOPIC_CLOSED 误判为删除。新增首页话题管理和 /topics，包含已解绑的记录；分页检查明确不存在时退役本地指针。选中后确认删除全部 Telegram 聊天记录，原生 Codex 会话保留。删除账本持久化且绑定权限先失效，未知结果及重启不重放，旧确认重新询问；跨私聊和管理话题拒绝删除。自动化仅删除 fixture 话题，没有删除用户的真实话题。

最终检查覆盖 101 项测试（Telegram 63），全 feature/all-targets Clippy -D warnings、无默认 feature app Clippy、fmt 与 release 构建通过。macOS arm64 binary 为 10,630,928 字节，SHA-256 `f788bdfd5ce845ba3ea1b5727ecae2e2d304de9391f561776ae98fd1d530607d`。新 schema 已在保留原配对/绑定的状态库中初始化；桥接已通过 launchd 重启并确认 Telegram ready，日志继续轮转，未重启 Codex daemon。临时标题维护 CLI 已清理。


## 统一静止图标、注册命令与全部清理

按实测反馈，未加载与空闲的会话图标统一为 💬，固定管理 topic 仍用 📁，运行/等待/异常保持各自图标；/status 保留真实加载状态。启动以 AllPrivateChats 注册缺省/zh/en 三份命令说明，失败不阻断手动输入。增加 /cleartopics 及编号菜单入口，显示已记录会话 topic 数量、删除聊天记录范围和保留管理/原生会话的确认。

批量确认固定具体 topic ID，确认后新增的 topic 不会被删除；逐项授权、账本、绑定失效与单项删除共用，删除间隔至少 1.1 秒。遇失败/限流/未知结果停止，不重试、不在重启后继续。fixture 覆盖跨确认新增话题、保留管理和原生会话、旧确认重询及批量未知回执停下；中英文命令的实际 SDK payload、私聊 scope 与无 stop 命令亦有覆盖。没有对真实用户话题执行清除。

本次最终 104 项测试（Telegram 66）、fmt、全 feature/all-targets Clippy 与无默认 feature app Clippy 全部通过。发布构建 10,672,880 字节，SHA-256 `386a28a9dc527c7ef07d4b8bb5c9e6496c3a55c37181ce92cbda7899d624aa6a`；新版桥接已重新启动。真实 getMyCommands 回读确认 zh/en 各 12 项私聊命令；四个已绑定会话的图标更新回执成功，缓存均为 💬。真实用户话题清除仍由用户在 Telegram 内确认。
