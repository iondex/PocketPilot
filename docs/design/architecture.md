# 架构、Rust 接口与数据设计

状态：架构契约与接口设计。当前已实现 Telegram 服务和可选 Discord / Slack 原型；下方接口示例表达职责，部分名称/高级能力仍为设计目标。实际文件与验证边界见实施计划和验证记录，不能将示例当作已导出的 Rust API。

## 当前源码边界

十个 crate 均为 publish=false，源码物理目录保持 src/ 下单层文件。

| crate | 所有权 |
| --- | --- |
| i18n | rust-i18n 共享中英文 ID 资源、显式 locale、参数插值与类型化错误展示；不依赖其他项目 crate |
| ssh | 原生 SSH 配置、ProxyJump/ProxyCommand、连接参数、askpass、TOTP 与认证导出解析；不依赖 app/engine/codex |
| session | 统一 AgentProvider、领域模型、允许的 Mutation、历史与消息规则 |
| connections | local / SSH proxy / websocket endpoint 和 Codex transport seam；不持有聊天授权 |
| adapters | agent runtime seam 与 Codex adapter；Claude Code / Pi runtime server 仍在规划 |
| codex | Codex DTO、RPC/framing 和 app-server 原生实现，由 adapters 作为 runtime adapter 使用 |
| engine | 配置快照、agent runtime registry、操作账本、会话锁、catalog 与执行编排 |
| integrations | Telegram、Discord、Slack 的消息接收和控制端；各平台协议、菜单、SQL、投递及内部 network 模块按目录隔离 |
| app | 根配置组合、日常 CLI、tracing、catalog 后台刷新、Integration 装配与退出 |

依赖方向：各 crate → i18n；app → integrations/engine；integrations → engine → codex → ssh/session。engine 不引用 integrations，integrations 不依赖 app。各平台持有自己的 Store 包装，共用 engine 的 runtime registry 与服务锁；平台 SQL 不放回 app。这是一份应用内边界，不是数据库权限隔离。

Telegram runtime 只组装任务；commands/input/routing/binding/navigation/new_session/topics/history_view/management/observation/delivery/inbox 分担职责。持久收发进一步分为 inbox_store/outbox/ledger。Discord 使用对应平铺模块。engine::operations 统一处理已获授权写操作的回执与 OutcomeUnknown，平台分别保留授权/时效/绑定检查与 dispatch 原子边界。

模块按功能和变化原因拆分，物理文件仍平铺在各 crate 的 src/ 下；内部实现通过私有模块和最小可见性组合，外部调用继续使用原有入口：

- Codex 的 provider 定义能力并实现只读会话观察；listing 管理已加载/已存储会话分页及游标作用域，mapping 将生成 DTO 转为 session 模型，mutations 集中显式授权写操作及原生输入编码。协议转换与写操作测试分别靠近所属实现。
- Telegram 的 navigation 编排菜单发布、选择和项目管理；navigation_model 保存菜单内容、文案类型和原生按钮布局，navigation_store 负责菜单快照、发布回执与原子消费。浏览刷新和项目增删共用 catalog 快照加载与发布流程。
- app 的 cli 负责命令解析、调度和输出；doctor 持有具名诊断选项、离线凭据检查和显式连接探测，返回可序列化报告，由 CLI 输出。诊断模块不直接写标准输出。

Integrations 的公共 contracts 定义平台身份、生命周期、附件来源和渲染能力；telegram、discord、slack 目录各自保留协议特有实现。Discord 的 gateway 只管理 Gateway、代理 relay 和退出；command_menu 定义及注册命令，interactions 负责交互 ACK、输入解析、授权和私有回复。menus 读取目录与会话并描述导航页面，menu_model 将显示选项与目标配对，统一保存有作用域的动作并生成原生控件；ui 保留持久化导航类型和显示规则。目录选择继续共用 engine::navigation，平台动作 SQL 留在 Discord Store。

根 TOML 与 catalog 合成一个不可变运行快照。provider/workspace 定义在 engine::model；配置反序列化跳过这两个运行字段，拒绝旧 TOML 中的静态目录。workspace_discovery 从分页会话 metadata 中补充允许路径内的 cwd，保留手工入口和移除排除记录。catalog 更新约两秒内可见；CAS 发布防止并发配置 reload 把已撤销主机重新带回。

