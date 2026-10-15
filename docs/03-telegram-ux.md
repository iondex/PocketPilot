# 03 · Telegram 交互设计

## 1. 信息结构

MVP 使用一个私有、开启 Topics 的 forum supergroup。一个专门的「PocketPilot 控制台」topic 管工作区和会话列表，其余每个 topic 绑定一个具体 Codex thread。工作区通过菜单筛选，不给 Telegram 人为套一层不存在的 topic 分组。

```text
PocketPilot 私有群
  PocketPilot 控制台
  🟢 PP 修复测试 · a13f5b
  🟡 WEB 登录回归 · d8e271
  ⚪ PP 发布准备 · c051a2
  ❔ GPU 性能调查 · 6b9a02
```

前缀是工作区短名，中段是可读短标题，末尾是会话短标识，状态 emoji 是缓存提示。完整信息在每个 topic 的固定状态卡；topic 名称不是权威状态。

最初仅配置一个受信用户；数据模型允许多用户，但个人 MVP 不承诺团队权限管理。Bot 创建、加入群、授予权限和填写 token 属于安装准备，不在当前文档任务中自动执行。

## 2. 安装与权限

Telegram adapter 启动前检查 getMe/getChat/getChatMember 等必要信息：目标 chat 是 forum supergroup、bot 有 topic 管理和发消息能力。固定状态卡需 can_pin_messages；缺权限时给出具体修复步骤，不能无提示失败。

Bot 作为管理员通常可收群内普通用户消息，官方 FAQ 明确 bot admins 可收到普通群消息（其他 bot 消息除外）。安装验收必须发送一条非命令文本确认实际接收，不仅看 privacy mode 字段。若需要调整 privacy mode，在 BotFather 处理；不能默认所有群消息都已可见。

授权同时检查允许的 bot/chat/user、绑定的 workspace/instance。私聊默认只显示帮助或安装状态，不允许转发到任意会话。未授权群/用户的按钮和文本均不触发 agent；不根据 username 授权。匿名管理员、sender_chat、转发来源均不能替代真实用户身份。

接收更新至少覆盖 message、callback_query、必要的 my_chat_member 与 topic 服务事件。编辑消息、普通回复、媒体、bot/service 消息分开处理；不因用户编辑旧 Telegram 文本而自动再次发送或回滚 Codex 输入。

## 3. 控制台入口

`/start` 或 `/workspaces` 显示：

```text
PocketPilot
本地 · 已连接      远程开发机 · 恢复连接中

选择工作区
[ PP · PocketPilot ] [ WEB · 网站 ]
[ GPU · 实验 ]       [ 添加工作区 ]
[ 连接诊断 ]         [ 刷新 ]
```

选择工作区：

```text
PP · PocketPilot
Host: 开发机   Agent: Codex   路径: /srv/code/PocketPilot

[ 选择已有会话 ] [ 新建会话 ]
[ 已绑定会话 ]   [ 修改本地显示名 ]
[ 返回工作区 ]
```

所有菜单按钮用 opaque action nonce。callback_data 限 1–64 bytes，不能塞完整路径、thread UUID JSON 或 secrets。服务端 nonce 绑定授权用户、chat、binding revision、操作类型与过期时间。先 answerCallbackQuery，慢操作再更新卡片。重复点击返回已有操作进度，不再执行一次创建。

## 4. 添加工作区

流程：选择已配置 host → 选择 Codex instance → 输入目标机绝对路径 → 输入显示名/短名 → 展示核对卡 → 保存。

```text
添加工作区
名称: PocketPilot    短名: PP
Host: 开发机          Agent: Codex
路径: /srv/code/PocketPilot
接入: 已验证共享实例

[ 保存 ] [ 修改 ]
```

这一步只登记，不启动会话。Host/SSH endpoint 初始在本机 TOML 配置；不允许从 Telegram 任意输入服务器命令、SSH options 或可执行文件路径。工作区路径限制在该 host 预配置的允许根目录下；校验 `..`、空字节、相对路径。远程 symlink 归属必须由只读验证或显式登记确认，不能只做字符串 starts_with。

可增加「从已有会话发现路径」：列出未登记 cwd 候选，用户选定后仍走保存流程。明确标为候选，不称“全部工作区”。无会话的项目不会出现。

