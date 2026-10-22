.DEFAULT_GOAL := build

.PHONY: build check package release clean

build:
	cargo build -p app --release --locked

check:
	python3 scripts/check-docs.py
	cargo fmt --all -- --check
	cargo clippy --release --workspace --all-targets --locked -- -D warnings
	cargo test --release --workspace --locked

package: build
	mkdir -p dist
	cp target/release/pocketpilot dist/pocketpilot.new
	mv -f dist/pocketpilot.new dist/pocketpilot
	./dist/pocketpilot --version > dist/BUILD.txt
	rustc -vV >> dist/BUILD.txt
	@printf 'integrations: telegram,discord,slack,feishu\n' >> dist/BUILD.txt
	@if command -v sha256sum >/dev/null 2>&1; then sha256sum dist/pocketpilot > dist/pocketpilot.sha256; else shasum -a 256 dist/pocketpilot > dist/pocketpilot.sha256; fi

release:
	$(MAKE) check
	$(MAKE) package
	$(MAKE) clean

clean:
	cargo clean

# The runtime binary is uploaded to the target account, independently of the bridge.
RUNTIME_TARGET ?= x86_64-unknown-linux-musl

.PHONY: package-agent-server

package-agent-server:
	cargo build --release --locked -p agent-server --target $(RUNTIME_TARGET)
	mkdir -p dist
	cp target/$(RUNTIME_TARGET)/release/pocketpilot-agent-server dist/pocketpilot-agent-server-$(RUNTIME_TARGET).new
	mv -f dist/pocketpilot-agent-server-$(RUNTIME_TARGET).new dist/pocketpilot-agent-server-$(RUNTIME_TARGET)
	sha256sum dist/pocketpilot-agent-server-$(RUNTIME_TARGET) > dist/pocketpilot-agent-server-$(RUNTIME_TARGET).sha256
