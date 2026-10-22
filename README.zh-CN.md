# PocketPilot

![PocketPilot — 从你常用的聊天工具，接入不同主机上的 AI 编程会话。](assets/brand/banner.zh-CN.png)

[English](README.md) · [文档导航](docs/README.md) · [参与开发](CONTRIBUTING.md) · [品牌资源](assets/brand/README.zh-CN.md)

PocketPilot 是面向开发者的自托管会话入口。它连接聊天平台与本机或 SSH 主机上的 AI 编程运行时，让你选择执行环境、项目和会话，查看进度、补充任务、上传文件并回答 agent 的结构化问题。执行、上下文和会话生命周期由原生 runtime 管理。

## 当前能力

| 维度 | 当前支持 | 使用边界 |
| --- | --- | --- |
| 消息入口 | Telegram、Slack、Discord、国内飞书 | 四端默认编译，配置决定启用哪些入口 |
| 执行环境 | 本机、OpenSSH 远程主机 | 仅访问配置允许的项目根目录 |
| Codex | 已运行且可共享的 app-server | 新发现实例默认只读，显式验证后允许任务输入 |
| Claude Code | PocketPilot 独立 runtime server | 仅管理 server 自己创建的会话，尚不支持活跃轮次 steer |
| 界面语言 | 中文、英文 | 显式 locale，不翻译用户和 agent 正文 |

Telegram 在指定用户私聊中用 topic 区分会话；Slack 使用 owner DM 中的独立 thread；Discord 使用独立 session channel；飞书通过私聊导航，在授权群话题内提交任务。不同平台可以绑定同一个原生 session，共用桥接进程内的连接、会话锁和操作账本。

飞书目前完成离线协议和隔离测试，仍需真实租户安装验收。国际 Lark、Pi Coding Agent 尚未支持，见[后续规划](docs/support.md)。

## 快速开始

桥接面向 Linux/macOS，需要仓库指定的 stable Rust（manifest 最低 1.94）、Make、Python 3.10+（文档检查）、C 编译工具链，以及远程连接所需的 OpenSSH。构建需安装与 CI 一致的 **Codex CLI 0.162.1** 来导出协议类型；此过程不会启动模型任务。

```sh
git clone --branch public https://github.com/iondex/PocketPilot.git
cd PocketPilot
npm install --global @openai/codex@0.162.1
cp pocketpilot.example.toml pocketpilot.toml
chmod 600 pocketpilot.toml
```

编辑配置：填写要启用的平台凭据和 owner，修改主机与允许目录，删除未使用的平台段。Telegram 必须配置本人的数字 `owner_user_id`。飞书固定直连，禁止 `proxy` 字段。程序不读取 `.env`。

```sh
make check
make package
cargo clean
./dist/pocketpilot doctor --credentials
./dist/pocketpilot doctor --connect
./dist/pocketpilot serve
```

先浏览只读会话，再停止桥接，按[控制资格验证](docs/getting-started.md#启用任务输入)启用 Codex 输入。验证会创建专用测试会话并调用模型，普通连接诊断不授予控制权。完整依赖、配置和操作步骤见[首次安装](docs/getting-started.md)。

## 执行与恢复

- 浏览和后台观察只读；创建、resume、续轮、steer 和回答问题需要明确授权。
- 写入回执未知时保存 unknown，不自动重发。重连不会重复提交任务。
- 桥接停止只断开客户端连接，原生 runtime 继续拥有任务。
- 工具权限审批在原生界面完成；不提供 stop/interrupt 或审批代答。
- `retry` 是单独显式启动的工具，不随 `serve` 运行，详见[重试规则](docs/architecture.md#独立-retry-工具)。

当前是 **0.1 原型**，配置与原生协议可能变化，SQLite 已有列不自动迁移。升级前应备份完整状态目录。

## 文档

| 需求 | 入口 |
| --- | --- |
| 首次部署 | [安装指南](docs/getting-started.md) |
| 凭据、主机、网络和数据目录 | [配置说明](docs/configuration.md)、[公开配置示例](pocketpilot.example.toml) |
| 平台安装、会话操作、附件和问答 | [接入方案](docs/integrations.md) |
| 诊断连接、授权或消息问题 | [故障排查](docs/troubleshooting.md) |
| 理解模块、授权和持久化 | [架构说明](docs/architecture.md) |
| 扩展平台或 runtime | [开发指南](docs/development.md)、[开发约定](CONTRIBUTING.md) |
| 了解 OpenLark 本地补丁 | [依赖维护](docs/dependencies.md) |
| 准备发布 | [发布流程](docs/releasing.md)、[变更记录](CHANGELOG.md) |

## 参与和许可证

欢迎提交聚焦的改进及验证行为边界的测试。公开 issue 使用合成配置和脱敏日志；敏感问题按 [SECURITY.md](SECURITY.md) 私下报告。

项目采用 [Apache-2.0](LICENSE)。第三方来源和修改说明见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。本项目独立开发，与接入的平台及 agent 厂商没有隶属关系。