同仓库 worktree 默认单列候选；主工作区关联 worktree 要显式确认。按精确 cwd 查会话；多个关联 cwd 合并结果按 SessionRef 去重。

## 5. 选择并绑定已有会话

菜单分页，每页约 8 项，支持按标题搜索。默认近期非归档交互会话；用户可展开历史/归档只读，且明确过滤条件。不自动导入所有子 agent、不自动 unarchive。

```text
PP · 选择会话                 1/4

🟢 修复异步测试         2 分钟前 · a13f5b
🟡 等待命令批准         8 分钟前 · d8e271
⚪ 发布准备             昨天 · c051a2
❔ 性能调查             状态不可确认 · 6b9a02

[ 上一页 ] [ 下一页 ] [ 搜索 ] [ 返回 ]
```

点击某项先展示详情：完整标题、host、cwd、thread 短 ID、真实状态及更新时间、当前是否已绑定。

- 已验证 loaded thread：按钮「连接到此会话」，说明加入原会话、若等待期间已卸载则可能加载历史；添加订阅不开始新 turn。
- 存储历史且未加载：按钮「恢复并连接」，说明会在此服务加载该历史；只有可证明 ownership 的共享实例允许。
- 活跃状态 ownership 不明：仅「查看历史」「连接诊断」，不提供继续按钮。
- 已绑定：直接返回 topic 链接，不创建副本。

Codex 当前没有原子 loaded-only resume，详情页不能承诺“绝不触发历史加载”。用户未同意加载时只读浏览；是否允许重连时加载同一历史也随 binding 明确保存。MVP 同一 topic 永远只关联同一 thread；解绑后要连接另一个 thread 必须新建 topic，避免迟到文本串台。

绑定是外部资源 saga：先预留 operation/唯一键，创建未绑定 topic，保存返回 ID，再 join，验证成功后建立 binding。失败时 topic 显示“尚未连接”，用户可重试**已知未执行步骤**或清理空 topic；不删除/归档 Codex thread。某步超时未知时暂停核对，不再自动创建第二个 topic。

新绑定默认只显示最近 5 条已完成可见文本和当前 active 文本（若可得），标记“历史摘录”；完整历史用 `/history` 分页。不是把旧历史默认全部灌进群。实时绑定以后每条可见 user/agent 文本按队列同步，历史默认显示条数不限制实时转发覆盖范围。

## 6. 新建会话

流程：选择工作区 → 可选本地短标题 → 输入第一条任务消息 → 展示发送目标 → 「创建并发送」。

```text
新建 Codex 会话
工作区: PP · PocketPilot（开发机）
标题: 调查测试失败
任务: 找出最近新增测试失败的原因，先给结论。
策略: 使用工作区已配置的新会话默认策略

[ 创建并发送 ] [ 修改 ]
```

顺序：持久化 provisioning → 创建 topic 保存 ID → thread/start 保存 thread ID → join/bind → 新 operation 发送首条消息。thread/start 与首条消息分开记录，避免首条消息未知时再次创建 thread。空 thread 是否持久化/桌面可见属于版本验收事项；首条消息成功后才宣告“会话已创建并可继续”。

新建专用 profile 可在安装时明确设 model/approval/sandbox；默认继承服务/工作区配置。界面不把“关闭审批”作为便捷默认。输入短标题只存在 PocketPilot，既有 Codex 标题不被改写；新线程在 desktop 的标题可能由首条消息产生，验收需核对展示效果。

## 7. 会话 topic 与状态卡

状态卡始终保留一个可更新的 bot 消息，并固定（若有权限）：

```text
调查测试失败
PP · 开发机 · Codex
Thread: a13f5b     当前轮: 77cd21
状态: 正在运行 · 3 秒前确认
最近一轮: 已完成
同步: 正常    待发送回复: 0

[ 刷新状态 ] [ 最近消息 ] [ 完整信息 ]
[ 修改显示名 ] [ 解除绑定 ]
```

实际不同状态：

