# PocketPilot

通过 Telegram 管理并继续 Codex 会话的 Rust 项目。默认在本机运行，聚合本地及配置允许的 SSH 实例；优先支持 Telegram forum topic + 最新 Codex app server，后续支持 Lark、Discord 与 Claude shim。

当前阶段：**探索与设计，尚未实现可运行的机器人**。调研日期：2026-10-15。

核心要求是接入同一个正在运行的 Codex 实例：Telegram 可以查看消息、状态和发送 steer，但不能中断任务、重启服务、修改现有会话的配置或通过 fork 替代原会话。

## 文档

建议按以下顺序阅读：

1. [探索结论与可行性边界](docs/01-exploration.md)：官方协议、本机版本差异、共享会话与 SSH 接入的证据。
2. [架构、Rust 接口与数据设计](docs/02-architecture.md)：领域模型、模块边界、持久化、并发与恢复。
3. [Telegram 交互设计](docs/03-telegram-ux.md)：添加工作区、绑定/新建会话、topic、状态、消息和异常交互。
4. [MVP 实施与验收计划](docs/04-mvp-plan.md)：依赖选型、目录结构、配置、实现顺序、运行与验收矩阵。
5. [资料与研究记录](docs/05-sources.md)：官方资料、固定源码版本、实际验证与未验证事项。
6. [部署、配置热更新与 SSH / OTP](docs/06-deployment-config-ssh.md)：本机/远程共享实例、纯 config、慢服务器、TOTP 和生命周期隔离。
7. [Lark 与 Claude shim 规划](docs/07-future-adapters.md)：平台可行性、兼容子集、远端常驻与能力边界。
8. [历史集合与消息合并](docs/08-history-and-delivery.md)：最后输入/最终输出、活跃会话完整历史、单消息附件与防刷屏。

## 关键决策

| 问题 | 决策 |
| --- | --- |
| 自动列出所有工作区 | MVP 手动登记；由 thread 的 cwd 提供候选，不能宣称完整枚举桌面项目 |
| 每个工作区的会话 | app-server `thread/list` 分页；显式区分原始 cwd、项目归属和 worktree |
| 同时使用桌面、CLI、Telegram | 同一 app-server 实例是前提；共享历史目录不足以证明共享运行状态 |
| 本地/远程连接 | 优先已有 endpoint；OpenSSH 普通认证、russh + TOTP 交互认证；共同连接远端已有 daemon |
| 正在运行的会话 | 获取真实 active turn ID 后调用 `turn/steer`，带 `expectedTurnId` |
| 停止会话 | 不提供；协议发送器使用方法白名单，生命周期操作也受限制 |
| 消息转发 | 转发 user/agent 的可见文本，包括中间说明；过滤工具调用及其输出、推理内容 |
| 一次发送结果未知 | 进入待核对状态，不自动重复执行；不能承诺端到端 exactly-once |
| Telegram 组织 | 一个私有 forum supergroup；一个控制 topic；每个绑定会话一个 topic |
| 扩展 | AgentProvider、ChatPlatform、RpcTransport 三条独立接口边界 |
| 配置 | 全部配置文件管理：TOML + 既有 SSH config，可包含秘密；不使用 .env；host/workspace 和授权原子热更新 |
| 生命周期 | 启停/崩溃/reload/重连不写 session；不自动 resume，不回复审批，不 unsubscribe |
| 进入历史 | 空闲显示最后输入/对应最终输出；活跃显示全部可读历史的单条集合，超长用一条附件 |
| 防刷屏 | 短时窗口/阈值合并同会话输出，保留正文；叠加平台限速 |

目前没有把真实会话接入、发送提示词、改变审批策略、启动/停止 Codex daemon，或创建 Telegram/Discord 资源。详细限制和实施前置验证见文档。
