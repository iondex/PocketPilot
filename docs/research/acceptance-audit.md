# A01–A52 交付审计

日期：2026-10-16。此审计针对完整原始标准；不会用单元测试通过推断真实三端行为已通过。结果为部分证明、未运行或外部阻塞时，完整 MVP 验收仍不成立。Telegram 已准备用户测试，Discord 原型已有实现与 mock 验证。

## 证据分类

- S：当前源码/类型/配置/SQL 的静态约束。
- T：已执行的 Rust、SQLite、模拟 Telegram/Discord API、类型化 WebSocket 行为测试。
- N：真实 Codex binary + 隔离临时 home + 模拟模型；本机 TCP/Unix proxy 多客户端实验，不接触用户任务。
- R：用户授权 SSH 共享 daemon 上专用真实模型会话的创建、active、steer、全部桥接客户端断开后继续完成、最终输出与显式 resume；Desktop 只读观察相同 active turn。
- P：真实 Telegram 账号/Topics/webhook 检查，以及后台长轮询成功推进。

T/N/R/P 的具体范围见 [实现记录](implementation-checks.md)。测试入口与源文件见 [测试指南](../testing.md)、telegram/src/service_tests.rs、telegram/src/ledger.rs、engine/src/storage.rs、codex/src/live_tests.rs、codex/src/provider.rs 和 telegram/src/api.rs。完整标准见 [验收清单](../planning/acceptance.md)。

## 按场景核对

