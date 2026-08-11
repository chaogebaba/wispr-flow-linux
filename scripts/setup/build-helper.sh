#!/usr/bin/env bash
#===============================================================================
# build-helper.sh -- build the bundled clean-room Rust helper for a target arch.
#
# Usage: build-helper.sh <arch>
#   <arch>  x86_64 | aarch64 | amd64 | arm64 | x64
#
# Prints the absolute path to the built binary on stdout. Cargo diagnostics go to
# stderr. CARGO_TARGET_DIR may override helper/target; a relative value is
# resolved from the repository root.
#===============================================================================
set -uo pipefail

log() { printf '%s\n' "$*" >&2; }
die() { printf 'build-helper: %s\n' "$*" >&2; exit 1; }

[[ $# -eq 1 ]] || die 'usage: build-helper.sh <arch>'

case "$1" in
	x86_64|amd64|x64) target='x86_64-unknown-linux-gnu' ;;
	aarch64|arm64)    target='aarch64-unknown-linux-gnu' ;;
	*) die "unsupported arch: $1 (want x86_64|aarch64|amd64|arm64|x64)" ;;
esac

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/../.." && pwd)"
manifest="$repo_root/helper/Cargo.toml"
cargo_cmd="${CARGO:-cargo}"
target_dir="${CARGO_TARGET_DIR:-$repo_root/helper/target}"

[[ -f $manifest ]] || die "helper manifest not found: $manifest"
command -v "$cargo_cmd" >/dev/null 2>&1 \
	|| die "cargo command not found: $cargo_cmd"

if [[ $target_dir != /* ]]; then
	target_dir="$repo_root/$target_dir"
fi

log "Building bundled helper for ${target} ..."
"$cargo_cmd" build --locked --release --manifest-path "$manifest" \
	--target "$target" --target-dir "$target_dir" >&2 \
	|| die "cargo build failed for ${target}"

helper_bin="$target_dir/$target/release/wispr-flow-linux-helper"
[[ -x $helper_bin ]] || die "built helper is not executable: $helper_bin"

printf '%s\n' "$helper_bin"