根 schema.sql 是原型期唯一数据库定义，幂等初始化不恢复写操作；稳定 0.1 前不维护 migrations。Codex JSON Schema 与 DTO 仍只由 build.rs 生成到 OUT_DIR。

## 1. 架构目标

PocketPilot 是会话的附加客户端和聊天平台桥接器。Codex 继续拥有 thread、turn、任务执行、历史与审批状态；PocketPilot 只拥有地址映射、平台交互、投递记录和本地观察缓存。

边界独立于三个变化方向：

- AgentProvider：Codex/未来 Claude 的会话行为。
- ChatPlatform：Telegram、Discord、Slack/未来 Lark 的用户交互和消息投递。
- RpcTransport：本地 proxy、SSH proxy、WebSocket 的连接和 framing。

运行时采用单进程 Tokio 服务、SQLite 持久化和 有界任务并发与每会话串行锁。MVP 不需要 Redis、Kafka、HTTP 管理后台、动态插件 ABI 或分布式调度。

```mermaid
flowchart TB
  T[Telegram adapter] --> I[持久化 Inbox + 授权]
  D[可选 Discord adapter] --> I
  S[可选 Slack adapter] --> I
  I --> C[应用服务 / Command router]
  C --> A[每 thread 串行 SessionActor]
  A --> P[AgentProvider]
  P --> X[Codex adapter + JSON-RPC dispatcher]
  P -.-> CL[未来 Claude adapter]
  X --> R[RpcTransport]
  R --> L[本地 proxy]
  R --> S[SSH proxy]
  R --> W[WebSocket]
  X --> E[事件归一化 + 状态核对]
  E --> DB[(SQLite)]
  DB --> O[持久化 Outbox + 平台限速]
  O --> T
  O --> D
  A --> DB
```

## 2. 领域模型与地址

| 概念 | 定义/唯一性 |
| --- | --- |
| Host | 本地或 SSH 机器；UUID 是身份，显示名和 SSH alias 可变 |
| ProviderInstance | 某 host 上一个确定的 agent 服务；provider kind + 本地配置身份，不按 cwd 建实例 |
| Workspace | PocketPilot 登记的项目入口：instance + 主 cwd + 明确允许的关联 cwd/worktree |
| SessionRef | `instance_id + provider_thread_id`；不以新协议 sessionId 作主键 |
| TurnRef | session + provider turn ID；用于 steer 前置条件 |
| ConversationBinding | 某平台的容器地址到一个 SessionRef 的绑定 |
| PlatformAddress | Telegram `(chat_id, message_thread_id)`；Discord `(guild_id, channel_id)`；Slack `(app_id, team_id, channel_id, root_ts)` |
| Observation | 服务端状态 + active turn + 连接 epoch + 观察时间 + freshness |
| TextItem | session + turn + item ID + role/phase + 可见文本 + 完成标记 |
| Operation | 用户一次操作的持久化 ID、输入地址、执行模式、结果与不确定性 |
| PendingRequest | agent 发来的审批/输入请求；仅观察，由 desktop/CLI 原端处理 |

Telegram 平台资源增加 BotInstanceId（由 getMe.id 确认账号，不用 token 字符串作身份）。bindings、inbox/offset、operations 的来源、deliveries/outbox、ui_actions 和 provisioning 均带账号 namespace；同 bot token 轮换保留资源，更换 bot 冻结旧资源并显式重新绑定、创建新状态卡。容器不能跨 thread 复用的约束仍跨账号迁移保持。细节见 [凭据与配置](connections-and-config.md)。

同一远程 thread 在不同 host/provider 中可同名；ID 不跨实例合并。instance 的 runtime ownership 需要受测连接 profile 和安装记录验证。initialize 不提供通用、可靠的进程身份认证；不能靠响应版本字符串证明两连接属于同一进程。

MVP 禁止同一平台容器地址改绑到不同 SessionRef：解绑后保留地址墓碑，只能重新连接原 session；更换 session 必须创建新 topic/channel。这样迟到的普通消息不能落入另一个会话。binding revision 仍用于同一 session 的解绑/恢复操作和旧按钮失效。

