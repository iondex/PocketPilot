# 04 · MVP 实施与验收计划

## 1. 范围与完成定义

完整 MVP 是：授权用户在 Telegram 私有 forum 群中选择已登记工作区，列出会话，绑定或新建 thread；可看到所有绑定后产生的可见用户/agent 文本、当前运行状态，并能向运行中的 turn 发送 steer。默认本机运行，聚合允许的本地与多个 SSH 实例；SSH desktop/CLI/PocketPilot 连接同一远端 daemon。支持慢 host、配置 TOTP 和 host/workspace 热更新，PocketPilot 生命周期不触发任何 session 写动作。

MVP 不包含 Claude shim、Lark、Discord、网页后台、语音/图片/附件输入、任意 shell 操作、自动修改 Codex 配置、自动发现所有桌面项目、自动安装或重启 Codex daemon。

不得把“独立进程恢复相同历史”作为三端同时使用的验收。核心门禁失败时只交付受限只读诊断，完整 MVP 状态仍为未完成。

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
| 普通 SSH | openssh | 0.11.6 | 545 万 | 复用系统 OpenSSH、ssh_config、认证、转义与 multiplexing |
| OTP SSH | russh / totp-rs | 0.64.1 / 6.0.0 | 841 / 1128 万 | keyboard-interactive、多阶段认证与 TOTP |
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

SSH 选型采用双后端：openssh 负责普通 key/agent 和复杂 ssh_config 兼容；配置 OTP 时使用 russh 原生 keyboard-interactive + totp-rs，不能仅依赖 OpenSSH BatchMode。直接读取本机 SSH config/Include 发现 alias，受控 ssh -G 获取有效参数；实际认证链由服务端协商。慢 host 的超时、心跳和认证退避逐台配置，详细规则见 [06](06-deployment-config-ssh.md)。

JSON-RPC 不直接套标准 jsonrpsee server/client：Codex wire 省略 jsonrpc 字段且存在双向服务端请求和特殊 transport。使用 serde 的小型、可测试 dispatcher 对接受测方法；不要为了复用框架改变 wire。Codex 源码的内部 workspace crate 不直接作为 git 依赖引入整个 agent 引擎。JSON Schema 用于协议对照和 fixture 校验；实施时检查可用的生成工具能否处理该 schema，不能声称全部 Rust 类型已可无损自动生成。

## 3. 推荐源码布局

初期采用一个 Cargo workspace、四个成员，避免每个小功能一个 crate：

```text
PocketPilot/
  Cargo.toml                     # workspace + shared dependencies
  Cargo.lock
  rust-toolchain.toml
  crates/
    pocketpilot-core/
      src/
        lib.rs
        domain/                  # IDs, workspace, session, state, text, request
        ports/                   # AgentProvider, ChatPlatform, Store
        application/             # list/bind/create/send/unbind
        runtime/                 # SessionActor, recovery, inbox/outbox rules
    pocketpilot-codex/
      src/
        lib.rs
        provider.rs
        wire/                    # permitted RPCs, DTOs, profiles, parser
        rpc/                     # dispatcher, request IDs, deadlines
        transport/               # proxy, ssh_proxy, websocket; ssh connector backends
        normalize.rs
        reconcile.rs
    pocketpilot-telegram/
      src/
        lib.rs
        polling.rs               # persist before advancing offset
        authorization.rs
        commands.rs
        menus.rs
        topics.rs
        rendering.rs
        rate_limits.rs
    pocketpilot/
      src/
        main.rs
        config.rs                  # snapshot/writer/hot reload
        telemetry.rs
        cli.rs
        storage/                 # sqlx-backed Store implementation
        bootstrap.rs
  migrations/
  config/pocketpilot.example.toml
  tests/
    protocol/                    # synthetic/anonymized fixtures
    scenarios/                   # concurrency, crash, reconnect
  docs/
```

依赖方向：codex/telegram 依赖 core；core 不依赖 teloxide、Codex DTO、sqlx 或进程 API；可执行 crate 组合 adapter 和存储。Store 只暴露应用事务所需操作，例如 ingest_update、reserve_operation、persist_items_and_deliveries；不要为每张表定义一个泛型 repository trait。

后续增加 pocketpilot-claude-shim、pocketpilot-lark 与 pocketpilot-discord，不改 core 的会话地址、操作不确定性和不中断规则。不存在实现前就搭建通用插件注册中心的必要。

## 4. 配置与 CLI 草案

所有设置来自 TOML 和本机 SSH config，不读取 .env、不使用 token_env 或 RUST_LOG。完整 example 字段、OTP seed、include、hot reload、ConfigWriter 和生命周期白名单统一见 [06](06-deployment-config-ssh.md)。host/workspace 只在配置中授权，DB 是运行投递与绑定存储，不是第二份配置。

