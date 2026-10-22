# 依赖与本地补丁

依赖版本记录在 Cargo.lock，构建和验证使用 `--locked`。升级时围绕真实接口和行为验证，不以能生成 DTO 或能编译代替共享会话、权限或回执验收。第三方许可证说明见 [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md)。

## OpenLark 为什么需要本地补丁

飞书和 Lark 复用 OpenLark 的认证、token 刷新、REST Transport、protobuf WebSocket、心跳、事件分发与卡片 ACK。仅 `openlark-client` 0.21.0 通过根 `[patch.crates-io]` 指向 vendor；core、auth 等仍是 registry 依赖。来源与改动清单保存在 [PATCHES.md](../vendor/openlark-client/PATCHES.md)。

当前保留三类修改：

| 修改 | 原因 | 为什么不能只在业务回调中处理 |
| --- | --- | --- |
| 有界分片重组 | 最多 32 片、单事件 8 MiB、64 个未完成事件、总缓冲 16 MiB、15 秒失效；约束 header 和重复/冲突元数据 | 业务收到 payload 前 SDK 已分配并缓存分片，回调的 512 KiB 限制发生得太晚 |
| 连接日志脱敏 | 不输出携带认证参数的 WebSocket URL | 应用独立过滤 SDK 日志作为第二道保护；依赖源码仍应避免生成敏感日志 |
| 入站活动刷新存活时间 | 有流量的连接不因仅匹配 WebSocket Ping 而误判空闲 | 存活时间保存在 SDK 的 session loop 内，应用回调无法更新 |

飞书和 Lark 固定直连。此前增加的 `open_with_url_mapper` 和 dial URL 映射路径已删除，应用调用上游公开的 `LarkWsClient::open`。HTTP 显式 `.no_proxy()`，WebSocket 直接拨号；不使用本地 relay，不提供平台 `proxy` 配置，包括空字符串。

应用仍独立校验 app/tenant/owner/group、业务事件大小、时效和 action nonce；这些是业务权限，不放进 SDK。协议分发、原生 ACK 和 token 管理继续由上游实现。

## 如何避免 vendor

长期优先方案是将通用的资源边界、日志修正和存活修正反馈上游，在正式版本提供等价行为后删除本地补丁。这里记录退出条件，不代表已经向上游提交或已确认某个新版本修复。

移除补丁前需逐项验证：

1. 注册版本对分片数量、总字节、未完成消息数、重复/冲突元数据和失效时间提供等价限制。
2. 日志不输出认证 URL，应用的独立过滤仍有效。
3. 持续收取数据、Pong 等活动时不发生错误的 idle 断开，真正空闲时仍能退出。
4. 飞书和 Lark 直接连接、持久化入队后 ACK、新版卡片回调和 unknown 不重发语义保持。
5. 更新 Cargo.lock、删除 `[patch.crates-io]` 和 vendor 后通过完整 `make check`、打包及真实应用验收。

另一个方案是保留 OpenLark 的 auth/core，在 `integrations::feishu` 自行维护 WebSocket transport。它可以去掉 vendor，但同时接管协议、分片、心跳、ACK 和重连的维护责任；不应为缩短依赖目录而重复实现整套传输。

## 补丁维护规则

保持改动聚焦，不修改无关 SDK 模块或丢失来源和许可证。升级前比较 registry 发布源码与 vendor，记录剩余差异；不要把整个上游 workspace 复制进项目。

`feishu/sdk_tests.rs` 将确切的 vendor 重组实现及其边界测试纳入 workspace 验证。`make check` 不运行 vendor 独立 crate 的全部上游测试；会话与 ACK 的其他行为由平台 fixture 和升级验收共同覆盖。两端实际安装仍需真实租户验证，不能据离线测试宣称已完成。

## Codex 构建协议

CI 固定使用 Codex CLI 0.162.1。`adapters/build.rs` 从 PATH 或 `CODEX_SCHEMA_BIN` 选择可执行文件，检查版本、导出 schema，并在 OUT_DIR 生成 Rust 类型。导出只生成文件，不启动模型或 runtime。

升级 CLI 时同步 CI、安装文档与协议 fixture，运行解析、分页、错误分类、写回执和结构化问题测试，再在专用项目验证共享实例。schema 可生成只证明协议结构可处理，不证明执行端共享生命周期。