MVP 允许同一 session 在每个已授权平台目标中各有一个绑定；同一 Telegram 私聊内重复绑定直接返回已有 topic。会话执行锁跨这些绑定共享。后续跨平台输入只转发一次，并保留来源；不让机器人回声重新进入 agent。

Workspace 主路径只在对应 host 上解析，不能在 PocketPilot 机器上 canonicalize 远程路径。保存用户录入值和经目标端确认的规范值；没有可靠的只读路径验证接口时要求显式路径配置。执行工作区关联的增改时不调用 agent 任意命令工具。

## 3. Rust 接口

### 3.1 Provider 能力

能力由**受测版本 profile + 当前连接实际结果 + 部署验证结果**共同决定，不能只看 semver。Schema 文件用于构建/诊断，MVP 不在运行时下载并执行未知版本 Codex。

```rust
pub struct ProviderCapabilities {
    pub list_sessions: bool,
    pub history: HistorySupport,
    pub observe_live: bool,
    pub join_running_without_restart: bool,
    pub steer: bool,
    pub multi_client: bool,
    pub survives_client_disconnect: bool,
    pub correlation_ids: bool,
    pub request_observation: RequestObservationSupport,
}

pub enum HistorySupport { Paginated, Unavailable }
pub enum SendMode {
    Continue,                      // 当前上下文或下一轮；由应用服务解析
    SteerExact { turn: TurnId },    // 不得默默跨 turn
}

#[async_trait::async_trait]
pub trait AgentProvider: Send + Sync {
    fn capabilities(&self) -> ProviderCapabilities;
    async fn list_sessions(&self, query: SessionQuery)
        -> Result<Page<SessionSummary>, ProviderError>;
    async fn snapshot(&self, session: &SessionRef)
        -> Result<SessionSnapshot, ProviderError>;
    async fn history(&self, query: HistoryQuery)
        -> Result<Page<TextItem>, ProviderError>;
    async fn observe(&self, session: &SessionRef)
        -> Result<SessionSnapshot, ProviderError>;
    async fn create(&self, request: CreateSession)
        -> Result<SessionSnapshot, ProviderError>;
    async fn send(&self, request: SendRequest)
        -> Result<SendReceipt, ProviderError>;
    async fn release_observation(&self, session: &SessionRef)
        -> Result<(), ProviderError>;
    fn take_events(&self)
        -> Result<futures::stream::BoxStream<'static, ProviderEvent>, ProviderError>;
}
```

`take_events` 是实例级单消费者事件源，第二次调用明确失败。创建 stream 后再列/观察 session，避免事件丢失；事件汇聚器按 session 分发。断线在同一 stream 中产生连接事件，连接 supervisor 负责重建底层 transport；不会偷偷创建新 agent 服务。

`observe`、snapshot、history 全部只读，不隐式 resume；默认状态/历史轮询。只有受测协议提供真正无加载副作用的订阅，才可作为观察优化。`release_observation` 只删除本地观察登记，不发送 thread/unsubscribe。当前协议没有 atomic loaded-only resume，不能通过 loaded/list 后再 resume 伪装只读附着。

send 的续轮路径可在用户明确动作内 resume 未加载历史，再发 turn/start；不能在程序启动、重连、reload 或恢复积压操作时执行该准备。运行中 steer 取得 active turn ID 后直接发给同一实例，不能为了订阅先冷恢复。创建返回的合法订阅可使用，但不能由重启触发重新创建。

接口**没有 stop、interrupt、kill、rollback、fork、archive 或服务重启**。create 必须使用登记 Workspace 和明确的新会话配置；不能用它实现 resume。现有会话的 SendRequest 不含 model/cwd/sandbox 覆盖字段。

Provider 接口没有 answer/approve；服务端 request 只解析、显示和等待原端处理。MVP 只允许创建、续轮、steer、只读查看四类业务动作。

### 3.2 Provider 事件与错误

事件至少包含 instance、connection_epoch、session、provider 原始对象 ID 和 observed_at：