只支持实施时最新且通过测试的 Codex。可以在独立维护步骤升级 CLI；不能在启动/重连/reload 中升级并重启活跃服务。

CLI 草案：

| 命令 | 用途 |
| --- | --- |
| `pocketpilot run --config …` | 启动桥接器；不启动/重启 agent 服务 |
| `pocketpilot doctor` | 检查配置、版本、socket/proxy/endpoint、Telegram 权限和 gate |
| `pocketpilot status` | 连接、状态新鲜度、积压、待核对操作，不读取工具输出 |
| `pocketpilot workspace add/list` | 本地管理登记，Telegram 菜单使用同一应用服务 |
| `pocketpilot bindings list` | 查平台映射和 orphan/suspended 状态 |
| `pocketpilot operations inspect ID` | 核对不确定结果；输出默认不含全文 |

不提供 `raw-rpc` 或 daemon restart 命令。doctor 默认只读；行为验收在隔离测试服务上显式运行，不在用户活跃会话上发实验提示词。网络诊断也不能暗中设置 webhook/删除 webhook 或修改 agent 配置。

## 5. 实施阶段

### P0 · 兼容性与共享实例验证（最高优先级）

产物：版本 profile、隔离探针、匿名 protocol fixtures、三端部署说明和验证报告。

1. 选定本地/远程 Codex 服务版本，获得该二进制的 schema；记录安装方式和 transport。
2. 在隔离服务验证 initialize、list/read、warm resume、active turn ID、steer、服务端请求。
3. 验证 desktop/CLI 如何连接同一实例；新增 thread 在实际桌面出现且能继续。
4. 分别在 running、waitingOnApproval、waitingOnUserInput 时断开 PocketPilot，包括最后订阅者情形。
   同时验证只读 API 的 live 状态/active turn/文本覆盖；不靠后台 resume 获得订阅。验证启动、退出和热更新不会发出修改 session 的方法。
5. 查明 proxy/SSH 生命周期、history pagination、clientId 回显与 write unknown 行为。

退出标准：不中断、共同运行和权限保留通过。否则修正接入策略或限制只读；不继续堆 UI 掩盖不可行性。本机当前 socket 不存在且 binary 不同，不能跳过这一阶段。

### P1 · Rust 基础与只读浏览

建立 workspace、tracing、配置、SQLite 迁移、错误类型、fake provider；Codex proxy/SSH/WS、握手、双向 dispatcher、方法白名单和版本 gate；实现手动登记、分页 list、只读 history/status。

退出标准：只读浏览不 resume；支持路径过滤、游标、未知枚举与断线 stale 状态；日志没有 token/正文。

### P2 · Telegram topic 与映射

实现授权、持久化 polling inbox、控制菜单、工作区登记、topic 创建 saga、状态卡和标题、重复绑定返回已有 topic、历史摘录和基本 outbox 限速。

退出标准：未授权不能控制；重启不丢 binding；重复 update/callback 不创建双 topic；普通群消息可见；同群多 topic 共用预算。

### P3 · 继续、新建与实时同步

实现 per-session actor、用户文本与来源关联、用户动作内 thread/start/resume、turn/start/steer、active turn precondition、OutcomeUnknown、最后输入/最终输出、活跃全历史单集合、短时合并、历史核对与 gap 提示。

退出标准：同 thread 多端持续可见；正在执行的轮次被 steer 后不变化/重启；用户一条消息只发生一次确定的远程操作；未知结果不自动重试；文本不只保留最终回答。

### P4 · 状态观察、恢复、OTP 与热更新

实现审批/问题只读状态和跨端 resolved 观察、unknown request 降级、离线/重连、topic 关闭/删除、OTP 挑战与慢 SSH、host/workspace 原子热更新、服务退出、消息合并与积压可见性。不发送任何审批/RPC answer。生成 macOS/Linux 运行说明与 service 示例，重点说明 agent daemon 预先独立运行。

退出标准：全部核心场景验收通过；没有发出禁止 RPC/生命周期操作。单实例运行用进程/文件锁防止两个 bot poller 消费同 token；滚动重启不启动第二个写者。先支持 Linux x86_64 与 macOS arm64，其他组合按 CI/实测补充。

### P5 · 后续扩展

Lark/Discord 可独立实现 adapter（thread/card 或 category/channel、权限、事件、限速），Claude shim 单独验证独立 executor 与兼容子集。候选库和 gateway 见 [07](07-future-adapters.md)。若 Claude 无法安全共享活跃 session，产品明确限制支持范围，不在 core 中模拟“停止再继续”。

## 6. 核心验收矩阵

这些是实施时需要执行的测试/实测；当前文档任务没有运行它们。

