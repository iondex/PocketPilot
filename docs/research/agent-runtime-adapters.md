# Agent runtime server adapters 规划

核验日期：2026-10-20（Asia/Shanghai）。本轮仅规划；没有安装 Pi、启动 agent 任务、读取私人会话或实现新 runtime。
本机只执行 `claude --version`、`claude --help`，版本为 Claude Code 2.1.289。

## 结论与边界

Codex app-server 已提供较完整的持久会话、事件和控制服务；Claude Code 的 SDK/CLI 与 Pi RPC 不能直接视为同构 server。
建议每个执行主机部署一层 runtime server：保留 Codex 原生服务，对 Claude/Pi 包装其官方接口，统一会话目录、连接恢复和写操作账本。
`integrations` 接收消息并呈现控制界面；`connections` 仅承担 local/SSH proxy 传输；`adapters` 隐藏 agent runtime 协议。
runtime server 的 API/持久化/授权与具体 runtime driver 分层，不能让 Slack、Discord、Telegram 或 SSH transport 依赖 Codex DTO。
这是一项后续设计，不要求现在为 Claude/Pi 添加空壳实现或声明已经支持。

```text
integrations → application/session orchestration → connections (local / SSH proxy)
                                                     ↓
                                            per-host runtime server
                                           API + registry + journal
                                                     ↓
                                    adapters: Codex / Claude / Pi drivers
```

已有原生会话可以被发现，不等于运行中的原生进程可以被接管。
必须分别记录 `discovered`、`observable`、`owned`、`writable` 和 `live_attach` 能力；仅有磁盘转录时状态应为 unknown/stale。
浏览、会话列表和只读历史读取不应启动 CLI、恢复上下文、运行 hooks 或调用模型。
只有用户显式提交任务，且验证控制能力后，才可创建、恢复、上传和写入。

## 能力矩阵

“server补齐”表示 PocketPilot 需实现并验证；不代表上游已经保证。各能力以运行时版本协商为准。[C1][A1][P1][S1]

| 能力 | Codex app-server | Claude SDK/CLI | Pi RPC 0.73.1 | ACP v1 / v2 |
|---|---|---|---|---|
| 持久会话发现 | `thread/list`，过滤、分页 | SDK `listSessions`，dir/limit/offset | RPC 无全局 list；SDK/文件索引 | v1 optional `session/list`；v2 session baseline |
| 独立只读历史 | `thread/read`及历史分页 | SDK `getSessionMessages`/`getSessionInfo` | 只读解析 JSONL；`get_messages`仅当前会话 | v1 load重放，非独立read；v2 resume可重放 |
| 观察已有进程 | 同一server中的线程可读取状态；跨server另行验证 | 自有stream可观察；普通TTY不能假定可附着 | 自有RPC进程可观察；普通TTY无公开附着RPC | 有update不等于有纯observe/attach端点 |
| 多客户端 | 原生server支持连接；共享权限需验证 | server持有child并分发；daemon/remote能力单独评估 | server持有stdin/stdout并分发 | 未统一规定共享writer仲裁或任意TTY接管 |
| 创建/恢复 | `thread/start` / `thread/resume` | SDK query选项/CLI `--session-id`、`--resume` | `new_session` / `switch_session`、`--session` | `session/new`；v1 optional load/resume |
| 运行中输入 | `turn/steer`，指定当前turn | stream-input可追加；调度语义版本相关 | `steer`与`follow_up`明确分开 | v1无标准steer策略；v2支持运行中prompt，策略未统一 |
| 状态/完成 | thread状态、turn/item事件 | system/init、assistant、stream_event、result；新SDK有状态事件 | get_state、agent/turn/message/tool/queue事件 | v1 prompt响应代表轮次结束；v2 state_update独立 |
| 审批/用户输入 | 服务器请求及相应响应 | SDK canUseTool/control请求；取决于permission配置 | extension UI；无统一内置工具审批屏障 | request_permission标准；elicitation按能力 |
| 文件/图片 | fs API；原生图片输入 | user content图片；文件需host暂存/读取 | prompt/steer images；文件需host暂存/读取 | resource_link基础；图片/embeddedContext协商 |
| 重连/缺口 | 重读快照及历史；不能假定通知永久重放 | SDK reinitialize恢复待答控制请求，非完整event日志 | 管道断开无标准重附着；server须持有进程 | v1 resume/load；v2 replayFrom并非持久event offset |
| 未知写结果 | 保留unknown，禁止盲目重试 | 同左；UUID/请求ID不是通用幂等承诺 | 同左；RPC id只用于相关性 | JSON-RPC id不保证exactly-once |

