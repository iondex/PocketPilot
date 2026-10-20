# PocketPilot

PocketPilot 将聊天平台上的用户操作连接到执行环境中的 agent 会话。原生 agent runtime 拥有任务执行和会话生命周期。

## Language

**Integration**：消息的接收和控制端，当前为 Telegram、Discord、Slack。它表达用户导航、输入、授权和会话结果。
_Avoid_：将 integration 称为 agent adapter 或主机 connection。

**Connection**：到执行环境的连接方式，当前关注 local proxy 与 SSH proxy；它不决定使用哪一种 agent。
_Avoid_：将聊天平台的 HTTP 代理或某个 agent runtime 本身称为 connection。

**Adapter**：agent runtime 的适配端；Codex 是当前运行实现，Claude Code 和 Pi Coding Agent 是规划目标。适配器表达原生会话、状态、事件和允许的控制能力。
_Avoid_：把 ACP 当作所有 runtime 的完整能力模型。

**Workspace**：用户导航中的执行环境或主机，如本机或一台远程机器。
_Avoid_：在聊天导航中用 workspace 表示项目目录。

**Project**：某一执行环境中允许访问的项目目录，可含已明确允许的关联目录。
_Avoid_：将项目与会话、主机混用。

**Session**：某个 agent runtime 中的原生会话。不同 integration 可以关联同一个会话，而不取得其原生执行生命周期的所有权。

**Runtime server**：向 PocketPilot 提供会话发现、状态观察、事件与明确控制接口的服务。Codex app-server 是现有参照；其他 runtime 可由适配服务提供这些能力。
