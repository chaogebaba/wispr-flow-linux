#!/usr/bin/env bats
# Tests the bundled helper build wrapper without compiling Rust.

setup() {
	TEST_ROOT=$(mktemp -d)
	export TEST_ROOT
	mkdir -p "$TEST_ROOT/scripts/setup" "$TEST_ROOT/helper"
	cp "$BATS_TEST_DIRNAME/../scripts/setup/build-helper.sh" \
		"$TEST_ROOT/scripts/setup/build-helper.sh"
	cp "$BATS_TEST_DIRNAME/../scripts/build-linux.sh" \
		"$TEST_ROOT/scripts/build-linux.sh"
	printf '%s\n' '[package]' 'name = "wispr-flow-linux-helper"' \
		'version = "0.0.0"' > "$TEST_ROOT/helper/Cargo.toml"
	: > "$TEST_ROOT/helper/Cargo.lock"

	cat > "$TEST_ROOT/mock-cargo" <<'SH'
#!/usr/bin/env bash
target=''
target_dir=''
printf '%s\n' "$@" > "$MOCK_CARGO_ARGS"
while [[ $# -gt 0 ]]; do
	case "$1" in
		--target) target="$2"; shift 2 ;;
		--target-dir) target_dir="$2"; shift 2 ;;
		*) shift ;;
	esac
done
mkdir -p "$target_dir/$target/release"
printf '#!/usr/bin/env bash\nexit 0\n' \
	> "$target_dir/$target/release/wispr-flow-linux-helper"
chmod +x "$target_dir/$target/release/wispr-flow-linux-helper"
SH
	chmod +x "$TEST_ROOT/mock-cargo"
	export MOCK_CARGO_ARGS="$TEST_ROOT/cargo-args"
}

teardown() {
	rm -rf "$TEST_ROOT"
}

make_elf() {
	local path="$1"
	local machine="$2"
	python3 - "$path" "$machine" <<'PY'
from pathlib import Path
import sys

payload = bytearray(64)
payload[:4] = b'\x7fELF'
payload[18:20] = int(sys.argv[2]).to_bytes(2, 'little')
Path(sys.argv[1]).write_bytes(payload)
PY
	chmod +x "$path"
}

@test "helper build: maps x64 and produces an executable" {
	run env CARGO="$TEST_ROOT/mock-cargo" CARGO_TARGET_DIR='cargo-out' \
		MOCK_CARGO_ARGS="$MOCK_CARGO_ARGS" \
		bash "$TEST_ROOT/scripts/setup/build-helper.sh" x64

	[[ $status -eq 0 ]]
	local helper_bin="$TEST_ROOT/cargo-out/x86_64-unknown-linux-gnu/"
	helper_bin+='release/wispr-flow-linux-helper'
	[[ -x $helper_bin ]]
	[[ $output == *"$helper_bin"* ]]
	grep -Fxq -- '--locked' "$MOCK_CARGO_ARGS"
	grep -Fxq -- 'x86_64-unknown-linux-gnu' "$MOCK_CARGO_ARGS"
}

@test "helper build: maps arm64 to the GNU Rust target" {
	run env CARGO="$TEST_ROOT/mock-cargo" \
		CARGO_TARGET_DIR="$TEST_ROOT/cargo-out" \
		MOCK_CARGO_ARGS="$MOCK_CARGO_ARGS" \
		bash "$TEST_ROOT/scripts/setup/build-helper.sh" arm64

	[[ $status -eq 0 ]]
	local helper_bin="$TEST_ROOT/cargo-out/aarch64-unknown-linux-gnu/"
	helper_bin+='release/wispr-flow-linux-helper'
	[[ -x $helper_bin ]]
	grep -Fxq -- 'aarch64-unknown-linux-gnu' "$MOCK_CARGO_ARGS"
}

@test "helper build: rejects an unsupported architecture" {
	run bash "$TEST_ROOT/scripts/setup/build-helper.sh" riscv64

	[[ $status -ne 0 ]]
	[[ $output == *'unsupported arch'* ]]
	[[ ! -e $MOCK_CARGO_ARGS ]]
}

@test "helper validation: accepts matching x86_64 ELF" {
	local helper_bin="$TEST_ROOT/helper-x86_64"
	make_elf "$helper_bin" 62

	run env HELPER_BIN="$helper_bin" ARCH=x64 bash -c \
		'source "$1"; validate_helper_bin' _ \
		"$TEST_ROOT/scripts/build-linux.sh"

	[[ $status -eq 0 ]]
}

@test "helper validation: rejects a wrong-architecture ELF" {
	local helper_bin="$TEST_ROOT/helper-aarch64"
	make_elf "$helper_bin" 183

	run env HELPER_BIN="$helper_bin" ARCH=x64 bash -c \
		'source "$1"; validate_helper_bin' _ \
		"$TEST_ROOT/scripts/build-linux.sh"

	[[ $status -ne 0 ]]
	[[ $output == *'wrong architecture'* ]]
}

@test "helper validation: rejects a non-ELF executable" {
	local helper_bin="$TEST_ROOT/helper-script"
	printf '#!/usr/bin/env bash\nexit 0\n' > "$helper_bin"
	chmod +x "$helper_bin"

	run env HELPER_BIN="$helper_bin" ARCH=x64 bash -c \
		'source "$1"; validate_helper_bin' _ \
		"$TEST_ROOT/scripts/build-linux.sh"

	[[ $status -ne 0 ]]
	[[ $output == *'not an ELF executable'* ]]
}
