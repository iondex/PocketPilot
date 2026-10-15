# Codex 协议类型生成

全部生成逻辑在 crates/codex/build.rs。构建时调用所选 Codex 的 `app-server generate-json-schema`，筛选只读接口与四类允许写接口的传递引用，再通过 typify 生成 Rust DTO。provider 使用类型化字段，不直接操作 serde_json::Value。

官方 schema、协议投影、版本记录和 protocol.rs 都写入 Cargo OUT_DIR，通常位于 target/debug/build/codex-*/out/。仓库不提交生成的 schema、代码或版本快照，不保留 Python 生成脚本。

## 选择构建工具

默认使用 PATH 中的 codex。安装当前版本后运行 cargo build/test 即会自动导出，不需手动执行脚本。CI 显式安装 @openai/codex 0.162.1 作为当前验证的构建输入。

如需使用桌面内置或其他指定 binary，可通过 **仅构建时** 的 CODEX_SCHEMA_BIN 选择；它不读取 .env，也不覆盖运行时 TOML 或远程连接配置：

```sh
CODEX_SCHEMA_BIN=/absolute/path/to/codex cargo build --locked
```

Cargo 监听 build.rs、PATH、CODEX_SCHEMA_BIN 和实际 executable 的变化。替换工具后必要时执行 cargo clean -p codex，确保重新生成。构建记录实际 --version 到 OUT_DIR/schema-version.txt；Rust 侧可通过 codex::protocol::SCHEMA_CLI_VERSION 查看。

## 约束

只导出 schema，不启动、升级、重启 app-server 或恢复 thread。CLI 不存在、导出失败、缺少所需类型或类型生成失败时构建明确失败，不回退到旧快照。生成使用受测的官方导出表面。连接显式协商 experimentalApi，以获取已部署 0.161 服务的分页历史模式及接口；这不扩大封闭的方法集合，不允许停止、取消或审批代答。

schema 兼容仅证明类型可编译；共享实例、实时观察和断线语义仍需行为验收。RPC 的未知扩展参数只作为不透明 RawValue 保留，由明确 DTO 解码，不做业务字段猜测。

协议定义来自 OpenAI Codex（Apache-2.0）；许可证保留在 crates/codex/LICENSE-APACHE。

## 开放错误联合类型

CodexErrorInfo 的官方 schema 是包含字符串、结构化错误和开放分支的 anyOf。typify 0.5 遇到重叠分支时默认生成 flatten struct，会拒绝合法的字符串错误（例如 other），使包含失败轮次的历史无法读取。build.rs 仅在传给生成器时将该类型建模为按顺序匹配的 untagged union，保留所有官方分支；保存在 OUT_DIR 的投影保持原始 anyOf。错误码仍由 schema 生成，不改成业务 raw JSON。回归测试覆盖已知字符串/对象、未知开放字符串/对象与非法数字。

RPC 解码失败只记录白名单 schema 路径与固定错误类别，不记录响应正文或 serde 错误文本。
