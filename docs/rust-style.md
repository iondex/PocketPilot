# Rust 风格、结构与注释指南

本指南将 Rust 官方、Google 与 Microsoft 的建议落到 PocketPilot 的模块、协议和并发边界。仓库的具体约束以 [CONTRIBUTING.md](../CONTRIBUTING.md) 为准，职责分工见[架构说明](architecture.md)。来源核对日期：2026-10-22。以下规则用于评审和后续改动，不表示所有历史代码已经完成改造。

## 格式与命名

- **用工具统一机械格式。** Rust Style Guide 以可读性、可扫描性和一致性为目标；使用仓库工具链的 `cargo fmt --all -- --check`。不要用手工对齐、超长调用链或额外空行表达业务分组；独立步骤用局部变量与职责明确的函数表达。[Rust Style Guide](https://doc.rust-lang.org/style-guide/)
- **命名表达领域，而非存储细节。** 遵循 Rust API Guidelines 的 `UpperCamelCase` 类型与 `snake_case` 函数、字段和模块约定。`user_id`、`operation_id`、`session_id` 等边界值应保留领域含义；只有会影响正确性或防止误用时才引入新类型。[C-CASE](https://rust-lang.github.io/api-guidelines/naming.html#casing-conforms-to-rfc-430-c-case)、[C-NEWTYPE](https://rust-lang.github.io/api-guidelines/type-safety.html#newtypes-provide-static-distinctions-c-newtype)
- **明确仓库例外。** Rust 通常将缩写视为一个单词，如 `Uuid`；本仓库按既有约定保留 `SSH`、`TOTP` 全大写。只在确实触发检查的具体定义上允许相应 Clippy lint，不能据此全局关闭命名检查。源码目录布局、定义之间留一空行和 release 构建也属于仓库约定，并非上游 Rust 的通用强制规则。

## 结构与接口

- **按可独立说明的职责拆分。** 模块名应帮助读者定位行为，如授权、消息投影、附件处理或传输。优先把复杂函数中的完整步骤抽成具名函数；当一组类型、状态和辅助函数拥有同一职责时再形成模块。遵守仓库现有平铺与平台目录规则，避免把混合职责搬进另一个同样庞大的 `utils` 文件。Microsoft 建议平衡模块的大小与职责；具体拆分位置仍取决于本仓库的领域边界。[M-BALANCED-MODULES](https://microsoft.github.io/rust-guidelines/guidelines/libs/ux/index.html#M-BALANCED-MODULES)
- **保持边界小而明确。** 默认私有；跨模块共享优先 `pub(super)` 或 `pub(crate)`。Google 的 Comprehensive Rust 课程以默认私有的模块边界介绍可见性；它是教学资料，并非独立的强制风格标准。[Google Visibility](https://google.github.io/comprehensive-rust/modules/visibility.html)入口保留编排，协议适配、平台 SQL、连接认证留在各自所属 crate/平台目录。单纯缩短文件不是抽象的理由；不为一个调用点添加只转发参数的 trait、泛型层或包装类型。[C-STRUCT-PRIVATE](https://rust-lang.github.io/api-guidelines/future-proofing.html#structs-have-private-fields-c-struct-private)、[M-SIMPLE-ABSTRACTIONS](https://microsoft.github.io/rust-guidelines/guidelines/libs/ux/index.html#M-SIMPLE-ABSTRACTIONS)
- **让类型承担真实约束。** 对可枚举状态使用 `enum`；对权限、身份与协议映射保留已有具名类型。解析外部数据时验证约束，后续业务代码使用已验证的数据。避免用相邻的多个 `bool` 组合出难以解释或不合法的状态。[C-CUSTOM-TYPE](https://rust-lang.github.io/api-guidelines/type-safety.html#arguments-convey-meaning-through-types-not-bool-or-option-c-custom-type)、[C-VALIDATE](https://rust-lang.github.io/api-guidelines/dependability.html#functions-validate-their-arguments-c-validate)
- **错误属于接口。** 可恢复的 I/O、配置和协议错误返回 `Result`；保留错误类型与来源，在界面边界翻译。`expect` 仅用于确实成立的不变量，并说明成立依据；不要以 `unwrap` 隐藏外部输入失败。避免为降低缩进而吞掉错误，也不要在每一层重复记录同一错误。[C-GOOD-ERR](https://rust-lang.github.io/api-guidelines/interoperability.html#error-types-are-meaningful-and-well-behaved-c-good-err)、[M-PANIC-ON-BUG](https://microsoft.github.io/rust-guidelines/guidelines/correctness/index.html#M-PANIC-ON-BUG)

## 注释与 rustdoc

本仓库的注释优先回答“为什么必须这样做”“这里保证什么”“修改时不能破坏什么”。代码本身应表达普通控制流。Microsoft 对魔法值明确要求解释选择原因、修改影响和关联外部系统；Rust Style Guide 建议优先使用独立行的 `//` 注释，以 `///` 描述条目、以 `//!` 描述模块或 crate。[M-DOCUMENTED-MAGIC](https://microsoft.github.io/rust-guidelines/guidelines/universal/index.html#M-DOCUMENTED-MAGIC)、[Comments](https://doc.rust-lang.org/style-guide/#comments)

| 位置 | 应说明的内容 | 避免的内容 |
| --- | --- | --- |
| `//!` 模块文档 | 模块职责、拥有的状态、依赖方向和重要边界 | 重复目录名；声称尚未实现的功能 |
| `///` 类型与 trait | 领域含义、合法状态、调用方与实现方的责任 | 把类型名翻译成一句没有新增信息的话 |
| `///` 公共或复杂方法 | 可观察行为、前置条件、错误、必要的副作用 | 逐个复述参数名；只写“处理请求” |
| `//` 局部注释 | 授权顺序、并发约束、协议限制或取舍的依据 | “遍历列表”“增加计数”等代码复述 |
| 常量与限制 | 单位、限制对象、边界含义和协议/产品依据 | 无来源的“安全值”；与实现数值重复且容易漂移的长段落 |

首句简短概括接口行为，更多解释放在后续段落；相关类型使用 rustdoc 链接。需要时提供最小、可运行示例。只有展示用法的完整示例才值得加入 doctest，不为无 I/O 的私有转发函数编造示例。[C-CRATE-DOC](https://rust-lang.github.io/api-guidelines/documentation.html#crate-level-docs-are-thorough-and-include-examples-c-crate-doc)、[C-LINK](https://rust-lang.github.io/api-guidelines/documentation.html#prose-contains-hyperlinks-to-relevant-things-c-link)、[rustdoc 测试](https://doc.rust-lang.org/rustdoc/write-documentation/documentation-tests.html)

按实际契约使用以下小节；无需为不存在的行为添加空标题：

- **`# Errors`**：说明错误出现的条件及对调用方可见的后果。例如“写入意图已持久化但传输回执未知”应与“尚未发送”区分，便于调用方决定是否允许重试。
- **`# Panics`**：接口存在可触发的 panic 时说明条件；内部推导出的不变量应紧邻 `expect` 写清依据。
- **`# Safety`**：仅用于 `unsafe` API，逐项说明调用方必须维护的条件；调用处另用局部安全注释说明本次为何满足。

上述约定来自 [C-FAILURE](https://rust-lang.github.io/api-guidelines/documentation.html#function-docs-include-error-panic-and-safety-considerations-c-failure)。本仓库对本地部署、认证提示和消息内容的保密约束仍须由代码与测试维护，注释不能代替检查。

## 异步、所有权与取消

- **记录谁拥有后台工作。** 对跨任务、跨连接或跨进程存活的资源，说明创建者、终止条件、持有的 sender/child/socket 以及掉线后的行为。不要仅因为函数使用 `async` 就添加固定模板；只记录调用方无法从签名看出的生命周期契约。[Microsoft M-MODULE-DOCS](https://microsoft.github.io/rust-guidelines/guidelines/docs/index.html#M-MODULE-DOCS)
- **取消是状态转换的一部分。** 进入 `select!`、超时或可被调用方丢弃的 future 前，检查已经发生的副作用。说明中途取消会否消耗队列项、丢弃响应、保留持久化意图或使结果未知；不能把“future 被丢弃”当成“远端操作未发生”。Tokio 明确列出 `select!` 常见操作的取消安全性，应逐项核对具体 API。[Tokio `select!` cancellation safety](https://docs.rs/tokio/latest/tokio/macro.select.html#cancellation-safety)
- **锁注释描述保护对象与顺序。** Tokio 教程解释同步锁跨 `await` 的风险，以及异步锁用于 I/O 的取舍。本仓库进一步区分全局元数据锁与必要的串行锁：全局锁不等待网络，建连锁和会话操作锁可以跨 `await` 以维持串行。后者须解释保护的不变量、取得顺序与释放范围，避免将项目特例误解成所有锁都可长期持有。[Tokio Shared state](https://tokio.rs/tokio/tutorial/shared-state)
- **有界应能找到依据。** 队列、帧、附件、并发和重连限制旁说明单位及所保护的资源；持久化写入与网络写入之间说明先后关系。在本仓库中，未知回执不自动重发是业务授权与幂等性约束，不是所有 Rust 网络程序都必须使用的策略。依据见[架构说明](architecture.md)。

## 评审与验证

按[仓库开发约定](../CONTRIBUTING.md)，评审先确认职责、可见性、类型与副作用是否清楚，再看格式。结构调整应保留已有行为和测试入口；新增测试针对状态转换、边界和失败路径，不为纯移动、私有改名或注释增加重复实现的测试。

使用仓库的 `make check` 完成格式、默认 Clippy lint、rustdoc 生成与内部链接检查、测试与公开 Markdown 链接检查；需要可执行交付时运行 `make package`。所有 Cargo 构建、检查、测试和 Clippy 使用 `--release`，完成批次后按仓库要求保留 `dist/` 并顺序执行 `cargo clean`。这些命令与清理策略是本仓库规则。

Clippy 提供改进信号，不代替接口与行为评审。不要全局开启 `clippy::restriction` 后机械修复；官方明确反对整体启用。`clippy::pedantic` 更严格且可能产生有意接受的误报，需要逐项评估。确需例外时缩小作用域并说明原因；`#[expect(...)]` 可用于当前确实存在、希望在失效时得到提醒的 lint 例外；长期的 `#[allow(...)]` 也应缩小范围并给出原因。[Clippy 使用指南](https://doc.rust-lang.org/clippy/usage.html)、[Microsoft M-LINT-OVERRIDE-EXPECT](https://microsoft.github.io/rust-guidelines/guidelines/universal/index.html#M-LINT-OVERRIDE-EXPECT)
