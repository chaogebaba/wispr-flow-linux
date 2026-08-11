//! Wispr Flow — Linux helper (clean-room).
//!
//! Speaks the helper IPC contract recovered in `docs/reference/ipc-contract.md`:
//!   * reads commands on **stdin (fd 0)**
//!   * writes all protocol output to **fd 3** (stdout/fd 1 is non-IPC; stderr/fd 2 is logging)
//!   * framing: escaped pretty-JSON envelopes joined by `'|'`
//!   * answers `IsReady` with `ACK` (the readiness + keepalive handshake)
//!
//! This is the skeleton: handshake + dispatch + the highest-value X11 commands
//! (PasteText, SimulateKeyPress, GetActiveAppInfo, GetRunningApps,
//! GetSelectedTextViaCopy, GetAccessibilityStatus). Everything else is ACK'd as a
//! safe no-op so the app stays healthy. See README.md for what's next.

mod backend;
mod capture;
mod input_device;
mod ipc;
mod keymap;
mod proto;

use std::io::{Read, Write};
use std::os::unix::io::FromRawFd;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use backend::Backend;
use ipc::{EventSink, WriterMessage};
use proto::{Incoming, Kind};

/// Spawn the sole owner of fd 3: drains the channel, encodes each envelope, and
/// writes+flushes it. Handles both Envelope and Barrier messages in order.
fn spawn_fd3_writer(rx: std::sync::mpsc::Receiver<WriterMessage>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut file = unsafe { std::fs::File::from_raw_fd(3) };
        for msg in rx {
            match msg {
                WriterMessage::Envelope(envelope) => match proto::encode(&envelope) {
                    Ok(frame) => {
                        if let Err(e) = file.write_all(&frame).and_then(|_| file.flush()) {
                            log::error!("fd3 write failed: {e}");
                            break;
                        }
                    }
                    Err(e) => log::error!("encode failed: {e}"),
                },
                WriterMessage::Barrier(reply) => {
                    let _ = reply.send(file.flush());
                }
            }
        }
    })
}

fn main() {
    if std::env::args().any(|a| a == "--version") {
        println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        return;
    }

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .target(env_logger::Target::Stderr)
        .init();

    let started = Instant::now();
    log::info!(
        "wispr-flow-linux-helper starting (pid {})",
        std::process::id()
    );

    let (ipc, rx) = ipc::channel();
    let _writer = spawn_fd3_writer(rx);
    // Global key capture: streams `KeypressEvent`s on fd 3 so push-to-talk and
    // the in-app shortcut recorder work. The returned handle answers
    // `CheckStaleKeys`. Independent of the focus/injection backend.
    let held_keys = capture::spawn(ipc.clone());
    // The backend gets its own sink for async helper-initiated events (focus).
    let mut be = backend::detect(ipc.clone());
    log::info!("backend = {}", be.name());

    let mut decoder = proto::FrameDecoder::default();
    let mut stdin = std::io::stdin();
    let mut chunk = [0u8; 8192];

    'main: loop {
        let n = match stdin.read(&mut chunk) {
            Ok(0) => {
                log::info!("stdin closed (EOF) — shutting down");
                break;
            }
            Ok(n) => n,
            Err(e) => {
                log::error!("stdin read error: {e}");
                break;
            }
        };
        for body in decoder.feed(&chunk[..n]) {
            match Incoming::parse(&body) {
                Ok(msg) => {
                    if msg.kind == Kind::Response {
                        // Electron-initiated responses (e.g. ACK to our events) — nothing to do yet.
                        log::debug!("<- response {} uuid={}", msg.command, msg.uuid);
                        continue;
                    }
                    if handle_request(&mut *be, held_keys.as_ref(), &ipc, &msg, started) {
                        break 'main;
                    }
                }
                Err(e) => log::error!("failed to parse message: {e}; body={body:?}"),
            }
        }
    }
}

