# OpenLark client 0.21.0

Source: crates.io `openlark-client` 0.21.0, upstream foxzool/openlark commit
393f140138f81c5237a7289b2823885423bb4cba. Apache-2.0; upstream authorship is
preserved in Cargo.toml and source. Only the client crate is vendored.

PocketPilot modifications:

- Cap fragmented events at 32 pieces, 8 MiB per event, 64 pending events and
  16 MiB aggregate buffered bytes; expire incomplete events after 15 seconds.
  Reject invalid/changed fragment metadata and account duplicate fragments.
  Cap headers at 64 entries, 128-byte keys and 1024-byte values.
- Remove the credential-bearing WebSocket URL from connection diagnostics.
- Refresh idle liveness on inbound WebSocket activity.
- Expose an async dial-URL mapper, preserving original service metadata, so the
  host can route WebSocket traffic through its existing bounded proxy relay.

Protocol dispatch, authentication integration, protobuf, heartbeat and callback
ACK behavior otherwise remain upstream. The application independently suppresses
SDK transport logs and maps errors to fixed categories.
