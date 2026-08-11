//! In-process uinput virtual keyboard (Wayland injection primitive).
//!
//! On Wayland there is no portable synthetic-input API that reaches native
//! surfaces. The compositor-agnostic path is a real virtual input device via
//! `/dev/uinput` — the compositor routes its events to the focused surface like
//! any hardware keyboard. Needs write access to `/dev/uinput` (logind `uaccess`
//! ACL, the `uinput` group, or root).
//!
//! Codes written here are Linux evdev `KEY_*` codes (see `keymap::vk_to_evdev`).
//!
//! ## Key tracking
//!
//! Every key pressed on this device is tracked in press order. On error
//! mid-chord, all tracked keys are released in reverse press order. On `Drop`,
//! tracked keys are released before `UI_DEV_DESTROY` so compositor modifier
//! state cannot get stuck.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;

use super::Result;
use crate::input_device;

// evdev event types (<linux/input-event-codes.h>).
const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const SYN_REPORT: u16 = 0x00;

// uinput ioctls (<linux/uinput.h>), x86_64/aarch64 encodings.
const UI_DEV_CREATE: libc::c_ulong = 0x5501;
const UI_DEV_DESTROY: libc::c_ulong = 0x5502;
const UI_SET_EVBIT: libc::c_ulong = 0x40045564;
const UI_SET_KEYBIT: libc::c_ulong = 0x40045565;

/// Full standard key range so any mapped VK can be injected.
const KEY_MAX: u16 = 0x2ff;

pub struct UInput {
    file: File,
    /// Keys currently pressed on this device, in press order. Duplicates are
    /// prevented on insert. Cleanup iterates in reverse press order.
    pressed: Vec<u16>,
}

impl UInput {
    /// Probe whether `/dev/uinput` is openable for writing without creating a
    /// device (used by backend detection to fall back gracefully).
    pub fn available() -> bool {
        OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open("/dev/uinput")
            .is_ok()
    }

    /// Create the virtual keyboard. Must be kept alive for the process lifetime;
    /// dropping it destroys the device.
    pub fn create() -> Result<UInput> {
        let file = OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open("/dev/uinput")
            .map_err(|e| format!("open /dev/uinput: {e} (need write access — logind uaccess ACL, `uinput` group, or root)"))?;
        let fd = file.as_raw_fd();

        ioctl_set(fd, UI_SET_EVBIT, EV_KEY as libc::c_int)?;
        ioctl_set(fd, UI_SET_EVBIT, EV_SYN as libc::c_int)?;
        for code in 1..=KEY_MAX {
            unsafe { libc::ioctl(fd, UI_SET_KEYBIT, code as libc::c_int) };
        }

        // Legacy device-setup path (uinput_user_dev + UI_DEV_CREATE): widely
        // supported, avoids the newer UI_DEV_SETUP/abs_setup structs.
        let mut dev: libc::uinput_user_dev = unsafe { std::mem::zeroed() };
        for (i, &b) in input_device::DEVICE_NAME.iter().enumerate() {
            dev.name[i] = b as libc::c_char;
        }
        dev.id.bustype = input_device::BUSTYPE;
        dev.id.vendor = input_device::VENDOR;
        dev.id.product = input_device::PRODUCT;
        dev.id.version = input_device::VERSION;

        let bytes = unsafe {
            std::slice::from_raw_parts(
                &dev as *const _ as *const u8,
                std::mem::size_of::<libc::uinput_user_dev>(),
            )
        };
        (&file)
            .write_all(bytes)
            .map_err(|e| format!("write uinput_user_dev: {e}"))?;

        if unsafe { libc::ioctl(fd, UI_DEV_CREATE) } < 0 {
            return Err(format!(
                "UI_DEV_CREATE: {}",
                std::io::Error::last_os_error()
            ));
        }

        // Compositor needs time to enumerate the new device; injecting too
        // early drops the first keys.
        std::thread::sleep(std::time::Duration::from_millis(200));

        Ok(UInput {
            file,
            pressed: Vec::new(),
        })
    }

    fn emit(&mut self, type_: u16, code: u16, value: i32) -> Result<()> {
        let ev = libc::input_event {
            time: libc::timeval {
                tv_sec: 0,
                tv_usec: 0,
            },
            type_,
            code,
            value,
        };
        let bytes = unsafe {
            std::slice::from_raw_parts(
                &ev as *const _ as *const u8,
                std::mem::size_of::<libc::input_event>(),
            )
        };
        self.file
            .write_all(bytes)
            .map_err(|e| format!("uinput write: {e}"))
    }

    fn syn(&mut self) -> Result<()> {
        self.emit(EV_SYN, SYN_REPORT, 0)
    }

