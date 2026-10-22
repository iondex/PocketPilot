# 文档导航 / Documentation

PocketPilot 将聊天入口、执行主机和 agent runtime 分开。首次使用先安装一个消息入口并只读浏览，再验证需要控制的实例。

Detailed guides are currently written in Simplified Chinese. Start with the [English README](../README.md) for capabilities, prerequisites and build commands. These guides contain the configuration and operational details; the source and public TOML example define the current interface.

## 用户文档

| 文档 | 解决的问题 |
| --- | --- |
| [首次安装](getting-started.md) | 系统依赖、构建、最小配置、只读启动、控制资格 |
| [配置说明](configuration.md) | 单一 TOML、owner、主机、网络、状态和热更新边界 |
| [接入方案](integrations.md) | Telegram / Slack / Discord / 飞书的安装、日常命令、附件、结构化问答 |
| [故障排查](troubleshooting.md) | 根据现象执行只读诊断，处理未知写结果和升级问题 |
| [安全策略](../SECURITY.md) | 信任边界、敏感问题报告、公开材料脱敏 |

## 开发者文档

| 文档 | 解决的问题 |
| --- | --- |
| [参与开发](../CONTRIBUTING.md) | Rust 风格、开发流程、必要检查和产物清理 |
| [架构说明](architecture.md) | crate 分工、状态机、写入账本、协议和进程生命周期 |
| [开发指南](development.md) | 从入口追踪调用，扩展消息平台或 runtime，测试与 i18n |
| [依赖维护](dependencies.md) | OpenLark 补丁依据、缩减范围和退出条件、协议版本 |
| [发布流程](releasing.md) | 验证、打包、许可证、发布材料及升级说明 |
| [后续规划](support.md) | 未完成或尚待真实环境验证的能力 |
| [变更记录](../CHANGELOG.md) | 面向使用者的未发布变化与配置兼容性 |

公开配置以 [pocketpilot.example.toml](../pocketpilot.example.toml) 为起点。机器迁移见[状态数据与迁移](architecture.md#状态数据与迁移)。文档描述当前行为，单次联调过程不作为使用指南累积。