```text
ConnectionChanged { connected / recovering / offline / incompatible }
SessionObserved { state, active_turn_id?, freshness }
TextItemObserved { turn_id, item_id, role, phase?, text, final }
TurnEnded { turn_id, completed / failed / interrupted }
UserRequestOpened { scoped_request_id, kind, safe_summary }
UserRequestResolved { scoped_request_id }
HistoryGapDetected { scope, reason }
CapabilityChanged { restricted_reason }
```

错误区分 `Unsupported`、`NotAuthorized`、`NotConnected`、`StaleObservation`、`TurnConflict`、`NotSteerable`、`DefinitelyRejected`、`OutcomeUnknown` 和服务端错误。业务层不得对所有错误统一 retry。

Codex wire enum 接受未知扩展并保留安全诊断信息，未知事件不让全服务 panic；未知运行状态立即变为 Unknown，不能映射 idle。新增只读字段可忽略，但审批类型、执行语义等未知行为必须限制控制能力。

### 3.3 集成接口（当前实现）

```rust
#[async_trait::async_trait]
pub trait Integration: Send + Sync {
    fn name(&self) -> &'static str;

    async fn run(
        &self,
        context: Context,
        stop: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), IntegrationError>;
}
```

Context 提供共享 ConfigFile、Store 和 RuntimeRegistry；Telegram、Discord 和 Slack 均实现 integrations::Integration，由 app::runtime 同一条装配路径启动。run 拥有平台收发任务，stop 只退出桥接工作。平台不能把 stop 翻译为 agent 生命周期操作。

按钮菜单/topic 和 slash/category/channel 的交互差异保留在平台内部；统一的是启动、退出、状态依赖、会话锁、AgentProvider 与操作结果语义。没有为了统一外观强行设计通用整数地址或按钮 API。未来 Lark 实现同一 Integration，Claude shim 实现 AgentProvider。

### 3.4 RpcTransport

传输层只连接、收发 wire 消息和报告连接终止。Codex adapter 拥有 JSON-RPC dispatcher，负责 ID、pending map、超时、双向请求、版本 profile 和事件解析。

发送器使用强类型 `AllowedCodexRequest`，且按上下文分为只读 ObserverPermit 与用户动作 MutationPermit；不能只靠一张全局字符串白名单。观察许可只有 initialize、thread/list/read/loaded/list 和当前分页历史 API。thread/start、thread/resume、turn/start/steer 只能在明确的创建/续轮/steer operation 内使用；后台启停/重连/reload 不持有 MutationPermit。审批回复和 thread/unsubscribe 不在许可内；服务端 request 仍解析并显示，不能自动返回取消/错误决议。

禁止路径包括 `turn/interrupt`、thread rollback/revert/delete/archive/fork/compact、backgroundTerminal terminate/clean、process kill、shellCommand、command/exec、写配置/写文件、daemon stop/restart、给 agent PID 发信号。程序不提供原始 RPC 透传命令。

## 4. 运行时与任务所有权

每个 provider instance：一个 ConnectionSupervisor、一个读循环、一个有界写队列、一个 RPC dispatcher 和事件归一化器。SSH 每个 host 复用连接；不同 instance 的 agent 会话不按工作区重复初始化。

每个已绑定 session：一个 actor，序列化 PocketPilot 的发送、状态核对和 binding 变更，观察审批/问题状态。等待 RPC 的过程中继续接收事件与服务端 request；不能拿全局 Mutex 等网络。

取消 token 分两级：PocketPilot 服务级和**本地连接任务级**；都不能转换成 agent interrupt。SSH/proxy 子进程是桥接器所有，daemon/agent 子进程不是。退出可以回收本地 proxy，但必须先确认连接消失不会触发任务取消的受测保证。

队列原则：

- 核心 user/agent 文本、请求与决议落盘，不静默丢弃。
- delta 可合并为 item 最新文本；状态卡更新允许合并。
- stdout/stderr 工具事件在解析层过滤，不持久化海量工具正文。
- 数据库异常/磁盘满：停收新的控制操作，保持读取连接并尽力保留必要通知；报警，不向 agent 发 cancel。
- 不让 Telegram HTTP 限速阻塞 app-server 读循环。

