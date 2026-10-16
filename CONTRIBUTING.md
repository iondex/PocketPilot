# Rust 开发约定

遵循 Rust Style Guide 和 Rust API Guidelines，使用仓库工具链与 Rust 2024 edition。rustfmt 是格式的最终依据；提交前运行：

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo clippy -p app --no-default-features --locked -- -D warnings
```

- crate 使用简短职责名，binary 名为 pocketpilot。源码物理目录平铺为 crates/*/src/*.rs；按职责增加文件，避免多层文件目录。
- 每个类型、函数、方法、impl、trait、常量和模块定义之间留一个空行，文档注释和属性紧贴所属定义。rustfmt 不会主动补齐全部空行，编辑时须保留。
- 类型和枚举变体使用 UpperCamelCase，但按项目约定，SSH、TOTP 保留全大写，例如 SSHOptions、SSHProxy、TOTP。字段、函数、crate 与配置键仍使用 snake_case。外部依赖的公开类型不改名。仅对实际触发的具体定义允许 Clippy acronym 例外，不全局禁用。
- 用具名类型表达权限、身份和状态。业务逻辑使用生成的 Codex DTO；禁止直接操作 serde_json::Value。协议生成物只在 Cargo OUT_DIR。
- SSH 只负责有效 OpenSSH 配置、连接命令与认证；Codex 才负责发现自己的安装、socket 和协议。平台代码及平台 SQL 全部放在对应 integration crate。app 只负责装配、CLI、日志和进程级任务。
- 配置文件控制允许连接的主机、凭据和运行参数；provider/workspace 是应用状态，由 catalog 持久化。状态变化发布为完整快照，重新验证授权范围。
- 底层错误使用 thiserror，在平台边界翻译为简短用户提示。tracing 记录平台/主机/会话/operation ID、耗时和结果；不记录消息正文、OTP seed、验证码、token、认证提示或完整传输帧。
- 后台任务只读。只有新鲜、明确授权并持久化的用户操作可创建、resume、续轮或 steer；不存在 stop/interrupt/审批代答。超时、断线或丢失回执一律不重发写操作。
- 不在全局元数据锁中等待网络；每实例建连锁和每会话操作锁可以跨 await，以保证实例及会话串行。队列、并发、响应大小与重连均须有界。
- 测试验证行为与边界，不复刻实现。真实测试只对自己创建的连接/测试会话断连；不能停止用户 daemon/session。真实秘密仅放 Git 忽略的根 pocketpilot.toml。
- 稳定 0.1 前直接维护根 schema.sql，不维护 migrations 或运行迁移器。初始化幂等且不清空现有记录；修改已有列时，开发者需明确处理自己的原型状态，不能自动删除用户数据。

Telegram 文案使用 rust-i18n 的显式 locale 参数，不调用全局 set_locale；语言资源放在 locales/，编译进 binary。Agent 正文交给 pulldown-cmark 渲染，不能直接当 MarkdownV2 或可信 HTML 发送。空标题补齐仅限显式用户动作且须先验证实际 cwd，后台不得改原生标题。
