# 文档入口

当前阶段：Telegram 功能链路已接通并进入真实 SSH 验收，另提供可选 Discord 原型，源代码和协议边界已有自动测试。源码平铺，schema 和 DTO 由 build.rs 生成到 OUT_DIR；真实部署状态见 [测试入口](testing.md) 与 [验证记录](research/implementation-checks.md)。

## 按目的阅读

| 目的 | 文档 | 负责维护的内容 |
| --- | --- | --- |
| 开始测试 | [测试入口](testing.md) | 启动、CLI 配对、共享实例准备、测试顺序与当前限制 |
| 开始实现 | [实施计划](planning/implementation.md) | crate 边界、任务依赖、提交粒度、阶段门禁与当前状态 |
| 确认验收 | [验收清单](planning/acceptance.md)、[逐项审计](research/acceptance-audit.md) | A01–A52 的行为条件、当前证据与未验证项 |
| 协议生成 | [构建时生成](design/protocol-generation.md) | build.rs、Codex binary 选择与生成物边界 |
| 理解接口 | [架构](design/architecture.md) | 领域模型、provider/platform 接口、操作许可、数据与恢复 |
| 做 Telegram 交互 | [Telegram](design/telegram.md) | 菜单、topic、绑定、新建、状态与用户可见异常 |
| 做 Discord 交互 | [Discord 原型](design/discord.md) | category/channel、owner slash、可选运行与原型限制 |
| 接入本地 Desktop | [本地会话说明](design/local-desktop.md) | 当前限制、已确认入口与后续共享部署验证 |
| 做配置和连接 | [连接与配置](design/connections-and-config.md) | 根目录单一 TOML、内联 token、CLI 私聊配对、SSH config、OTP、热更新、生命周期 |
| 做历史与投递 | [历史与消息](design/history-and-delivery.md) | 最后输入/最终输出、单条历史集合、合并、附件与限速 |
| 查事实与限制 | [Codex 探索](research/codex-app-server.md)、[资料](research/sources.md) | 静态观察、来源、版本差异和未验证能力 |
| 构建和体积 | [构建说明](build.md) | release profile、体积实测、保留的运行能力 |
| 后期扩展 | [Lark 与 Claude](future/integrations.md) | 可行性与候选实现，不构成首版已支持能力 |

公开的 [配置示例](../pocketpilot.example.toml) 是字段和默认值的唯一示例来源；设计文档不再维护另一份大段 TOML。当前程序可验证并读取该配置；示例 SSH alias、允许的项目根路径和 token 需填写；bin/socket 自动探测，provider/workspace 动态保存；不设置 chat/user 允许列表。

## 文档与源码对应

| crate | 职责 | 对应设计 |
| --- | --- | --- |
| `session` | 统一模型、接口、允许的动作、历史与投递规则 | 架构、历史与消息 |
| `codex` | Codex wire/RPC、provider、只读观察、本地/SSH transport | 架构、连接与配置、研究 |
| `telegram` | Bot API、poller、授权、菜单/topic、渲染和平台限速 | Telegram、历史与消息 |
| `discord` | Serenity Bot API、owner slash gateway、category/channel 平台操作 | Discord 原型 |
| `ssh` | SSH 有效配置、连接、TOTP/askpass | 连接与配置 |
| `engine` | 共享执行、操作账本、配置快照与动态 catalog | 架构、连接与配置 |
| `app` | 根配置组合、装配、CLI、tracing、生命周期 | 连接与配置、实施计划 |

package、目录和 Rust crate 名都不用 PocketPilot 前缀；可执行程序保留 pocketpilot 产品名。所有 package 都是 publish=false；discord 已有原型代码，claude/lark 尚未创建 crate。

## 维护约定

设计变更先改对应主题文档，再同步实施任务与验收条件。计划只引用设计，不复制完整交互/配置/恢复算法；研究文档记录事实及证据，不覆盖当前实现决策。

任务状态只在实施计划维护；验收结果单独保存报告，不把“文档检查通过”写成 A 场景通过。A 编号和 W 编号保持稳定。首次实现必须完成共享实例与无副作用观察门禁，不能为演示 UI 绕过。
