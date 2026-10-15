# MVP 实施计划

状态：Telegram MVP 的消息、控制、持久化、OTP helper 和热更新已落地；授权 SSH 共享 daemon 已通过专用会话的 active/steer/断线完成验证。真实 Telegram 交互待用户测试，本地既有 Desktop 缺少共享 endpoint。Discord 可选原型已加入，见 [原型说明](../design/discord.md)。

下文 W00–W17 保留为设计工作包；描述中的目标接口不全部与当前源码名称一一对应。当前行为与边界以 [验证记录](../research/implementation-checks.md)、[配置实现](../design/connections-and-config.md) 和源码为准。
## 1. 范围与当前输入

首版是最新受测 Codex + Telegram，默认本机运行，聚合允许的本地/SSH 实例。具备只读列表/状态/历史、topic 绑定、新建、续轮、steer、输出合并、OTP、热更新与恢复。允许的 provider 业务动作仅创建、用户明确续轮、steer、只读查看。

已有输入：token 已迁入根目录私有 pocketpilot.toml 并通过只读账号验证。配置不 include、不读取 .env；Telegram 仅私聊，经 CLI 配对后可用，无 chat/user 配置列表。已登记授权 SSH host 的九个工作区和共享 socket；其他实例需填写已存在的共享入口。

硬门禁：同一运行实例、无副作用观察、程序断线/退出不结束 agent。无法通过时只开放相应只读能力，并将完整 MVP 标为未完成。Lark、Claude shim、任意旧 Codex 兼容和 agent 生命周期管理不在首版任务中；Discord 按后续请求提供有明确边界的原型。

## 2. Rust 依赖选择

用成熟库处理通用能力，只自行实现业务映射、Codex wire 适配、状态核对和可靠投递规则。下表是 2026-10-15 crates.io API 实际查询的快照；版本仅供依赖初始化，不是未经编译验证的兼容保证。下载总量含历史和传递依赖，用来说明生态成熟度，不等同质量评分。

| 用途 | 推荐 | 快照稳定版本 | 累计下载约数 | 理由 |
| --- | --- | --- | --- | --- |
| async runtime | tokio | 1.53.3 | 10.66 亿 | actor、子进程、网络、signal |
| tracing | tracing / tracing-subscriber | 0.1.44 / 0.3.23 | 9.34 / 6.68 亿 | 统一结构化日志与 span |
| Telegram | teloxide | 0.17.0 | 212 万 | 成熟 API 类型、命令与 bot 请求；poller 需自有持久确认边界 |
| SQL | sqlx（SQLite） | 0.9.0 | 1.65 亿 | Tokio 集成、事务、迁移，不自写 SQLite 包装 |
| serialization | serde / serde_json | 1.0.229 / 1.0.154 | 15.33 / 14.36 亿 | wire/config/持久化 payload |
| WebSocket | tokio-tungstenite | 0.30.0 | 3.10 亿 | 协议与握手，不自写 WebSocket |
| SSH | 系统 OpenSSH + tokio::process | 系统安装 | — | 复用原生 config、认证链与 keepalive，固定 proxy 命令 |
| OTP SSH | 受限 SSH_ASKPASS + totp-rs | 6.x | 1128 万（TOTP 调研快照） | OpenSSH 处理 keyboard-interactive，helper 只回答明确挑战 |
| 热更新 | notify / arc-swap | 8.2.0 / 1.9.2 | 1.72 / 3.61 亿 | 文件监听与配置快照原子切换 |
| SSH 配置发现 | ssh2-config | 0.8.2 | 19.2 万 | 候选 alias/Include 解析，实际连接参数由 ssh -G 评估 |
| CLI | clap | 4.6.7 | 12.28 亿 | daemon/doctor/config/status 命令 |
| errors | thiserror | 2.0.21 | 16.29 亿 | 库层强类型错误，应用边界可用 anyhow |
| trait object async | async-trait | 0.1.92 | 7.27 亿 | dyn AgentProvider/ChatPlatform |
| Unicode | unicode-segmentation | 1.13.3 | 6.09 亿 | topic 名和文本分片 |
| retry | backon | 1.6.0 | 9640 万 | 合理退避，不自造通用 retry 框架 |
| secrets | secrecy | 0.10.3 | 1.79 亿 | token 不意外 Debug 输出 |
| Discord 后续 | serenity | 0.12.5 | 756 万 | Gateway、HTTP 与生态整合 |