## 5. 状态模型与一致性

不使用单一 running 字段。保存四组正交信息：

| 维度 | 值 |
| --- | --- |
| 连接 | Connected / Recovering / Offline / Incompatible |
| thread runtime | NotLoaded / Idle / Active(flags) / SystemError / Unknown |
| active turn | turn ID、kind（若可得）、status、观察时间 |
| 最近 turn 结果 | Completed / Failed / Interrupted / None |

运行状态仅服务端权威。Telegram 发送成功不能把状态直接改成 active；RPC 接受回执也只更新操作状态，再由响应快照/事件收敛实际 runtime。

Freshness 保存 observed_at 和连接 epoch。推荐新鲜状态窗口 15 秒，绑定会话定期低频核对（默认 30 秒，带 jitter，按实例限流）；发消息前主动核对，必要时获取 live turn 快照。超过窗口可显示“最后状态”，控制需要最新核对，不把 silence 当完成。

快照/事件竞态处理：每次重连提升 epoch，旧 epoch 事件作废；观察前建立事件缓冲（如连接有合法订阅），否则使用只读轮询核对。用户动作中 resume 的响应与通知按受测版本顺序合并；发现冲突重新读状态/活跃 turn。没有通用服务端全局 revision，不能用本地 receive time 或 updatedAt 当强一致版本号。历史 item 通过 ID 与完成标志合并，completed 文本优先于 delta。

活跃 turn ID 优先来自受测版本的只读 live turns API；合法既有订阅的 turn/started 或用户明确续轮内 resume snapshot 可作补充。不能为取得 ID 在后台 resume；metadata-only active 标记不保证带 turn ID。拿不到 ID 时显示“运行中，暂不能追加”，不猜 ID、不用时间最新的已完成 turn 代替。

## 6. 持久化

SQLite WAL、foreign_keys=ON、busy_timeout，短事务；单机单写者部署。原型期只维护根 schema.sql，幂等初始化不执行迁移器；已有列的升级需开发者备份后显式处理。数据只存 PocketPilot 目录，不修改 Codex DB/rollout。根目录单一 TOML 管普通设置、host、授权范围和内联 token，不支持 include；provider/workspace 由 catalog 持久化。配对/撤销由 CLI 管理 SQLite；没有 chat/user 配置允许列表。DB 管配对、派生索引、绑定与投递。配置 ID 是稳定 UUID。notify + arc-swap 热更新及原子授权检查见 [部署与配置](connections-and-config.md)。

建议表：

| 表 | 主要字段/约束 |
| --- | --- |
| hosts | id、display_name、transport_profile_ref；不存私钥/token |
| provider_instances | id、host_id、kind、endpoint_ref、tested_profile、ownership_verified_at |
| workspaces / workspace_paths | id、instance_id、display_name、primary_cwd；附加 cwd 显式登记 |
| sessions | instance_id + external_thread_id 唯一；workspace_id、title、status、last_observed_at、epoch |
| bindings | platform、tenant/chat、container/topic、session_key、revision、mode；容器地址唯一 |
| inbound_updates | bot_instance + polling_epoch + external_update_id 唯一；stable_event_key、payload、platform_sent_at、ingested_at |
| platform_offsets | bot_instance、polling_epoch、last_durable_update_id、last_update_received_at、last_update_generation_lower_bound、cursor_mode |
| operations | operation_id、stable_event_key/action_nonce 唯一、session_key、binding_revision、connection_epoch、mode、expected_turn_id、state、receipt/error |
| text_items | instance/thread/turn/item 唯一；role、phase、canonical_text、final、client_id |
| deliveries / outbox | binding、item、render_revision、chunk_index 唯一；payload、state、next_attempt_at、platform_message_id |
| pending_requests | instance + epoch + rpc_request_id；session、turn、kind、observed_at、resolved；只观察 |
| ui_actions | opaque nonce、authorized user/chat、binding revision、operation scope、expiry、used_at |
| provisionings | operation_id、desired session/topic、known external IDs、state/unknown_reason |

Telegram chat ID 使用 i64；topic/message ID 用框架类型边界检查。Discord snowflake 在统一持久层存字符串或无符号安全编码，避免有符号溢出。JSON-RPC request ID 保留 string/integer 类型，不统一 parse 成整数。

