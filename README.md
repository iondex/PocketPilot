# PocketPilot

Rust 实现的 Codex 会话桥接器：默认在本机运行，通过 Telegram topic 查看、创建和继续本地/SSH 会话，运行中支持 steer。后续规划 Lark、Discord 和 Claude shim。

**已实现可运行的只读 CLI 基础版。** 包含配置/凭据校验、tracing、官方 schema 生成协议类型、Codex 只读列表/状态/历史、本地/普通 SSH/WS transport、SSH 配置发现和 SQLite 基础账本。完整 Telegram bot、OTP、热更新、续轮/steer 尚未完成，控制能力始终关闭。

## 从这里开始

- [文档目录与阅读路线](docs/README.md)
- [细化实施计划](docs/planning/implementation.md)
- [52 项验收场景](docs/planning/acceptance.md)
- [不含秘密的配置示例](pocketpilot.example.toml)

## 运行

需要当前 stable Rust（声明最低 1.94）。

```sh
cargo run -p app --bin pocketpilot -- --help
cargo run -p app --bin pocketpilot -- config-check --credentials
cargo run -p app --bin pocketpilot -- doctor
cargo run -p app --bin pocketpilot -- telegram-check
```

config-check 只检查文件，doctor 默认不连接网络。telegram-check 仅调用账号/webhook 只读 API，不发送消息；根配置中的令牌已通过本机只读账号检查。需要连接已有服务时先填写实际 endpoint/workspace，再使用 doctor --connect 或 sessions 子命令。

```sh
cargo run -p app --bin pocketpilot -- hosts discover
cargo run -p app --bin pocketpilot -- sessions --workspace WORKSPACE_ID list
cargo run -p app --bin pocketpilot -- sessions --workspace WORKSPACE_ID history THREAD_ID --latest
```

默认读取根目录 pocketpilot.toml，token 直接在 [telegram].token；不读取 .env、不支持 include。根目录 pocketpilot.example.toml 是无秘密的模板。私有配置已准备好，host/workspace 尚需按部署填写；没有配置 chat/user 允许列表。

私聊配对的本机管理入口：

```sh
cargo run -p app --bin pocketpilot -- pair pending
cargo run -p app --bin pocketpilot -- pair approve CODE
cargo run -p app --bin pocketpilot -- pair list
cargo run -p app --bin pocketpilot -- pair revoke BOT_ID USER_ID
```

配对账本/CLI 已实现；私聊 /start 生成并发送申请码的 bot 消息入口仍待 poller 接通。群聊不会获得业务访问。

协议在构建时动态生成，见 [生成说明](docs/design/protocol-generation.md)，源码全部是各 crate 的 src/ 平铺文件。docs 不包含 JSON。

## 当前约定

| 范围 | 决策 |
| --- | --- |
| crate 名称 | `session`、`codex`、`telegram`、`app`；binary 仍叫 `pocketpilot` |
| 首版 | 最新受测 Codex + Telegram；本地和配置允许的 SSH 实例 |
| 会话共享 | desktop、CLI、桥接器连接同一执行实例；相同历史目录不足以证明共享 |
| 允许动作 | 创建、用户明确续轮、steer、只读查看；审批由原端处理 |
| 生命周期 | 程序启停、崩溃、reload、重连不自动 resume/start/unsubscribe 或停止 agent |
| 配置 | 根目录单一 pocketpilot.toml，token 直接存储；不 include、不读取 .env；继承既有 SSH config |
| Telegram 访问 | 仅私聊；通过 CLI 批准一次性码配对，无 chat/user 配置列表 |
| SSH | 读取 Host/Include，按 OpenSSH 有效配置连接；TOTP 使用 russh + totp-rs |
| 热更新 | host/workspace/授权可更新，排队操作再次授权；不改变已运行任务 |
| 历史 | 空闲显示最后输入/对应 final；活跃会话全历史一个集合，超限一条附件 |
| 输出 | 保留可见文本、过滤工具/推理，短时合并同会话消息并统一限速 |
| 不确定结果 | 远程写入或平台投递未知时核对，不盲目重复执行 |

真实 token、OTP seed、用户配置、数据库和日志不提交。当前可行性证据与尚待验证事项见 [研究记录](docs/research/sources.md)。
