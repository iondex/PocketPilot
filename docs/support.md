# 后续支持规划

PocketPilot 的扩展目标是让同一套会话体验覆盖更多消息工具和编程运行时。新增消息端复用已有 agent 能力；新增 runtime 复用已有聊天入口、授权和操作账本。

| 方向 | 目标 | 当前边界 |
| --- | --- | --- |
| 飞书 / 国际 Lark | 群 thread、丰富卡片与附件 | 国内接入已实现，待真实安装验收；国际 Lark 待独立验证 |
| Claude Code | 已有独立 server 首版；后续补外部会话索引与结构化问答 | server 管理会话；DeepSeek 兼容接口已验证输入及最终回复，结构化问答和权限流程仍待实现 |
| Pi Coding Agent | 长驻 RPC 与会话管理适配 | 待实现，需要明确进程所有权、版本和输入调度语义 |
| 公共交互 | 降低平台间重复逻辑，统一错误、恢复与能力呈现 | 按实际相同规则逐步收束，保留原生控件和授权差异 |

当前能力见[接入方案](integrations.md)和[当前架构](architecture.md)。以下保留尚未交付或仍需真实环境验证的能力。

## 飞书的后续验证与国际 Lark

国内飞书接入已实现，安装与权限步骤见[接入方案](integrations.md#飞书企业自建应用)。当前完成离线协议、scope、重复输入、绑定状态转换、附件资源与富文本预算验证；尚无真实飞书应用凭据，不能把离线 fixture 等同于安装和客户端验收。

后续优先完成真实租户的长连接、发布权限、卡片回调和跨项目 thread 隔离，检查独立附件投递、移除机器人/权限撤销、长时间根卡片过期及断线后的用户恢复。原生宽表、更多富文本扩展与消息直达链接按客户端实际能力开放；未核实的数学、脚注和折叠结构保留可读文本降级。

国际 Lark 尚未开放配置。需要独立应用凭据、域、tenant/用户/group scope 及端到端验证；不能仅切换 base URL 就复用国内安装状态。未来若增加国际部署，仍使用同一 integration 体系和 SDK seam，不复制一套导航与原生 runtime 控制逻辑。

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

后续补充只读外部 transcript 索引、状态事件覆盖、结构化问题和真实端到端验证。活跃轮次 steer 未证明，不能将排队输入当成即时 steer。工具权限请求目前保持 pending，不自动批准或拒绝；原生提问的回答接口按下面的公共模型单独交付。

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
3. 独立实现并验收国际 Lark 部署域，逐项开放已验证能力。
4. 在已交付的 server 生命周期和能力契约上扩展 Claude Code 的观察/问答覆盖，并交付 Pi 的只读观察及受控输入，不让聊天入口重复实现 agent 协议。
5. 持续将已经相同的导航、确认、错误和恢复规则放入公共模块；平台身份、ACK 和原生容器管理保留各自实现。

每一阶段都以授权隔离、重复事件、过期操作、断线后任务存活、未知结果不重发及真实客户端展示为交付门槛。完成后把“已支持”行为更新到当前架构与接入方案，后续支持文档只保留尚未完成的内容。


## Agent 提问与用户回答（规划，尚未实现）

聊天入口应能展示 agent 的问题，并将用户答案送回原始请求。此功能与工具权限审批分开；问题按钮不会批准 Bash、文件操作或修改权限规则。普通消息输入继续使用现有授权、绑定、文件和操作账本。

### 两条回复路径

| 场景 | 展示与回复 | 原生操作 |
| --- | --- | --- |
| 普通 agent 文本提问 | 保留正文 Markdown；用户直接在绑定 thread/topic/channel 回复 | Codex 活跃轮次使用带 `expectedTurnId` 的 steer，空闲轮次使用 start；Claude 空闲时发送下一轮 user input |
| Codex 结构化提问 | 按问题顺序显示标题、选项说明与允许的自由输入；提交前显示待提交答案 | 对原始 JSON-RPC request ID 返回 `ToolRequestUserInputResponse`；不新建 turn，不以 steer 冒充答案 |
| Claude `AskUserQuestion` | 保留原问题、单选/多选和自由输入；答案提交到原始 tool request | 对匹配的 `control_request.request_id` 返回 `control_response`，保留原工具输入并补充 answers |
| 其他工具权限请求 | 仅展示“等待原生端权限处理”状态 | 保持 pending，不批准、不拒绝、不发送 interrupt |

Claude 的活跃输入不能因传入 stream-json 就宣称为 steer。当前 server 对活跃轮次返回不支持；普通文本回答在 agent 已结束轮次时通过 user input 继续。以后只有证实原生插入时机及前置轮次约束后，才开放真正 steer；不会偷偷排队下一轮或中断现有执行。

### 公共模型和模块边界

`session` 定义具名 `QuestionSet`、`Question`、`Option`、`AnswerDraft`、`AnswerSubmission` 与 `QuestionState`，描述顺序、稳定 ID、单选/多选、可选自由输入和是否阻塞。运行时能力单独声明结构化问题的读取、回答、多选与自由输入支持；不将 `control_verified` 当成所有能力都存在。

问题的逻辑身份至少包括 provider/instance、session、turn、item/tool-use、原生 request ID、连接 epoch 和问题 revision。问题正文与答案是用户数据，不能放进日志或按钮 value；原生请求数据只在 adapter/server 内保存为有界的具名 DTO，integration 只拿不透明 question handle。Claude 无稳定 question ID 时生成本请求内 ID，同时保存到原完整问题文本的映射。

`adapters` 负责捕获原生请求、能力协商和原样返回协议响应。独立 server 持有 Claude 的 stdin 与 pending request，因此聊天断开不撤销原生问题。`engine` 负责问题仓库、有效性核验与提交账本。`integrations` 公共层负责选择、修改、确认和提交的相同状态转换；各平台仅负责组件、回调身份和输入路由，不复制 Codex/Claude 的协议。

初版接口建议为 `pending_questions(session)`、`question_changes()` 和显式 `answer_question(submission)`，与 `execute(Mutation)` 的普通输入分开。只读观察不持有自动回答路径。提交必须在持久化授权后，经实例锁/会话锁重新检查目标、revision、epoch、当前原生 pending 状态和允许回答的用户。

### 原生协议映射

Codex：已核对本机 `codex-cli 0.162.1` 生成的 JSON schema。方法是实验性 `item/tool/requestUserInput`；params 包含 `threadId`、`turnId`、`itemId`、`isBlocking` 和 questions，单题有 `id`、`header`、`question`、`options`、`isOther`、`isSecret`。不要从旧 `autoResolutionMs` 自行推算默认答案或自动回答期限。响应形状为 `answers: {question_id: {answers: [string, ...]}}`，通过原连接返回原 request ID。构建时继续使用生成 DTO；升级 CLI 后复核实验性字段，并保留 initialize 的实验能力协商。只能回答投递到本连接且仍可核验的原生 request；不能把其他 Desktop/CLI 连接上的 request ID 复制过来接管问题。

Claude：按官方 Agent SDK 的 `can_use_tool` / `AskUserQuestion` 边界实现。保存原 questions、`request_id` 与 `tool_use_id`；成功响应为 `control_response.response = {subtype: success, request_id, response: {behavior: allow, updatedInput: ...}}`。这里的 allow 只准用于已经核验为 AskUserQuestion 的原请求；不能给任意 `can_use_tool` 共用一个“允许”函数。`updatedInput` 保留原 questions，answers 以完整问题文本为 key、所选 label/自由文本为 value；当前 SDK 为 string map，多选按该版本的约定序列化，不能擅改为数组或用展示序号代替 label。不发送 `updatedPermissions`。

Claude CLI 2.1.304 / SDK 0.3.304 的 initialize 响应还提供 `pending_permission_requests` / `pending_user_dialog_requests`。恢复时应合并快照和 live/replayed request，以原 request ID 去重；字段缺失不能当成空列表。继承自旧 worker 的请求未必出现在列表中，不能从本地 pending 为空推断原生端没有等待项；未知继承请求仍需只读核验，不能重建猜测的 request ID。

Claude 的 `control_cancel_request` 立即使 pending 及未提交草稿失效。Codex 的请求取消、连接替换、原生轮次结束或原始 request 不再 pending 时也要废弃旧入口；事件缺口只能触发只读重新核验，不能触发默认回答。仅在原生协议证明非阻塞问题仍有效时保留其回答入口，不能单凭 turn 已结束推断所有问题有效或失效。

### 聊天交互

| 平台 | 选项与回答入口 | 路由边界 |
| --- | --- | --- |
| Slack | Block Kit 单选/多选组件；Modal 收集自由文本和多题；thread 中保留状态摘要 | App、team、owner、绑定 thread 与 question revision |
| Discord | Components V2 展示；select/menu 选择；Modal 输入自由文本；多页只保留草稿，不提前发送 | guild、owner、绑定 channel、question revision；不向用户展示 raw ID |
| Telegram | Inline keyboard 选择；多选有“确认”按钮；自由文本回复指定问题消息 | 固定允许 user ID、群 topic、问题消息；无关联文本继续走普通输入 |
| Lark / 飞书 | JSON 2.0 卡片单选/多选与 form；超过 input 长度上限时使用带明确问题引用的 thread 文本 | 域、app、tenant、owner、允许群、root/thread、revision |

全平台显示“❓ Agent 提问”、问题序号、选项说明、草稿/已提交/已取消状态；回答成功后关闭旧控件，避免多次点击。多题完成并确认后才一次性回传。只存在一个 pending 问题也不能把所有普通文本自动截走：自由文本回答必须来自专用表单、明确 reply-to 或显式“回答此问题”上下文；没有这个上下文的文本按普通输入处理。群内卡片是群可见信息，不能用按钮的用户校验假装内容私密；`isSecret` 问题不提供群内填写，初版提示回原生端处理。

### 可靠性与验收顺序

1. 先补 adapter 的请求/响应读写边界和 fake-runtime fixtures：结构化请求、取消、重复 request、错误 session、旧 epoch、事件缺口；不启动真实模型。
2. 引入问题状态机：pending → draft → submitting → answered；取消进入 cancelled，原生状态无法核实进入 unavailable，发送后丢回执进入 unknown。草稿修改不发送原生响应；取消/失效后不能复活。
3. 以提交 ID 记录答案 digest、目标 revision 和 dispatch intent。一次提交最多写一次；unknown 不重发。重连后先查询可恢复 pending，绝不将旧 JSON-RPC ID 回到新连接；server 重启不能仅凭数据库中的旧 pending 发送答案。
4. 用同一组公共行为测试覆盖四个平台的越权点击、多选修改、自由输入、重复提交、陈旧菜单和并发取消；另验证 native wire 返回原 ID、原 questions 未丢字段、其他工具权限没有被回答。
5. 最后逐个平台做真实测试，确认富文本、交互响应期限及长题分段，再将对应能力由“规划”升为“已支持”。没有凭据的平台保持离线验证状态。

参考：[Codex app-server 协议](https://developers.openai.com/codex/app-server)、[Claude Agent SDK 用户输入](https://platform.claude.com/docs/en/agent-sdk/user-input)、[Claude Agent SDK Python 实现](https://github.com/anthropics/claude-agent-sdk-python)。
