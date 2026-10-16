# 实现与验证记录

日期：2026-10-16。Telegram MVP 已具备可运行功能链路，SSH 共享实例控制已通过专用会话验收；真实 Telegram 交互等待用户测试。Discord 是可选、可运行的 slash-command 原型。

## 实现边界

- 平铺源码：session、codex、telegram、app，加上可选 Discord adapter 的 discord crate。普通配置与所有秘密保留在单根 TOML。
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

已授权配置中的该 SSH provider 已开启 control_verified；九个工作区保留在私有根配置，公开示例不包含个人 endpoint/凭据。测试产生的专用会话留在原生服务供核对，未主动归档或停止。

## 自动与隔离验证

全功能测试共 69 项通过。完整命令为 cargo fmt --all -- --check、cargo clippy --workspace --all-targets --all-features --locked -- -D warnings、cargo test --workspace --all-features --locked。

行为测试覆盖私聊配对/topic/历史/创建/续轮/steer、重复与丢回执、按 peer 顺序 dispatch、撤销授权/配置与绑定校验、批次重启/固定截止、bot 私聊账号隔离；Discord mock 覆盖 owner 授权、category/channel、共享 provider 与重复/未知写不重发、解绑不结束任务。构建与测试没有真实聊天平台写入。

本机真实 Codex 0.162.1 和 0.161.0 均使用独立临时 CODEX_HOME、模拟模型运行 TCP WS/Unix proxy 生命周期实验：多客户端 active、steer、最后客户端断开后仍运行、最终历史均通过。这些 fixture 不读取用户凭据，也不调用付费模型。

Telegram getMe/getWebhookInfo 已通过：私聊 Topics 已开启，无 webhook；不会自行删除 webhook。正式服务启动后才消费 updates 和处理用户请求。真实交互证据将在用户测试后补充。

## 未验收事项

本机现有 Desktop 0.162.0-alpha.17.2 的服务采用 stdio，缺少公开共享入口；独立 native fixture 不能证明既有本地任务可接入。未重启 Desktop。真实 OTP/复杂跳板、慢服务器长时故障、Telegram 实际收发和 Discord 真实 guild 尚需部署验证。

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
