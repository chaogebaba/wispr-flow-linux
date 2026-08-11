//! Centralized identity for the helper's virtual keyboard.
//!
//! The helper creates a uinput device with a fixed name/id tuple. Other
//! subsystems (evdev capture, stale-key scans) exclude it via
//! `is_helper_device(fd)` to avoid reading back injected events.
//!
//! Also provides `wait_modifiers_released()`: polls EVIOCGKEY across all
//! readable event devices (excluding our own) until no modifier keys are held,
//! or a timeout expires. Used before chord injection so a physically-held PTT
//! hotkey (e.g. Ctrl+Super) does not corrupt the synthetic chord.
//!
//! Identity is checked with `EVIOCGNAME` + `EVIOCGID` -- no libudev dependency.

use std::fs::{File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::time::{Duration, Instant};

/// Fixed name written to `uinput_user_dev.name`.
pub const DEVICE_NAME: &[u8] = b"Wispr Flow Linux Helper";
pub const BUSTYPE: u16 = 0x03; // BUS_USB
pub const VENDOR: u16 = 0x1234;
pub const PRODUCT: u16 = 0x5678;
pub const VERSION: u16 = 1;

/// Modifier evdev codes we wait on before chord injection.
const MODIFIER_CODES: &[u16] = &[
    29,  // KEY_LEFTCTRL
    97,  // KEY_RIGHTCTRL
    42,  // KEY_LEFTSHIFT
    54,  // KEY_RIGHTSHIFT
    56,  // KEY_LEFTALT
    100, // KEY_RIGHTALT
    125, // KEY_LEFTMETA
    126, // KEY_RIGHTMETA
];

const KEY_MAX: usize = 0x2ff;
const BITMAP_LEN: usize = (KEY_MAX / 8) + 1;

/// Timeout for waiting on modifier release.
const MODIFIER_WAIT_TIMEOUT: Duration = Duration::from_secs(1);
/// Poll interval between EVIOCGKEY scans.
const MODIFIER_POLL_INTERVAL: Duration = Duration::from_millis(8);

// ioctl encodings: _IOC(dir=2, type='E', nr, size).
const fn eviocgname(len: usize) -> libc::c_ulong {
    ((2u64 << 30) | ((len as u64) << 16) | (0x45u64 << 8) | 0x06) as libc::c_ulong
}

const fn eviocgid() -> libc::c_ulong {
    ((2u64 << 30) | (8u64 << 16) | (0x45u64 << 8) | 0x02) as libc::c_ulong
}

const fn eviocgkey() -> libc::c_ulong {
    ((2u64 << 30) | ((BITMAP_LEN as u64) << 16) | (0x45u64 << 8) | 0x18) as libc::c_ulong
}

/// Kernel `struct input_id` layout.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InputId {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
}

/// Pure identity check: does `name` and `id` match our helper device?
fn matches_helper_identity(name: &[u8], id: &InputId) -> bool {
    name == DEVICE_NAME
        && id.bustype == BUSTYPE
        && id.vendor == VENDOR
        && id.product == PRODUCT
        && id.version == VERSION
}

/// Check whether an open evdev fd belongs to our helper virtual keyboard.
pub fn is_helper_device(fd: libc::c_int) -> bool {
    let name = match read_device_name(fd) {
        Some(n) => n,
        None => return false,
    };
    let id = match read_device_id(fd) {
        Some(i) => i,
        None => return false,
    };
    matches_helper_identity(&name, &id)
}

/// Convenience wrapper for anything implementing `AsRawFd`.
pub fn is_helper_device_file(file: &impl AsRawFd) -> bool {
    is_helper_device(file.as_raw_fd())
}

fn read_device_name(fd: libc::c_int) -> Option<Vec<u8>> {
    let mut buf = [0u8; 256];
    let ret = unsafe { libc::ioctl(fd, eviocgname(buf.len()), buf.as_mut_ptr()) };
    if ret < 0 {
        return None;
    }
    let len = ret as usize;
    let end = if len > 0 && buf[len - 1] == 0 {
        len - 1
    } else {
        len
    };
    Some(buf[..end].to_vec())
}

fn read_device_id(fd: libc::c_int) -> Option<InputId> {
    let mut id: InputId = unsafe { std::mem::zeroed() };
    let ret = unsafe { libc::ioctl(fd, eviocgid(), &mut id as *mut InputId as *mut libc::c_void) };
    if ret < 0 {
        None
    } else {
        Some(id)
    }
}

// ---------------------------------------------------------------------------
// Modifier state scanning.
// ---------------------------------------------------------------------------

/// Extract currently-held modifier codes from a raw EVIOCGKEY bitmap.
fn modifiers_from_bitmap(bitmap: &[u8]) -> Vec<u16> {
    MODIFIER_CODES
        .iter()
        .copied()
        .filter(|&code| {
            let (byte, bit) = (code as usize / 8, code as u32 % 8);
            byte < bitmap.len() && (bitmap[byte] >> bit) & 1 == 1
        })
        .collect()
}

/// Poll readable `/dev/input/event*` devices (excluding our helper) until no
/// modifier keys are held, or timeout expires.
///
/// Returns `Ok(())` when modifiers are clear, `Err` if timeout reached with
/// modifiers still held. If no input devices are readable, returns `Ok(())`
/// because the helper cannot determine physical state.
pub fn wait_modifiers_released() -> Result<(), String> {
    let devices = open_state_devices();
    if devices.is_empty() {
        return Ok(());
    }

    let deadline = Instant::now() + MODIFIER_WAIT_TIMEOUT;
    loop {
        let held = scan_held_modifiers(&devices);
        if held.is_empty() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "modifier keys still held after {}ms: {:?}",
                MODIFIER_WAIT_TIMEOUT.as_millis(),
                held
            ));
        }
        std::thread::sleep(MODIFIER_POLL_INTERVAL);
    }
}

