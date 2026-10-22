# 发布流程

`public` 是开源代码与默认开发分支；功能改动通过独立分支和 PR 进入它。分支准备、GitHub 默认分支和仓库可见性是不同操作：推送代码不会自动发布版本或将私有仓库改为公开。

## 准备发布材料

确认 README、公开配置和使用指南描述当前能力，CHANGELOG 的 Unreleased 记录配置兼容性及限制。飞书/Lark 的本地协议测试与真实租户验收要分开说明；Pi 的未完成状态不能被写成已交付。

许可证采用 Apache-2.0，保留 LICENSE、NOTICE、THIRD_PARTY_NOTICES，以及 adapter 和 vendor 的来源声明。依赖更新需复核许可证，打包二进制分发时一并提供这些材料。

公开材料只包含源码和合成示例。根私有 `pocketpilot.toml`、`.env*`、配置 data_dir、SSH 资料、日志、数据库和私人备份不提交。目录在 `.gitignore` 中不代表历史提交中也没有相关内容；首次公开前需检查所有将公开的分支、tag 和 Git 历史。若发现实际凭据，先撤销/轮换，再协调历史清理，不能只删除当前文件。

图标、头像及中英文 banner 位于 [品牌资源](../assets/brand/README.zh-CN.md)，包含可编辑 SVG、PNG 和可重复导出脚本。双语 README 已引用对应 PNG；发布时确认图片随源码提交、相对路径有效、文案和当前能力一致。它们采用仓库 Apache-2.0 许可，字体不随仓库分发。

## 验证与打包

安装 CI 固定的 Codex CLI，使用干净且可追溯的 commit：

```sh
make check
make package
cargo clean
```

也可运行 `make release` 顺序执行同一流程。`make package` 生成本机系统/架构的 `dist/pocketpilot`、BUILD.txt 和 SHA-256 校验文件，不自动交叉构建多平台产物。

需要 Claude 独立 server 时，先配置对应 musl 目标与 C toolchain，运行 `make package-agent-server`，保留 dist 后清理 target；目标安装与验收见[接入方案](integrations.md#claude-code-独立-runtime-server)。桥接和 server 的 JSONL 协议必须配套，不自动重启用户已有 server。

发布附件至少记录 commit、目标系统/CPU、Rust 与 Codex schema 版本、校验值、许可证和用户可执行的安装步骤。只上传经过该平台验证的产物，不把本机测试结果外推到其他平台。日志、真实配置和数据库不作为附件。

## GitHub 分支与版本

先推送 `public` 并核对远程 commit。仓库管理员在 Settings → General → Default branch 选择 `public`，随后检查分支规则、CI、PR 默认目标和克隆说明。除非另有明确安排，保留 main 和历史，不自动删分支或强推改写。

本次准备不自动创建 tag、GitHub Release 或发布 crates；八个 crate 均为 `publish = false`。确定版本与发布范围后，再把 Unreleased 条目归入具体版本，创建可追溯 tag，并附验证结果和兼容性说明。

## 升级已有部署

1. 停止旧桥接并禁止自动重启，保留原生 agent 进程。
2. 一致性备份私有 TOML 与完整 data_dir。SQLite 使用 WAL，不能只复制主数据库文件；备份期间不修改 catalog。
3. 比较配置和 schema。本次需添加 Telegram `owner_user_id`，删除飞书 `proxy`。0.1 前没有自动迁移器，已有列变化必须明确处理自己的原型状态，不自动删库。
4. 用新 binary 运行离线配置诊断，再进行显式网络探测。核对账号、endpoint、允许目录及绑定归属。
5. 启动一份 `serve` 并确认只读导航和投递，保留原端对未知写结果的确认路径。

回退需要与旧版匹配的 binary、配置和一致性状态备份。不要在已变化的数据库上盲目运行旧 binary，也不要通过重复任务来验证回退。
