# 构建与发布体积

所有构建、检查、测试和 Clippy 统一使用 release profile。Telegram、Discord、Slack 始终编译，运行配置决定启用哪些账号；Agent runtime 当前实现 Codex：

```sh
cargo build -p app --release --locked
```

输出为 `target/release/pocketpilot`，对应当前 Rust target 的操作系统和架构。其他平台需要独立构建与验收，不能直接复用本机产物。运行需要系统 OpenSSH 与已有 Codex app-server；build.rs 使用选定的 Codex CLI 生成协议类型，不需要分发 schema/DTO 文件，也不把 Codex CLI 打包进 binary。

## 发布设置

release profile 使用 `opt-level=z`、thin LTO、单个 codegen unit 和符号裁剪。保留正常 panic unwind、TLS/证书检查、SQLite、tracing 与会话边界保护，不采用运行时解包。

发布体积随目标、工具链和依赖变化。debug 产物包含调试信息，不宜直接用来评价 release 优化效果。精确大小和校验值应随对应发布产物保存，不在设计文档累计每次本地构建结果。

## 分析方法

1. 在相同 target、工具链、依赖与 profile 下比较文件大小。
2. 用链接 map 或符号分析定位存活对象，区分机器码、符号和链接元数据。
3. 优先检查 SDK 泛型、依赖 feature、SQLite、HTTP/TLS 和正则依赖的实际贡献。
4. 泛型会在调用方单态化，对象归属不等于某项功能的可删除成本；不能将各模块数字简单相加。
5. 修改依赖后运行完整 workspace 检查，确保三端都经过编译与行为验证。

构建依赖只负责生成，不等于运行时成本。删除未用依赖有助于维护，但体积优化应以链接证据为准，不通过移除安全连接、日志、持久化或未知结果保护来压缩。

检查命令见 [开发约定](../CONTRIBUTING.md)，验证范围见 [实现概览](research/implementation-checks.md)。

## 日常构建与清理

```sh
make check
make package
make clean
```

`make package` 构建并将 binary、SHA-256 和构建说明保存到 Git 忽略的 `dist/`；`make clean` 使用 `cargo clean` 清理 `target/`，保留已打包文件。默认 `make` 只构建 release；仓库的 VS Code Rust Analyzer 工作区设置也将构建脚本与编辑器检查指向 release，其他编辑器应使用同样参数。工具链包含 rust-analyzer，VS Code 工作区使用 PATH 中的 rustup rust-analyzer，更新工具链后重启编辑器的语言服务。使用 `make release` 可顺序完成检查、打包和清理。无需传入 FEATURES；三端均在默认构建中。

每轮验证和打包完成后清理构建目录，避免长期积累。本任务生成的 `/tmp` 调研副本、临时输出和验证日志在对应任务结束、结果汇总后一起清理。检查与编译期间保持 `target/` 独占，不与清理并行。清理后的下次构建会重新生成依赖和协议；运行中的部署应引用 `dist/pocketpilot`，不要依赖将被清理的 target 路径。其他开发机所需的配置和账本见 [迁移指南](migration.md)。
