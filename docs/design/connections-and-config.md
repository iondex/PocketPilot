# 部署、配置热更新与 SSH / OTP

本文描述当前实现；真实部署证据见 [验证记录](../research/implementation-checks.md)，操作入口见 [测试说明](../testing.md)。

## 共享运行实例

PocketPilot 是附加客户端，只连接已有服务。本机 Desktop 的入口与每台 SSH 机器上的执行 daemon 是不同实例；本地 endpoint 不自动提供所有远端任务。根配置只允许主机与项目根路径；执行实例和工作区由应用 catalog 动态管理。分别连接后聚合，不扫描登录全部 SSH alias。

SSH 的共享条件是相同远程用户、有效控制 socket 和 daemon。Desktop、CLI remote 模式与 PocketPilot 可以使用各自的 SSH 连接，但必须最终进入同一 daemon。仅共用历史目录不能证明活跃执行对象相同。

本地和远端 Unix socket 均通过官方 `codex app-server proxy --sock` 建立字节隧道，再在隧道上执行 WebSocket 握手和 framing。proxy stdout 不是 JSONL。直接连接支持 loopback WS 或远程 WSS；生产不接受远程明文 WS。64 MiB frame 上限与历史读取预算均有边界检查。

PocketPilot 不启动、升级、重启或停止原生 daemon。缺少共享入口时明确失败，不启动一个共用历史文件的替代服务。当前本地 Desktop 以 stdio 运行且没有公开监听入口，因此既有本地活跃任务尚不能接入；独立本地原生 fixture 的通过不改变这个部署限制。

构建协议使用 Codex 0.162.1，已验证远端 0.161.0 的当前协议。按服务端类型化 historyMode 路由 legacy/full-read 与 paginated/turns-list；这是当前官方协议中的两种历史模式，不承诺任意旧版 CLI 兼容。initialize 开启 experimentalApi 以保留 historyMode 元数据，客户端仍只暴露固定的只读方法和四类写方法。

## 根目录单一配置

普通设置和秘密都在根目录 pocketpilot.toml；不 include、不读取 .env。示例见 [pocketpilot.example.toml](../../pocketpilot.example.toml)。真实配置被 Git 忽略，使用 0600；数据库及 WAL/SHM 也为 0600。相对文件路径以配置文件目录为基准，远端 cwd 保持目标机器的绝对路径。

Telegram 配置只有 token。bot ID 来自 getMe，私聊用户须经本机 CLI approve，无配置 chat/user 允许列表。SQLite 配对申请表只保存一次性码 hash；投递队列为发送而短暂保存明文，进入终态后清除。更换 bot 按 bot ID 隔离账号数据，不继承旧配对。

Discord 是可选编译功能，配置 token 和 guild_id。原型只允许该服务器 owner 使用；category 默认对 @everyone 隐藏，对 owner 和 bot 开放。服务器管理员依旧受 Discord 自身权限规则约束，不能把 category 当成隔绝管理员的安全边界。

密码、key passphrase 和 TOTP seed 均从根 TOML 读取，Debug/日志/错误脱敏。普通设置不存在环境变量覆盖；CODEX_SCHEMA_BIN 仅选择编译期 schema 工具，CODEX_TEST_BIN 仅供隔离原生兼容测试。

## 动态 provider 与 workspace

provider 是一台主机上一个现有 Codex 执行实例，记录检测到的 binary/socket 和只读/控制部署确认；workspace 是关联该实例的绝对项目目录。二者持久化在 SQLite catalog，ID 不因重启改变；不再接受根 TOML 的 providers/workspaces 或 host.codex_bin/codex_control_socket 字段。

启动时读取已有目录；新允许主机由后台有界发现任务处理。SSH 只读探测使用固定 POSIX shell 程序，查找标准安装位置、原生 package/NVM binary 与现有控制 socket，支持根目录符号链接和自定义登录 shell。不会启动 daemon 或用共用历史文件的服务替代现有实例。未找到入口时该主机保持不可连接。

管理 topic 先选择主机，再 /add /绝对路径 可选名称；/remove 名称 只删除入口，不删除远端会话。CLI 的 providers list、workspaces list/add/remove 可在服务运行时使用，变更约两秒内发布为完整快照。CLI providers trust ID 记录共享生命周期验证，--revoke 撤销；新发现的 provider 默认只读，不能继承另一实例的控制确认。

## 热更新的实际范围

notify 监听根配置的父目录，处理原子替换；debounce 和周期 rescan 后完整解析、静态验证，再通过 arc-swap 替换不可变快照。未知字段（包括 include）、重复 ID/短名、错误引用、越界 cwd、无效认证/速率全部拒绝，继续使用最后有效快照。

