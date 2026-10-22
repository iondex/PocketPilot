# Security policy

PocketPilot is a self-hosted 0.1 prototype. The current development branch receives fixes; there is no supported stable release or guaranteed response schedule yet.

## Reporting a vulnerability

Use the repository's **Security → Report a vulnerability** private reporting entry when it is available. Include the affected commit, platform/runtime versions, a minimal synthetic reproduction and the authorization or data boundary involved.

Do not post tokens, OTP seeds, account identities, private endpoints, session contents, attachments, databases or working exploit details in public issues. If private reporting is not enabled, ask a maintainer for a private contact through an issue that contains no sensitive details, then send the report privately. The repository does not currently publish a separate security email address.

## Trust boundaries

- The bridge runs with its process user's local and SSH permissions. Use a dedicated account and restrict allowed project roots.
- Each chat integration verifies its owner and installation scope. A bot installation or a platform administrator role alone does not grant access to agent control.
- Feishu and future Lark use direct connections only; proxy configuration is rejected. Telegram, Slack and Discord can use an explicit configured proxy.
- The native agent still controls tool permissions and execution. PocketPilot does not automatically answer tool approvals or stop native tasks.
- Unknown write outcomes are persisted and are not automatically replayed. The optional retry process requires explicit launch and follows its own eligibility checks.
- Files, event queues and message rendering have limits. Uploaded content is agent input, not trusted executable content for the bridge.

Keep private TOML files, data directories, SSH credentials and backups outside Git. Restrict their permissions, and back up complete SQLite state before upgrades. If credentials have entered Git history, revoke or rotate them before addressing history removal.

The [architecture](docs/architecture.md) describes state and authorization boundaries. OpenLark modifications and their remaining verification limits are documented in the [dependency policy](docs/dependencies.md).