| ID | 场景 | 通过标准 |
| --- | --- | --- |
| A01 | 列表/历史浏览 | 无 resume/start/steer；原任务 turn ID、策略不变 |
| A02 | desktop 正在运行，Telegram 绑定 | 只读观察同一 thread/turn；未出现 interrupted；无 resume/重建运行对象 |
| A03 | Telegram 向 active turn 发文本 | turn/steer 带真实 expectedTurnId；当前 turn 接受文本，其他端可见 |
| A04 | active turn 恰好结束/更换 | 明确冲突，保留文本供选择；不自动发给另一个 turn |
| A05 | 空闲发送 | 开始下一轮；cwd/model/审批/sandbox 不被覆盖 |
| A06 | 同时 CLI/desktop 输入 | 无 PocketPilot 自行取消；turn/start 的实际竞争语义被验证/正确显示 |
| A07 | PocketPilot/SSH 在生成中退出 | 服务仍存活并继续任务；无 cancel/interrupt/daemon restart |
| A08 | 最后连接在审批/等输入时断线 | 不因桥接器退出自动拒绝或取消；否则该版本不具备控制资格 |
| A09 | RPC 发出后响应丢失/本地崩溃 | OutcomeUnknown；重连核对，不自动重复输入 |
| A10 | 同一个 update/callback 重复 | 单 operation/单 provisioning；已知 ID 被复用 |
| A11 | bot createTopic/threadStart 结果未知 | 不再次自动创建；显示需要资源核对 |
| A12 | tool 调用与 commentary 交替 | 每条可见文本按 item 保留，工具/stdout/reasoning 不转发 |
| A13 | 中途绑定/掉线补历史 | 不漏掉可恢复文本；stable item ID 去重；不可恢复缺口有提示 |
| A14 | 长中文、emoji、代码文本 | 无 UTF 截断/解析失败/正文丢失；合法分片 |
| A15 | 同群 5 个 topic 突发消息/429 | 共享限速、不阻塞 agent 读循环、retry_after、可见积压 |
| A16 | Telegram 发送后丢响应 | DeliveryUnknown 不无限重发；不承诺 exactly-once |
| A17 | 桌面处理审批/问题 | PocketPilot 仅更新观察状态，没有 approve/decline/cancel/answer RPC |
| A18 | 未授权/伪造 callback/匿名 sender | 不能列敏感工作区、发送消息或批准请求 |
| A19 | topic 关闭/删除、bot 被踢 | 只影响映射/投递；agent 无归档/停止，积压可恢复 |
| A20 | 非目标版本/未知状态/缺 live turn ID | 以最新受测版本为基线，其他降级；不能猜 idle 或 steer |
| A21 | 新建 Telegram thread | 同 provider thread ID 在 desktop 与 CLI 可见、可继续 |
| A22 | 现有 thread 策略保持 | 绑定/发送前后 model/cwd/approval/sandbox/指令来源一致 |
| A23 | 禁止动作审计 | fake transport 捕获的 RPC 和子进程命令不含 interrupt/kill/重启/配置覆盖 |
| A24 | 慢平台/SQLite 忙/磁盘满 | 有背压和可见失败，不通过停止 agent 解压，不静默丢关键文本 |
| A25 | SSH host key 更换/认证失败 | fail closed，需要人工处理，不关闭 host key 检查 |
| A26 | 多实例/同 sessionId 的 fork | 不串会话，topic 以 instance + thread ID 唯一绑定 |
| A27 | 离线/重启/长时间排队后的普通文本 | 草稿待确认，不自动送入后来变化的 turn；容器地址不跨 thread 复用 |
| A28 | 旧更新延迟接收，随后一周无生成且 update_id 随机变小 | 用生成时间保守下界提前停用旧高 offset；cursor 重建、跨 epoch 去重成功 |
| A29 | 观察与后台重连期间 thread 卸载 | 不执行 resume；冷加载仅限用户明确续轮动作 |
| A30 | 断线期间可能存在未持久化 commentary | 没有完整连续性证据就显示可能遗漏，不能误报完整同步 |
| A31 | delta 后失败/外部中断，缺 completed | 已观察文本以未完成标记转发，后续权威文本更新，不静默丢弃 |
| A32 | 本机 + 多 SSH 聚合 | 独立 host 超时/队列，分页不丢会话，慢服务器不拖住其余实例 |
| A33 | SSH desktop/CLI/桥接器共享 | 相同远端用户/socket/daemon，默认独立 CLI 不误报共享 |
| A34 | key partial_success 后 OTP | 正确完成 keyboard-interactive，多 prompt 规则和数量匹配 |
| A35 | TOTP 临近过期/慢认证/未知挑战 | 临发送生成、按配置等待新周期；未知挑战不泄密，失败次数受限 |
| A36 | SSH config 的 Host/Include/Match/ProxyJump、默认与自定义 -F | 解析和实际连接使用同一配置模式；不把 wildcard 或默认 kbd=yes 当 OTP 证据 |
| A37 | config 无效/并发编辑/原子替换 | 保留最后有效配置；ConfigWriter CAS；无部分授权快照 |
| A38 | 热删除 host/workspace/user/chat | 已排队输入/输出重新授权并阻止；远端任务不变 |
| A39 | OTP seed/SSH config 轮换 | 新认证用新值；原有效连接不强制断开；日志无 seed/code |
| A40 | endpoint/user/socket 更换 | 新实例身份，旧 binding 不自动指向新服务 |
| A41 | 新绑定空闲 thread | 返回最后用户输入/同轮 final；无 final 明确提示，不错配上一轮 |
| A42 | 进入活跃 thread 全历史 | 只读分页、一个集合；超长一次附件，无逐条历史刷屏 |
| A43 | 单附件仍超过平台上限 | 一条明确失败/主动分片入口，不截断或宣称完整发送 |
| A44 | 短时 3 条以上文本/持续高频 | 合并保留正文与顺序，有最大等待，不跨 session 合并 |
| A45 | batch 崩溃/回执丢失、历史附件失败/超限 | 成员/投递事务一致，不重复分组；历史待核对可见且不永久阻塞后续实时输出 |
| A46 | 程序启停/崩溃/reload/重连 | ObserverPermit 无 resume/start/steer/approval/unsubscribe；进程/EOF 不杀 agent |
| A47 | 最新 CLI 升级 | 维护步骤记录实际 daemon/schema；不自动重启活跃会话 |
| A48 | 全部 config 模式 | 无 .env/业务环境变量依赖，秘密配置不进入日志或仓库 |


