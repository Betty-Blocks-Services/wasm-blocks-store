build:
	#!/usr/bin/env bash
	set -euo pipefail
	if [ -z "$(just index)" ]; then
		echo "No functions in functions/ yet — nothing to build."
		exit 0
	fi
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
	#!/usr/bin/env bash
	set -euo pipefail
	if [ -z "$(just index)" ]; then
		echo "No functions in functions/ yet — nothing to test."
		exit 0
	fi
	cargo test

format:
	#!/usr/bin/env bash
	set -euo pipefail
	if [ -z "$(just index)" ]; then
		echo "No functions in functions/ yet — nothing to format."
		exit 0
	fi
	cargo fmt

format-check:
	#!/usr/bin/env bash
	set -euo pipefail
	if [ -z "$(just index)" ]; then
		echo "No functions in functions/ yet — nothing to format-check."
		exit 0
	fi
	cargo fmt --check

quality-check:
	#!/usr/bin/env bash
	set -euo pipefail
	if [ -z "$(just index)" ]; then
		echo "No functions in functions/ yet — nothing to quality-check."
		exit 0
	fi
	cargo clippy --all-targets -- -D warnings

clean:
	cargo clean

index:
	find functions -type f -name "Cargo.toml" -exec dirname {} \;