fn open_state_devices() -> Vec<File> {
    let dir = match std::fs::read_dir("/dev/input") {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    let mut devices = Vec::new();
    for entry in dir.flatten() {
        let path = entry.path();
        let is_event = path
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|n| n.starts_with("event"));
        if !is_event {
            continue;
        }
        let file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
        {
            Ok(f) => f,
            Err(_) => continue,
        };
        if !is_helper_device_file(&file) {
            devices.push(file);
        }
    }
    devices
}

fn scan_held_modifiers(devices: &[File]) -> Vec<u16> {
    let mut held = Vec::new();
    for file in devices {
        let mut bitmap = [0u8; BITMAP_LEN];
        if unsafe { libc::ioctl(file.as_raw_fd(), eviocgkey(), bitmap.as_mut_ptr()) } < 0 {
            continue;
        }
        for code in modifiers_from_bitmap(&bitmap) {
            if !held.contains(&code) {
                held.push(code);
            }
        }
    }
    held
}

#[cfg(test)]
mod tests {
    use super::*;

    fn helper_id() -> InputId {
        InputId {
            bustype: BUSTYPE,
            vendor: VENDOR,
            product: PRODUCT,
            version: VERSION,
        }
    }

    // --- Identity tests ---

    #[test]
    fn matches_exact_identity() {
        assert!(matches_helper_identity(DEVICE_NAME, &helper_id()));
    }

    #[test]
    fn rejects_wrong_name() {
        assert!(!matches_helper_identity(
            b"AT Translated Set 2 keyboard",
            &helper_id()
        ));
        assert!(!matches_helper_identity(b"", &helper_id()));
        assert!(!matches_helper_identity(
            b"Wispr Flow Linux Helper Extra",
            &helper_id()
        ));
        assert!(!matches_helper_identity(
            b"Wispr Flow Linux Helper\x00",
            &helper_id()
        ));
    }

    #[test]
    fn rejects_wrong_vendor() {
        let mut id = helper_id();
        id.vendor = 0x0000;
        assert!(!matches_helper_identity(DEVICE_NAME, &id));
    }

    #[test]
    fn rejects_wrong_product() {
        let mut id = helper_id();
        id.product = 0x9999;
        assert!(!matches_helper_identity(DEVICE_NAME, &id));
    }

    #[test]
    fn rejects_wrong_bustype() {
        let mut id = helper_id();
        id.bustype = 0x06;
        assert!(!matches_helper_identity(DEVICE_NAME, &id));
    }

    #[test]
    fn rejects_wrong_version() {
        let mut id = helper_id();
        id.version = 2;
        assert!(!matches_helper_identity(DEVICE_NAME, &id));
    }

    #[test]
    fn input_id_is_8_bytes() {
        assert_eq!(std::mem::size_of::<InputId>(), 8);
    }

    #[test]
    fn eviocgname_encoding() {
        let expected: u64 = (2 << 30) | (256 << 16) | (0x45 << 8) | 0x06;
        assert_eq!(eviocgname(256) as u64, expected);
    }

    #[test]
    fn eviocgid_encoding() {
        let expected: u64 = (2 << 30) | (8 << 16) | (0x45 << 8) | 0x02;
        assert_eq!(eviocgid() as u64, expected);
    }

    // --- Modifier bitmap tests ---

    #[test]
    fn empty_bitmap_no_modifiers() {
        let bitmap = [0u8; BITMAP_LEN];
        assert!(modifiers_from_bitmap(&bitmap).is_empty());
    }

    #[test]
    fn ctrl_held_in_bitmap() {
        let mut bitmap = [0u8; BITMAP_LEN];
        bitmap[29 / 8] |= 1 << (29 % 8);
        let held = modifiers_from_bitmap(&bitmap);
        assert_eq!(held, vec![29]);
    }

    #[test]
    fn ctrl_and_super_held_ptt_case() {
        let mut bitmap = [0u8; BITMAP_LEN];
        bitmap[29 / 8] |= 1 << (29 % 8); // KEY_LEFTCTRL
        bitmap[125 / 8] |= 1 << (125 % 8); // KEY_LEFTMETA
        let held = modifiers_from_bitmap(&bitmap);
        assert_eq!(held, vec![29, 125]);
    }

    #[test]
    fn non_modifier_key_not_detected() {
        let mut bitmap = [0u8; BITMAP_LEN];
        bitmap[30 / 8] |= 1 << (30 % 8); // KEY_A
        assert!(modifiers_from_bitmap(&bitmap).is_empty());
    }

    #[test]
    fn all_modifiers_detected() {
        let mut bitmap = [0u8; BITMAP_LEN];
        for &code in MODIFIER_CODES {
            bitmap[code as usize / 8] |= 1 << (code % 8);
        }
        let held = modifiers_from_bitmap(&bitmap);
        assert_eq!(held.len(), MODIFIER_CODES.len());
        for &code in MODIFIER_CODES {
            assert!(held.contains(&code));
        }
    }

    #[test]
    fn short_bitmap_safe() {
        // Bitmap too short to contain high modifier codes -- must not panic.
        let bitmap = [0xFFu8; 4]; // only 32 bits, covers codes 0-31
        let held = modifiers_from_bitmap(&bitmap);
        assert!(held.contains(&29)); // KEY_LEFTCTRL fits
        assert!(!held.contains(&42)); // KEY_LEFTSHIFT beyond byte 4
    }
}
