#!/usr/bin/env bats
#
# linux-patches.bats
# Unit tests for the four renderer/main bundle patches added for the Linux port:
#   * linux-renderer-chrome.sh           -> remaps the <html> platform class linux->win32
#   * linux-window-frame.sh              -> frameless hub/settings window on Linux
#   * linux-renderer-treat-as-windows.sh -> widens each renderer's isWindows bind
#                                           (bridge stays honest; no preload touched)
#   * linux-deeplink.sh                  -> cold-start wispr-flow: argv parse on Linux
#   * linux-hide-status-window.sh        -> keeps the floating status bar hidden
#                                           unless explicitly restored
#
# The real bundle is the proprietary, gitignored app -- not available in CI -- so
# each test drives a hermetic minified-JS FIXTURE carrying the exact anchor the
# patch keys on. Every patch is asserted to: apply (marker + transformation),
# leave unrelated sites alone, produce parseable JS (node --check, skipped if
# node is absent), be idempotent (second run is a no-op, byte-identical), and
# bail non-zero on a fixture whose anchor is absent (never silently no-op).
#

SCRIPT_DIR="$(cd "$(dirname "${BATS_TEST_FILENAME}")" && pwd)"
PATCH_DIR="$SCRIPT_DIR/../scripts/patches"

setup() {
	TEST_TMP=$(mktemp -d)
	export TEST_TMP
	FIX="$TEST_TMP/bundle.js"
	export FIX
}

teardown() {
	if [[ -n "${TEST_TMP:-}" && -d "$TEST_TMP" ]]; then
		rm -rf "$TEST_TMP"
	fi
}

# node --check the fixture, but only if node is installed (it is in the build
# env; a bare bats runner may lack it).
node_check() {
	if command -v node >/dev/null; then
		node --check "$1"
	fi
}

# Assert a second run is a no-op and the file is byte-identical to the first run.
assert_idempotent() {
	local script="$1" target="$2" before after
	before=$(md5sum "$target" | cut -d' ' -f1)
	run bash "$script" "$target"
	[[ "$status" -eq 0 ]]
	[[ "$output" == *lready\ patched* ]]
	after=$(md5sum "$target" | cut -d' ' -f1)
	[[ "$before" == "$after" ]]
}

# =============================================================================
# linux-renderer-chrome.sh
# =============================================================================

@test "chrome: remaps every classList.add(...platform.os) site, leaves others" {
	cat > "$FIX" <<'JS'
document.documentElement.classList.add(window.electron.platform.os);
function f(el){el.classList.add(window.electron.platform.os)}
requestAnimationFrame(()=>x.classList.add(Yw.animated));
JS
	run bash "$PATCH_DIR/linux-renderer-chrome.sh" "$FIX"
	[[ "$status" -eq 0 ]]
	grep -q 'WISPR_LINUX_WIN32_CHROME' "$FIX"
	# both platform.os sites became a linux->win32 ternary
	[[ "$(grep -c '"linux"===window.electron.platform.os?"win32"' "$FIX")" -eq 2 ]]
	# the unrelated animated site is untouched
	grep -qF 'classList.add(Yw.animated)' "$FIX"
	node_check "$FIX"
}

@test "chrome: idempotent on second run" {
	cat > "$FIX" <<'JS'
document.documentElement.classList.add(window.electron.platform.os);
JS
	bash "$PATCH_DIR/linux-renderer-chrome.sh" "$FIX"
	assert_idempotent "$PATCH_DIR/linux-renderer-chrome.sh" "$FIX"
}

@test "chrome: bails non-zero when the anchor is absent" {
	cat > "$FIX" <<'JS'
requestAnimationFrame(()=>x.classList.add(Yw.animated));
JS
	run bash "$PATCH_DIR/linux-renderer-chrome.sh" "$FIX"
	[[ "$status" -ne 0 ]]
	! grep -q 'WISPR_LINUX_WIN32_CHROME' "$FIX"
}

# =============================================================================
# linux-window-frame.sh
# =============================================================================

@test "window-frame: widens the win32 hidden-titlebar predicate to include linux" {
	# The 1.5.695 mac branch dropped "hiddenInset" -- it now sets
	# {frame:!1,titleBarStyle:"hidden",trafficLightPosition,...}. The anchor no
	# longer keys on the mac branch, only on the win32 predicate + hidden assign.
	cat > "$FIX" <<'JS'
var s={tD:false},t={};
s.tD?Object.assign(t,{frame:!1,titleBarStyle:"hidden",trafficLightPosition:{x:1e4,y:10},transparent:!0,hasShadow:!0}):"win32"===process.platform&&Object.assign(t,{titleBarStyle:"hidden",autoHideMenuBar:!0});
JS
	run bash "$PATCH_DIR/linux-window-frame.sh" "$FIX"
	[[ "$status" -eq 0 ]]
	grep -q 'WISPR_LINUX_FRAMELESS' "$FIX"
	grep -qF '"win32"===process.platform||"linux"===process.platform' "$FIX"
	# the mac branch is left untouched
	grep -qF 'trafficLightPosition:{x:1e4,y:10}' "$FIX"
	node_check "$FIX"
}