| 展示 | 含义 | 输入行为 |
| --- | --- | --- |
| 🟢 正在运行 | active 且可确认当前 turn | 普通文本作为 steer |
| 🟡 等待审批 | active + waitingOnApproval | 审批卡优先；允许普通补充但说明不能代替审批 |
| 🟡 等待回答 | active + waitingOnUserInput | 用问题卡回答；普通文本不冒充 RPC 问题答案 |
| ⚪ 空闲 | 此共享实例 idle | 普通文本开始下一轮 |
| ⚫ 历史未加载 | notLoaded | 用户明确恢复后才允许发送 |
| 🔴 最近一轮失败 | last turn failed；runtime 单独显示 | 可继续，但不自动重试旧任务 |
| 🔴 服务错误 | systemError | 限制控制，诊断后恢复 |
| ❔ 状态未知/已断线 | stale、离线或 ownership 未验证 | 保存本地草稿或拒绝发送，不自动补发 |

最近一轮 interrupted 可显示“由其他客户端/服务中断，原因未知”，不能归咎 PocketPilot 或自动重新运行。状态卡可以明确显示该结果，但产品没有停止入口。

topic 标题：工作区短名 + 清理空白后的原名称/本地 alias + 会话短标识，目标总长约 48 个 Unicode grapheme；用 unicode-segmentation 截断，再按 Telegram 官方 128 字符上限保守校验。短 ID 冲突扩展长度。标题不使用额外模型生成，避免成本、延迟和把代码标题外发。

标题只在稳定状态转换后低频更新（例如至少 10 秒间隔并合并），状态卡更及时。编辑 topic 图标的 API 不支持任意改 icon_color，故状态用名称 emoji，不设计无法实现的动态颜色效果。Telegram 标题编辑失败不影响 provider 运行，标记展示延迟即可。

## 8. 发送消息与 steer

绑定且状态新鲜的 topic 中，普通文本直接送往该 thread。发送后用轻量回执/状态卡显示：

```text
已追加到正在运行的这一轮 · 77cd21
```

或：

```text
已发送，开始下一轮 · 9df3ba
```

回执区分已保存、已接受和运行结果。不能 Telegram 收到消息就显示“Codex 已收到”。减少每条额外发送回执，优先更新状态卡；有错误/冲突才新发明确提示。

运行中明确指定可用 `/steer 文本`，绑定发送时捕获的 turn ID。其他推荐命令：

| 命令 | 行为 |
| --- | --- |
| `/status` | 拉取真实状态并更新卡片 |
| `/history` | 查看可见文本历史并分页，不执行 resume |
| `/info` | 完整标题、workspace、host、thread、状态年龄与接入模式 |
| `/ask 文本` | 明确将文本交给 agent；支持发送以斜杠开头的内容 |
| `/steer 文本` | 仅追加到捕获的当前 turn |
| `/rename 标题` | 只改 PocketPilot/Telegram 名称 |
| `/unbind` | 确认后解除平台映射，不结束 agent 工作 |
| `/workspaces` | 回到控制菜单 |

没有 `/stop`、`/cancel`、`/kill`、`/archive`。未知 slash 命令显示帮助，不自动执行 Codex slash command 或 shell command。用户回复某条旧消息仍送当前 thread，不恢复旧 turn、不 fork。

若 steer 前置条件冲突：

```text
目标轮次已变化，这条补充尚未发送。
原目标: 77cd21    当前: 空闲

[ 作为下一轮发送 ] [ 保留不发送 ]
```

若当前另有新 active turn，则改成「追加到当前轮」且绑定新的预期 ID。按钮和普通文本在执行前都必须验证 binding revision、连接 epoch、授权与最新状态；同一 topic 禁止改绑不同 thread。

若请求结果未知：

```text
连接中断，暂时无法确认 Codex 是否已收到。
正在核对会话记录；不会自动重复发送。
[ 查看核对状态 ]
```

长期无法核对时，提供显式“重新发送（可能重复）”且新建 operation 关联旧 operation，默认不重发。断线保存的草稿不会在网络恢复时自行变成远程执行指令。

启动前、离线期间或等待过久的 Telegram 普通文本先显示“这是一条较早的消息，尚未发送”，提供「确认发给此会话」和「保留不发送」。重新确认时显示当前状态/轮次。不能仅因为恢复时读到这条 update，就把它当作用户针对当前轮次的新输入。

## 9. 文本转发与可读性

默认转发 item 完成后的完整可见消息，commentary 也发送。每个 item 保留边界，必要时加“说明/回答/用户”标签；不等整轮结束才发所有中间说明。工具调用、终端输出、diff、reasoning 不发送。

