# Repository instructions

Follow [CONTRIBUTING.md](CONTRIBUTING.md) for Rust conventions and required checks.

## Builds and disk space

- Use `--release` for every Cargo build, check, test and Clippy invocation. Keep one release artifact tree; rustfmt does not take a profile flag.
- Use `make check` for validation and `make package`; all chat integrations are included by default. Build commands live in [docs/integrations.md](docs/integrations.md); architecture and state transfer are documented in [docs/architecture.md](docs/architecture.md).
- After a completed build/validation batch, preserve any needed executable in the ignored `dist/` directory, then run `cargo clean` to reclaim `target/`. Clean before handoff as well. Remove temporary files created for the task in `/tmp` after their jobs finish and verification results are recorded. Keep cleanup sequential with compilation.
- Treat `pocketpilot.toml`, `.env*`, the configured data directory and private backups as local deployment data. Commit source and public examples; transfer private deployment data separately.