补充依赖按需加入 tokio-util、futures、toml、uuid、time、tracing-appender、directories、fs2、tempfile；没有实际使用就不引入。锁定 Rust 工具链和 Cargo.lock，确认 MSRV、TLS feature、依赖版本冲突后再固定具体版本。Telegram 不需要 Discord/Claude 的 feature 和依赖随 MVP 一起编译。

SSH 选型采用双后端：openssh 负责普通 key/agent 和复杂 ssh_config 兼容；配置 OTP 时使用 系统 OpenSSH 原生 keyboard-interactive + totp-rs，不能仅依赖 OpenSSH BatchMode。直接读取本机 SSH config/Include 发现 alias，受控 ssh -G 获取有效参数；实际认证链由服务端协商。慢 host 的超时、心跳和认证退避逐台配置，详细规则见 [部署与配置](../design/connections-and-config.md)。

JSON-RPC 不直接套标准 jsonrpsee server/client：Codex wire 省略 jsonrpc 字段且存在双向服务端请求和特殊 transport。使用 serde 的小型、可测试 dispatcher 对接受测方法；不要为了复用框架改变 wire。Codex 源码的内部 workspace crate 不直接作为 git 依赖引入整个 agent 引擎。build.rs 调用所选 Codex 导出官方 JSON Schema，在 OUT_DIR 筛选只读投影并用 typify 生成 DTO，不提交 schema/生成代码。provider 访问生成类型；RPC 边界只用不透明 RawValue 保留扩展，不在业务层操作 serde_json::Value。

## 3. crate 与文件边界

采用四个简短的本地 package，全部 publish=false：

| package/目录 | Rust crate | 职责与依赖 |
| --- | --- | --- |
| crates/session | session | 模型、ports、应用操作、actor、历史/投递规则；不依赖 teloxide/sqlx/进程 API |
| crates/codex | codex | Codex provider、wire、RPC、观察与 transport；依赖 session |
| crates/telegram | telegram | Bot API、poller、菜单/topic、授权与渲染；依赖 session |
| crates/app | app | CLI、配置/凭据、SQLite、tracing、装配与进程生命周期；依赖前三者 |

app 的 [[bin]] 名称为 pocketpilot；package 名仍为 app。源码目录、package 和 Rust import 均不使用项目名前缀。后续新增 claude/lark/discord 时复用同一边界，不预建空 crate。

```text
crates/
  session/src/     lib.rs model.rs provider.rs operation.rs delivery.rs
  codex/src/       lib.rs protocol.rs rpc.rs transport.rs ssh.rs provider.rs
  telegram/src/    lib.rs
  app/src/         lib.rs main.rs cli.rs config.rs credentials.rs storage.rs telemetry.rs pairing.rs
migrations/
crates/codex/build.rs  导出 schema、筛选与生成 DTO
# 所有生成物写入 Cargo OUT_DIR，不在源码目录中
```

源码保持 src/ 下单层文件；新增功能按职责增加平铺文件，不预建 model/ports/service/runtime/transport 等多层目录。生成的协议代码仅写到 Cargo OUT_DIR。

Store 暴露应用事务，不为每张表建立泛型 repository。共享 SSH transport 初期留在 codex；只有实际新增第二个 provider 后才评估抽成独立库，避免提前搭插件框架。

## 4. 顺序与阶段门禁

| 阶段 | 工作 | 放行条件 | 当前状态 |
| --- | --- | --- | --- |
| P0 | W00–W03：骨架、配置、类型化 RPC 与兼容探针 | 原生本地隔离实验和真实 SSH 共享实例通过；既有本地 Desktop 入口待部署 | 已实现，部分部署未验收 |
| P1 | W04–W07：远程、持久化、只读 provider、动作许可 | OpenSSH、账本、actor、白名单与未知结果不重发已有行为测试 | 已实现 |
| P2 | W08–W11：Telegram、绑定、历史与投递 | mock 完整链路通过；实际私聊 topic 收发等待用户 | 代码完成，待用户验收 |
| P3 | W12–W14：OTP、热更新、创建/续轮/steer | 实际 SSH 写入/断线完成通过；真实 OTP 待验收 | 代码完成，部分部署未验收 |
| P4 | W15–W17：恢复、打包与验收 | 重启/unknown/持久批次测试通过；A01–A52 非全量实测 | 部分验收 |
| D0 | Discord 原型 | owner slash、category/channel、共享锁/操作账本、输出队列；未提供真实凭据 | 原型完成，待真实 guild 验收 |

