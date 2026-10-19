# 迁移到另一台开发机

Git 保存源码、公共示例和安装文档。私有配置、平台绑定与会话操作账本需要单独转移；仅拉取仓库不会恢复现有 Telegram topics、Discord channels 或 Slack threads 的对应关系。

## 1. 在新机器准备源码与运行条件

1. 拉取最新提交，使用仓库指定的 Rust stable 工具链、系统 OpenSSH 和受测 Codex CLI；协议生成版本与 `CODEX_SCHEMA_BIN` 说明见 [协议文档](design/protocol-generation.md)。
2. 在目标操作系统和架构上构建。默认只启用 Telegram；还使用 Discord/Slack 时执行：

   ```sh
   make package FEATURES=discord,slack
   ```

3. 可运行文件在 `dist/pocketpilot`，旁边的 `BUILD.txt` 记录工具链与功能，`pocketpilot.sha256` 保存校验值。其他操作系统/架构应独立构建，不直接搬用旧机器的 binary。
4. 检查新机器的 OpenSSH alias、密钥/agent 和 known_hosts。私钥与信任记录通过自己的受控方式配置；不要关闭 host-key 校验。确认远端项目路径和原生 app-server 控制入口仍然有效。

`make release FEATURES=discord,slack` 可以顺序执行完整 release 验证、打包和 `target/` 清理。构建说明见 [build.md](build.md)。

## 2. 停止旧桥接后转移私有数据

先停止旧机器对应的 PocketPilot job/service，确认它不会被守护进程重新拉起。只停止桥接，保留原生 Codex daemon 和正在执行的任务。旧机器停止后，再启动新机器，避免相同 bot 被两套独立账本同时控制。

| 内容 | 迁移方式 |
| --- | --- |
| 根 `pocketpilot.toml` | 通过受控传输复制，保留 token、ID、允许范围、认证配置；新位置使用文件权限 `0600` |
| `[service].data_dir` 整个目录 | 旧桥接停止后复制整目录，包含 `state.db` 及仍存在的 WAL/SHM；保留目录 `0700`、数据库文件 `0600` |
| SSH 配置与认证环境 | 按新机器路径、账号和密钥方案准备；核对同一 alias 仍指向同一实际账户与执行实例 |
| 服务管理配置 | 在新机器设置可执行文件、根配置绝对路径和 WorkingDirectory，避免沿用旧机器目录 |
| `.env.slack`、日志、`target/` | 程序不读取 `.env.slack`；已整理的 Slack 设置在根 TOML。日志和构建缓存不是恢复绑定所必需的数据 |

SQLite 的一致性复制在桥接停止后进行；存在 WAL 时不能仅复制 `state.db`。暂停复制期间的 CLI 配对/catalog 修改。根 TOML 中的相对路径以配置文件所在目录为基准，自定义绝对路径应逐项核对。

私有数据不要加入 Git，也不要把真实 token、OTP seed、配置或账本附在 issue 中。数据库包含任务正文与投递状态，应和配置采用同等私有传输方式。

## 3. 验证身份和共享实例

在新仓库根目录运行：

```sh
./dist/pocketpilot doctor --credentials
./dist/pocketpilot doctor --connect
./dist/pocketpilot providers list
./dist/pocketpilot workspaces list
```

离线凭据检查不证明 token 或平台权限有效，`doctor --connect` 也不会自动确认控制资格。SSH 远端若仍是同一执行实例，可核对原 provider/thread 身份继续使用；旧机器上的本地 stdio/Unix socket 不会随 PocketPilot 数据自动迁移。执行 endpoint 或账户变更时，分配新 provider 并按共享生命周期流程验证，不修改旧映射冒充同一实例。

## 4. 启动与平台验收

```sh
./dist/pocketpilot --config /absolute/path/PocketPilot/pocketpilot.toml serve
```

先查看已有会话的状态和历史，核对复用原平台容器、没有自动 resume 或补发旧输入，再用专用测试会话验证续轮/steer。恢复的旧文字需新确认；已经 Dispatching/OutcomeUnknown 的操作先检查原生结果，不重复提交。状态未知不等于任务已停止。

Slack 安装/权限见 [Slack 指南](slack-setup.md)，Discord 见 [服务器验收](testing-discord.md)，Telegram 和共享实例见 [测试入口](testing.md)。复制账本不能替代真实客户端与权限验收。

## 5. 收尾

确认服务运行的是 `dist/pocketpilot` 后，执行 `make clean` 释放构建目录。保留私有数据和必要的迁移备份，待新机器验收完成后按自己的备份策略处理旧副本；清理 `target/` 不涉及配置、账本或 Codex 历史。

## Slack 旧频道模式升级

新版本仅接受配置 owner 与 App 的一对一私聊。删除 `[slack].allowed_channel_ids`，按 [Slack 指南](slack-setup.md) 更新 manifest 和重新安装授权。DM ID 由 Slack 返回，不写入配置；一个 DM 支持多个项目。旧频道绑定不会被改成私聊绑定，也不会继续投递到频道；在 DM 中重新选择已有 session 建立新映射。旧表/额外字段保持原样，不自动删除数据库或历史。

## 分段消息账本升级

新数据库直接使用根 schema.sql。已有原型数据库需要停止桥接、备份后，检查下列三张表，补齐缺少的列；不要清空绑定或操作记录，已经存在的列不要重复执行 ALTER。

```sql
ALTER TABLE deliveries ADD COLUMN next_part INTEGER NOT NULL DEFAULT 0;
ALTER TABLE deliveries ADD COLUMN rendered_parts TEXT;
ALTER TABLE slack_outbox ADD COLUMN next_part INTEGER NOT NULL DEFAULT 0;
ALTER TABLE slack_outbox ADD COLUMN rendered_parts TEXT;
ALTER TABLE discord_deliveries ADD COLUMN next_part INTEGER NOT NULL DEFAULT 0;
ALTER TABLE discord_deliveries ADD COLUMN rendered_parts TEXT;
```

移除旧配置中的 `[delivery].overflow`；长文字统一分段，不再提供 single_document 模式。程序不会自动迁移旧列。已确认段号和固定的分段内容需要一起保留，不能将 next_part 重置后重投；Dispatching/OutcomeUnknown 继续遵循不重发规则。旧的历史文件投递不会再上传；旧按钮请求导出时提示不支持。Slack 新安装使用 `files:read` 读取入站附件，已有安装添加该 scope 后需要 Reinstall 授权。
