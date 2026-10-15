# 文档入口

当前阶段：设计与实施准备。仓库尚无 Cargo workspace 或运行 bot；本地 .env 中已有 Telegram token，仅核对键名和格式，未联网认证。真实凭据不进入文档、配置示例或提交。

## 按目的阅读

| 目的 | 文档 | 负责维护的内容 |
| --- | --- | --- |
| 开始实现 | [实施计划](planning/implementation.md) | crate 边界、任务依赖、提交粒度、阶段门禁与当前状态 |
| 确认验收 | [验收清单](planning/acceptance.md) | A01–A48 的行为条件、执行记录格式与证据要求 |
| 理解接口 | [架构](design/architecture.md) | 领域模型、provider/platform 接口、操作许可、数据与恢复 |
| 做 Telegram 交互 | [Telegram](design/telegram.md) | 菜单、topic、绑定、新建、状态与用户可见异常 |
| 做配置和连接 | [连接与配置](design/connections-and-config.md) | TOML、显式 Telegram 凭据源、SSH config、OTP、热更新、生命周期 |
| 做历史与投递 | [历史与消息](design/history-and-delivery.md) | 最后输入/最终输出、单条历史集合、合并、附件与限速 |
| 查事实与限制 | [Codex 探索](research/codex-app-server.md)、[资料](research/sources.md) | 静态观察、来源、版本差异和未验证能力 |
| 后期扩展 | [Lark、Discord 与 Claude](future/integrations.md) | 可行性与候选实现，不构成首版已支持能力 |

公开的 [配置示例](../config/pocketpilot.example.toml) 是字段和默认值的唯一示例来源；设计文档不再维护另一份大段 TOML。它是 schema 草案，尚不能由当前仓库程序执行。

## 文档与源码对应

| 规划中的 crate | 职责 | 对应设计 |
| --- | --- | --- |
| `session` | 统一模型、接口、命令、actor、历史与投递规则 | 架构、历史与消息 |
| `codex` | Codex wire/RPC、provider、只读观察、本地/SSH transport | 架构、连接与配置、研究 |
| `telegram` | Bot API、poller、授权、菜单/topic、渲染和平台限速 | Telegram、历史与消息 |
| `app` | 配置/凭据、SQLite、装配、CLI、tracing、生命周期 | 连接与配置、实施计划 |

package、目录和 Rust crate 名都不用 PocketPilot 前缀；可执行程序保留 pocketpilot 产品名。四个 package 初期 publish=false；后续按需新增 claude、lark、discord，不预建空 crate。

## 维护约定

设计变更先改对应主题文档，再同步实施任务与验收条件。计划只引用设计，不复制完整交互/配置/恢复算法；研究文档记录事实及证据，不覆盖当前实现决策。

任务状态只在实施计划维护；验收结果单独保存报告，不把“文档检查通过”写成 A 场景通过。A 编号和 W 编号保持稳定。首次实现必须完成共享实例与无副作用观察门禁，不能为演示 UI 绕过。
