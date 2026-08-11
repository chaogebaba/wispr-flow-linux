#!/usr/bin/env bash
#===============================================================================
# fetch-helper-bin.sh -- download the pinned prebuilt clean-room helper binary
# for a target arch and print its path (for the HELPER_BIN env var build.sh
# consumes).
#
# The helper source, release tag, and binary checksums are pinned in
# helper-source.txt, helper-version.txt, and helper-checksums.txt. Each release
# ships one binary per arch:
#   wispr-flow-linux-helper-x86_64    (amd64)
#   wispr-flow-linux-helper-aarch64   (arm64)
#
# This stages the matching asset into <dest>/wispr-flow-linux-helper, marks it
# executable, stamps the source, tag, and verified digest into the destination
# (so the staging engine can refetch when any pin moves past a cached copy), and
# prints that absolute path to stdout (diagnostics go to stderr).
# build-linux.sh invokes this automatically when HELPER_BIN is unset.
#
# Usage:   fetch-helper-bin.sh <arch> [dest_dir]
#   <arch>      x86_64 | aarch64 | amd64 | arm64
#   dest_dir    where to place the binary (default: <repo>/helper-bin)
#
# Requires: gh (authenticated) OR curl. Exit 0 on success.
#===============================================================================
set -uo pipefail

log() { printf '%s\n' "$*" >&2; }
die() { printf 'fetch-helper-bin: %s\n' "$*" >&2; exit 1; }

[[ $# -ge 1 ]] || die 'usage: fetch-helper-bin.sh <arch> [dest_dir]'

# Normalize arch aliases to the asset suffix the helper release uses.
case "$1" in
	x86_64|amd64|x64) asset_arch='x86_64' ;;
	aarch64|arm64)    asset_arch='aarch64' ;;
	*) die "unsupported arch: $1 (want x86_64|aarch64|amd64|arm64)" ;;
esac

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/../.." && pwd)"
dest_dir="${2:-$repo_root/helper-bin}"

source_file="$repo_root/helper-source.txt"
version_file="$repo_root/helper-version.txt"
checksum_file="$repo_root/helper-checksums.txt"
[[ -f $source_file ]] || die "helper-source.txt not found at ${source_file}"
[[ -f $version_file ]] || die "helper-version.txt not found at ${version_file}"
[[ -f $checksum_file ]] \
	|| die "helper-checksums.txt not found at ${checksum_file}"
helper_repo="${HELPER_REPO:-$(tr -d '[:space:]' < "$source_file")}"
tag="$(tr -d '[:space:]' < "$version_file")"
[[ -n $helper_repo ]] || die 'helper source is empty'
[[ -n $tag ]] || die 'helper-version.txt is empty'

asset="wispr-flow-linux-helper-${asset_arch}"
dest_bin="$dest_dir/wispr-flow-linux-helper"

mkdir -p "$dest_dir" || die "cannot create ${dest_dir}"

log "Fetching ${asset} from ${helper_repo}@${tag} ..."

url="https://github.com/${helper_repo}/releases/download/${tag}/${asset}"
fetched=false

# Prefer gh (honors auth/rate limits); fall back to curl on absence OR failure
# (github.token may lack cross-repo scope, but the release is public).
if command -v gh >/dev/null 2>&1; then
	tmp_dir="$(mktemp -d)"
	if gh release download "$tag" --repo "$helper_repo" \
		--pattern "$asset" --dir "$tmp_dir" --clobber >&2; then
		mv "$tmp_dir/$asset" "$dest_bin" || die 'failed to move downloaded helper'
		fetched=true
	else
		log "gh release download failed; falling back to curl"
	fi
	rm -rf "$tmp_dir"
fi

if [[ $fetched == false ]]; then
	command -v curl >/dev/null 2>&1 \
		|| die 'gh unavailable/failed and curl is not installed'
	curl -fSL -o "$dest_bin" "$url" || die "curl download failed: ${url}"
fi

[[ -s $dest_bin ]] || die "downloaded helper is empty: ${dest_bin}"
checksum_line="$(grep -E "^[[:xdigit:]]{64}[[:space:]]+${asset}$" \
	"$checksum_file")" \
	|| die "no checksum pinned for ${asset}"
expected_checksum="${checksum_line%%[[:space:]]*}"
actual_checksum="$(sha256sum "$dest_bin")" \
	|| die "cannot checksum ${dest_bin}"
actual_checksum="${actual_checksum%%[[:space:]]*}"
if [[ $actual_checksum != "$expected_checksum" ]]; then
	rm -f "$dest_bin"
	die "checksum mismatch for ${asset}: expected ${expected_checksum}, got ${actual_checksum}"
fi
log "Verified SHA-256: ${actual_checksum}"
chmod 0755 "$dest_bin" || die "cannot chmod ${dest_bin}"

# Stamp every pin used for this binary. resolve_helper_bin (build-linux.sh)
# compares them and re-verifies the binary digest before reusing a cached fetch.
printf '%s\n' "$helper_repo" > "$dest_dir/.source" \
	|| die "cannot write ${dest_dir}/.source"
printf '%s\n' "$tag" > "$dest_dir/.tag" \
	|| die "cannot write ${dest_dir}/.tag"
printf '%s\n' "$actual_checksum" > "$dest_dir/.sha256" \
	|| die "cannot write ${dest_dir}/.sha256"
log "Staged helper at ${dest_bin}"
printf '%s\n' "$dest_bin"
