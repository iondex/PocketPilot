# 参与开发

请从 `public` 分支创建聚焦的工作分支，说明改动解决的问题、最终行为和验证结果。首次配置开发环境见[开发指南](docs/development.md)，模块和授权边界见[架构说明](docs/architecture.md)，依赖修改见[补丁维护](docs/dependencies.md)。

提交前运行 `make check`；需要交付可执行文件时运行 `make package`，保留 dist 后执行 `cargo clean`。CI 在 Linux/macOS 执行相同检查。文档、配置或协议发生变化时同步公开示例和 CHANGELOG。公开 issue/PR 只使用合成数据，不提交私人配置、会话、日志或备份；敏感问题见 [SECURITY.md](SECURITY.md)。

项目采用 [Apache-2.0](LICENSE)。贡献需保证有权提交相关内容，并保留第三方来源及许可证，不要求额外签署 CLA。

# Rust 开发约定

遵循 Rust Style Guide 和 Rust API Guidelines，使用仓库工具链与 Rust 2024 edition。rustfmt 是格式的最终依据；提交前运行：

```sh
cargo fmt --all -- --check
cargo clippy --release --workspace --all-targets --locked -- -D warnings
cargo test --release --workspace --locked
```

构建、检查、测试与 Clippy 统一使用 release profile；可运行 `make check` 完成上述验证及本地文档链接检查（需要 Python 3.10+，无第三方 Python 依赖）。需要保留的可执行文件先通过 `make package` 放入 Git 忽略的 `dist/`，每轮验证/打包完成后运行 `cargo clean` 清理 `target/`；`make release` 顺序执行检查、当前系统/架构打包和清理。构建期间不并行清理。

- crate 使用简短职责名，binary 名为 pocketpilot。session、engine、app 与 i18n 的源码保持 src/ 下平铺；session 的 Markdown 实现按 syntax、segmentation、formatting、html 命名，通过 markdown 模块组合，不建立子目录。integrations 按 src/telegram、src/discord、src/slack、src/network 分职责组织；connections/src/ssh 与 adapters/src/codex 分别容纳原生连接和运行时实现，公共接口位于各 crate 根部。测试拆入相邻的 *_tests.rs 或 tests.rs，通过 cfg(test) 模块加载。
- 每个类型、函数、方法、impl、trait、常量和模块定义之间留一个空行，文档注释和属性紧贴所属定义。rustfmt 不会主动补齐全部空行，编辑时须保留。
- 类型和枚举变体使用 UpperCamelCase，但按项目约定，SSH、TOTP 保留全大写，例如 SSHOptions、SSHProxy、TOTP。字段、函数、crate 与配置键仍使用 snake_case。外部依赖的公开类型不改名。仅对实际触发的具体定义允许 Clippy acronym 例外，不全局禁用。
- 用具名类型表达权限、身份和状态。业务逻辑使用生成的 Codex DTO；禁止直接操作 serde_json::Value。协议生成物只在 Cargo OUT_DIR。
- SSH 只负责有效 OpenSSH 配置、连接命令与认证；Codex 才负责发现自己的安装、socket 和协议。平台代码及平台 SQL 全部放在 integrations 内对应平台目录。app 只负责装配、CLI、日志和进程级任务。
- 配置文件控制允许连接的主机、凭据和运行参数；provider/workspace 是应用状态，由 catalog 持久化。状态变化发布为完整快照，重新验证授权范围。
- 底层错误使用 thiserror，在平台边界翻译为简短用户提示。tracing 记录平台/主机/会话/operation ID、耗时和结果；不记录消息正文、OTP seed、验证码、token、认证提示或完整传输帧。
- 默认后台任务只读。只有新鲜、明确授权并持久化的用户操作可创建、resume、续轮或 steer；显式启动独立 `retry` 工具授权当前进程按 [重试规则](docs/architecture.md#独立-retry-工具) 对最新失败轮次等待 60 秒后发送“继续”，每次发送前持久化授权并复查状态。不存在 stop/interrupt/审批代答。超时、断线或丢失回执一律不重发写操作。
- 不在全局元数据锁中等待网络；每实例建连锁和每会话操作锁可以跨 await，以保证实例及会话串行。队列、并发、响应大小与重连均须有界。
- 测试验证行为与边界，不复刻实现。真实测试只对自己创建的连接/测试会话断连；不能停止用户 daemon/session。真实秘密仅放 Git 忽略的根 pocketpilot.toml。
- 稳定 0.1 前直接维护根 schema.sql，不维护 migrations 或运行迁移器。初始化幂等且不清空现有记录；修改已有列时，开发者需明确处理自己的原型状态，不能自动删除用户数据。

所有 crate 共用 i18n crate，以稳定 ASCII ID 索引文案；资源只放 crates/i18n/locales/。rust-i18n 调用必须显式传 locale，不调用全局 set_locale。错误保留类型并实现 LocalizedError，在界面边界翻译；用户内容与文案 ID 用具名类型区分，禁止对任意原文做翻译查找。Agent 正文交给 pulldown-cmark 渲染，不能直接当 MarkdownV2 或可信 HTML 发送。空标题补齐仅限显式用户动作且须先验证实际 cwd，后台不得改原生标题。

公开文档以当前行为、接口边界和可复用流程为主，不累积单次故障与联调流水。示例、测试和公开诊断材料使用合成名称与保留地址，不复制真实 SSH alias、账号、IP、工作区或会话内容。TOTP 的 prompt_patterns 匹配挑战类型和通用上下文格式，不嵌入实际用户名或地址；认证提示不得进入日志。
