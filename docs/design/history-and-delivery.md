# 历史入口、单条集合与消息合并

## 1. 两种进入会话的展示

| 场景 | 默认展示 | 读取规则 |
| --- | --- | --- |
| 新绑定空闲/历史会话 | 最后一次用户输入 + 对应 agent 最终输出，一个集合 | 不显示中间 commentary；不启动任务 |
| 进入正在运行的会话 | 全部可读取的历史可见文本，一个集合；标当前仍运行 | 包含用户与 agent 可见文本，过滤工具/推理；不停止/重建任务 |
| PocketPilot 重启/网络恢复 | 核对已知投递，不重新自动发一份全历史 | 必须由用户主动进入/请求历史才生成新集合 |

这里“新添加 session”指绑定既有 session 时取已有记录；真正新建的 thread 尚无 agent 输出，显示“尚未产生最终输出”，不能伪造历史。

空闲入口的“最后一次”不能只取 thread 最后两条 item：从最新 turn 开始向前分页，过滤工具/中间文本，识别 userMessage 与 agentMessage.phase=final_answer。phase 缺失时使用**受测版本明确的最终输出标识或 turn 结构**；若无法判定，标“最终输出无法确认”，不能把最后一条 commentary 当 final。

用户输入可能有同轮多条 steer，输出也可能有多个 final item。保存相同 turn 中最后一次用户输入以及该轮最终输出集合，保留 item ID 和顺序，不以文本相似度配对。若最新用户输入所在轮仍无 final，说明“这次输入尚无最终输出”；可另列最近已完成轮的输入/输出并明确标旧轮，不能把旧 final 冒充这次回答。

示例：

```text
最近一次交互 · 调查测试失败
用户：请先确认异步测试是否存在竞态。
最终回答：确认是测试未等待后台任务结束……
本轮已完成 · 截至 20:30
```

## 2. “一条消息”与平台硬限制

Telegram sendMessage 上限是 4096 字符，不能保证任意长度的全部历史都塞进一条纯文本。默认按约 3500 UTF-16 code units 保守测量，长标题/标签预留空间。

- 集合能放下：发送一条纯文本集合。
- 放不下：把完整集合生成 UTF-8 Markdown 文档，通过一次 sendDocument 发送，caption 只放会话名、范围、状态和条数；仍是一条 Telegram 消息，不再逐条灌历史。
- 文件大于当前官方单文件上限（查阅时 sendDocument 为 50 MB）：可尝试 gzip，仍超限则发送**一条说明**，提供用户主动请求分片的入口；不自动刷屏、不截断、不谎称已发出全部历史。

附件与短文本保存相同 HistoryBundle ID、内容 hash、included item IDs、读取区间、完整性标志和 delivery receipt。上传失败/超时结果未知沿用 DeliveryUnknown；不能再用一条“成功”消息掩盖未知结果。

所有附件按当前配对状态和私聊目标重新授权；本地临时文件权限受控，保留未成功投递的必要产物，成功后按清理策略处理。历史查询只调用 provider 的只读 API，不向 agent 发送“总结历史”的提示词。

## 3. 活跃会话历史集合的边界

定义 `HistoryBundle { session_ref, bundle_id, observed_at, items, active_turn, coverage, complete }`。分页遍历全部可读取历史，按 turn/item 顺序组合用户与 agent 可见文本，排除工具调用、工具输出和 reasoning。当前 active item 只附已知文本，并标“仍在生成”；从未得到的未来输出不属于这份集合。

“全部”指服务端实际提供并可恢复的历史范围。被原端删除、不可见、未持久化或压缩掉的内容不能恢复，要说明缺口；不能从磁盘私有格式偷偷补齐来冒充协议保证。

API 不保证原子全历史快照。记录开始/结束时间、历史锚点，重叠读取末页并按 ID 去重；检测其他客户端 revert/历史变化时重试有界次数，无法稳定则标“读取期间有变化”。不能仅凭 updatedAt 或一次分页成功就声称精确一致快照。正常持续生成时允许截至观察时间的集合，不等待 turn 完成再进入。