## Claude Code：采用官方 SDK 语义，谨慎维护 Rust wire driver

官方 TypeScript SDK 当前发布版 `@anthropic-ai/claude-agent-sdk@0.3.304` 导出 typed 消息、Query 与控制信封；
Python SDK 开源实现可核验实际 CLI 参数、JSONL framing、initialize 和权限请求相关性。[A1][A2]
本机2.1.289与最新SDK不是同一版本，最新SDK字段不能直接假定本机全部支持；需锁版本和协议fixture。
推荐优先使用官方 SDK sidecar；若坚持纯 Rust，wire decoder需隔离在 `adapters::claude`，接受未知事件，不把未文档化行为变成公共契约。

```text
cwd = 用户已选项目的绝对路径
claude -p --input-format stream-json --output-format stream-json --verbose \
  --include-partial-messages --replay-user-messages --resume <session-uuid>
```

创建会话时使用新 `--session-id <uuid>`，与resume分开；持久化应开启，不能使用 `--no-session-persistence`。
上述仅为启动参数规划，没有执行。默认不添加绕过权限参数，也不自动扩大tools/目录授权。
最小 user 输入及控制相关性如下；初始SDK控制握手应按锁定版本生成，不能只拼下面一行就声称支持完整SDK。[A1][A2]

```json
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"用户请求"}]},"parent_tool_use_id":null}
{"type":"control_request","request_id":"request-1","request":{"subtype":"initialize"}}
{"type":"control_response","response":{"subtype":"success","request_id":"request-1","response":{}}}
```

读取路径：官方SDK `listSessions({dir,limit,offset})`、`getSessionInfo(id,{dir})`、`getSessionMessages(id,{dir,limit,offset})`；
它们读取 `~/.claude/projects/<encoded-project>/` 下的JSONL，不要求运行模型；尊重 `CLAUDE_CONFIG_DIR`，编码规则不能自行猜测。[A1][A3]
SDK读取恢复parentUuid会话链，比逐行把全部JSONL当当前会话更可靠；对外只返回产品需要的摘要和消息，不提供历史导出。
实时路径：消费 `system/init`、`assistant`、`stream_event`、`result`、控制请求及版本支持的 `session_state_changed`。
`result` 是一轮的完成信号，后续仍可能有系统/后台通知；不可把stdout关闭作为唯一轮次完成信号。[A1]
图片使用Anthropic content block；任意文件由server在目标主机授权目录暂存后提供路径，不能把聊天附件URL直接交给runtime。

本机help另有 `--bg`、`agents`、`attach`、Remote Control；最新SDK也有daemon重连和browser transport相关类型。
这些能力值得后续单独验证，但不能据此推断任意正在运行的TTY都提供公开稳定的Rust可用共享socket。[A1]
SDK `reinitialize()`可重新获取仍阻塞的权限/对话请求；文档明确重注册hooks可能取消待答回调，因而不属于无副作用observe。[A1]
最新消息类型的 `priority: now|next|later` 不能无验证映射为Codex steer；首版应只开放空闲续轮，完成调度语义验证后再开放运行中输入。

## Pi Coding Agent：长驻 RPC，由 server 管理进程与会话目录

用户确认的包是 `@mariozechner/pi-coding-agent`。核验时旧包latest为 **0.73.1**，官方deprecated提示改用
`@earendil-works/pi-coding-agent`，新包latest为 **1.3.0**；原GitHub仓库已迁移。[P0]
包迁移和版本升级需作为显式部署选择；此处以旧0.73.1的固定源码描述最小契约，另标新版本能力。[P1]

```text
cwd = 用户已选项目的绝对路径
pi --mode rpc --session <已验证的绝对session路径>
pi --mode rpc --session-dir <server管理的会话目录>
```