W00–W02 是制作 P0 探针必需的最小骨架，不代表先做完整产品再验证共享能力。可独立实现 fake/持久化/渲染，但 G0 未通过时不对真实任务启用写入。

依赖链：W00 → W01 → W02 → W03；W04/W05 接基础后推进；W06 → W07；W08/W09/W10/W11 形成只读 Telegram 闭环；W12 与 W13 分别完成 OTP 和 reload，W14 依赖这些基础后开放控制；最后 W15 → W16 → W17。

G0 按实际 runtime + transport/profile 认证，不按全局版本号一键放行：W03 可先验证本地；W04 的 SSH 或 W12 的 OTP 后端完成后补充只读/连接证据。G0 仅允许继续建设该组合，不能代替 G3 的真实写语义和三端场景。未通过的 host 明确 degraded，不能继承另一个 host 的控制资格。

## 5. 可执行任务

每个 W 是一个可审阅工作包，必要时拆成小提交；顺序以依赖为准，不按行号强行串行。测试针对行为和故障边界，不写只复述实现的测试。

### W00 · Workspace 与运行骨架

依赖：无。归属：根 Cargo 配置、四个 crates 的 lib/main、工具链与最小 CI。

- 创建四个 publish=false package；app 声明 pocketpilot binary，确保 cargo run -p app --bin pocketpilot 的目标清晰。
- 配置共享依赖/TLS feature、lockfile、fmt/clippy；主程序先提供本地帮助，不在初始化时连接服务。
- 产物：可编译 workspace 与职责一致的空骨架，不拉入 Lark/Claude 依赖；Discord 为独立可选 feature。
- 验证：workspace build、依赖方向检查、CLI help；无网络/agent 子进程副作用。覆盖 A23/A46 的初始结构约束。

### W01 · 单根配置、内联 token 与 tracing

依赖：W00。归属：app/src/config.rs、credentials.rs、telemetry.rs、session 的 config/authorization 模型。

- 实现根目录单一 TOML、相对路径和稳定 ID/引用校验。include 与 chat/user 允许列表字段明确拒绝；不存在配置合并优先级。
- Telegram token 直接在 [telegram].token，格式校验与 Debug/错误脱敏，不从 .env 或环境变量回退。
- 日志 filter 来自 TOML；错误、URL、Debug、解析失败统一脱敏；不依赖 RUST_LOG 自动覆盖。
- 产物：根目录公开示例可解析；真实 pocketpilot.toml 不提交；空/坏 token 不伪报认证成功。
- 验证：临时单根配置、路径解析、禁用 include/允许列表、错误脱敏、公开文件不含秘密；A48。真实 token 不进入 fixture/输出。

### W02 · Codex wire、RPC 与只读连接

依赖：W01。归属：codex/src/protocol.rs、rpc.rs、transport.rs 的本地 proxy/WS。

- 按目标 binary schema 选择必要 DTO；initialize → initialized 严格握手，区分双向 request/response/notification。
- 支持 string/integer RPC ID、pending map、连接 epoch、deadline、迟到响应和未知 enum；限制缓冲与 frame 大小。
- ObserverPermit 只能调用白名单只读方法；未授权方法不能通过 raw string 绕过。
- 产物：可用于 P0 的只读探针/合成 fake transport，不启动新 app-server。
- 验证：握手、乱序/重复/未知消息、EOF 和 timeout；A20/A23/A46。

### W03 · 共享实例与生命周期行为验证

依赖：W02；SSH profile 追加验证依赖 W04，OTP profile 依赖 W12。归属：app CLI probe、匿名 fixture、受测 profile 与研究报告。

