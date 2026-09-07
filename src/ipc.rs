//! Single-instance socket, used by both modes.
//!
//! Whichever process owns the window binds the socket. A later invocation - the user
//! hitting the hotkey again - finds it, sends "toggle", and exits, so the shortcut
//! closes an open picker instead of stacking up processes.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

pub fn socket_path() -> PathBuf {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(dir).join("pika.sock")
}

/// Ask the running instance to toggle its window. Returns false if there is none, in
/// which case the caller owns the UI itself.
pub fn request_toggle() -> bool {
    let Ok(mut stream) = UnixStream::connect(socket_path()) else {
        return false;
    };
    if stream.write_all(b"toggle\n").is_err() {
        return false;
    }
    let _ = stream.flush();
    let mut ack = [0u8; 2];
    let _ = stream.read(&mut ack);
    true
}

/// Bind the single-instance socket. Separate from pumping it, since the native backend
/// hands the listener straight to calloop rather than reading from it here.
pub fn bind() -> std::io::Result<UnixListener> {
    let path = socket_path();
    // A stale socket from a killed daemon would block bind(); nothing is listening on it
    // if the connect probe just failed.
    if !request_toggle() {
        let _ = std::fs::remove_file(&path);
    }
    UnixListener::bind(&path)
}

/// Read a toggle request and acknowledge it, so the sender does not block on our reply.
pub fn ack(stream: &mut UnixStream) {
    let mut buf = [0u8; 16];
    let _ = stream.read(&mut buf);
    let _ = stream.write_all(b"ok");
}

pub fn cleanup() {
    let _ = std::fs::remove_file(socket_path());
}