| ID | 当前证据 | 结果与尚缺证据 |
| --- | --- | --- |
| A01 | S/T：list/read/history 无写方法；R：既有 SSH 历史只读浏览 | 部分证明；真实 Telegram 浏览待用户 |
| A02 | N/R：独立连接与 Desktop 看到相同 active turn；T：topic 绑定无 resume | 部分证明；从真实 Telegram 进入既有 Desktop 活跃会话待用户 |
| A03 | R：steer 同 turn 被接受；T：Telegram topic 输入带 expectedTurnId | 部分证明；实际 Telegram 三端输入待用户 |
| A04 | T：服务端拒绝已变化的 turn，保留文本为草稿；无自动 turn/start；新确认才能发 | 规则已证明；真实跨端竞争未运行 |
| A05 | T：续轮参数不覆盖 cwd/model/approval/sandbox；N：新轮原生运行 | 部分证明；既有真实会话空闲续轮与策略对比待用户 |
| A06 | S：本地锁 + 原生 expectedTurnId；T：冲突不自动换轮 | 部分证明；真实 CLI/Desktop 同时输入未运行 |
| A07 | N/R：最后桥接连接退出后仍 active 并完成 | SSH provider 范围已证明；完整 Telegram 生成中退出待用户 |
| A08 | T：等待审批标记下只 steer、不回答审批；S：断线无中止命令 | 未完成；真实最后客户端在审批/结构化输入中退出未运行 |
| A09 | T：丢回执只记录 unknown；重启不重发 | 规则已证明；真实进程崩溃/网络断点组合未运行 |
| A10 | T：重复 input/callback、nonce/operation 去重、资源 ID 复用 | 规则已证明；真实平台重复投递待用户 |
| A11 | T：未知资源创建不重做；S：create/首轮独立回执 | 部分证明；实际 Telegram createTopic/threadStart 丢回执未运行 |
| A12 | T：过滤工具/推理，保留可见 user/agent/plan；S：typed ThreadItem | 规则已证明；真实工具/commentary 混合输出待用户 |
| A13 | T：历史/观察游标与输出同事务，stable item 去重；R：最终历史可读取 | 部分证明；实际掉线恢复与临时文本缺口未完整验证 |
| A14 | S/T：Unicode 短标题、UTF-16 长度边界、单 Markdown 附件 | 部分证明；长中文/emoji/代码实际上传未运行 |
| A15 | S/T：私聊共享预算、持久合并；S：429 retry_after | 部分证明；真实多 topic 突发与 429/可见积压未运行 |
| A16 | T：发送结果未知不自动重发，明确不保证 exactly-once | 规则已证明；真实消息发出后丢回执未运行 |
| A17 | T：服务端审批请求只观察，无回答；S：固定 RPC 白名单 | 部分证明；真实原端审批与问题处理未运行 |
| A18 | T：群聊、错误 peer、未配对、错误 nonce 拒绝 | 规则已证明；实际 Telegram 身份输入待用户 |
| A19 | S：topic/recipient 错误只停映射；T：容器墓碑与替换规则 | 部分证明；真实关闭/删除/阻止 bot 未运行 |
| A20 | T：未知状态拒绝解码，未验证 provider 无写入，无 active ID 不猜测 | 规则已证明；真实不兼容服务组合未运行 |
| A21 | R：专用新 thread 在 Desktop 可见；T：创建/首轮/topic 链路 | 部分证明；从真实 Telegram 创建并在 CLI/Desktop 继续待用户 |
| A22 | T：旧会话写 DTO 未覆盖策略字段；S：不代答审批 | 部分证明；真实既有会话前后完整策略/指令来源比较未运行 |
| A23 | S/T：RPC 方法封闭，固定 proxy 子进程，无 interrupt/unsubscribe/管理动作 | 当前代码与捕获范围已证明；真实生命周期由 N/R 提供补充 |
| A24 | S：bounded workers、SQLite timeout、历史/帧预算；T：单写者锁 | 部分证明；磁盘满、SQLite 忙与慢平台故障注入未完整运行 |
| A25 | S：StrictHostKeyChecking=yes，未知挑战不答 | 部分证明；真实 host key 更换/认证失败未运行 |
| A26 | T：instance/thread 与 bot/chat/topic 隔离，容器不可改绑 | 规则已证明；真实多实例同 ID/fork 组合未运行 |
| A27 | T：重启前 pending、旧秒级时间戳、poll outage 恢复需新确认；旧 nonce 作废；绑定版本校验；S：五分钟过期也转草稿 | 规则已证明；真实离线用户输入待用户 |
| A28 | T：一周 epoch 重建与跨 epoch 去重 | 规则已证明；实际长期空闲 Telegram 行为未运行 |
| A29 | S/T：观察与重连无 resume，NotLoaded 仅用户明确输入才 resume | 规则已证明；真实外部卸载/重连组合未运行 |
| A30 | S：历史集合注明读取记录/非原子快照，不承诺 delta 连续 | 未完全证明；真实未持久化 commentary 丢失及缺口提示需进一步验证 |
| A31 | T：失败/活跃文本不误标 final；S：按权威 item 内容补投递 | 部分证明；真实 delta 后中断/缺 completed 未运行 |
| A32 | S/T：每 provider 建连、退避与独立绑定观察；显式 worktree 分页 | 部分证明；无共享入口的 Desktop 不能接入，多实例组合需按部署验收 |
| A33 | R：确认既有远端 daemon/socket，Desktop 与桥接读到相同 active turn | 已在授权 SSH profile 证明；不扩展到其他主机 |
| A34 | S/T：OpenSSH askpass 与 TOTP/RFC 向量 | 外部阻塞；未提供实际 OTP host，key partial_success/multi-prompt 未实测 |
| A35 | T：TOTP 窗口/重复挑战；S：显式提示匹配、尝试限制 | 部分证明；真实慢认证/OTP 挑战未运行 |
| A36 | S/T：OpenSSH 配置发现/ssh -G、-F 一致、禁止 wildcard 展开；R：实际 alias 密钥连接 | 部分证明；实际复杂 Match/ProxyJump/OTP/-F 组合未运行 |
| A37 | T：单根 TOML 无效更新保留旧快照；S：父目录 watcher + rescan | 部分证明；ConfigWriter CAS 仍为设计接口，当前入口为人工编辑根文件；并发编辑/原子替换需实测 |
| A38 | T：撤销配对在 dispatch 事务拒绝，输出重新检查范围/revision | 规则已证明；真实慢连接期间热删除/撤销未运行 |
| A39 | S：新认证 hash 绑定根配置，现有连接不主动断开；T：secret Debug/错误脱敏 | 部分证明；真实 OTP seed/SSH config 轮换未运行 |
| A40 | S/T：endpoint 指纹持久化，原 ID 拒绝转向新地址/用户/socket | 规则已证明；真实目标变更未运行 |
| A41 | T：最后用户与同轮 final，不继承上一轮答案 | 规则已证明；实际空闲 session 进入待用户 |
| A42 | T：active 完整可读历史为一集合；S：超长单附件；R：真实历史读回 | 部分证明；Telegram 实际附件待用户 |
| A43 | S：附件/读取上限明确失败，不截断冒充完整 | 部分证明；真实超限历史平台请求未运行 |
| A44 | T：持久 burst 固定 flush 截止，成员正文/顺序保留 | 规则已证明；真实连续高频文本待用户 |
| A45 | T：batch 重启固定截止与 unknown 不重发；S：附件失败分类 | 部分证明；实际上传丢回执/屏障故障组合未运行 |
| A46 | S/T：后台无修改许可、恢复 unknown 不重放；N/R：连接 EOF 不结束正在生成任务 | 部分证明；本地既有 Desktop 与真实审批等待 EOF 未验证 |
| A47 | S/R：schema binary 与运行 daemon 分别核对，未重启既有服务 | 当前维护路径已证明；不承诺新版本自动兼容 |
| A48 | S/T：单根 TOML，bot scope 隔离；token/data_dir 改动需重启桥接 | 部分证明；同 bot token 真实轮换与不同 bot 交接未运行 |
| A49 | T：群聊与错误 private peer 拒绝 | 规则已证明；实际平台输入待用户 |
| A50 | T：十分钟、单次、bot scope、CLI 批准；P：bot ready | 规则已证明；实际配对及交互由部署者按目标环境验收 |
| A51 | T：CLI 与持锁服务并行批准/撤销，未恢复 operations | 规则已证明；实际批准和撤销由部署者验收 |
| A52 | S/T：根 TOML/0600、拒绝 include/.env/旧允许列表、日志脱敏、Git 秘密边界检查 | 当前交付文件与已测路径已证明；不将静态扫描称作全面秘密审计 |

