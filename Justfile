build:
	cargo build --release --target wasm32-wasip2
	just distribute-wasm

distribute-wasm:
	#!/usr/bin/env bash
	for directory in $(just index); do
		library_name="$(grep '^name' "$directory/Cargo.toml" | head -1 | sed 's/.*"\(.*\)"/\1/' | tr '-' '_')"
		wasm_path="target/wasm32-wasip2/release/${library_name}.wasm"
		if [ -f "$wasm_path" ]; then
			cp "$wasm_path" "$directory/"
		fi
	done

test: build
	cargo test

format:
	cargo fmt

format-check:
	cargo fmt --check

quality-check:
	cargo clippy --all-targets -- -D warnings

clean:
	cargo clean

index:
	find functions -type f -name "Cargo.toml" -exec dirname {} \;
