# 故障排查

先确定问题发生在哪一层：配置 → 平台身份 → 执行主机 → runtime → 控制资格 → 绑定 → 投递。默认使用只读诊断，核对 operation ID 和原生会话状态；不通过重复发送来试探未知写结果。

## 常见现象

| 现象 | 检查与处理 |
| --- | --- |
| 构建无法生成 Codex schema | 运行 `codex --version`，确认 CI 固定版本 0.162.1；检查 PATH 或显式 `CODEX_SCHEMA_BIN`。不能改用旧 schema 快照绕过失败 |
| TOML 解析失败 | 从公开示例逐项比较表名和字段；不支持 include、`.env`、旧 provider/workspace 数组。错误不回显原文，以免泄露凭据 |
| 升级后 Telegram 配置失败 | 添加正整数 `[telegram].owner_user_id`；不再依赖源码固定账号。修改后重启 |
| 飞书或 Lark 出现未知 proxy 字段 | 删除 `[feishu].proxy` 或 `[lark].proxy`，包括空字符串。两端均固定直连 |
| 服务提示状态目录已加锁 | 确认同一 data_dir 的其他 `serve` 已正常退出；不要删除锁文件或结束原生 agent。`providers verify` 也需独占服务状态 |
| 主机可 SSH，但看不到 runtime | 使用 `doctor --connect`，核对目标账号的已运行共享入口。历史目录存在不等于具有共享 app-server |
| 菜单可浏览但输入被拒绝 | 查看 `providers list` 的控制资格，核对允许 cwd。Codex 按专用项目验证；Claude 按独立验收登记 |
| Telegram 消息被静默忽略 | 核对 owner 数字 ID、本人的 bot 私聊、Topics 配置；其他账号和群聊本来就拒绝 |
| Slack 主输入框不提交任务 | 主 DM 只打开导航；在对应 session 根卡片的 thread 内回复 |
| Discord 普通文字没有作用 | 在绑定 channel 使用 `/pp-ask text:任务`，普通消息不是 agent 输入；操作者必须是服务器 owner |
| 飞书菜单可用，群任务没有响应 | 核对发布权限、tenant/owner、授权群和绑定话题；文字/富文本需 @机器人，独立文件需 owner 在绑定话题中提交 |
| 文件上传失败 | 公共限制为一次最多四个、单个 20 MiB；Discord 只提供一个 file 选项。Claude 图片另限 5 MiB 和支持的 MIME |
| 绑定显示旧状态 | 观察列表显示缓存年龄，手动查询确认；暂停仅阻止后续自动观察，不停止原生任务 |
| 升级后数据库错误 | 0.1 原型不迁移已有列。停止桥接，保留完整备份，按 schema 差异明确处理原型状态；不要删库“恢复” |

## 诊断命令

```sh
./dist/pocketpilot doctor
./dist/pocketpilot doctor --credentials
./dist/pocketpilot doctor --connect
./dist/pocketpilot hosts list
./dist/pocketpilot providers list
./dist/pocketpilot workspaces list
```

无选项 doctor 不发网络探测，但正常 CLI 会打开或初始化本地 catalog；它不是不触碰磁盘的纯文本检查。`--credentials` 离线检查格式；`--connect` 探测执行端，平台探测单独使用 `--telegram`、`--discord`、`--feishu`。这些命令不创建模型任务，也不授予控制资格。

日志中的关联 ID、错误类别、时间和结果可用于追踪。底层传输和 SQL 日志受独立过滤，不能通过提高日志级别取得 token、消息正文或原始帧。

## 回执未知与重启

unknown 表示写入可能已发生，但无法确认结果。先在原生 runtime 确认是否已有会话、输入或执行轮次；平台容器创建也要检查是否已成功。不要连续点击创建、重复提交同一输入，或重新运行验证来覆盖不确定性。

正常重启仅重连客户端，不重放旧写操作。过期菜单需要刷新；旧输入可能进入待确认草稿，需要当前绑定上的新鲜授权。工具权限审批和秘密问题回到原生界面处理。

## 提交问题

请提供系统/架构、PocketPilot commit、Codex/Claude 版本、平台类型、复现步骤、预期与实际结果、脱敏后的错误类别和关联 ID。把配置改为合成 host、项目和账号，删除 token、OTP、消息正文、真实 endpoint 与附件内容。

普通问题使用 GitHub issue；安全和私人数据问题按 [SECURITY.md](../SECURITY.md) 私下报告。诊断不需要上传真实 `pocketpilot.toml`、`.env` 或数据库。