## 其他交付与外部条件

| 交付 | 当前结果 |
| --- | --- |
| Rust、简短 crate、平铺源码 | Cargo workspace 为 i18n/ssh/session/codex/engine/telegram/discord/app；src 下无多级目录 |
| 官方协议动态生成 | build.rs 生成到 OUT_DIR；不提交 schema、DTO 或 docs JSON |
| 配置、tracing、流行库 | 单根 TOML；tokio/sqlx/teloxide/Serenity/reqwest/tracing/totp-rs 等，依赖由 Cargo.lock 固定 |
| Telegram 可测试入口 | 支持 CLI 配对和私聊 Topics，部署凭据与进程状态由部署者管理 |
| Discord 原型 | 可选 feature，同进程 category/channel + owner slash + 共享锁/操作账本；真实服务器与客户端交互需单独验收 |
| Claude shim、Lark | 规划与候选实现保留在 future/integrations.md，不作为已实现能力 |
| 本地现有 Desktop | 若只有 stdio 且缺少共享 endpoint，则无法直接附加；不得另建同历史服务冒充接入 |
| GitHub CI | 远端 CI 成功与本地检查分别记录，不由本地检查推断平台结果 |

完整目标保持未验收状态。此表不缩减原始支持范围，也不把“准备用户测试”解释为真实平台行为已全部证明。

## 展示范围

Telegram 普通转发包括 agent 正文和同一原生会话的外部用户输入，并精确去重当前 Telegram 用户自身的输入。阶段由 emoji 表示，空历史不占位；完整 user/agent 模型仍保留。管理交互使用持久编号导航，每次操作发送新菜单。展示方式不放宽操作授权、未知结果不重发或原生生命周期约束。

平台能力与话题配置条件见 [Topics 边界](telegram-topics.md)。完整目标仍需按目标部署完成验收，本文不记录真实账号、话题、主机或程序的当前运行状态。
