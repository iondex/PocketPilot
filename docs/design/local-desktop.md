# 本地 Codex Desktop 会话接入

2026-10-16 复核：当前 Desktop 内置 app-server 使用默认 stdio 传输；未发现 TCP listener、具名 Unix socket，默认 app-server daemon socket 查询也失败。PocketPilot 本地 Unix/loopback WS transport 已在隔离原生实验通过，但这不能证明当前 Desktop 运行实例可附加。

## 当前能做什么

| 会话位置 | 当前接入状态 |
| --- | --- |
| Desktop 中的 anjie-bsc-arb SSH 会话 | 已连接同一远端 daemon，可列历史、状态、创建、续轮和 steer |
| Desktop 原生“本机”中的活跃会话 | 暂不能从 PocketPilot 直接接入；它们仍由 Desktop 的 stdio 服务执行 |
| 本地共享 Unix/WS app-server 的会话 | 传输已实现；需要实际服务部署及共享执行实例验收后加入配置 |

单独启动另一个 app-server 并共用 ~/.codex，最多说明可能看到同一份持久化历史。它不会获得原 Desktop 活跃 turn 的 owner，也不能通过偷偷 resume 把原任务变为可控。当前未加入这种易误导的本地 provider，也没有为测试重启 Desktop。

## 确认过的官方能力

[OpenAI Docs：App Server](https://developers.openai.com/codex/app-server) 描述 stdio、Unix socket 和 WebSocket 传输，以及 CLI 的 --remote 连接方式；[Desktop 命令文档](https://developers.openai.com/codex/app/commands) 公开了从 ~/.ssh/config 添加 SSH host 的入口。当前安装的 codex app 启动命令没有“连接任意 app-server endpoint”的选项。

因此不能简单把一个 --listen 配置加给 PocketPilot，或给本机多起一个 daemon，就宣称 Desktop/CLI/Telegram 已共享当前本地活跃任务。当前官方资料也没有证明可以把正在运行的 stdio app-server 无中断切换为监听模式。

## 后续可验证的本机共享路径

候选方案是把这台 Mac 以一个明确的 SSH alias 加入 Desktop，让后续会话通过 SSH 使用本机常驻共享 daemon；PocketPilot 和 CLI remote 模式连接同一 socket。它复用已有的 SSH 接入边界，而不是假设 Desktop 有任意 WS endpoint 设置。

此方案尚未在这台 Mac 验收，不能保证所有 Desktop 版本都支持这种回环部署。当前本机 127.0.0.1:22 不可连接，也未开启或修改 macOS Remote Login。验证步骤需要明确的本机 SSH 部署工作：

1. 用户在 macOS 开启仅授权账号可用的 Remote Login，配置本机 alias、密钥与 known_hosts。PocketPilot 不自动修改系统服务。
2. 通过 Desktop 的 Settings → Connections → SSH 添加该 alias，核对该版本支持本机 Mac 作为 SSH 目标及原生 daemon 的部署方式。
3. 记录确切用户、binary、CODEX_HOME 和共享控制 socket；先在一个新测试 workspace 中只读检查。
4. 专用新会话验证 Desktop、CLI remote 和 PocketPilot 同时观察相同 active turn，验证 steer、最后桥接连接退出后继续运行和历史读取，再为该 provider 开启控制。
5. 后续本机新任务在这个 SSH 连接下创建。已有 Desktop 原生本机会话不自动迁移；只能在原任务自然完成后，另行验证空闲历史的显式接入，且不能同时由两套执行器恢复同一 thread。

如果回环 SSH 在目标版本不支持，就等待/采用 Desktop 正式支持的共享本地 daemon 入口。当前“管理本机 Desktop 活跃会话”仍是未满足的原始要求，不能用仅历史浏览或独立 CLI 服务代替。