| 修改 | 当前行为 |
| --- | --- |
| 根配置增删 host / catalog 增删 workspace | 菜单、执行与投递重新检查当前范围；不自动启动服务或任务 |
| 状态中的 workspace.additional_cwds | 精确查询各个允许 cwd，并将分页游标绑定到该路径集合 |
| 显示名、Telegram 合并与速率 | 新操作采用当前配置；持久批次已有固定 flush 截止时间 |
| 密码/seed/认证策略 | 现有连接保持；后续认证读取相同已验证配置 hash |
| 自动检测到其他 socket / 有效远程用户变化 | 原 provider ID 拒绝重定向，须分配新 ID 并重新绑定 |
| Telegram/Discord token、Discord guild、data_dir | 整份候选拒绝，需重启桥接后应用 |
| logging | 启动时装配，需重启桥接应用 |
| OpenSSH config | 发现时读取，重连时重新解析；不监控全部 Include 文件并主动断开有效连接 |

输入捕获配置快照；慢连接完成后、实际写入前重新检查配置、配对和映射。未开始的投递重新授权。已经发出的网络请求不能撤回；结果未知时核对原端，不自动重发。配置恢复不会自动重放旧 agent 输入。Telegram 普通消息超过五分钟、早于本次进程启动、重启时仍 pending 或 poll 失败后补收时，展示待确认草稿；只有新的确认按钮才能发给当前状态，旧 nonce 在重启时失效。草稿绑定 instance/thread/映射 revision；未获明确接受的 turn 竞争也需用户重新确认，未知结果不套用此流程。

## OpenSSH 与挑战认证

发现阶段使用 ssh2-config 提取 ~/.ssh/config 中明确 alias；通配/否定项不展开成待登录主机。真正连接仅使用 TOML 中允许的主机。对允许 alias 调用系统 ssh -G，显式 ssh_config 则使用相同 -F；保留系统 OpenSSH 的 Host/Match、ProxyJump、ProxyCommand、证书和 agent 语义。

当前只使用系统 OpenSSH 后端，backend=auto 和 openssh 均进入同一路径。之前的 russh 双后端是研究方案，没有进入实现。SSH 本身负责多阶段 key/password/keyboard-interactive 认证；KbdInteractiveAuthentication=yes 只是候选能力，不能证明服务器需要 OTP。

配置秘密时启用受限 SSH_ASKPASS helper：helper 路径与 force 标志可以进入环境；秘密不进入环境或命令参数。helper 回调自己的隐藏 CLI，从同一根 TOML、相同已验证 hash 读取凭据。认证期间配置改变会拒绝旧 hash 的回答，后续连接重新建立，不混用旧快照与新秘密。

只回答显式匹配的挑战：password、加密 key passphrase、TOTP prompt。未知提示及 host-key 确认拒绝；StrictHostKeyChecking=yes，不能以回答 yes 绕过 known_hosts。TOTP 用 totp-rs，支持配置的 SHA1/SHA256/SHA512、6/8 位与周期；临近过期等待下一窗口，不在同一次认证重复使用同窗口，限制尝试次数。短信、push、硬件确认和 HOTP 不冒充可由 TOTP seed 自动完成的能力。

已用用户提供的 Google Authenticator 导出离线解析后进行真实 TOTP 认证；专用 SSH 子连接强制断开后重新认证成功。aws-work 的 ProxyJump 和 ProxyCommand 均已实测；导出 URI、seed 与验证码不写入仓库或日志。多段跳转的 prompt_patterns 应匹配对应网关账号，不能把网关 seed 用于任意目标的提示。

## tracing 与日志轮转

logging.filter 直接控制 tracing，普通会话的成功续轮/steer 与详情写日志。终端输出由 console 控制；配置 directory 时同时写文件。rotation 支持 daily/hourly，retained_files 默认 14，并在轮转时清理旧日志；日志目录权限 0700。logging 参数变更需重启桥接，现有 agent 不受影响。

认证 helper 不初始化日志，避免验证码流混入普通输出。HTTP/TLS/SDK/SQL 传输 target 独立过滤，即使用户指定更细日志也不记录 token URL、原始帧或 SQL 参数。所有诊断错误脱敏，操作记录使用稳定 ID，不记录任务正文。

## 慢连接与生命周期

连接按 provider 单次建连、心跳、独立退避与 jitter 缓存；各绑定独立观察，慢服务器不串行阻塞其他服务器。连接/认证/proxy-ready 预算组合为启动 deadline，RPC 读取与写回执使用独立超时。默认 30 秒心跳、允许 6 次缺失，真实时间取决于网络和系统调度。

只对识别到的原生空 rollout 读取竞态进行最多五秒、受 RPC 总 deadline 限制的重读；其他读取失败交给下一次观察/用户刷新。写超时、掉线或协议错误进入 OutcomeUnknown，不自动重试。SSH 断线只回收本机 proxy，不关闭远端 daemon。

后台仅轮询列表/状态/历史，不发 resume。用户明确发消息且会话 NotLoaded 时才 resume；空闲 turn/start，活跃 turn/steer 必须携带实际 expectedTurnId。等待审批也可 steer，审批仍由原客户端处理。所有接口均没有 interrupt、unsubscribe、kill、archive 或 daemon 管理动作。
