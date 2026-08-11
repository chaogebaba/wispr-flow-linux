//! Crate-level IPC writer: typed channel to the fd-3 writer thread.
//!
//! Wraps `mpsc::Sender` with a message enum so both envelope writes and
//! synchronous flush barriers can be sent over the same channel. Existing
//! call sites that do `events.send(value)` keep working via `EventSink::send`.

use std::io;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::Duration;

use serde_json::Value;

/// Messages the fd-3 writer thread processes in order.
pub(crate) enum WriterMessage {
    /// A JSON envelope to encode and write to fd 3.
    Envelope(Value),
    /// A synchronization barrier: after processing all prior envelopes, the
    /// writer sends `Ok(())` (or an io::Error) back on the channel so the
    /// caller knows everything prior has been flushed to the fd.
    Barrier(SyncSender<io::Result<()>>),
}

/// Cloneable handle to the fd-3 writer. Replaces the old `mpsc::Sender<Value>`
/// type alias while keeping `.send(Value)` call sites working.
#[derive(Clone)]
pub(crate) struct EventSink {
    tx: mpsc::Sender<WriterMessage>,
}

impl EventSink {
    pub(crate) fn new(tx: mpsc::Sender<WriterMessage>) -> Self {
        Self { tx }
    }

    /// Send an envelope to the writer thread. Returns Err only if the writer
    /// thread has exited (channel disconnected).
    pub(crate) fn send(&self, envelope: Value) -> Result<(), String> {
        self.tx
            .send(WriterMessage::Envelope(envelope))
            .map_err(|_| {
                log::error!("fd3 writer thread gone — dropping message");
                "fd3 writer thread gone".to_string()
            })
    }

    /// Enqueue a barrier and block until the writer has flushed all prior
    /// messages. Returns the writer's flush result. Bounded timeout prevents
    /// deadlock if the writer dies.
    pub(crate) fn flush(&self, timeout: Duration) -> io::Result<()> {
        let (barrier_tx, barrier_rx) = mpsc::sync_channel(1);
        if self.tx.send(WriterMessage::Barrier(barrier_tx)).is_err() {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "fd3 writer thread gone",
            ));
        }
        match barrier_rx.recv_timeout(timeout) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "flush barrier timed out",
            )),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "fd3 writer stopped before flush barrier",
            )),
        }
    }
}

/// Create an (EventSink, Receiver) pair.
pub(crate) fn channel() -> (EventSink, Receiver<WriterMessage>) {
    let (tx, rx) = mpsc::channel();
    (EventSink::new(tx), rx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn send_envelope_received() {
        let (sink, rx) = channel();
        sink.send(json!({"test": true})).unwrap();
        match rx.recv().unwrap() {
            WriterMessage::Envelope(v) => assert_eq!(v, json!({"test": true})),
            WriterMessage::Barrier(_) => panic!("expected envelope"),
        }
    }

    #[test]
    fn flush_completes_when_writer_acks() {
        let (sink, rx) = channel();
        let handle = std::thread::spawn(move || {
            // Simulate writer: process messages in order.
            for msg in rx {
                match msg {
                    WriterMessage::Envelope(_) => {}
                    WriterMessage::Barrier(tx) => {
                        let _ = tx.send(Ok(()));
                    }
                }
            }
        });
        sink.send(json!({"a": 1})).unwrap();
        sink.send(json!({"b": 2})).unwrap();
        let result = sink.flush(Duration::from_secs(1));
        assert!(result.is_ok());
        drop(sink);
        handle.join().unwrap();
    }

    #[test]
    fn flush_returns_error_when_writer_reports_error() {
        let (sink, rx) = channel();
        let handle = std::thread::spawn(move || {
            for msg in rx {
                match msg {
                    WriterMessage::Envelope(_) => {}
                    WriterMessage::Barrier(tx) => {
                        let _ = tx.send(Err(io::Error::new(
                            io::ErrorKind::BrokenPipe,
                            "write failed",
                        )));
                    }
                }
            }
        });
        let result = sink.flush(Duration::from_secs(1));
        assert!(result.is_err());
        drop(sink);
        handle.join().unwrap();
    }

    #[test]
    fn flush_times_out_when_writer_gone() {
        let (sink, rx) = channel();
        drop(rx); // writer "dead"
        let result = sink.flush(Duration::from_millis(50));
        assert!(result.is_err());
    }

    #[test]
    fn send_fails_when_writer_gone() {
        let (sink, rx) = channel();
        drop(rx);
        assert!(sink.send(json!({})).is_err());
    }
}