SQLite 中的文本仍可能含源码和个人信息。目录权限限制本机用户；日志不复制正文。默认保留未投递内容，已确认文本缓存可按 30 天配置清理，映射/操作幂等记录按独立策略保留。不能因缓存清理自动从历史再次推送旧消息；保留投递水位与稳定 ID 墓碑。

## 7. 入站操作：不重复控制 agent

Telegram long polling 由一个受控 poller 负责：接收批次 → inbox + offset 同事务持久化 → 下一次 getUpdates 才使用更高 offset。不能直接假设框架 dispatcher 的内存 offset 提供持久化确认。重复 update 命中唯一键，不再执行副作用；处理完与否不决定是否可确认网络 update，**落盘**才决定。

Telegram 在至少一周无新更新后，下一 update_id 可以重新随机选择，不能把 offset 当永久递增计数。不能从“本地收到更新”起计该窗口，因为更新可能已在服务端等待近 24 小时并经历传输延迟。记录产生该批次的请求开始时间，以“请求开始时间减 24 小时、再扣时钟裕量”作为最近更新生成时间的保守下界；只在这个下界起 5 天的保守窗口内使用已持久化 offset。超过窗口、时钟不可信或恢复时无法确认 cursor 时代，立即切换成不传 offset 的 polling，直到拿到新批次。批次落盘后在新的 polling epoch 中建立 cursor，不能继续使用历史最大值，否则可能确认掉较小的新 ID。不使用负 offset 丢弃队列。普通 message 的 stable_event_key 用 bot/chat/message/kind，callback 用 callback_query.id，副作用按该稳定来源去重，避免更换 polling epoch 后重放同一操作。游标重建和跨 epoch 去重需要专门实测。

普通文本在 dispatch 前再次校验 binding revision、epoch、状态和消息年龄。进程启动/网络恢复前产生的消息、经过离线区间的已保存输入，以及超过最大输入等待期的消息（建议初值 120 秒）只保存草稿，必须重新确认。平台时间以秒计时，切换边界含同秒消息保守进入确认；时钟明显不可信也要求确认。接收时已捕获 active turn ID 的操作不得跨 turn 自动转投；状态变化按 steer 冲突处理。已确认为 idle 的普通 Continue 输入保留已说明的 start-or-steer 竞争语义。

操作状态机：

```mermaid
stateDiagram-v2
  [*] --> Received
  Received --> Validated
  Received --> Rejected
  Validated --> Dispatching
  Dispatching --> Accepted
  Dispatching --> DefinitelyRejected
  Dispatching --> OutcomeUnknown
  OutcomeUnknown --> Accepted: 找到匹配的 clientId/权威回执
  OutcomeUnknown --> NeedsUserReview: 证据不足
```

RPC 写入前事务记录 Dispatching。进程崩溃遗留 Dispatching 按 OutcomeUnknown 处理，不能重置为未发送。Accepted 表示输入已接受，不表示 agent 工作已完成。用户输入原文保留在 operation 以便核对；clientId 可关联 platform message，避免回声。

对确定尚未写入 transport 的失败，可以安全重试；已写但未知的失败不得自动重试。服务明确拒绝 overloaded 等错误是否保证未执行，由受测错误分类确定；只读请求指数退避重试。精确 steer 冲突保留原文本和 turn 条件供用户重新选择。

## 8. 出站文本与 Telegram 一致性

`TextItemObserved` → upsert 权威文本 → DeliveryAggregator 形成批次/历史集合 → 同一事务封存成员和 outbox。Sender 独立限速投递，成功保存 message ID。按 session/turn/item 序列展示，外部 agent 没有可证明全局顺序时按服务事件/历史顺序保持，不用 ID 字典序假定时间顺序。message_batches/batch_items/history_bundles 与超长单附件规则见 [历史与投递](history-and-delivery.md)。

本 topic 用户消息已经在 Telegram 可见，匹配 clientId 后只补“已送达”状态，不再原样复读。CLI/desktop 发来的 userMessage 加来源标签转发；未知来源标为“Codex 用户”，不凭文本相同判定回声。多平台绑定时只排除原始输入位置，其他授权位置仍可同步。

