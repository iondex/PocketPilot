# PocketPilot

![PocketPilot — Your coding sessions, from the chat tools you already use.](assets/brand/banner.en.png)

[简体中文](README.zh-CN.md) · [Documentation](docs/README.md) · [Contributing](CONTRIBUTING.md)

PocketPilot is a self-hosted bridge between messaging platforms and AI coding runtimes on your local or SSH-connected hosts. Choose a host, project and session; read progress, send tasks, upload files and answer structured agent questions. The native runtime owns execution and session history.

## What works today

| Layer | Implementation | Boundary |
| --- | --- | --- |
| Chat | Telegram, Slack, Discord, Feishu, Lark | All five compile by default; only configured integrations run |
| Hosts | Local processes and OpenSSH | Access is restricted to configured project roots |
| Codex | An existing, shared app-server | Newly discovered instances are read-only until explicitly verified |
| Claude Code | A separate PocketPilot runtime server | Only sessions created by that server; active-turn steer is unavailable |
| Interface languages | English and Simplified Chinese | Explicit locale; user content is not translated |

Telegram uses private-chat topics, Slack uses owner DM threads, Discord uses dedicated session channels, and Feishu/Lark use authorized group topics with a private navigation menu. Several chat platforms can access the same native session through the bridge's shared connection and operation ledger.

Feishu and Lark share one implementation, with separate application credentials and state. Local protocol and isolation tests cover both; real tenant installations still need acceptance testing. Pi Coding Agent is planned. See the [support roadmap](docs/support.md).

## Quick start

The bridge targets Linux and macOS. You need the repository's stable Rust toolchain (manifest minimum: 1.94), Make, Python 3 (test fixtures), a C build toolchain, OpenSSH for remote hosts, and **Codex CLI 0.162.1**, the protocol version pinned in CI. The build exports typed protocol definitions from that CLI without starting a model task. See [installation](docs/getting-started.md) for prerequisites and runtime setup.

```sh
git clone --branch public https://github.com/iondex/PocketPilot.git
cd PocketPilot
npm install --global @openai/codex@0.162.1
cp pocketpilot.example.toml pocketpilot.toml
chmod 600 pocketpilot.toml
```

Edit `pocketpilot.toml`: enable the chat platform you use, enter its credentials and owner identity, and configure your host and allowed project roots. Telegram requires your numeric `owner_user_id`; **Feishu and Lark accept no proxy setting**. Omit unused platform sections. The program does not load `.env` files.

```sh
make check
make package
cargo clean
./dist/pocketpilot doctor --credentials
./dist/pocketpilot doctor --connect
./dist/pocketpilot serve
```

Start with read-only navigation. For Codex task input, stop the bridge and follow the explicit [control verification procedure](docs/getting-started.md#启用任务输入). Verification creates a dedicated test session and performs model operations; connection diagnostics alone do not enable control.

## Execution and recovery

- Browsing and background observation are read-only. Creating, resuming, steering or answering a question requires an explicit authorized action.
- Unknown write outcomes are persisted and never automatically replayed. Reconnecting does not duplicate a task.
- Stopping the bridge disconnects its clients; the native runtime keeps ownership of running tasks.
- Tool permission approvals stay in the native interface. PocketPilot does not stop or interrupt agent tasks.
- The optional `retry` command is separate from `serve` and requires explicit launch; see its [authorization and retry rules](docs/architecture.md#独立-retry-工具).

This is a **0.1 prototype**. Configuration and native protocols can change. Existing SQLite schemas are not automatically migrated; back up the complete state directory before upgrading.

## Documentation

Detailed guides are currently in Simplified Chinese. The [documentation index](docs/README.md) maps user and developer workflows.

| I want to… | Read |
| --- | --- |
| Install my first bridge | [Getting started](docs/getting-started.md) |
| Configure credentials, hosts and networking | [Configuration](docs/configuration.md) and [public example](pocketpilot.example.toml) |
| Install a bot and use sessions or attachments | [Platform setup and usage](docs/integrations.md) |
| Diagnose connection or delivery problems | [Troubleshooting](docs/troubleshooting.md) |
| Understand authorization, state and recovery | [Architecture](docs/architecture.md) |
| Add a platform or runtime | [Developer guide](docs/development.md) |
| Review the OpenLark patch | [Dependency policy](docs/dependencies.md) |
| Prepare a release | [Release process](docs/releasing.md) and [changelog](CHANGELOG.md) |

## Contributing and security

Small, focused changes with behavior and boundary tests are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) before building; use release-profile Cargo commands and clean build artifacts after packaging. Report bugs through the repository's [issue tracker](https://github.com/iondex/PocketPilot/issues), using synthetic examples and redacted logs. Follow [SECURITY.md](SECURITY.md) for sensitive reports.

## License

PocketPilot is licensed under [Apache-2.0](LICENSE). Third-party licenses and local modifications are listed in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md); dependencies retain their own licenses. PocketPilot is an independent project and is not affiliated with the messaging or agent vendors it integrates.