- 固定实际 desktop/runtime/CLI/schema，确认 endpoint ownership；不能只比较 thread ID 或历史路径。
- 在专用测试 workspace 读取活跃状态/turn ID/文本，确认不通过后台 resume 取得观察能力。
- 任务由原生 CLI/桌面在专用 workspace 发起；探针仅观察，并验证它自身退出/崩溃/SSH EOF 后任务未改变。此时不实现或发送 steer/new/continue，也不把原生客户端的成功当成 PocketPilot 写路径成功。
- 产物：只读 compatibility report、受测 profile、能力缺口与证据；steer/新建/只剩桥接器拥有合法订阅等尚缺条件明确标 not_run，留 W14/W15。
- 验证：为 A01/A02/A07/A08/A20/A22/A29/A33/A46/A47 提供 provider/连接侧基础证据，不能宣告这些跨端场景已全量通过。A03–A06/A21 的正式写入与 Telegram 证据在 W14/W17 完成。

### W04 · SSH config 发现与普通连接

依赖：W01/W02。归属：codex/transport.rs、ssh.rs，app host registry。

- 从本机 Host/Include 发现明确 alias；按 allowlist 选择。默认 ssh -G alias，显式 -F 与实际连接必须一致。
- 解析有效 user/port/key/agent/jump/known_hosts；kbd=yes 只是候选，不证明 OTP 必需。
- OpenSSH/proxy 独立超时、keepalive、stderr 有界脱敏；host 间连接并行但同 host single-flight。
- 产物：普通 SSH host 的只读连接，不执行 daemon start/restart/bootstrap。
- 验证：Include/Match/-F/跳板、慢 host 隔离、主机公钥异常；A25/A32/A33/A36。

### W05 · SQLite 迁移与操作账本

依赖：W01。归属：app/src/storage.rs、migrations、session::Store。

- 创建 bindings、operations、inbox/offset、text_items、deliveries、requests observation、provisionings；配置表只存派生索引。
- 实现 ingest+offset、操作预留/Dispatching、item+outbox 等短事务；W11 再补批次表迁移。
- 唯一键按 instance/thread、平台地址、stable_event_key/nonce 建立；保留容器墓碑和绑定 revision。
- 产物：可恢复 Store 实现；WAL/foreign_keys/busy_timeout、版本迁移、单写者锁。
- 验证：重复/崩溃点、迁移、DB 忙/磁盘失败；A09/A10/A11/A24/A26。

### W06 · 只读 Provider 与状态核对

依赖：W02/W04/W05。归属：codex provider/observe/reconcile。只读访问可用于收集 W03 的 SSH/OTP 证据；G0 未证明的能力保持受限，不宣称可实时观察或控制，避免与 W12 形成依赖循环。

- 分页 list/read/history，按精确 cwd 合并显式 worktree 路径；保留来源/归档过滤。
- 分开 connection、thread runtime、active turn、last outcome 与 freshness；未知状态不映射 idle。
- observe/release_observation 只改本地观察集合，不发 resume/unsubscribe。轮询能否提供实时内容以 G0 证据为准。
- 产物：本地和 SSH 只读列表/状态/历史，断线区间可能缺失的提示。
- 验证：A01/A13/A20/A26/A29/A30/A32。

### W07 · Actor、动作许可与不确定结果

依赖：W05/W06。归属：session/model.rs、provider.rs、operation.rs，codex 发送许可检查。

- 按 session 串行化本应用输入；ObserverPermit 与显式用户 operation 的 MutationPermit 分离，后台不能构造写许可。
- 输入带 binding/config revision、epoch、平台时间、expectedTurnId；过期/离线草稿重新确认。
- Dispatching 先落盘再写 transport；未知结果不重试。进程恢复的 Dispatching 只能核对，不能续跑写步骤。
- 产物：fake provider 可验证的完整操作状态机和错误分类。
- 验证：A04/A06/A09/A18/A23/A27/A38/A46；桌面/CLI 并发依靠服务端 turn 条件，不能只靠本地锁。

### W08 · Telegram 身份验证与持久 Poller

依赖：W01/W05。归属：telegram/api、polling、authorization；app 装配凭据。

- getMe 确认 bot ID；getWebhookInfo 只读检查，发现 webhook 不自动删除。
- 接收批次后先 durable inbox+offset，再确认更新；支持长期空闲随机 update ID 的 polling epoch 重建。
- 真实 token 从根 TOML 载入；接入严格检查 private chat 与 sender 数值 ID 一致，不按用户名授权。
- 未配对私聊 /start 创建十分钟一次性码；只有本机 CLI approve 能授权。pending/list/revoke 与服务并行运行，不触碰执行账本恢复；申请/配对按 bot ID 隔离，禁止群聊申请。
- 产物：测试私聊 update 的可靠入口与 Ready/degraded 状态；尚不向 Codex 写入。
- 验证：A10/A18/A28/A48/A49/A50/A51；bot identity 变化不得沿用旧 offset。

