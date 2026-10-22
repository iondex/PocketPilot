# 后续支持规划

PocketPilot 的扩展目标是让同一套会话体验覆盖更多消息工具和编程运行时。新增消息端复用已有 agent 能力；新增 runtime 复用已有聊天入口、授权和操作账本。

| 方向 | 目标 | 当前边界 |
| --- | --- | --- |
| 飞书 / Lark | 群 thread、丰富卡片与附件 | 两端共用实现与本地测试；均待真实租户安装验收 |
| Claude Code | 已有独立 server 与结构化问答；后续补外部会话索引 | server 管理会话；DeepSeek 兼容配置已验证普通问答与 AskUserQuestion 往返；其他工具权限保持等待 |
| Pi Coding Agent | 长驻 RPC 与会话管理适配 | 待实现，需要明确进程所有权、版本和输入调度语义 |
| 公共交互 | 降低平台间重复逻辑，统一错误、恢复与能力呈现 | 按实际相同规则逐步收束，保留原生控件和授权差异 |

当前能力见[接入方案](integrations.md)和[当前架构](architecture.md)。以下保留尚未交付或仍需真实环境验证的能力。

## 飞书与 Lark 的后续验证

飞书与 Lark 接入已实现，安装与权限步骤见[接入方案](integrations.md#飞书企业自建应用)。当前完成离线协议、scope、重复输入、绑定状态转换、附件资源与富文本预算验证；两端均待真实租户安装验证，不能把离线 fixture 等同于安装和客户端验收。

后续优先完成真实租户的长连接、发布权限、卡片回调和跨项目 thread 隔离，检查独立附件投递、移除机器人/权限撤销、长时间根卡片过期及断线后的用户恢复。原生宽表、更多富文本扩展与消息直达链接按客户端实际能力开放；未核实的数学、脚注和折叠结构保留可读文本降级。

飞书与 Lark 共用协议和业务实现，分别以 `[feishu]` / `[lark]` 配置应用凭据、租户、owner 和群组。固定端点选择、SDK 本地 WebSocket 消息与卡片 ACK、事件去重、绑定及操作隔离已有测试；两端都固定直连且拒绝代理配置。后续完成真实应用的权限、卡片展示与回调、附件和重连验收，见[共享实现与验证](feishu-lark.md)。

## Claude Code 与 Pi 的 runtime 适配

### 运行时服务的职责

Codex 已有 app-server，继续接入原生服务。其他 agent 可能需要独立的 runtime server，负责会话目录、状态和事件、受控文件暂存、写操作账本与长驻进程。聊天连接通过通用 `AdapterFactory` / `AgentProvider` 契约访问它，不将 CLI 参数或进程管理泄露到 integration。

服务进程与连接生命周期分开：桥接退出、SSH 断开或一个客户端关闭，不应结束执行任务。服务可以拥有自己创建的 agent 进程；已有原生终端进程是否可附着必须单独证明。能读 transcript 不代表知道实时状态，也不代表获得控制权。

`RuntimeKind` 对应 Codex 原生入口和 Claude Code server 首版；`pi` 仍只有枚举与安装协议的扩展位置，没有可运行 adapter。后续按以下能力逐项协商，而非要求各 runtime 假装与 Codex 完全等价：

| 能力 | 要求 |
| --- | --- |
| 会话目录与读取 | 按项目筛选、稳定分页、只读历史；不能为了浏览而启动或 resume agent |
| 状态与观察 | 区分活跃、空闲、需用户操作、断线和未知；快照、事件缺口及缓存时间有明确定义 |
| 所有权与控制 | 分清可发现、可观察、可控制和可实时附着；只控制已授权并可证明归属的实例 |
| 输入与调度 | 空闲续轮、运行中追加、排队下一轮分别表达；不得用 interrupt 模拟 steer |
| 回执与重连 | 接受输入与完成执行分开；同一操作去重，回执未知不重发 |
| 文件与图片 | 有界暂存与模态能力声明；不靠工具调用伪造文件上传或扩大项目权限 |

### Claude Code

首版已采用纯 Rust `pocketpilot-agent-server` 管理 Claude stream-json 进程，不要求额外 Node/Python SDK sidecar。当前能力与安装方式见[接入方案](integrations.md#claude-code-独立-runtime-server)。只控制 server 自己管理的会话，不宣称能附着任意终端；官方 SDK 的 subprocess 协议仍需随 CLI 版本复核，本机核对版本为 CLI 2.1.304 / SDK 0.3.304。

后续补充只读外部 transcript 索引、状态事件覆盖和更多真实客户端验证。活跃轮次 steer 未证明，不能将排队输入当成即时 steer。工具权限请求目前保持 pending，不自动批准或拒绝；原生提问已有公共显式回答接口，后续补各平台的原生控件。

参考：[官方 TypeScript SDK](https://github.com/anthropics/claude-agent-sdk-typescript)、[官方 Python SDK](https://github.com/anthropics/claude-agent-sdk-python)。

### Pi Coding Agent

目标为 Pi Coding Agent，先确认所部署包和仓库对应的版本。评估以独立服务持有长驻 RPC 进程，通过其原生会话与事件接口映射公共模型。普通交互终端不自动等同于可共享的 RPC server。

重点区分 `steer` 与 `follow_up` 的实际交付时机，以及会话切换是否可能被扩展取消、历史分支如何还原、进程重启后如何确认任务状态。只读目录不能用可能修改会话文件的恢复操作代替；没有通用文件接口时，由 runtime server 提供受限暂存，不用 shell 任务模拟上传。

参考：[Pi Coding Agent](https://github.com/badlogic/pi-mono/tree/main/packages/coding-agent)。包名或仓库迁移时重新核对部署版本和协议，不沿用旧版本的能力假设。

### ACP 的位置

ACP 可作为兼容入口或 runtime driver 的一种协议选择，不作为内部能力上限。选择时核对实际实现声明的会话列表、恢复、历史、事件和权限能力；不能把“支持 ACP”直接解释为纯只读观察、多客户端共享写入或任意活跃终端附着。公共操作账本、目标验证与 unknown 语义仍由 PocketPilot 保持。

参考：[Agent Client Protocol](https://agentclientprotocol.com/)、[Rust SDK](https://github.com/agentclientprotocol/rust-sdk)。

## 交付顺序

1. 完成飞书真实应用的安装、长连接和卡片回调验收，核对完整附件权限。
2. 验证飞书跨项目话题、富文本/附件、未知写入和长期状态卡片的恢复路径。
3. 独立实现并验收Lark 部署域，逐项开放已验证能力。
4. 在已交付的 server 生命周期和能力契约上扩展 Claude Code 的观察/问答覆盖，并交付 Pi 的只读观察及受控输入，不让聊天入口重复实现 agent 协议。
5. 持续将已经相同的导航、确认、错误和恢复规则放入公共模块；平台身份、ACK 和原生容器管理保留各自实现。

每一阶段都以授权隔离、重复事件、过期操作、断线后任务存活、未知结果不重发及真实客户端展示为交付门槛。完成后把“已支持”行为更新到当前架构与接入方案，后续支持文档只保留尚未完成的内容。

## Agent 提问交互的后续改进

当前已实现 Codex/Claude 的原始请求回答、共用问题投影与 `!answer` 文本提交，见[操作步骤](integrations.md#回答-agent-的问题)和[接口边界](architecture.md#结构化问题与显式回答)。以下仅为尚未交付的原生交互能力：

| 平台 | 后续控件 | 必须保留的约束 |
| --- | --- | --- |
| Slack | Block Kit 单选/多选与 Modal | owner、team、绑定 thread、问题 revision |
| Discord | Components V2、选择器与 Modal | owner、guild、绑定 channel；不展示原生 raw ID |
| Telegram | Inline keyboard、多选确认、明确 reply-to | 配置 owner user ID、绑定 topic；不捕获无关联的普通消息 |
| 飞书 | JSON 2.0 卡片选择器与 form | tenant、owner、允许群与绑定 root/thread |

控件需共用 pending → draft → submitting → answered 的转换；取消后显示 cancelled，连接不可核实时显示 unavailable，发送后丢回执显示 unknown。修改草稿不写原端；确认后一次性提交全部题目。过期控件不复活，unknown 不自动重发。当前文本模板没有单独的草稿服务或持续更新的已回答/已取消卡片，不应把它们计作已经交付。

继续验证原生协议升级、非阻塞问题生命周期、更多题型、长题分段和五端真实客户端的展示与操作期限。Codex 的 `item/tool/requestUserInput` 仍为实验性协议，只有实际向当前连接投递的问题才有回答入口；不能把其他客户端的请求 ID 复制过来。Claude 的 `request_user_dialog` 尚未声明支持，不将任意 `can_use_tool` 当成用户问题。

秘密问题继续留在原生界面。工具权限审批、修改权限规则和 interrupt 不属于问答能力，不能通过添加选择器顺带开放。

参考：[Codex app-server 协议](https://developers.openai.com/codex/app-server)、[Claude Agent SDK 用户输入](https://platform.claude.com/docs/en/agent-sdk/user-input)、[Claude Agent SDK Python 实现](https://github.com/anthropics/claude-agent-sdk-python)。