fn handle_request(
    be: &mut dyn Backend,
    held_keys: &dyn capture::HeldKeys,
    ipc: &EventSink,
    msg: &Incoming,
    started: Instant,
) -> bool {
    let uuid = msg.uuid.as_str();
    // payload-bearing commands wrap their data under `.payload`
    let payload = msg.payload.get("payload").cloned().unwrap_or(Value::Null);

    if msg.command == "HelperAppShutdown" {
        log::info!("HelperAppShutdown received — shutting down cleanly");
        let _ = ipc.send(proto::ack(uuid));
        // Flush: wait for the ACK to be written to fd 3 before exiting.
        if let Err(e) = ipc.flush(Duration::from_secs(2)) {
            log::warn!("shutdown ACK flush failed: {e}");
        }
        return true;
    }

    match msg.command.as_str() {
        // ---- readiness / keepalive ----
        "IsReady" => {
            let ms = started.elapsed().as_millis() as u64;
            let mut inner = serde_json::Map::new();
            inner.insert("ACK".to_string(), json!(true));
            inner.insert(
                "HelperLaunchTiming".to_string(),
                json!({ "helperMainEntryMs": ms }),
            );
            let _ = ipc.send(proto::envelope("HelperAPIResponse", inner, uuid));
        }

        // ---- core paste / keys ----
        "PasteText" => {
            let text = payload.get("text").and_then(Value::as_str).unwrap_or("");
            let html = payload
                .get("htmlText")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty());
            match be.paste_text(text, html) {
                Ok(()) => {
                    let _ = ipc.send(proto::ack(uuid));
                }
                Err(e) => {
                    log::error!("PasteText failed: {e}");
                    let _ = ipc.send(proto::error(&format!("PasteText failed: {e}"), uuid));
                }
            }
        }
        "SimulateKeyPress" => {
            let keycode = payload.get("keycode").and_then(Value::as_u64).unwrap_or(0) as u32;
            let flags: Vec<String> = payload
                .get("flags")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            match be.simulate_key_press(keycode, &flags) {
                Ok(()) => {
                    let _ = ipc.send(proto::ack(uuid));
                }
                Err(e) => {
                    log::error!("SimulateKeyPress failed: {e}");
                    let _ = ipc.send(proto::error(&format!("SimulateKeyPress failed: {e}"), uuid));
                }
            }
        }

        // ---- active / running apps ----
        "GetActiveAppInfo" => match be.get_active_app() {
            Ok(a) => {
                let _ = ipc.send(proto::response(
                    "ActiveAppInfo",
                    backend::active_app_payload(&a),
                    uuid,
                ));
            }
            Err(e) => {
                let _ = ipc.send(proto::error(&format!("GetActiveAppInfo failed: {e}"), uuid));
            }
        },
        "GetAppInfo" => match be.get_active_app() {
            Ok(a) => {
                let _ = ipc.send(proto::response(
                    "AppInfo",
                    json!({ "payload": { "appName": a.app_name, "bundleId": a.bundle_id, "url": a.url } }),
                    uuid,
                ));
            }
            Err(e) => {
                let _ = ipc.send(proto::error(&format!("GetAppInfo failed: {e}"), uuid));
            }
        },
        "GetRunningApps" => match be.get_running_apps() {
            Ok(apps) => {
                let list: Vec<Value> = apps
                    .iter()
                    .map(|a| json!({ "bundleId": a.bundle_id, "name": a.name }))
                    .collect();
                let _ = ipc.send(proto::response(
                    "RunningApps",
                    json!({ "payload": { "apps": list } }),
                    uuid,
                ));
            }
            Err(e) => {
                let _ = ipc.send(proto::error(&format!("GetRunningApps failed: {e}"), uuid));
            }
        },

        // ---- selection / accessibility ----
        "GetSelectedTextViaCopy" => match be.get_selected_text() {
            Ok(s) => {
                let _ = ipc.send(proto::response(
                    "SelectedTextViaCopy",
                    json!({ "payload": {
                        "selectedText": s.selected_text,
                        "beforeText": s.before_text,
                        "afterText": s.after_text,
                        "contents": s.contents,
                    } }),
                    uuid,
                ));
            }
            Err(e) => {
                let _ = ipc.send(proto::error(
                    &format!("GetSelectedTextViaCopy failed: {e}"),
                    uuid,
                ));
            }
        },
        "GetAccessibilityStatus" => {
            let status = be.accessibility_status();
            let _ = ipc.send(proto::response(
                "AccessibilityStatus",
                json!({ "payload": { "status": status } }),
                uuid,
            ));
        }
        "RequestAccessibilityPermission" | "StartAccessibilityServices" => {
            let _ = ipc.send(proto::ack(uuid));
        }

        // ---- focus tracking ----
        "SetFocusChangeDetectorState" => {
            let active = payload
                .get("active")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            be.set_focus_detection(active);
            log::info!(
                "focus change detector -> {}",
                if active { "on" } else { "off" }
            );
            let _ = ipc.send(proto::ack(uuid));
        }

        // ---- stale-key recovery ----
        "CheckStaleKeys" => {
            let queried: Vec<u64> = payload
                .get("keycodes")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_u64).collect())
                .unwrap_or_default();
            let held = held_keys.held_vks();
            let stale: Vec<u64> = queried
                .into_iter()
                .filter(|vk| !held.contains(&(*vk as u32)))
                .collect();
            let _ = ipc.send(proto::response(
                "StaleKeysResponse",
                json!({ "payload": { "staleKeys": stale } }),
                uuid,
            ));
        }

        // ---- everything else: safe no-op ACK ----
        other => {
            log::debug!("unhandled command '{other}' (uuid={uuid}) — ACK no-op");
            let _ = ipc.send(proto::ack(uuid));
        }
    }
    false
}