正式会话不使用 `--no-session`；RPC模式不接受 `@file` prompt参数，输入通过stdin发送。
JSONL仅以LF分帧，不能用把U+2028/U+2029当换行的reader；stdout持续消费，stderr仅日志。[P1][P2]

```json
{"id":"state-1","type":"get_state"}
{"id":"state-1","type":"response","command":"get_state","success":true,"data":{"isStreaming":false,"isCompacting":false,"pendingMessageCount":0,"sessionId":"opaque-session"}}
{"id":"prompt-1","type":"prompt","message":"用户请求","images":[{"type":"image","mimeType":"image/png","data":"base64"}]}
{"id":"steer-1","type":"steer","message":"运行中的补充说明"}
{"id":"next-1","type":"follow_up","message":"本次完成后继续"}
```

`steer`在当前assistant工具调用完成、下次模型调用之前交付；`follow_up`等agent没有待续工作后交付，不能互换。[P1]
`new_session`/`switch_session`成功响应仍可能含 `cancelled:true`（extension拒绝）；必须再核对get_state身份。
旧版使用 `agent_start/end`、`turn_start/end`、message/tool事件以及queue/compaction/retry事件；不能看到agent_end就忽略后续自动retry。
新版1.3.0文档有 `agent_settled`、prompt `disposition`、`get_entries`/`get_tree`；旧0.73.1 typed协议没有这些字段/方法。[P1][P2]
会话是append-only树，路径通常为 `~/.pi/agent/sessions/--<path>--/<timestamp>_<uuid>.jsonl`；只读索引需尊重覆盖目录与id/parentId。[P3]
加载老格式会迁移文件，因此纯浏览不可用 `switch_session`/启动CLI代替只读解析。[P3]
通用文件没有上传RPC；host server应实现独立受限暂存接口，不能为了传文件而调用Pi `bash`污染上下文。
extension UI select/confirm/input是扩展交互，并不保证所有工具调用先经过统一审批；不得宣称与Codex沙箱/审批等价。

## ACP：可选兼容面，不应成为内部能力上限

ACP v1当前稳定schema为 **v1.26.0**（2026-10-16发布）；`session/list`于2026-03-09稳定，
`session/resume`于2026-04-22稳定，均需initialize能力声明，不能继续称为“ACP没有list/resume”。[S1][S2]
v1 `session/load`需 `loadSession` 并重放完整历史；`session/resume`需 `sessionCapabilities.resume`，不重放历史但恢复上下文/连接MCP。
因此它们并非纯只读 `session/read` 或 `observe`。图片、音频、embedded context按promptCapabilities协商。[S1]
ACP v1稳定包含权限请求、工具事件、模式/配置、会话信息更新，以及可选close/delete/additionalDirectories。
schema.unstable新增 `session/fork`、provider管理和MCP消息通道等；是否可用还取决于具体agent实现。[S1]

ACP **v2仍按Draft评估**：官方2026-07-20公告要求版本协商与谨慎启用，不能将其视为v1生产保证。[S3]
v2 session面将list/resume/close列为基础；prompt响应表示接受并返回messageId，工作结束用独立state_update表示。
state为running/idle/requires_action，resume后必须给当前状态；允许后台事件、消息ID patch和resume `replayFrom:{type:start}`。[S3][S4]
这明显改善server适配，但通用steer交付时机、持久event offset、写入幂等账本、共享writer仲裁仍不是统一保证。
不能把“支持多客户端观察的协议基础”描述成“可直接附着任意现存原生TTY”。
若后续接ACP，作为host server的一种边界driver/兼容入口，保留本项目更明确的观察、ownership和写入结果语义。

## Host server 的最小完备契约（建议，尚未实现）

- `capabilities`：协议/CLI版本、runtime种类、观察/控制/附件/steer/审批能力及限制；能力缺失返回typed Unsupported。
- `sessions.list/read`：project过滤、稳定分页、摘要、native id、state来源和时间；读取绝不隐式resume。
- `sessions.observe`：先快照再事件，明确journal cursor/epoch/缺口；客户端重连从cursor恢复，过期则重读快照。
- `sessions.create/resume`：显式写操作；按主机+runtime+native id管理单writer lease，外部活跃但不归server所有时拒绝控制。
- `input.submit`：typed Text/Image/StagedFile、delivery策略、expected session revision/active turn；返回accepted与最终完成两个阶段。
- `operations.read`：操作UUID、payload hash、prepared/sent/accepted/completed/rejected/unknown；冲突payload拒绝，unknown不重发。
- `files.stage`：受限目标目录、内容hash、大小/MIME/文件名校验；上传与prompt分别落账，SSH transport不承载业务语义。
- `events`：状态、消息delta/final、工具进展、待用户操作、轮次完成、连接缺口；外部未知状态不能伪造idle。

