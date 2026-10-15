# MVP 验收清单

状态：已有规则测试、隔离原生实验与授权 SSH 共享实例证据；真实 Telegram 用户交互、本地既有 Desktop 和 OTP 组合仍未完成验收。逐项证据及缺口见 [A01–A52 审计](../research/acceptance-audit.md)。执行任务见 [实施计划](implementation.md)。场景 ID 保持稳定，后续只追加，不因文档重组重新编号。

## 执行范围

| 范围 | 主要场景 |
| --- | --- |
| 协议/共享/状态与只读 | A01–A08、A20–A23、A26、A29、A33、A46–A47 |
| 幂等/持久化/断线与错误 | A09–A11、A13、A16、A19、A24、A27–A31、A45 |
| 文本/历史/防刷屏 | A12–A15、A31、A41–A45 |
| SSH/OTP/慢服务器 | A25、A32–A36、A39 |
| 配置/授权/生命周期 | A17–A18、A37–A40、A46、A48 |

同一场景可能跨层验证。单元/fake transport/SQLite/fake HTTP 测试先验证规则；真实端到端测试只在独立测试私聊和专用 workspace 执行。必须包含本地、普通 SSH、OTP/慢 host 的实际支持组合，不能把一次本地结果复制到其他 transport。

## 行为场景

这些标准保持不变；规则测试不能自动替代标准中要求的真实平台/多端验证。

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
| A15 | 同一私聊 5 个 topic 突发消息/429 | 共享私聊限速、不阻塞 agent 读循环、retry_after、可见积压 |
| A16 | Telegram 发送后丢响应 | DeliveryUnknown 不无限重发；不承诺 exactly-once |
| A17 | 桌面处理审批/问题 | PocketPilot 仅更新观察状态，没有 approve/decline/cancel/answer RPC |
| A18 | 未配对/群聊/伪造 callback/错误 private peer | 不能列敏感工作区、发送消息或批准请求 |
| A19 | topic 关闭/删除、用户阻止 bot | 只影响映射/投递；agent 无归档/停止，积压可恢复 |
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
| A37 | 根 config 无效/不支持的 include/并发编辑/原子替换 | 保留最后有效配置；ConfigWriter CAS；无部分授权快照 |
| A38 | 热删除 host/workspace、CLI 撤销配对 | 已排队输入/输出重新授权并阻止；远端任务不变 |
| A39 | OTP seed/SSH config 轮换 | 新认证用新值；原有效连接不强制断开；日志无 seed/code |
| A40 | endpoint/user/socket 更换 | 新实例身份，旧 binding 不自动指向新服务 |
| A41 | 新绑定空闲 thread | 返回最后用户输入/同轮 final；无 final 明确提示，不错配上一轮 |
| A42 | 进入活跃 thread 全历史 | 只读分页、一个集合；超长一次附件，无逐条历史刷屏 |
| A43 | 单附件仍超过平台上限 | 一条明确失败/主动分片入口，不截断或宣称完整发送 |
| A44 | 短时 3 条以上文本/持续高频 | 合并保留正文与顺序，有最大等待，不跨 session 合并 |
| A45 | batch 崩溃/回执丢失、历史附件失败/超限 | 成员/投递事务一致，不重复分组；历史待核对可见且不永久阻塞后续实时输出 |
| A46 | 程序启停/崩溃/reload/重连 | ObserverPermit 无 resume/start/steer/approval/unsubscribe；进程/EOF 不杀 agent |
| A47 | 最新 CLI 升级 | 维护步骤记录实际 daemon/schema；不自动重启活跃会话 |
| A48 | 根配置与内联 token | 不支持 include/.env/允许列表；同 bot 轮换交接，不同 bot 隔离配对/绑定/操作/积压/nonce/状态卡 |

## 私聊配对新增场景

| ID | 场景 | 通过标准 |
| --- | --- | --- |
| A49 | 已配对用户从群聊或错误 peer 发消息/按钮 | 一律拒绝，不创建配对申请、不列 workspace、不触发 agent |
| A50 | 未配对私聊申请、CLI 批准、过期/复用码 | 批准前无访问；10 分钟 TTL、一次性消费、bot namespace 隔离 |
| A51 | 服务持账本锁时 CLI 批准/撤销 | CLI 仍可更新配对，不恢复/重放 operations，不停止 session |
| A52 | 根配置单文件与秘密保护 | token 只存私有根 TOML；include 和旧允许列表报错，文件/日志/Git 不泄密 |

## 报告格式与放行

实施阶段按实际运行生成报告，至少记录：场景 ID、结果（pass/fail/blocked/not_run）、OS、服务与 CLI 版本、schema hash、instance/transport profile、配置 revision、步骤、预期/实际、匿名证据和对应 W 任务。敏感正文、令牌、OTP、个人主机地址不进入公开报告。

报告必须区分静态检查、fake 测试与真实多端行为证据。记录 interrupted 时核实是否由桥接器造成；不能以先停止再恢复任务的方式得到表面通过。平台或服务端本身不支持的组合标 blocked 并限制能力，不把未运行改成 pass。

关键放行条件：共享执行实例、真实 active turn、只读观察、审批路由保持、最后连接/EOF 不取消、无自动重试未知写入、授权撤销与后台无修改许可。这些失败时不开放真实控制。完整 MVP 要求目标支持范围内 A01–A52 均满足；其余组合明确列为不支持。

## 当前验证

当前已执行 Rust 规则/合成 RPC/SQLite/Bot API 测试、本机隔离 Codex 0.161.0/0.162.1 生命周期实验、授权 SSH 的共享实例与专用新会话控制探针，以及真实 Telegram 长轮询运行检查。原生既有任务未收到测试输入。

未把这些证据记作 A01–A52 全部通过。当前每项结果、还需什么证据、哪些条件属于外部阻塞，见 [验收审计](../research/acceptance-audit.md)；完整运行记录见 [实现验证记录](../research/implementation-checks.md)。
