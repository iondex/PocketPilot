# 首轮实现与验证记录

日期：2026-10-15。当前交付是可运行的只读基础版，不是完整 Telegram MVP。

## 已实现

- 四个 package：session、codex、telegram、app；源码在各自 src/ 下平铺。
- 根目录单一 TOML、路径/限制校验、内联 token、脱敏错误和 tracing 日志。
- build.rs 动态导出官方 Codex schema、筛选只读投影并用 typify 生成 Rust DTO；生成物不提交。provider 不使用 serde_json::Value 字段操作。
- 类型化只读 RPC：握手、并发关联、deadline、有界 pending、服务端请求只观察；本地 proxy、普通 OpenSSH proxy、loopback WS/远程 WSS。
- 只读 thread 列表、状态、分页文本历史；工具/推理过滤、最近输入/对应 final、基础合并规则。
- 本机 SSH config alias 发现、OpenSSH 有效配置解析；不会因发现 alias 自动登录。
- 独占锁保护的自有 SQLite 基础账本、输入去重、状态 CAS、重启后不确定结果、容器不跨 session 复用。
- 私聊配对账本及 CLI pending/approve/list/revoke；一次性码有效期十分钟，按 bot 隔离。管理 CLI 可与持有账本锁的服务并行运行，不恢复或改动执行中操作。
- Telegram 账号/webhook 只读 Inspector。尚无 poller、消息发送、topic 或 session 写接口。

## 已执行的检查

- cargo test --workspace：26 个测试通过，覆盖单文件配置、拒绝 include/旧允许列表、凭据脱敏、私聊配对、RPC、schema DTO、文本、SQLite 与连接边界。
- cargo clippy --workspace --all-targets --locked -- -D warnings；cargo fmt。
- CLI --help、config-check --credentials、doctor、pair --help/pending/list 的本机 smoke check；当前无待配对申请或已配对用户。
- Telegram getMe/getWebhookInfo 成功；当前没有 webhook。没有发送消息、消费 updates 或改 webhook。
- 本机 SSH config 只读发现成功；没有登录远端，未将个人 alias 写入仓库。
- 重新导出桌面内置 codex-cli 0.162.0-alpha.17.2 的 schema；仅生成文件，没有启动或重启 app-server。实际生成器版本保存在 Cargo OUT_DIR/schema-version.txt，不提交版本快照。

## 未完成的真实验收

检查时本机默认控制 socket 不存在；现有公开示例仍是占位 endpoint。没有验证同一运行对象、多端 active turn、最后连接退出的行为，也没有重启 app-server 或操作真实 thread。因此 G0/G3 未通过，控制能力在代码中保持关闭。

OTP 后端、完整配置热更新、durable Telegram poller、私聊 /start 配对入口、topic 交互、持久 outbox 和创建/续轮/steer 仍须后续实现。mock 测试不能代替这些实际行为。

依赖构建有上游 proc-macro-error2 的 future-incompat 通知；当前工具链编译和项目 lint 可通过，后续更新 teloxide 相关依赖时跟进，不把第三方通知隐去。

## 构建生成调整

按要求取消 schema/Python 脚本/版本快照提交，生成逻辑迁入 build.rs。构建时自动导出所选 CLI 的 schema、用类型化 Schema 模型筛选引用并生成 DTO，全部输出至 Cargo OUT_DIR；无快照 fallback。CI 安装明确的 @openai/codex 0.162.1，再运行构建。

本机全局 CLI 已从 0.149.1 更新到 0.162.1，用于构建生成；既有 desktop/远端 daemon 没有重启或升级。动态生成已在桌面内置 0.162.0-alpha.17.2 和全局 0.162.1 上验证编译，实际支持仍以运行时行为验收为准。
