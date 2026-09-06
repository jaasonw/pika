//! Daemon socket. Optional: without a daemon running, every invocation is a fresh
//! process and this module is only used for the "is one running?" probe.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

pub fn socket_path() -> PathBuf {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(dir).join("emoji-picker.sock")
}

/// Ask a running daemon to show its window. Returns false if there is none, in which
/// case the caller runs standalone.
pub fn request_show() -> bool {
    let Ok(mut stream) = UnixStream::connect(socket_path()) else {
        return false;
    };
    if stream.write_all(b"show\n").is_err() {
        return false;
    }
    let _ = stream.flush();
    let mut ack = [0u8; 2];
    let _ = stream.read(&mut ack);
    true
}

/// Listen for show requests, calling `on_show` on the GTK main thread for each.
pub fn serve(on_show: impl Fn() + 'static) -> std::io::Result<()> {
    let path = socket_path();
    // A stale socket from a killed daemon would block bind(); nothing is listening on it
    // if the connect probe just failed.
    if !request_show() {
        let _ = std::fs::remove_file(&path);
    }
    let listener = UnixListener::bind(&path)?;

    let (tx, rx) = async_channel::unbounded::<()>();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut buf = [0u8; 16];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(b"ok");
            if tx.send_blocking(()).is_err() {
                break;
            }
        }
    });

    gtk4::glib::spawn_future_local(async move {
        while rx.recv().await.is_ok() {
            on_show();
        }
    });
    Ok(())
}

pub fn cleanup() {
    let _ = std::fs::remove_file(socket_path());
}
