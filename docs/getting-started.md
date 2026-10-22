# 首次安装

目标是在自己的 Linux/macOS 机器上运行一份桥接，连接一个聊天平台与一个允许访问的 agent 实例。完整平台权限见[接入方案](integrations.md)，配置字段见[配置说明](configuration.md)。

## 1. 准备依赖

| 依赖 | 用途 |
| --- | --- |
| stable Rust，带 rustfmt / Clippy | 工具链由根 rust-toolchain.toml 声明；manifest 最低 1.94 |
| Make、Python 3、C 编译器和系统开发工具 | 构建 Rust 原生依赖；SQLite 从依赖构建，不要求预装数据库服务 |
| Node.js / npm 和 Codex CLI 0.162.1 | CI 固定版本；构建从 CLI 导出 typed DTO，即使只使用 Claude 也需要 |
| 系统 OpenSSH | SSH 主机配置、密钥、known_hosts、跳板和认证 |
| 一个可用的消息平台应用 | 各平台使用自己的 owner 和安装身份 |
| 已登录的原生 agent runtime | Codex 使用已有共享 app-server；Claude 需安装独立 server |

Debian/Ubuntu 可先安装 `build-essential pkg-config make openssh-client`；macOS 安装 Xcode Command Line Tools。Rust 通过 rustup 安装，进入仓库后由工具链文件选择组件。Windows 原生构建和部署尚未作为支持目标验证。

```sh
npm install --global @openai/codex@0.162.1
codex --version
rustc --version
```

构建时使用 PATH 中的 `codex`；需要选择其他路径时设置 `CODEX_SCHEMA_BIN`。协议生成失败会中止构建，不回退到旧快照。更换 CLI 版本要按[依赖维护](dependencies.md)验证。

## 2. 准备私有配置

```sh
cp pocketpilot.example.toml pocketpilot.toml
chmod 600 pocketpilot.toml
```

最小配置示例以本机 Codex 和 Telegram 为例；替换 token、owner 和项目路径。数字 ID 是合成示例。

```toml
language = "zh-CN"

[service]
data_dir = "var/data"

[telegram]
token = "REPLACE_WITH_BOT_TOKEN"
owner_user_id = 123456789
default_locale = "zh-CN"

[[hosts]]
id = "local-dev"
name = "本机"
transport = "local_proxy"
allowed_workspace_roots = ["/srv/code"]
```

Telegram 必须按[安装步骤](integrations.md#telegram)开启私聊 Topics。使用 Slack、Discord、飞书或 Lark 时，替换平台段并完成对应安装流程；所有平台无需同时配置。飞书和 Lark 固定直连，不允许 `proxy`，也不使用环境代理；Lark 使用 `[lark]` 配置段，详见[接入方案](integrations.md#lark-企业自建应用)。

远程主机把 transport 改为 `ssh_proxy` 并配置 `ssh_alias`。先独立确认 `ssh pilot-dev` 能在自己的账号上完成认证并执行命令，保留主机密钥校验。项目路径属于执行主机，不能填写桥接主机上不对应的路径。

## 3. 构建与只读诊断

```sh
make check
make package
cargo clean
./dist/pocketpilot doctor --credentials
./dist/pocketpilot doctor --connect
```

`make check` 运行 rustfmt、release Clippy 和 release 测试；`make package` 将当前系统/架构的可执行文件、构建信息及校验值放入忽略的 `dist/`。它不构建所有操作系统或 CPU 架构。可用 `make release` 顺序执行检查、打包和清理。

`doctor --credentials` 只检查凭据格式，不验证平台 token 是否有效。`doctor --connect` 只读探测主机及 runtime；不会启动 Codex daemon、创建任务或授予控制资格。可按需使用 `--telegram`、`--discord`、`--feishu` 探测平台身份。Slack 在 `serve` 启动时验证 DM 和 Socket Mode。

Codex 必须已存在可共享的 app-server 入口。共用历史目录不代表连到了同一实例。PocketPilot 不会为了浏览另起 daemon 或隐式 resume 原生会话。

## 4. 启动和浏览

```sh
./dist/pocketpilot serve
```

使用配置 owner 从平台菜单选择执行环境、项目、会话。启动后的 catalog 扫描登记允许范围内的 provider 和项目；新 provider 默认只读。浏览会话确认目标账号、项目 cwd 和状态正确。

```sh
./dist/pocketpilot hosts list
./dist/pocketpilot providers list
./dist/pocketpilot workspaces list
```

需要登记项目时执行 `workspaces add --provider PROVIDER_ID --cwd /srv/code/project --name 示例项目`。provider/workspace 属于 SQLite 状态，不在 TOML 中手工维护数组。

## 启用任务输入

先退出 `serve`，让服务锁释放；原生任务仍由 runtime 运行。对 Codex 使用专用测试项目：

```sh
./dist/pocketpilot providers verify PROVIDER_ID --project PROJECT_ID
./dist/pocketpilot serve
```

验证会创建测试会话、提交任务和附件，检查共享 active turn、steer、断开后的任务存活与最终读取，可能消耗模型额度。成功后启用该实例的控制资格。失败或回执未知时不反复执行；先检查原生端状态。

`providers trust PROVIDER_ID` 只登记人工确认，不运行验证；仅在已有独立证据时使用，`--revoke` 撤销资格。Claude 不使用 Codex 的 verify 探针，安装和人工验收步骤见[独立 runtime server](integrations.md#claude-code-独立-runtime-server)。

## 5. 长期运行和升级

由自己的进程管理器运行 `dist/pocketpilot --config /absolute/path/pocketpilot.toml serve`。进程用户需拥有该配置、data_dir 和必要 SSH 凭据，工作目录不应决定配置文件位置。同一 data_dir 只运行一份 `serve`。

升级前停止桥接并备份私有 TOML 和完整 data_dir；不要只复制 WAL 模式下的单个数据库文件。0.1 原型不自动迁移已有列，不删除用户数据来“修复”升级。详细步骤见[发布流程](releasing.md)和[状态迁移](architecture.md#状态数据与迁移)。
