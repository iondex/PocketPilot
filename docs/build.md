# 构建与发布体积

默认发布目标为 Telegram + Codex，不链接可选 Discord runtime：

```sh
cargo build -p app --release --no-default-features --locked
```

输出为 `target/release/pocketpilot`，对应当前 Rust target 的操作系统和架构。其他平台需要独立构建与验收，不能直接复用本机产物。运行需要系统 OpenSSH 与已有 Codex app-server；build.rs 使用选定的 Codex CLI 生成协议类型，不需要分发 schema/DTO 文件，也不把 Codex CLI 打包进 binary。

## 发布设置

release profile 使用 `opt-level=z`、thin LTO、单个 codegen unit 和符号裁剪。保留正常 panic unwind、TLS/证书检查、SQLite、tracing 与会话边界保护，不采用运行时解包。

默认发布体积约为十余 MB，随目标、工具链和依赖变化。debug 产物包含调试信息，不宜直接用来评价 release 优化效果。精确大小和校验值应随对应发布产物保存，不在设计文档累计每次本地构建结果。

## 分析方法

1. 在相同 target、工具链、feature 与 profile 下比较文件大小。
2. 用链接 map 或符号分析定位存活对象，区分机器码、符号和链接元数据。
3. 优先检查 SDK 泛型、平台 feature、SQLite、HTTP/TLS 和正则依赖的实际贡献。
4. 泛型会在调用方单态化，对象归属不等于某项功能的可删除成本；不能将各模块数字简单相加。
5. 修改依赖后运行最小发布配置与全 feature 检查，防止可选功能掩盖默认构建问题。

构建依赖只负责生成，不等于运行时成本。删除未用依赖有助于维护，但体积优化应以链接证据为准，不通过移除安全连接、日志、持久化或未知结果保护来压缩。

检查命令见 [开发约定](../CONTRIBUTING.md)，验证范围见 [实现概览](research/implementation-checks.md)。
