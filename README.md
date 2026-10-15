# PocketPilot

Rust 实现的 Codex 会话桥接器：默认在本机运行，通过 Telegram topic 查看、创建和继续本地/SSH 会话，运行中支持 steer。后续规划 Lark、Discord 和 Claude shim。

**当前仅有设计和实施计划，尚无可运行程序。** 既有 `.env` 已提供 `TELEGRAM_BOT_TOKEN`，仅核对格式，未联网认证或操作真实 session。

## 从这里开始

- [文档目录与阅读路线](docs/README.md)
- [细化实施计划](docs/planning/implementation.md)
- [48 项验收场景](docs/planning/acceptance.md)
- [不含秘密的配置示例](config/pocketpilot.example.toml)

## 当前约定

| 范围 | 决策 |
| --- | --- |
| crate 名称 | `session`、`codex`、`telegram`、`app`；binary 仍叫 `pocketpilot` |
| 首版 | 最新受测 Codex + Telegram；本地和配置允许的 SSH 实例 |
| 会话共享 | desktop、CLI、桥接器连接同一执行实例；相同历史目录不足以证明共享 |
| 允许动作 | 创建、用户明确续轮、steer、只读查看；审批由原端处理 |
| 生命周期 | 程序启停、崩溃、reload、重连不自动 resume/start/unsubscribe 或停止 agent |
| 配置 | 普通设置 TOML，继承本机 SSH config；Telegram 明确引用既有 .env token，不用环境变量覆盖普通配置 |
| SSH | 读取 Host/Include，按 OpenSSH 有效配置连接；TOTP 使用 russh + totp-rs |
| 热更新 | host/workspace/授权可更新，排队操作再次授权；不改变已运行任务 |
| 历史 | 空闲显示最后输入/对应 final；活跃会话全历史一个集合，超限一条附件 |
| 输出 | 保留可见文本、过滤工具/推理，短时合并同会话消息并统一限速 |
| 不确定结果 | 远程写入或平台投递未知时核对，不盲目重复执行 |

真实 token、OTP seed、用户配置、数据库和日志不提交。当前可行性证据与尚待验证事项见 [研究记录](docs/research/sources.md)。