默认等 item/completed 后发送完整文本，保留每个可见 item；可选低频 draft 编辑以后再开。断线补发通过 `(instance,thread,turn,item,revision,chunk)` 去重；历史 revision 变化更新既有可编辑消息，无法编辑则发送明确更正，不冒充新消息。

delta 的最新观察值也需保留为未完成 item。收到 turn 结束但 item 未 completed、服务退出或断线超过核对期限时，先查询历史权威值；无法补齐则发送已知文本并标“未完成，截至某时”，以后获得 completed 再更新同一 delivery。不能为等 completed 永久隐藏已见文本，也不把本地 partial 冒充权威最终回答。

Telegram sendMessage 没有本文可依赖的应用层幂等键。网络超时可能“已经发出但没拿到 ID”，因此只能做到本地去重 + 对已知成功幂等。结果未知的发送进入 DeliveryUnknown，状态卡提示可能缺失；不无限重试制造重复。用户可显式重投并看到“可能重复”的说明。已知 429 等明确拒绝可遵循 retry_after 重试。

create topic、create thread 也不是跨系统事务：使用 provisioning saga 保存每步已知 ID；重复按钮只查询同 operation。未知创建结果不再自动 create，提示核对孤立资源。MVP 不依赖 Telegram 提供完整 topic 枚举接口；管理员可以手动重新绑定已知 topic ID。

## 9. 重连与故障恢复

连接状态：Disconnected → Connecting → Initializing → Reconciling → Ready；任一步失败进 Backoff。采用 backon 全抖动指数退避，起始约 1 秒、上限 60 秒；运行一段稳定时间后重置。认证失败/host key mismatch/不兼容是需人工处理，不频繁重试。

重连流程：

1. 增加 epoch，所有原状态标 stale；原连接在途写操作标 OutcomeUnknown。
2. 重新连接 proxy/endpoint，initialize；只建立一个受控读循环。
3. 恢复当前配置仍允许的已绑定 session 的只读观察；不 resume、不 unsubscribe、不启动新服务。没有无副作用订阅时使用状态/历史轮询，不把重连当作续轮授权。
4. 核对 active turn、状态、pending requests；只保留新 epoch 的请求观察，不发送 answer/approve/decline/cancel。
5. 从分页历史重叠读取上次末尾及当前 active turn 的 items，upsert 去重；历史水位不能只用时间戳。
6. 若无稳定 cursor/历史可能变化，从较早锚点重新分页并按 ID 去重；不能假设翻页只追加不更新。
7. 无法证明断线区间连续性时就生成“可能遗漏消息”的 gap 提示，不要求先找到具体缺项；即使历史回补没发现缺项也不能显示完整覆盖。所有 gate 满足后才恢复新输入，积压旧输入仍需确认。

SSH keepalive用于探测连接，无法保证服务永不中断。丢失 SSH 连接只重建 proxy；不杀 daemon、不重启 Codex、不 fork。监督器只回收自己启动的连接进程。服务端真的退出时显示离线并保留未确认操作，等待人工修复。

## 10. 扩展边界

Claude 后续可通过远端 Rust shim 兼容 Codex wire 子集，保留不同 provider 身份与 capabilities；不保证完整 Codex Desktop 兼容。没有真正 steer 的 provider 返回 Unsupported，不以“停止再 resume”模拟。独立 daemon/executor、CLI stream-json、ownership 和 EOF 边界见 [后续扩展](../future/integrations.md)。

Discord adapter 把 workspace 映射为 category、session 为 text channel，使用同样的 inbox/operations/outbox。平台拥有能力多不代表 provider 可停止；管理员权限不绕过应用的 host/workspace 授权。

Lark adapter 使用 region/tenant/chat/thread/root-message 地址、根状态卡和线程回复；能力和 Rust 生态取舍见 [后续扩展](../future/integrations.md)。SSH 使用系统 OpenSSH 加受限 askpass/TOTP，传输超时逐 host 配置；具体认证与热更新边界见连接文档。

## 11. 日志与诊断