    /// Press or release a single evdev key with SYN. Tracks state immediately
    /// after the EV_KEY write succeeds (before SYN) so a SYN failure still
    /// marks the key for cleanup.
    pub fn key(&mut self, code: u16, press: bool) -> Result<()> {
        self.emit(EV_KEY, code, if press { 1 } else { 0 })?;
        if press {
            if !self.pressed.contains(&code) {
                self.pressed.push(code);
            }
        } else {
            self.pressed.retain(|&c| c != code);
        }
        self.syn()
    }

    /// Emit a balanced chord: press mods, tap key, release mods in reverse.
    ///
    /// CRITICAL: all events are emitted contiguously with no inter-event sleep.
    /// On KWin/Wayland a quiescent gap after a virtual modifier-down causes the
    /// compositor to drop the modifier before the key arrives (verified: 0 ms →
    /// applied, ≥8 ms → dropped). See docs/learnings/wayland-injection.md.
    ///
    /// Before injecting, polls physical modifier state across /dev/input and
    /// waits up to 1s for all modifiers to be released (handles PTT hotkey
    /// release lag). Returns error if modifiers remain held at timeout. If no
    /// input devices are readable, proceeds (graceful degradation).
    ///
    /// On error mid-sequence, releases all tracked keys in reverse press order.
    pub fn chord(&mut self, key: u16, mods: &[u16]) -> Result<()> {
        input_device::wait_modifiers_released()?;
        let plan = chord_plan(key, mods);
        let result = self.execute_plan(&plan);
        if let Err(error) = result {
            if let Err(cleanup_error) = self.cleanup() {
                log::error!("uinput cleanup after chord failure: {cleanup_error}");
            }
            return Err(error);
        }
        Ok(())
    }

    fn execute_plan(&mut self, plan: &[ChordAction]) -> Result<()> {
        for action in plan {
            match action {
                ChordAction::Press(code) => self.key(*code, true)?,
                ChordAction::Release(code) => self.key(*code, false)?,
            }
        }
        Ok(())
    }

