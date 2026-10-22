# Third-party notices

PocketPilot source is licensed under [Apache-2.0](LICENSE). Third-party components retain their own licenses; the project license does not relicense dependencies.

## OpenLark

- Component: `openlark-client` **0.21.0**, vendored under `vendor/openlark-client/`.
- Source: crates.io release, upstream [foxzool/openlark](https://github.com/foxzool/openlark), revision `393f140138f81c5237a7289b2823885423bb4cba` as recorded with the vendored source.
- License: [Apache-2.0](vendor/openlark-client/LICENSE); upstream authorship remains in its manifest and source.
- Modifications: bounded fragmented-event reassembly, credential-safe connection logging and inbound liveness updates. See [PATCHES.md](vendor/openlark-client/PATCHES.md) and the [maintenance rationale](docs/dependencies.md).

Other OpenLark crates are registry dependencies, not local source forks. The removed dial URL mapper is no longer part of this patch; Feishu and Lark connect directly.

## OpenAI Codex protocol

- Source: [openai/codex](https://github.com/openai/codex), Apache-2.0.
- The build invokes the installed CLI to export protocol schemas and generates Rust DTOs in Cargo OUT_DIR. Generated files are not checked in.
- The existing [adapter notice](crates/adapters/NOTICE) and [upstream license](crates/adapters/LICENSE-APACHE) are preserved. CI's selected CLI version is documented in the developer and installation guides.

## Other dependencies and binary distribution

[Cargo.lock](Cargo.lock) records the resolved Rust dependency graph. Dependencies may use licenses other than Apache-2.0. Before distributing binaries, review the resolved graph and include all notices and license texts required by the bundled components; this document identifies source-level inclusions and is not a complete generated license inventory of every transitive dependency.

Product and service names belong to their respective owners. PocketPilot is an independent integration and does not claim endorsement or affiliation.