API server负责鉴权、锁、持久账本、journal与背压；runtime driver负责官方协议映射、版本能力和进程生命周期。
Claude/Pi进程由server持有，客户端掉线不关闭其stdin、不kill正在执行的任务；host server重启无法证明存活时保留unknown。
默认先实现只读目录与明确owned会话控制，使用fixture验证接受后断连、重复输入、分支历史、失序事件和过期审批。
不自动重放模型任务、不后台恢复外部会话；不新增历史导出、自动停止/中断或自动回答审批。

## 主源

- [C1: OpenAI Codex App Server](https://learn.chatgpt.com/docs/app-server#api-overview)，官方OpenAI文档；部署仍以锁定schema为准。
- [A1: Anthropic官方SDK 0.3.304](https://www.npmjs.com/package/@anthropic-ai/claude-agent-sdk/v/0.3.304)，核验发布包 `sdk.d.ts` / `browser-sdk.d.ts`；[官方仓库](https://github.com/anthropics/claude-agent-sdk-typescript)。
- [A2: 官方Python SDK控制层](https://github.com/anthropics/claude-agent-sdk-python/blob/main/src/claude_agent_sdk/_internal/query.py)及[CLI transport](https://github.com/anthropics/claude-agent-sdk-python/blob/main/src/claude_agent_sdk/_internal/transport/subprocess_cli.py)。
- [A3: 官方SDK只读会话实现](https://github.com/anthropics/claude-agent-sdk-python/blob/main/src/claude_agent_sdk/_internal/sessions.py)；Anthropic文档站此次403，以上使用官方源码/发布包核验。
- [P0: 旧包官方元数据](https://registry.npmjs.org/@mariozechner/pi-coding-agent/latest)及[新包元数据](https://registry.npmjs.org/@earendil-works/pi-coding-agent/latest)。
- [P1: Pi 0.73.1固定RPC文档](https://github.com/badlogic/pi-mono/blob/781152fc24841dc54b22284514604048ebe5e2c9/packages/coding-agent/docs/rpc.md)及[typed协议](https://github.com/badlogic/pi-mono/blob/781152fc24841dc54b22284514604048ebe5e2c9/packages/coding-agent/src/modes/rpc/rpc-types.ts)。
- [P2: Pi当前RPC文档](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/rpc.md)及[commands](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/rpc-commands.md)。
- [P3: Pi 0.73.1会话格式](https://github.com/badlogic/pi-mono/blob/781152fc24841dc54b22284514604048ebe5e2c9/packages/coding-agent/docs/session-format.md)。
- [S1: ACP v1稳定schema](https://github.com/agentclientprotocol/agent-client-protocol/blob/main/schema/v1/schema.json)与[unstable schema](https://github.com/agentclientprotocol/agent-client-protocol/blob/main/schema/v1/schema.unstable.json)。
- [S2: List稳定公告](https://github.com/agentclientprotocol/agent-client-protocol/blob/main/docs/announcements/session-list-stabilized.mdx)、[Resume稳定公告](https://github.com/agentclientprotocol/agent-client-protocol/blob/main/docs/announcements/session-resume-stabilized.mdx)、[稳定发布](https://github.com/agentclientprotocol/agent-client-protocol/releases/tag/schema-v1.26.0)。
- [S3: ACP v2 Draft公告](https://github.com/agentclientprotocol/agent-client-protocol/blob/main/docs/announcements/acp-v2-draft.mdx)。
- [S4: ACP v2生命周期](https://github.com/agentclientprotocol/agent-client-protocol/blob/main/docs/protocol/v2/prompt-lifecycle.mdx)及[v2 schema](https://github.com/agentclientprotocol/agent-client-protocol/blob/main/schema/v2/schema.json)。
