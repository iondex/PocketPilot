# Repository instructions

Follow [CONTRIBUTING.md](CONTRIBUTING.md) for Rust conventions and required checks.

## Builds and disk space

- Use `--release` for every Cargo build, check, test and Clippy invocation. Keep one release artifact tree; rustfmt does not take a profile flag.
- Use `make check` for validation and `make package FEATURES=discord,slack` when all supported integrations are needed. Build commands and migration steps live in [docs/build.md](docs/build.md) and [docs/migration.md](docs/migration.md).
- After a completed build/validation batch, preserve any needed executable in the ignored `dist/` directory, then run `cargo clean` to reclaim `target/`. Clean before handoff as well. Remove temporary files created for the task in `/tmp` after their jobs finish and verification results are recorded. Keep cleanup sequential with compilation.
- Treat `pocketpilot.toml`, `.env*`, the configured data directory and private backups as local deployment data. Commit source and public examples; transfer private deployment data separately.
