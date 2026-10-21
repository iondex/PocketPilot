//! Exercise the exact vendored reassembly source in the workspace's required test suite.
// Other header constants belong to SDK dispatch, outside this narrow fragment harness.
#[allow(dead_code)]
#[path = "../../../../vendor/openlark-client/src/ws_client/headers.rs"]
mod headers;

#[path = "../../../../vendor/openlark-client/src/ws_client/package.rs"]
mod package;
