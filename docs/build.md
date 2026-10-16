# 构建与 binary 体积

默认发布目标是 Telegram + Codex，不链接 Discord runtime。所有平台代码均接受全 feature 检查；生产构建只选择 app：

```sh
cargo build -p app --release --no-default-features --locked
```

输出为 target/release/pocketpilot。当前生成的是 macOS arm64 binary；不能把它当作 Linux/Windows 可执行文件。需要本机可用的系统 OpenSSH 和所连主机已经运行的 Codex app-server。build.rs 使用本机当前 Codex CLI 生成协议，不需要分发 schema/DTO 文件，也不会把 Codex CLI 打包进 binary。

release profile 使用 opt-level=z、thin LTO、codegen-units=1 和 strip=symbols。保留正常 panic unwind、TLS/证书检查、SQLite 持久化、trace 和全部会话边界，未使用 UPX 或运行时解包。

## 2026-10-16 实测

本轮重构后同一机器默认 release 约 19.51 MB，体积优化后10.19 MB（9.72 MiB），缩小 47.76%。重构前 debug binary 约 68 MiB，包含调试/符号信息，不能作为 release 编译优化的公平基准。

默认 release 的链接 map 中，较大的存活对象来源如下。此表是链接对象归属，不是完整功能的独占成本；泛型会在调用方单态化，表中数值不能简单等同于各功能的可移除体积。

| 对象来源 | 存活符号约 MB |
| --- | ---: |
| Telegram adapter 与 SDK 泛型处理 | 2.98 |
| app binary 的单态化/装配 | 1.90 |
| SQLite 原生库 | 1.60 |
| Codex typed provider/RPC/DTO | 1.09 |
| reqwest | 0.75 |
| regex-syntax + regex-automata | 1.28 |
| rustls + ring | 0.95 |
| sqlx-sqlite | 0.43 |
| clap-builder | 0.31 |

Mach-O 的默认 __TEXT 约 13.86 MB，其中机器码约 10.51 MB；__LINKEDIT 约 5.10 MB。优化构建明显减少机器码与符号/链接开销。精确文件大小、SHA-256 与本轮验证结果记录在 [实现记录](research/implementation-checks.md)。

已删除 app 中迁出后多余的直接依赖和测试专用 CLI，Discord runtime 保持可选。typify/schemars/syn 等构建依赖不作为应用运行依赖；仅修改它们无法直接缩小发布 binary。后续优先根据链接证据缩减热点泛型和不必要的 SDK feature，不通过删除 TLS、日志、未知结果保护或状态持久化来压缩。
