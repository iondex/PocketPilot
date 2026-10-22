# Changelog

User-facing changes are recorded here. Unreleased entries describe work on `public`; they do not announce a published release.

## Unreleased

### Changed

- Separate shared runtime registration, connection policy and project pagination; isolate Claude native protocol and output processing; separate SQLite lifecycle from the durable operation ledger.
- Document provider authority, acceptance versus completion, uncertain writes, session locks and snapshot publication contracts.
- Telegram owner authorization is now deployment configuration. Add a positive numeric `[telegram].owner_user_id` and restart; the previous source-level account constant has been removed. Ingress, durable writes, bindings and background delivery share the configured owner check.
- Feishu now requires direct connections. Remove `[feishu].proxy`, including empty values. HTTP ignores environment proxies, WebSocket uses the SDK's direct connection entry, and the local OpenLark dial URL mapper has been removed. Future Lark support must also be direct only.
- Split Telegram service tests by navigation, history, recovery, input, topics and shared platform workflows while retaining shared local fixtures.
- Normalize first-party manifests and module entry points, and document crate responsibilities.

### Added

- Feishu/Lark protocol comparison and shared integration design, with separate regional credentials/state and direct-only transport.
- Rust style and annotation guide grounded in Rust, Google and Microsoft primary sources; `make check` now validates rustdoc links.
- PocketPilot brand assets with editable SVGs, PNG exports, shared logo and avatar variants, and localized English and Simplified Chinese README banners.
- Apache-2.0 project license, distribution notice and third-party source attributions.
- English and Chinese READMEs, documentation index, first-install and configuration guides, troubleshooting, developer onboarding, dependency patch policy and release procedures.
- Regression coverage for required Telegram owner configuration, owner changes and rejection of Feishu proxy settings.

### Known boundaries

- This remains a 0.1 prototype without automatic migration of existing SQLite columns.
- Feishu real-tenant acceptance remains outstanding; international Lark and Pi are not implemented.
- No release tag or binary release is created by this preparation.