@test "window-frame: idempotent on second run" {
	cat > "$FIX" <<'JS'
var s={tD:false},t={};
s.tD?Object.assign(t,{frame:!1,titleBarStyle:"hidden",trafficLightPosition:{x:1e4,y:10},transparent:!0,hasShadow:!0}):"win32"===process.platform&&Object.assign(t,{titleBarStyle:"hidden",autoHideMenuBar:!0});
JS
	bash "$PATCH_DIR/linux-window-frame.sh" "$FIX"
	assert_idempotent "$PATCH_DIR/linux-window-frame.sh" "$FIX"
}

@test "window-frame: bails non-zero when no matching window config exists" {
	cat > "$FIX" <<'JS'
var t={};Object.assign(t,{titleBarStyle:"default"});
JS
	run bash "$PATCH_DIR/linux-window-frame.sh" "$FIX"
	[[ "$status" -ne 0 ]]
	! grep -q 'WISPR_LINUX_FRAMELESS' "$FIX"
}

# =============================================================================
# linux-renderer-treat-as-windows.sh
# =============================================================================

@test "treat-as-windows: widens the isWindows bind to include linux, honest bridge" {
	cat > "$FIX" <<'JS'
const y="undefined"!=typeof window?window.electron:void 0,$=y?.platform?.isMacOS??!1,x=y?.platform?.isWindows??!1,k="2025-03-01";
const na=x?Yi:Li,delay=x?500:100;
JS
	run bash "$PATCH_DIR/linux-renderer-treat-as-windows.sh" "$FIX"
	[[ "$status" -eq 0 ]]
	grep -q 'WISPR_LINUX_RENDERER_ISWIN' "$FIX"
	# the bind is widened, reusing the SAME window.electron local (y) for the OS check
	grep -qF 'x=((y?.platform?.isWindows??!1)||"linux"===y?.platform?.os)/*WISPR_LINUX_RENDERER_ISWIN*/' "$FIX"
	# isMacOS bind is untouched; the bridge property name itself is never flipped
	grep -qF '$=y?.platform?.isMacOS??!1' "$FIX"
	# downstream consumers (na, delay) are left exactly as-is -- they ride on x
	grep -qF 'na=x?Yi:Li' "$FIX"
	grep -qF 'delay=x?500:100' "$FIX"
	node_check "$FIX"
}

@test "treat-as-windows: idempotent on second run" {
	cat > "$FIX" <<'JS'
const y=window.electron,$=y?.platform?.isMacOS??!1,x=y?.platform?.isWindows??!1;
JS
	bash "$PATCH_DIR/linux-renderer-treat-as-windows.sh" "$FIX"
	assert_idempotent "$PATCH_DIR/linux-renderer-treat-as-windows.sh" "$FIX"
}

@test "treat-as-windows: bails non-zero when the renderer has no isWindows bind" {
	cat > "$FIX" <<'JS'
const y=window.electron,$=y?.platform?.isMacOS??!1;
JS
	run bash "$PATCH_DIR/linux-renderer-treat-as-windows.sh" "$FIX"
	[[ "$status" -ne 0 ]]
	! grep -q 'WISPR_LINUX_RENDERER_ISWIN' "$FIX"
}

# =============================================================================
# linux-deeplink.sh
# =============================================================================

@test "deeplink: widens the cold-start win32 argv guard to include linux" {
	cat > "$FIX" <<'JS'
function L(x){}function B(x){return x}
if(f.H8){const e=B(process.argv.find(e=>e.startsWith("wispr-flow:")||e.startsWith("wispr-flow/")));e&&L(e)}
JS
	run bash "$PATCH_DIR/linux-deeplink.sh" "$FIX"
	[[ "$status" -eq 0 ]]
	grep -q 'WISPR_LINUX_DEEPLINK' "$FIX"
	grep -qF 'if(f.H8||"linux"===process.platform){' "$FIX"
	node_check "$FIX"
}

@test "deeplink: idempotent on second run" {
	cat > "$FIX" <<'JS'
function L(x){}function B(x){return x}
if(f.H8){const e=B(process.argv.find(e=>e.startsWith("wispr-flow:")||e.startsWith("wispr-flow/")));e&&L(e)}
JS
	bash "$PATCH_DIR/linux-deeplink.sh" "$FIX"
	assert_idempotent "$PATCH_DIR/linux-deeplink.sh" "$FIX"
}

@test "deeplink: leaves the cross-platform second-instance handler untouched" {
	# second-instance scans r.find(...), NOT process.argv.find(...) -- the anchor
	# must not match it, so the patch must bail (0 cold-start guards present).
	cat > "$FIX" <<'JS'
function L(x){}function B(x){return x}
app.on("second-instance",(e,r)=>{if(f.H8){const u=B(r.find(e=>e.startsWith("wispr-flow:")));u&&L(u)}});
JS
	run bash "$PATCH_DIR/linux-deeplink.sh" "$FIX"
	[[ "$status" -ne 0 ]]
	! grep -q 'WISPR_LINUX_DEEPLINK' "$FIX"
}

