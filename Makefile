.DEFAULT_GOAL := build

FEATURES ?=
FEATURE_ARGS = $(if $(strip $(FEATURES)),--features $(FEATURES),)

.PHONY: build check package release clean

build:
	cargo build -p app --release --locked $(FEATURE_ARGS)

check:
	cargo fmt --all -- --check
	cargo clippy --release --workspace --all-features --all-targets --locked -- -D warnings
	cargo test --release --workspace --all-features --locked
	cargo clippy --release -p app --no-default-features --locked -- -D warnings

package: build
	mkdir -p dist
	cp target/release/pocketpilot dist/pocketpilot
	./dist/pocketpilot --version > dist/BUILD.txt
	rustc -vV >> dist/BUILD.txt
	@printf 'features: %s\n' '$(FEATURES)' >> dist/BUILD.txt
	@if command -v sha256sum >/dev/null 2>&1; then sha256sum dist/pocketpilot > dist/pocketpilot.sha256; else shasum -a 256 dist/pocketpilot > dist/pocketpilot.sha256; fi

release:
	$(MAKE) check
	$(MAKE) package FEATURES="$(FEATURES)"
	$(MAKE) clean

clean:
	cargo clean
