# 开发指南

开发前先读 [CONTRIBUTING.md](../CONTRIBUTING.md)。架构文档解释[运行时边界与状态](architecture.md)，这里提供从代码入口到扩展和验证的路线。

## 开发循环

```sh
git switch public
git switch -c codex/describe-the-change
# 修改源码与相关文档
make check
make package
cargo clean
```

先安装 CI 固定的 Codex CLI 0.162.1，供 `adapters/build.rs` 导出 schema。所有 Cargo build/check/test/clippy 必须使用 `--release`；不要另建 debug artifact 树。`make package` 只打包当前系统/架构，独立 Linux runtime server 使用 `make package-agent-server`。

失败后修复并完成同一轮验证；清理与编译必须串行。需要保留的 binary 放到忽略的 `dist/`，完成验证批次后清理 target。测试默认使用临时目录和本地协议 fixture，不读取开发者真实凭据。

## 从哪里开始读

| 改动目标 | 入口 |
| --- | --- |
| CLI、doctor、进程启动与退出 | `crates/app/src/main.rs`、`cli.rs`、`doctor.rs`、`runtime.rs` |
| TOML 组合、热更新与主机权限 | `app/src/config.rs`、`engine/src/config.rs`、`reload.rs` |
| 项目发现、provider 连接与控制资格 | `engine/src/catalog.rs`、`runtimes.rs`、`control.rs` |
| 明确授权的任务与文件输入 | `engine/src/operations.rs`、`uploads.rs`、`session/src/operation.rs` |
| 平台导航与绑定管理 | `integrations/src/navigation.rs`、`bindings.rs`、各平台目录 |
| 原生协议映射与回执 | `adapters/src/codex/`、`adapters/src/claude.rs` |
| SSH 配置、认证与命令边界 | `connections/src/ssh/` |
| Claude 进程所有权与持久化 | `agent-server/src/server.rs`、`claude.rs`、`storage.rs` |
| Markdown、安全链接与分段 | `session/src/markdown.rs`、`syntax.rs`、`segmentation.rs`、`formatting.rs`、`html.rs` |
| 中英文文案 | `i18n/src/lib.rs` 和 `i18n/locales/` |

crate 根部给出公共模块和契约。`session` 不依赖平台 SDK；`engine` 不依赖聊天端；`connections` 只处理主机连接，不理解 Codex 协议。保持这些方向，避免为了减少几行代码把平台 SQL 或原生 DTO 搬到 app。

## 新增消息平台

1. 在 `integrations/src/<platform>/` 实现 `Integration`，通过 `Context` 获取配置、Store 和 RuntimeRegistry。只在显式运行服务时收发消息。
2. 先定义安装身份、owner、允许容器和回调 scope。入口、消费队列、发送前均要校验；不要用“第一个发消息的人”隐式授予控制权。
3. 复用 `navigation::Navigator`、`bindings::Manager` 和公共问题投影，平台只实现原生控件、容器及 scoped SQL。
4. 让 Renderer 保持无 I/O，输出有界且可持久化的消息段。按最终编码 payload 计预算，冻结的投递不能在重启后重新分段。
5. 将授权后的附件下载放入平台 `AttachmentSource`，限制来源、重定向、大小、数量与超时。
6. 在 app 组合配置并装配 integration，更新公开示例、平台安装文档和测试；不要扩大其他平台的权限。

Telegram 的 `Owner` 是部署级具名身份，由配置构造并存入该平台的 Store，入站解析、写操作和后台输出共用它；不使用全局可变身份。飞书及未来 Lark 固定直连，不复用可配置代理入口。

## 新增或修改 runtime

通过 `AdapterFactory` / `Adapter` 与 `session::AgentProvider` 协商能力。原生请求、错误、会话状态、输入回执和结构化问题都应映射到具名模型。读取不启动或 resume 会话；任务生命周期不能从客户端连接生命周期推断。

Codex DTO 在构建时从 CLI 生成到 OUT_DIR。不要提交生成协议、用任意 `serde_json::Value` 替代模型，或忽略未知写结果。第三方 SDK 强制 Value 的边界需先转换为具名 DTO，并限制使用范围。

独立 runtime 采用 `session::server` 的版本化 JSONL 契约。新增能力时同步桥接和 server 的协议版本，不自动重启用户运行中的旧 server。新 runtime 需要证明断开客户端后任务继续、写前账本持久化、未知结果不重发，再声明可控制。

## 测试与诊断

测试围绕行为和边界：未经授权的消息、重复事件、过期控件、cwd 变化、断线、回执未知、队列上限、渲染预算和状态恢复。真实连接测试需显式启用，并只处理自己创建的连接和测试会话。

Telegram 的 `service_tests.rs` 保存共用本地 fixture；具体行为按 `service_navigation_tests.rs`、`service_history_tests.rs`、`service_recovery_tests.rs`、`service_input_tests.rs`、`service_topics_tests.rs` 和 `cross_platform_tests.rs` 分组，其他小测试继续相邻放置。跨平台测试使用同一原生 session，检查共享连接和账本语义。

OpenLark 分片测试直接引用 vendor 的确切重组源码，纳入 `make check`；不要复制一份测试专用算法。SDK 的其余内置测试不因 workspace 验证而自动执行，升级时另行评估，见[依赖维护](dependencies.md)。

tracing 记录 operation ID、平台/主机/会话、耗时和结果，排查时保留这些元数据。消息正文、文件内容、token、完整帧、OTP 和认证提示不能加入日志。

## 文案和文档

新增文案使用稳定 ASCII ID，同时更新中英文资源，rust-i18n 必须显式传 locale。类型化错误实现 `LocalizedError`，只在平台边界翻译；用户和 agent 正文保持原样。

行为改变时同步更新用户步骤、配置示例、架构边界和 CHANGELOG 的 Unreleased。公开文档使用合成账号和保留地址，记录可重复流程；不要提交个人联调记录或部署备份。
