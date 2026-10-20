.DEFAULT_GOAL := build


.PHONY: build check package release clean

build:
	cargo build -p app --release --locked

check:
	cargo fmt --all -- --check
	cargo clippy --release --workspace --all-targets --locked -- -D warnings
	cargo test --release --workspace --locked

package: build
	mkdir -p dist
	cp target/release/pocketpilot dist/pocketpilot.new
	mv -f dist/pocketpilot.new dist/pocketpilot
	./dist/pocketpilot --version > dist/BUILD.txt
	rustc -vV >> dist/BUILD.txt
	@printf 'integrations: telegram,discord,slack\n' >> dist/BUILD.txt
	@if command -v sha256sum >/dev/null 2>&1; then sha256sum dist/pocketpilot > dist/pocketpilot.sha256; else shasum -a 256 dist/pocketpilot > dist/pocketpilot.sha256; fi

release:
	$(MAKE) check
	$(MAKE) package
	$(MAKE) clean

clean:
	cargo clean
