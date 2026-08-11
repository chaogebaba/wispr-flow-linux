#!/usr/bin/env bats
# Tests helper release download verification without network access.

setup() {
	TEST_ROOT=$(mktemp -d)
	export TEST_ROOT
	mkdir -p "$TEST_ROOT/scripts/setup" "$TEST_ROOT/mock-bin" "$TEST_ROOT/out"
	cp "$BATS_TEST_DIRNAME/../scripts/setup/fetch-helper-bin.sh" \
		"$TEST_ROOT/scripts/setup/fetch-helper-bin.sh"
	printf '%s\n' 'example/helper' > "$TEST_ROOT/helper-source.txt"
	printf '%s\n' 'v9.9.9' > "$TEST_ROOT/helper-version.txt"

	cat > "$TEST_ROOT/mock-bin/gh" <<'SH'
#!/usr/bin/env bash
asset=''
dest=''
while [[ $# -gt 0 ]]; do
	case "$1" in
		--pattern) asset="$2"; shift 2 ;;
		--dir) dest="$2"; shift 2 ;;
		*) shift ;;
	esac
done
printf '%s' "$MOCK_HELPER_PAYLOAD" > "$dest/$asset"
SH
	chmod +x "$TEST_ROOT/mock-bin/gh"
}

teardown() {
	rm -rf "$TEST_ROOT"
}

@test "helper fetch: verifies and stamps a pinned binary" {
	local payload='verified helper payload'
	local checksum
	checksum="$(printf '%s' "$payload" | sha256sum)"
	checksum="${checksum%% *}"
	printf '%s  %s\n' "$checksum" 'wispr-flow-linux-helper-x86_64' \
		> "$TEST_ROOT/helper-checksums.txt"

	run env PATH="$TEST_ROOT/mock-bin:$PATH" \
		MOCK_HELPER_PAYLOAD="$payload" \
		bash "$TEST_ROOT/scripts/setup/fetch-helper-bin.sh" \
		x86_64 "$TEST_ROOT/out"

	[[ $status -eq 0 ]]
	[[ -x "$TEST_ROOT/out/wispr-flow-linux-helper" ]]
	[[ $(< "$TEST_ROOT/out/.source") == 'example/helper' ]]
	[[ $(< "$TEST_ROOT/out/.tag") == 'v9.9.9' ]]
	[[ $(< "$TEST_ROOT/out/.sha256") == "$checksum" ]]
}

@test "helper fetch: rejects and removes a checksum mismatch" {
	printf '%064d  %s\n' 0 'wispr-flow-linux-helper-x86_64' \
		> "$TEST_ROOT/helper-checksums.txt"

	run env PATH="$TEST_ROOT/mock-bin:$PATH" \
		MOCK_HELPER_PAYLOAD='corrupt helper' \
		bash "$TEST_ROOT/scripts/setup/fetch-helper-bin.sh" \
		x86_64 "$TEST_ROOT/out"

	[[ $status -ne 0 ]]
	[[ $output == *'checksum mismatch'* ]]
	[[ ! -e "$TEST_ROOT/out/wispr-flow-linux-helper" ]]
	[[ ! -e "$TEST_ROOT/out/.tag" ]]
}
