# 本地 Codex Desktop 会话接入

PocketPilot 能否管理 Desktop 活跃会话，取决于双方是否连接同一个可共享的 app-server。支持本地传输不等于可以附加任意已运行的 Desktop 进程。

## 接入条件

| 服务方式 | 接入边界 |
| --- | --- |
| Desktop 使用可共享的 SSH daemon | 可通过独立 SSH 连接加入同一 daemon；需验证远程账号、控制 socket 和 active turn 一致 |
| Desktop 本地服务只提供 stdio | 第二个客户端无法直接附加现有管道；没有共享入口时保持不可接入 |
| 已有本地 Unix/loopback WebSocket 服务 | PocketPilot 提供相应传输；需验证 Desktop、CLI 和桥接实际使用同一执行实例 |

另起 app-server 并共用历史目录，不会获得原进程的活跃执行对象。不能通过自动 resume 把另一进程中的工作会话变为可控，也不能直接改 Desktop 私有状态补齐归属。

官方能力与版本边界见 [App Server](https://developers.openai.com/codex/app-server) 和 [Desktop 命令](https://developers.openai.com/codex/app/commands)。不要假定 Desktop 支持任意 endpoint 配置，或能将已运行的 stdio 服务无中断切换为 listener。

## 部署验证

1. 确认目标版本公开支持的共享入口，不修改原生运行策略。
2. 核对执行账号、Codex 安装、数据目录和控制 socket，仅在私有部署记录中保存实际值。
3. 在专用测试会话中，让 Desktop、CLI remote 和 PocketPilot 同时读取同一 active turn。
4. 验证 steer、桥接断线后继续运行与最终历史读取，再为该 provider 确认控制权限。
5. 已有原生本地任务不自动迁移；必须避免两个执行器同时恢复同一 thread。

停止 PocketPilot 桥接后，可执行 `./dist/pocketpilot providers verify PROVIDER_ID --project PROJECT_ID` 完成第 3–4 步并开启控制，再恢复桥接。该命令创建专用测试会话，验证两连接共享 active turn、steer、断线后继续与最终回复，并验证原生附件写入；输入和文件写入都有持久操作记录。失败不改变控制资格，不重发未知写入，也不停止原生任务。`providers trust` 保留给已通过外部验证的部署作人工登记，不能替代上述验证。

若目标版本允许，也可另行评估回环 SSH 部署，让后续会话通过明确的共享 daemon 运行。启用本机 SSH、密钥和 known_hosts 属于部署者的系统配置，不由 PocketPilot 自动完成；该方案需要单独验收。

缺少共享入口时，管理已有本地活跃任务仍是不满足的能力，不能以历史浏览或另起服务替代。