## 7. 测试层次与 CI

单元测试只覆盖有业务意义的规则：授权、方法白名单、状态/epoch、steer 冲突、unknown-write 状态机、回声关联、Unicode 分片和限速公平性。

协议测试用 synthetic fixtures + fake bidirectional transport：握手、响应先后、服务端 request、超时后迟到响应、重复通知、未知字段、重连重放请求。记录 fixture 对应 schema/version；不能把抓到的真实用户对话/token 放入仓库。

集成测试用临时 SQLite、可控时钟和 fake HTTP/agent，覆盖 inbox-offset 原子性、崩溃遗留 Dispatching、outbox、创建 saga、unique constraint 和 binding revision。安全行为要检查实际 outbound 方法/进程，不仅检查 UI 没有 stop 按钮。

真实环境验收在专门 test workspace + Telegram 测试群执行：Linux/macOS 服务、SSH 丢包/断线、desktop/CLI 同实例、审批并发。允许正常完成或由其他客户端明确中断，但 PocketPilot 不发中断。测试不污染用户正在工作的线程。

CI 建立后运行 fmt、clippy（warnings as errors）、workspace tests、必要的协议 fixture 检查和 Linux/macOS build。Windows 尚未完成双后端/配置/路径验收，不在 MVP 承诺内。cargo-audit/deny 可在引入依赖后加入维护检查，不能以下载量代替维护判断。

## 8. 运行与维护

PocketPilot 作为 macOS launchd 或 Linux systemd 的独立服务，可自动恢复自身；Codex daemon 用其已支持的方式独立管理。桥接器 service 的 stop/post-stop 不得 pkill Codex 或关闭 SSH 上的 daemon。测试退出路径只回收本桥接器资源。

更新流程：先锁定二进制/profile → 隔离兼容测试 → 更新依赖锁与文档 → 才启用真实控制。遇到未知 Codex 版本默认只读，不自动下载/替换桌面内置运行时。SQLite 备份需要 WAL 一致性（backup API 或受控 checkpoint），不直接只复制主 db 文件。

运维重点：状态 freshness、重连次数、审批积压、未知写操作、最老 outbox 年龄；群消息突发造成延迟是正常平台约束，必须展示。Telegram updates 最多保存 24 小时；PocketPilot 长时间离线后，不能保证 Telegram 上未被接收的输入仍能取回，应提示可能缺失并让用户核对。

## 9. 当前交付与后续决策

当前只交付设计文档和匿名研究快照，没有 Cargo scaffold、运行 bot 或真实三端行为验收。开始实施时默认按 P0 → P4 顺序推进。

用户后续需要提供的部署信息是实际使用的 host/允许路径、Telegram bot token/chat/user ID 和目标 Codex 连接方式；这些不阻塞本次设计完成。若本地 desktop 没有受支持的共享接入，必须报告该局限，不能牺牲“不影响已有运行会话”的约束来补齐表面功能。