# =============================================================================
# linux-hide-status-window.sh
# =============================================================================

write_status_fixture() {
	cat > "$FIX" <<'JS'
const events=[];
class BrowserWindow {
	constructor(options){this.options=options}
	show(){events.push("show")}
	showInactive(){events.push("showInactive")}
	hide(){events.push("hide")}
	isDestroyed(){return false}
}
const i={BrowserWindow};
const n=new i.BrowserWindow({show:!1,title:"Flow Status Indicator",webPreferences:{backgroundThrottling:!1},focusable:!1});let r;
const D={RA:{statusWindow:n}},a=()=>({error(){},info(){}}),q=()=>n;
let P,W,polls=0;
const pollMonitor=()=>{polls++;return 1},pollAlpha=()=>{polls++};
const showStatus=()=>{const e=(D.RA.statusWindow&&!D.RA.statusWindow.isDestroyed()||(a().error("Tried to show/hide status window, but it was destroyed. Rebuilding."),D.RA.statusWindow=q()),D.RA.statusWindow);Array.from([P,W]).forEach(()=>{}),P=pollMonitor(),pollAlpha(e),e.showInactive(),a().info("Showing status window")};
showStatus();n.showInactive();n.show();n.showInactive();
console.log(events.join("|")+";polls="+polls);
JS
}

@test "status-window: disables every show method on Linux and keeps the opt-in" {
	write_status_fixture
	run bash "$PATCH_DIR/linux-hide-status-window.sh" "$FIX"
	[[ "$status" -eq 0 ]]
	grep -q 'WISPR_LINUX_HIDE_STATUS_WINDOW' "$FIX"
	grep -qF 'process.env.WISPR_SHOW_STATUS_WINDOW' "$FIX"
	grep -qF 'n.show=()=>n.hide()' "$FIX"
	grep -qF 'n.showInactive=()=>n.hide()' "$FIX"
	node_check "$FIX"

	if command -v node >/dev/null; then
		run env -u WISPR_SHOW_STATUS_WINDOW node "$FIX"
		[[ "$status" -eq 0 ]]
		[[ "$output" == 'hide|hide|hide|hide|hide;polls=0' ]]

		run env WISPR_SHOW_STATUS_WINDOW=1 node "$FIX"
		[[ "$status" -eq 0 ]]
		[[ "$output" == 'showInactive|showInactive|show|showInactive;polls=2' ]]
	fi
}

@test "status-window: leaves unrelated BrowserWindows untouched" {
	write_status_fixture
	cat >> "$FIX" <<'JS'
const x=new i.BrowserWindow({show:!1,title:"Settings"});
x.showInactive();
JS
	run bash "$PATCH_DIR/linux-hide-status-window.sh" "$FIX"
	[[ "$status" -eq 0 ]]
	grep -qF 'x.showInactive();' "$FIX"
	! grep -qF 'x.showInactive=()=>x.hide()' "$FIX"
	node_check "$FIX"
}

@test "status-window: refuses an upstream constructor that may map immediately" {
	cat > "$FIX" <<'JS'
class BrowserWindow {hide(){}}
const i={BrowserWindow};
const n=new i.BrowserWindow({show:!0,title:"Flow Status Indicator",webPreferences:{backgroundThrottling:!1}});let r;
JS
	run bash "$PATCH_DIR/linux-hide-status-window.sh" "$FIX"
	[[ "$status" -ne 0 ]]
	[[ "$output" == *'missing `show:!1`'* ]]
	! grep -q 'WISPR_LINUX_HIDE_STATUS_WINDOW' "$FIX"
}

@test "status-window: rejects a member-expression constructor assignment" {
	cat > "$FIX" <<'JS'
class BrowserWindow {hide(){}}
const i={BrowserWindow},state={};
state.statusWindow=new i.BrowserWindow({show:!1,title:"Flow Status Indicator",webPreferences:{backgroundThrottling:!1}});let r;
JS
	run bash "$PATCH_DIR/linux-hide-status-window.sh" "$FIX"
	[[ "$status" -ne 0 ]]
	[[ "$output" == *'BrowserWindow constructor'* ]]
	! grep -q 'WISPR_LINUX_HIDE_STATUS_WINDOW' "$FIX"
}

@test "status-window: idempotent on second run" {
	write_status_fixture
	bash "$PATCH_DIR/linux-hide-status-window.sh" "$FIX"
	assert_idempotent "$PATCH_DIR/linux-hide-status-window.sh" "$FIX"
}

@test "status-window: bails non-zero when the show anchor is absent" {
	cat > "$FIX" <<'JS'
const e={show(){}};
e.show();
JS
	run bash "$PATCH_DIR/linux-hide-status-window.sh" "$FIX"
	[[ "$status" -ne 0 ]]
	! grep -q 'WISPR_LINUX_HIDE_STATUS_WINDOW' "$FIX"
}