若 turn 异常结束或服务断开，已经观察到的未完成文本会在历史核对后显示，标“未完成，截至某时”；以后可更新为最终文本。重连区间没有完整事件日志可证明时，状态卡显示“该区间可能遗漏消息”，即使未检测到某一条具体缺失也保留提示。

Telegram 原输入与 provider 返回的 userMessage 通过 clientId/operation 关联，只做送达确认；其他客户端的用户输入标“Codex 用户（其他客户端）”。不靠文本字符串相同推断来自 Telegram，不把 bot message 送回 provider。

默认纯文本，避免 MarkdownV2 转义错误导致消息丢失；后续可增加受限 HTML renderer。长消息按段落/行拆分，保守控制每片不超过约 3500 UTF-16 code units，为 `(1/3)` 标签留空间；不能截断丢正文。超长代码按纯文本逐片发送；可选附件导出以后添加，不能只发摘要替代原文。

Telegram 官方建议限制包括每 chat 约每秒 1 条、群每分钟 20 条、总体约每秒 30 条。**同一 supergroup 的不同 topics 共享群级预算**。采用 per-bot + per-chat 限速，初值保守为群每分钟 18 条，真实 429 按 retry_after；编辑与发送、标题/状态更新也进入受控调度，不假定编辑无限额。

多个 session 密集 commentary 时，完整消息同步可能延迟。状态卡展示积压数量/年龄；控制台分页与标题更新可合并，消息正文不可丢弃或无限饿死。审批提醒和明确错误优先，同时给普通文本公平预算。

## 10. 审批和用户问题

审批卡仅呈现作出决定所必需的信息：

```text
Codex 正在等待本次命令批准
命令: cargo test --workspace
目录: 已登记的工作区
原因: 验证修改

[ 仅允许这一次 ] [ 稍后在桌面处理 ]
```

这是必要的交互，不是工具调用流式输出。允许前再次验证用户、请求 ID/epoch、session/turn、请求未被其他端处理；不暴露 cancel/decline/永久授权。服务返回 availableDecisions 时只展示确实支持的允许项。

问题卡按协议提供选项和自由输入。用户输入与具体 request 绑定；回复问题卡的文本走 answer，不同时又被当作普通 steer。普通文本默认 Continue 语义，无法代替审批请求。MCP URL elicitation、额外权限等未支持类型提示去桌面处理；不自动打开 URL 或确认权限。

桌面先完成审批时，收到 serverRequest/resolved 后禁用按钮并显示“已在其他端处理”。重启后旧卡片全部失效，等新连接重放请求再生成新卡。若原端无法处理，明确显示受限/等待，不设超时自动拒绝。

## 11. 解绑、topic 被关闭与资源异常

解绑确认说明：“仅解除 Telegram 绑定，Codex 会话继续运行”。若存在 active turn/待处理请求，本地停止接收新消息但保留连接观察直到允许安全释放；不在此时主动 unsubscribe。现有其他客户端正常工作；进程本身退出仍必须满足版本断线验收。

用户手动关闭 topic：不发 agent 操作。绑定标 suspended，积压保留且提示控制台；用户可重新打开恢复。不要自动 reopen 对抗用户操作。手动删除 topic：标 orphaned，停止投递，可在控制台显式创建新 topic 并重新绑定；不自动复制大量历史。

Bot 被踢/权限移除：平台 degraded，Codex 不受操作；用户可在本机 status 看到异常。Telegram 群迁移到新 supergroup 时处理 migrate_to_chat_id，事务更新地址并核验授权，不能静默把任意新群加入允许列表。

## 12. Discord 后续映射

用户自建服务器、给予 bot 管理权限后：一个 workspace 对应一个 category，每个 session 对应一个 text channel；控制入口用 slash commands 和专用控制频道。名称为适配平台的短标题 + 短 ID，完整信息用置顶状态卡。应用授权仍限制允许 guild/user/workspace。

官方 category 最多容纳 50 个 channel，超出可拆分为 `PP-1`、`PP-2`；创建前检查当前容量/权限，不能无限尝试。更大的服务器频道总量限制应在 Discord 阶段重新查证，不在本文凭经验固定数值。

读取普通消息可能需要 Message Content privileged intent；管理员权限不自动开启该 intent。Discord 阶段选用 serenity，使用其 Gateway/HTTP rate limit 能力，再接同样的 provider 状态与操作模型。Discord 的按钮、interaction 时限和频道名规则在该阶段补充实测。