历史集合发送与实时输出有一道投递屏障：读取期间继续持久化实时文本；集合记录覆盖了哪些 item/文本 revision，成功后只释放未覆盖的新内容或明确的后续补充。不能先发集合又把同一批历史逐条重复推送。屏障只约束平台投递，不阻塞 agent 读循环或 steer。

屏障必须有终止出口，不能因附件失败永久堵住会话。确定失败可按有界退避重试，超时未知则标 HistoryDeliveryUnknown，超大文件标 HistoryTooLarge；在已有状态卡显示历史未确认/未投递。超过可配置的屏障期限后，释放快照边界之后的新实时文本，快照已覆盖的 item/revision 继续保留未决状态，不自动逐条补发或当成已送达。用户主动核对/重新请求集合才处理这批历史；已经落盘的新文本不会因释放屏障丢失。失败提示本身也受平台投递状态和限速约束。

进入命令和创建 topic/status card 是不同 UI 资源；“一条集合”是历史内容只占一条消息，状态卡可独立复用，不能把全部历史拆成多张状态卡绕过要求。

## 4. 短时消息合并

MVP 就加入 `DeliveryAggregator`，位于权威 text_items/outbox 之间。它只合并**展示**，不合并输入命令，不改变 provider turn/steer 顺序。

可配置初始规则：

```text
窗口 2 秒，阈值 3 条，最长等待 5 秒
同一 session + 平台地址 + config_generation 的连续文本可进入同一 batch
不足阈值也在窗口结束发送；持续高频不得无限等新消息
```

进入 burst 状态后，把按顺序到达的已完成 user/agent 文本合并成一个 MessageBatch，使用简短 role/阶段分隔保留每条边界。窗口内不删内容、不只保留最后一条、不把多条 commentary 摘要替代原文。单条已经很长或合并后超长，优先一条附件；若用户配置选择实时分片，则统一限速并展示积压，不能假装仍满足单条历史集合模式。

不能跨 session/topic/workspace/平台目标合并，不能把任意不同用户的私有来源混到另一个授权目标。turn 完成可触发 flush，但仍受速率限制；异步合并期间新配置生效可封存旧 batch，并重新授权后投递，不将撤销目标的消息送出。

错误/状态通知相同内容可 debounce/coalesce；任务完成只发一次；状态卡、topic 标题低频更新，不为每条输入再额外发独立 ACK。需要用户决定的冲突/未知结果可优先提示，但平台限速不是可绕过约束。

## 5. 数据与幂等

保留原始逻辑 text item 和独立批次，不用批次覆盖 item：

| 表/记录 | 责任 |
| --- | --- |
| text_items | provider 的文本、phase、final、clientId 和 revision |
| message_batches | batch_id、binding、config_generation、状态、开窗/截止时间 |
| batch_items | batch_id + item/revision/order；记录原始边界 |
| history_bundles | 查询范围、included IDs/revisions、完整性、文件 hash |
| deliveries | batch/bundle/单 item 到平台 message ID 的映射 |

批次封存、成员和 outbox 同一事务提交；重启恢复已封存批次，不能重新分组后又投递一遍。未封存批次按已有成员恢复，不触发任何 agent 写操作。平台发送未知时，整个批次进入 DeliveryUnknown，不能只把部分 item 标已发送。

后续 completed 权威文本与已发 partial 不同：优先修改该批次既有消息；附件不能安全原地替换时发一次明确的更正集合，保存关联。不要把所有旧批次重新生成一遍。

## 6. 限速和公平性

合并与限速同时存在。Telegram 同一私聊的多个 topics 共享 chat 预算，默认保守设为每分钟 18 条，再叠加 per-bot、per-chat 的实际 API 限制和 retry_after。发送、编辑和标题操作统一调度；附件也占预算，不能用附件绕过限速。

按绑定公平调度，优先级只提升重要交互，不能无限饿死慢 session 的文本。窗口、阈值、max delay、每平台预算全部来自 config，可热更新。平台积压只影响投递，不减少状态核对、阻塞 SSH 读循环或停止任务。

Lark 的 batch renderer 可用富文本或卡片，Discord 可用文本或附件；core 保存相同 MessageBatch，长度、编辑能力与限额由 adapter 决定，不把 Telegram 字符数写进领域模型。
