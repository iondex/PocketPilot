# PocketPilot

**从你常用的聊天工具，接入不同主机上的 AI 编程会话。**

PocketPilot 是面向开发者的自托管会话入口，适合在电脑、手机和远程开发环境之间切换使用。它把消息平台作为统一入口，让你选择执行环境、项目和 session，查看进度、补充任务、上传文件并接收结果。原生 agent runtime 继续负责执行、上下文和会话生命周期。

产品围绕三个独立维度扩展：消息接入、主机连接和 agent 运行时。一个新消息平台可以使用已有运行时；一个新运行时也可以复用已有的聊天入口。

| 维度 | 当前支持 | 后续方向 |
| --- | --- | --- |
| 消息入口 | Telegram、Slack、Discord | Lark / 飞书 |
| 执行环境 | 本机、通过 OpenSSH 连接的远程主机 | 继续完善现有连接方式 |
| Agent runtime | Codex app-server | Claude Code、Pi Coding Agent |

## 使用体验

统一导航是 **执行环境 → 项目目录 → 会话**。打开会话可以查看状态和最近交互；发出任务后，在同一聊天入口跟踪结果。多个消息平台可以关联同一个原生会话，共用服务进程内的连接、会话锁和操作账本。

- Telegram：指定用户在 bot 私聊中用 topic 区分会话。
- Slack：owner 与 App 私聊，每个会话使用独立 thread。
- Discord：服务器 owner 通过 `/pp` 导航，每个会话使用独立 channel；支持原生新建表单。

正文按平台能力呈现 Markdown、代码、表格与图片；用户文件可作为独立附件提交。绑定可暂停观察、恢复或解除。已有任务的执行独立于 PocketPilot 的启停。

## 开始使用

准备仓库指定的 Rust 工具链、系统 OpenSSH 和可共享连接的 Codex app-server。复制 [配置示例](pocketpilot.example.toml) 为私有 `pocketpilot.toml`，填写要启用的消息平台和主机配置。

```sh
make package
cargo clean
./dist/pocketpilot doctor --credentials
./dist/pocketpilot doctor --connect
./dist/pocketpilot serve
```

默认构建包含三个已实现的消息平台，配置决定运行哪些入口。新发现的运行实例默认只读；启用任务输入前需确认共享实例与控制能力。完整安装、授权和日常操作见接入方案。

## 文档

- [当前架构](docs/architecture.md)：产品概念、crate 分工、会话控制、消息投递与状态数据。
- [接入方案](docs/integrations.md)：运行准备、三个平台的安装使用、附件、代理与日常维护。
- [后续支持](docs/support.md)：Lark / 飞书接入规划、Claude Code / Pi 的运行时适配方向及交付门槛。

开发约定见 [CONTRIBUTING.md](CONTRIBUTING.md)。旧设计、调研、安装 manifest 和实施记录可从 Git 历史查阅。