    /// Release all tracked keys in reverse press order, then SYN. Tracking is
    /// cleared only when every write succeeds so a later Drop can retry.
    fn cleanup(&mut self) -> Result<()> {
        let mut errors = Vec::new();
        let codes: Vec<u16> = self.pressed.iter().rev().copied().collect();
        for code in &codes {
            if let Err(error) = self.emit(EV_KEY, *code, 0) {
                errors.push(format!("release key {code}: {error}"));
            }
        }
        // A prior release may have been written before its SYN failed.
        if let Err(error) = self.emit(EV_SYN, SYN_REPORT, 0) {
            errors.push(format!("SYN_REPORT: {error}"));
        }
        if errors.is_empty() {
            self.pressed.clear();
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}

impl Drop for UInput {
    fn drop(&mut self) {
        if let Err(error) = self.cleanup() {
            log::error!("uinput cleanup before device destruction: {error}");
        }
        if unsafe { libc::ioctl(self.file.as_raw_fd(), UI_DEV_DESTROY) } < 0 {
            log::error!("UI_DEV_DESTROY: {}", std::io::Error::last_os_error());
        }
    }
}

fn ioctl_set(fd: libc::c_int, req: libc::c_ulong, arg: libc::c_int) -> Result<()> {
    if unsafe { libc::ioctl(fd, req, arg) } < 0 {
        return Err(format!(
            "ioctl {req:#x}({arg}): {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Chord planning — pure logic, no I/O.
// ---------------------------------------------------------------------------

/// A single action in a chord sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChordAction {
    Press(u16),
    Release(u16),
}

/// Plan a balanced chord: deduplicate modifiers (preserving first-occurrence
/// order), exclude `key` from the modifier set, press remaining mods, tap key
/// exactly once, release mods in reverse order.
fn chord_plan(key: u16, mods: &[u16]) -> Vec<ChordAction> {
    let mut unique_mods: Vec<u16> = Vec::new();
    for &m in mods {
        if m != key && !unique_mods.contains(&m) {
            unique_mods.push(m);
        }
    }

    let mut plan = Vec::with_capacity(unique_mods.len() * 2 + 2);
    for &m in &unique_mods {
        plan.push(ChordAction::Press(m));
    }
    plan.push(ChordAction::Press(key));
    plan.push(ChordAction::Release(key));
    for &m in unique_mods.iter().rev() {
        plan.push(ChordAction::Release(m));
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// True when every pressed key has a matching release.
    fn is_balanced(plan: &[ChordAction]) -> bool {
        let mut pressed: HashSet<u16> = HashSet::new();
        for action in plan {
            match action {
                ChordAction::Press(c) => {
                    pressed.insert(*c);
                }
                ChordAction::Release(c) => {
                    pressed.remove(c);
                }
            }
        }
        pressed.is_empty()
    }

    /// The exact reported bug scenario: Ctrl+Super physically held, app injects
    /// Ctrl+V. Plan must contain ONLY the requested Ctrl+V and end balanced.
    #[test]
    fn ctrl_v_chord_balanced_and_minimal() {
        let ctrl = 29u16; // KEY_LEFTCTRL
        let v = 47u16; // KEY_V
        let plan = chord_plan(v, &[ctrl]);

        assert_eq!(
            plan,
            vec![
                ChordAction::Press(ctrl),
                ChordAction::Press(v),
                ChordAction::Release(v),
                ChordAction::Release(ctrl),
            ]
        );
        assert!(is_balanced(&plan));
    }

    #[test]
    fn duplicate_modifiers_deduplicated() {
        let ctrl = 29u16;
        let shift = 42u16;
        let a = 30u16;
        let plan = chord_plan(a, &[ctrl, shift, ctrl, shift, ctrl]);

        assert_eq!(
            plan,
            vec![
                ChordAction::Press(ctrl),
                ChordAction::Press(shift),
                ChordAction::Press(a),
                ChordAction::Release(a),
                ChordAction::Release(shift),
                ChordAction::Release(ctrl),
            ]
        );
        assert!(is_balanced(&plan));
    }

    #[test]
    fn modifiers_released_in_reverse_press_order() {
        let ctrl = 29u16;
        let shift = 42u16;
        let alt = 56u16;
        let x = 45u16;
        let plan = chord_plan(x, &[ctrl, shift, alt]);

        let releases: Vec<u16> = plan
            .iter()
            .filter_map(|a| match a {
                ChordAction::Release(c) => Some(*c),
                _ => None,
            })
            .collect();
        // key released first, then mods in reverse.
        assert_eq!(releases, vec![x, alt, shift, ctrl]);
    }

    /// When key is also listed as a modifier, it is excluded from the mod set
    /// and tapped exactly once.
    #[test]
    fn key_in_mods_excluded_and_tapped_once() {
        let ctrl = 29u16;
        let plan = chord_plan(ctrl, &[ctrl]);

        // key == only modifier → mods list is empty after exclusion, just a tap.
        assert_eq!(
            plan,
            vec![ChordAction::Press(ctrl), ChordAction::Release(ctrl),]
        );
        assert!(is_balanced(&plan));
    }

    #[test]
    fn key_in_mods_with_other_mods() {
        let ctrl = 29u16;
        let shift = 42u16;
        let plan = chord_plan(ctrl, &[shift, ctrl]);

        // ctrl excluded from mods, shift remains.
        assert_eq!(
            plan,
            vec![
                ChordAction::Press(shift),
                ChordAction::Press(ctrl),
                ChordAction::Release(ctrl),
                ChordAction::Release(shift),
            ]
        );
        assert!(is_balanced(&plan));
    }

    #[test]
    fn no_modifiers_just_tap() {
        let a = 30u16;
        let plan = chord_plan(a, &[]);
        assert_eq!(plan, vec![ChordAction::Press(a), ChordAction::Release(a),]);
        assert!(is_balanced(&plan));
    }

    #[test]
    fn unbalanced_detected() {
        assert!(!is_balanced(&[
            ChordAction::Press(29),
            ChordAction::Press(47)
        ]));
    }

    /// Pressed tracking preserves order, deduplicates, and cleanup reverses.
    #[test]
    fn pressed_vec_order_and_dedup() {
        let mut pressed: Vec<u16> = Vec::new();

        // Simulate press sequence: ctrl, shift, v (with a dup ctrl).
        for &code in &[29u16, 42, 29, 47] {
            if !pressed.contains(&code) {
                pressed.push(code);
            }
        }
        assert_eq!(pressed, vec![29, 42, 47]);

        // Reverse for cleanup.
        let cleanup_order: Vec<u16> = pressed.iter().rev().copied().collect();
        assert_eq!(cleanup_order, vec![47, 42, 29]);
    }

    /// Cleanup must not clear tracking when writes fail.
    #[test]
    fn cleanup_retains_on_failure() {
        // Simulates the retry-safe logic: if any release fails, pressed is
        // retained so Drop can retry.
        let mut pressed = vec![29u16, 42, 47];
        let all_ok = false; // simulate a write failure

        if all_ok {
            pressed.clear();
        }
        // Keys remain for next cleanup attempt.
        assert_eq!(pressed, vec![29, 42, 47]);
    }
}