### W09 · 菜单、工作区与 Topic 绑定

依赖：W06/W07/W08。归属：telegram menus/topics，session bind/unbind，app ConfigWriter。

- 选择 host/workspace/session、分页搜索、状态卡和短标题；ConfigWriter CAS 原子登记 workspace。
- 创建 topic saga 持久化每步 ID；结果未知不重复 create。重复绑定返回原 topic，容器不跨 thread 复用。
- callback nonce 绑定用户/地址/revision/expiry；关闭、删除、权限撤销只影响平台映射。
- 产物：可只读使用的 Telegram topic 工作流。
- 验证：A10/A11/A18/A19/A26/A37；不为了绑定调用 resume。

### W10 · 最近交互与完整历史集合

依赖：W06/W09。归属：session/delivery.rs、telegram/rendering。

- 识别最后 user/对应 final，缺 final/缺 phase 明确标示；不拿上一轮输出冒充本轮回答。
- 活跃会话分页取全部可读文本，保存范围、revision 和完整性；超限一次 Markdown 附件，仍过大提供主动分片入口。
- 历史屏障有成功、失败、unknown、超限、用户放弃和 deadline 出口，不挡住之后的新实时文本。
- 产物：HistoryBundle 和文本/文件 renderer；不发 agent 总结任务。
- 验证：A13/A14/A41/A42/A43/A45；静态和活跃变化场景分别检查。

### W11 · 合并、Outbox 与平台调度

依赖：W05/W08/W10。归属：session/delivery.rs、telegram/scheduler、批次迁移。

- 保存 item 原边界；窗口/阈值/max delay 生成同 session/地址/config generation 的 MessageBatch。
- batch/member/outbox 同事务；Sender 单独限速，同一私聊内 topics 共享预算、遵守 retry_after，状态更新可合并。
- clientId 回声关联；工具/推理过滤；partial 在异常结束后保留。平台未知投递不自动重发。
- 产物：单消息/附件投递、已知 message ID、未知回执和积压状态。
- 验证：A12/A14/A15/A16/A24/A31/A44/A45；使用受控时钟和 fake HTTP 测限速/公平性。

### W12 · Keyboard-interactive / TOTP

依赖：W04/W07。归属：系统 OpenSSH transport/askpass，app 私有 OTP profile。

- OpenSSH 完成 key 与 keyboard-interactive 的认证链；受限 askpass 按明确 prompt 规则回答，未知挑战不发送 secret。
- totp-rs 临发送生成、跨周期异步等待、次数/时间/退避上限；不能用短信/FIDO/HOTP 冒充 TOTP。
- 已有 key/jump/agent profile 不静默丢字段；不支持的组合返回明确错误，原端已运行任务不变。
- 产物：OTP 后端与逐 host deadline；W03 追加该 profile 的共享/断线验证。
- 验证：A25/A34/A35/A36/A39，fake SSH + 专用测试主机；不把真实 seed/code 写日志或 fixture。

### W13 · 配置热更新与撤销边界

依赖：W01/W04/W07/W09/W11。归属：app config/snapshot/writer/reload，所有 dispatch/deliver admission。

- notify + periodic rescan 监听根 TOML/SSH config；完整候选验证后原子替换。
- 发送许可与配置撤销共用短临界区；未开始的旧队列重新授权，已在途不能伪称撤回。
- endpoint/user/socket 更换使用新实例身份；同 bot token 轮换保留资源并交接 poller，不同 bot 冻结旧绑定/积压/nonce/provisioning、重新核验并建新状态卡；不重启 Codex。
- 产物：新增/删除 host/workspace、授权变更、显示与限速热更新及 requires_restart 报告。
- 验证：A37/A38/A39/A40/A46/A48；并发编辑、原子 rename、坏配置、发送瞬间撤销。

### W14 · 新建、续轮与精确 Steer