统一 tracing，tracing-subscriber JSON 输出，config 的 logging.filter 控制级别和热更新，不依赖 RUST_LOG；token 仅从根 TOML 读取；第三方 log 桥接到 tracing。span 字段含 operation_id、instance_id、内部 workspace_id、thread/turn/item ID、epoch、config_generation、method、耗时、结果分类。用户/平台 ID 可哈希化。TOTP seed、动态 code、密码、secret config 路径/正文和实际展开认证响应不进入日志。

禁止默认记录 token、Authorization、完整 URL（Telegram token 在 URL path）、SSH 命令展开后的秘密、全文、配置内容、原始 RPC body、工具输出。错误显示结构化错误码和经清理摘要，不直接打印 reqwest/teloxide 的含 token URL。DEBUG 也必须脱敏。

诊断包含重连次数/原因、状态年龄、队列深度、未决审批数、OutcomeUnknown 数、outbox 最老等待时长、429 和投递失败。MVP 通过日志和 CLI doctor/status 展示；以后再加 metrics endpoint。health “进程活着”与 “provider 可控制”分开报告。


## 标题元数据的新增许可

按后续明确需求，session::Mutation 增加 SetTitleIfMissing，Codex 从 build.rs 导出的 ThreadSetNameParams/Response 调用 thread/name/set。仅显式新建、打开或重命名时，在 cwd/绑定/权限与新鲜度检查后尝试；执行前再次读取原生 name，非空时保留原名。不发送用户输入、不 resume、不启动或中断 turn。官方 API 没有 compare-and-set 条件，已有标题检查与写入之间仍可能遇到原客户端并发改名；不声称跨客户端原子性。后台观察、进程启停和重连没有此操作许可。

Telegram 的 presentation/markdown/presence/i18n 是独立平铺模块。Appearance 元数据通过持久投递队列、合并与限速，typing 是独立短期 lease；二者均不能调用 AgentProvider::execute。消息正文的 Markdown 与管理菜单 MarkdownV2 是不同的渲染入口。

## 公共导航

聊天界面的 workspace 表示执行环境/主机（Host），project 表示该环境内的项目目录（持久化 Workspace），session 位于项目内。engine::navigation 是 Slack/Discord 的公共目录选择模块，集中处理归属校验、排序、歧义拒绝与分页；平台模块只生成原生控件或命令。Discord /pp 使用私有的原生 select/buttons，逐层更新同一菜单；持久动作句柄验证 owner、服务器、channel、配置和有效期，UI 不展示内部 ID。session 查询、操作账本、会话 actor 和 provider 连接仍由现有 engine/session 模块共享。


## 公共正文与平台显示

session::markdown 以 pulldown-cmark 为语法解析器，集中提供有界文档结构、中文粗体兼容、安全链接、表格降级和纯文本 fallback；Telegram 复用输入解析，保留已有 HTML 渲染。Slack 把公共结构映射为 SDK 的普通 rich_text 元素，Discord 映射为其 Markdown 子集。平台菜单、状态和错误使用字面文本入口，不把动态标题作为正文语法。RuntimeStatus::text_in 提供共同的无图标状态文案，图标由平台入口决定。

原始 Markdown 与显示文本分别保留；文字按文档结构分段并分别渲染，显示超限时继续分段，不能保真的语法保持字面内容；不提供历史导出或自动生成历史附件。UTF-16 长度与公共预览的 grapheme 边界分别处理。Slack 顶层 fallback 禁用标记解析，Discord 所有消息和编辑保留 allowed_mentions；格式优化不改变 outbox、未知结果、权限或原生操作语义。能力、验证范围和第三方库选型见 [Markdown 与 emoji](../research/chat-formatting.md)。

入站文件通过公共 Incoming 引用、下载预算和 Uploaded 路径上下文进入会话。平台只负责解析文件引用和通过自身代理/凭据下载；engine::uploads 生成所选执行账号的私有附件路径，Codex adapter 以类型化 fs/createDirectory 与 fs/writeFile 写入文件。每次上传是明确用户动作的独立持久 operation，后台观察不能上传；图片再作为 LocalImage 输入，其余文件以路径上下文送入。过期输入的确认保留文件引用，未知写入不重发。
