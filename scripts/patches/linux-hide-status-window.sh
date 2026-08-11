#!/usr/bin/env bash
#===============================================================================
# linux-hide-status-window.sh -- keep Wispr Flow's status renderer alive without
# mapping its floating BrowserWindow on Linux by default.
#
# The app creates a transparent, always-on-top "Flow Status Indicator" window.
# It already sets skipTaskbar=true, focusable=false, and type="toolbar", but
# native Wayland has no standard skip-taskbar request, so GNOME still exposes
# the surface in Alt+Tab and Overview. The same window also overlays other apps
# and displays hover prompts such as "Speak to ChatGPT".
#
# Preserve the BrowserWindow and renderer because app code sends status IPC to
# them. Immediately after the unique status BrowserWindow constructor, override
# that instance's show() and showInactive() methods on Linux so every current or
# future show path remains hidden. WISPR_SHOW_STATUS_WINDOW=1 keeps the upstream
# methods. macOS and Windows never enter the injected guard.
#
# The stable anchor is the unique developer title "Flow Status Indicator". The
# minified BrowserWindow variable is derived from the nearest constructor rather
# than hardcoded, and the insertion point is the constructor's `);let` boundary.
#
# Usage: linux-hide-status-window.sh <path-to-.webpack/main/index.js>
#===============================================================================
set -uo pipefail

BUNDLE="${1:-}"
if [[ -z $BUNDLE ]]; then
	BUNDLE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
	BUNDLE="$BUNDLE/extract/app/.webpack/main/index.js"
fi

if [[ ! -f $BUNDLE ]]; then
	printf 'ERROR: bundle not found: %s\n' "$BUNDLE" >&2
	exit 1
fi

LINUX_MARKER='WISPR_LINUX_HIDE_STATUS_WINDOW'
if grep -q "$LINUX_MARKER" "$BUNDLE"; then
	printf 'Already patched (%s present in %s) - nothing to do.\n' \
		"$LINUX_MARKER" "$BUNDLE"
	exit 0
fi

if [[ ! -f $BUNDLE.orig ]]; then
	cp -p "$BUNDLE" "$BUNDLE.orig" || {
		printf 'ERROR: could not create backup: %s.orig\n' "$BUNDLE" >&2
		exit 1
	}
	printf 'Backup written: %s.orig\n' "$BUNDLE"
fi

current_backup=$(mktemp) || {
	printf 'ERROR: mktemp failed\n' >&2
	exit 1
}
cp -p "$BUNDLE" "$current_backup" || {
	rm -f "$current_backup"
	printf 'ERROR: could not create temporary patch backup\n' >&2
	exit 1
}
# Expand the temporary path when installing the trap.
# shellcheck disable=SC2064
trap "rm -f '$current_backup'" EXIT

python3 - "$BUNDLE" "$LINUX_MARKER" <<'PY'
import io
import re
import sys

path, marker = sys.argv[1], sys.argv[2]
with io.open(path, "r", encoding="utf-8", errors="surrogateescape") as bundle:
    data = bundle.read()

title = 'title:"Flow Status Indicator"'
if data.count(title) != 1:
    sys.exit(
        f"ERROR: expected exactly 1 Flow Status Indicator title, found "
        f"{data.count(title)}. Re-audit the status BrowserWindow constructor."
    )

title_pos = data.index(title)
search_start = max(0, title_pos - 2000)
prefix = data[search_start:title_pos]
constructor = re.compile(
    r'(?<![\w$.])(?P<window>[\w$]+)=new '
    r'[\w$]+(?:\.[\w$]+)*\.BrowserWindow\(\{'
)
constructors = list(constructor.finditer(prefix))
if len(constructors) != 1:
    sys.exit(
        f"ERROR: expected exactly 1 BrowserWindow constructor within 2000 bytes "
        f"before the status title, found {len(constructors)}."
    )

window = constructors[0].group("window")
close_pos = data.find(');let ', title_pos, title_pos + 2500)
if close_pos < 0:
    sys.exit(
        "ERROR: status BrowserWindow constructor boundary `);let` not found "
        "within 2500 bytes after its title."
    )

constructor_start = search_start + constructors[0].start()
config = data[constructor_start:close_pos + 2]
for required in ('show:!1', 'backgroundThrottling:!1'):
    if required not in config:
        sys.exit(
            f"ERROR: status BrowserWindow is missing `{required}`. Refusing to "
            "hide a window whose renderer/mapping behavior changed upstream."
        )

injection = (
    '"linux"===process.platform&&'
    '"1"!==process.env.WISPR_SHOW_STATUS_WINDOW&&('
    + window
    + '.show=()=>'
    + window
    + '.hide(),'
    + window
    + '.showInactive=()=>'
    + window
    + '.hide(),'
    + window
    + '.hide())/*'
    + marker
    + '*/;'
)
insert_at = close_pos + 2
data = data[:insert_at] + injection + data[insert_at:]

rebuild_log = (
    '"Tried to show/hide status window, but it was destroyed. Rebuilding."'
)
show_site = re.compile(
    r'const (?P<window>[\w$]+)=\('
    r'(?=[^;]{0,700}?' + re.escape(rebuild_log) + r')'
    r'[\w$]+(?:\.[\w$]+)*\.statusWindow'
)
show_matches = list(show_site.finditer(data))
if len(show_matches) != 1:
    sys.exit(
        f"ERROR: expected exactly 1 centralized status-show function, found "
        f"{len(show_matches)}. Re-audit the status polling startup path."
    )
show_window = show_matches[0].group("window")
rebuild_pos = data.index(rebuild_log)
show_boundary = data.find(');Array.from(', rebuild_pos, rebuild_pos + 1200)
if show_boundary < 0:
    sys.exit(
        "ERROR: centralized status-show declaration boundary not found before "
        "its polling setup."
    )
early_return = (
    'if("linux"===process.platform&&'
    '"1"!==process.env.WISPR_SHOW_STATUS_WINDOW)'
    'return void '
    + show_window
    + '.hide();'
)
show_insert_at = show_boundary + 2
data = data[:show_insert_at] + early_return + data[show_insert_at:]

with io.open(path, "w", encoding="utf-8", errors="surrogateescape") as bundle:
    bundle.write(data)

print(
    "Patched: status BrowserWindow show methods and polling disabled on Linux "
    f"by default (window={window!r}; 1 constructor, 1 show function)."
)
PY
patch_status=$?
if ((patch_status != 0)); then
	cp -p "$current_backup" "$BUNDLE"
	exit "$patch_status"
fi

if ! grep -q "$LINUX_MARKER" "$BUNDLE"; then
	printf 'ERROR: post-patch verification failed (marker missing).\n' >&2
	cp -p "$current_backup" "$BUNDLE"
	exit 1
fi

if ! grep -qF \
	'"1"!==process.env.WISPR_SHOW_STATUS_WINDOW&&' "$BUNDLE"; then
	printf 'ERROR: status-window opt-in guard is malformed.\n' >&2
	cp -p "$current_backup" "$BUNDLE"
	exit 1
fi

if command -v node >/dev/null; then
	if ! node --check "$BUNDLE"; then
		printf 'ERROR: node --check failed; restoring pre-patch bytes.\n' >&2
		cp -p "$current_backup" "$BUNDLE"
		exit 1
	fi
	printf 'node --check OK\n'
fi

printf 'OK: Linux status window is hidden by default in %s\n\n' "$BUNDLE"
printf '%s\n' \
	'Set WISPR_SHOW_STATUS_WINDOW=1 before launch to restore the floating bar.'