依赖：W03 的目标 profile 通过、W07/W09/W11/W12/W13。归属：session service/actor、codex write mapping、telegram command handling。

- thread/start 与首条 turn 分别记账；已知 thread ID 不重复创建；首条结果未知只核对。
- 先通过 fake 行为规则，再在显式限定的测试私聊/workspace/session profile 开启隔离写实验；确认写语义、三端可见和断线后执行存活后形成 G3。G3 前不允许普通真实任务借测试入口控制；W03 的 G0 不是写路径认证。
- 空闲续轮只在明确用户动作内必要时 resume，再 turn/start；不得修改 model/cwd/审批/sandbox。
- active 使用 turn/steer + expectedTurnId；冲突保留原文本供重新确认，不自动跨轮转投；审批仅显示。
- 产物：测试私聊创建/继续/steer 的完整回执与三端可见结果。
- 验证：A02–A06/A09/A11/A17/A21/A22/A23/A27/A33，真实验收仅使用独立测试 session。

### W15 · 故障与重启恢复

依赖：W11/W13/W14。归属：session recovery、codex supervisor、app bootstrap。

- 重连提高 epoch、初始化、只读核对当前允许绑定；不 resume/start/unsubscribe，不执行旧草稿。
- 历史重叠分页按 ID 去重；缺连续性证据就提示 gap；pending request 只观察。
- 本地 proxy/SSH handle、cancel token、进程组和 service stop 路径都检查；只回收桥接器资源。
- 产物：running/等审批/等回答/平台离线/DB 失败时的恢复报告。
- 验证：A07/A08/A09/A13/A19/A24/A27/A29/A30/A31/A45/A46。

### W16 · CLI、打包与运维

依赖：W15。归属：app cli、service 模板、CI、运行手册。

- 完成 run/doctor/status/workspace/bindings/operations；默认输出不含正文或秘密，不提供 raw RPC/stop/restart agent。
- Linux x86_64/macOS arm64 build；桥接器服务与 agent daemon 独立，配置路径显式、运行目录不决定 secret 来源。
- 单写者/poller 锁、日志滚动、队列/状态年龄诊断、SQLite 一致性备份与受控升级说明。
- 产物：可安装产物、运行步骤、健康/能力分离的诊断。
- 验证：A23/A24/A32/A46/A47/A48；fmt/clippy/workspace tests/build，通过后不无理由扩大测试。

### W17 · 完整 MVP 验收与发布门禁

依赖：W16。归属：测试报告、受测 profile 清单、发布说明。

- 在专用私聊/workspace 执行 A01–A52，覆盖本地、普通 SSH、OTP、慢 host、多端并发、热更新与故障。
- 每条报告记录 binary/schema/profile、操作、预期/实际和证据；测试数据匿名化，真实 token/seed 不入库。
- 未通过关键条件的组合明确限制，完整 MVP 不冒充完成；回归失败先修对应 W，不通过取消任务绕过。
- 产物：可审阅验收报告与最终支持范围；发布行为另按用户授权执行。

## 6. 提交、验收与状态维护

建议每个 W 至少一个清晰提交，标题描述最终行为；禁止把大量骨架、UI、未知写重试和远端生命周期改动混在一个提交。任务完成必须有产物和验证，不能只看代码数量。

完整 [A01–A52 清单](acceptance.md) 是验收条件唯一来源。报告格式与测试层次见清单；W03 先证明协议能力，W17 汇总行为验收。任何任务中的测试编号不代表已经执行。

实现开始时按本计划更新 W 状态和证据链接，不再复制一份容易过期的 TODO 文件。当前 W00/W01/W02 的基础实现已完成；W04/W05/W06 为部分实现；W08 已有只读账号与私聊访问/CLI 配对基础，poller 待接入；其余任务未完成。W03 的真实共享/生命周期证据待实际 endpoint 与独立测试环境，不得把 mock 测试当成通过。后续先完成该门禁，再接 durable poller、topic 和用户写操作。

## 7. 不随本轮实施的内容

Lark/Discord/Claude 的能力和候选库保留在 [后续扩展](../future/integrations.md)，不进入首版依赖。研究来源和匿名快照保留在 [研究记录](../research/sources.md)。文档检查已做；没有进行 session 写操作或服务升级；真实多端/SSH/OTP 行为仍须独立验收。
