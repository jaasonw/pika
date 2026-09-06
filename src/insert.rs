//! Getting the emoji into the focused text field.
//!
//! KWin exposes no `zwp_virtual_keyboard_manager_v1`, so `wtype` and everything built on
//! it (rofimoji, rofi-emoji) silently does nothing on this desktop. The path that does
//! work is the RemoteDesktop portal: put the emoji on the clipboard, then have the
//! portal synthesize Ctrl+V into whatever regained focus after we closed.

use ashpd::desktop::remote_desktop::{
    DeviceType, KeyState, NotifyKeyboardKeycodeOptions, RemoteDesktop, SelectDevicesOptions,
};
use ashpd::desktop::PersistMode;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

/// Linux evdev key codes, as the portal expects.
const KEY_LEFTCTRL: i32 = 29;
const KEY_V: i32 = 47;

/// How long to wait after hiding the window before typing, so the compositor has handed
/// focus back to the original text field.
const FOCUS_SETTLE: Duration = Duration::from_millis(80);

pub fn copy_to_clipboard(text: &str) -> Result<(), String> {
    use wl_clipboard_rs::copy::{MimeType, Options, Source};
    let mut opts = Options::new();
    // Fork a server for the selection: a Wayland clipboard needs a live owner, and this
    // process is about to exit.
    opts.foreground(false);
    opts.copy(
        Source::Bytes(text.as_bytes().into()),
        MimeType::Text,
    )
    .map_err(|e| e.to_string())
}

pub enum Reply {
    /// Portal session is up; Ctrl+V was delivered. Carries a refreshed restore token.
    Pasted(Option<String>),
    /// Nothing was typed; the emoji is still on the clipboard.
    Failed(String),
}

/// Owns the portal session on its own thread. Created at startup so the D-Bus round
/// trips and the (first-run only) permission dialog overlap with the user picking.
pub struct PasteAgent {
    tx: Sender<()>,
    rx: Receiver<Reply>,
}

impl PasteAgent {
    pub fn spawn(restore_token: Option<String>) -> Self {
        let (req_tx, req_rx) = std::sync::mpsc::channel::<()>();
        let (rep_tx, rep_rx) = std::sync::mpsc::channel::<Reply>();

        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = rep_tx.send(Reply::Failed(e.to_string()));
                    return;
                }
            };
            rt.block_on(async move {
                let session = match open_session(restore_token.as_deref()).await {
                    Ok(s) => s,
                    Err(e) => {
                        // Wait for the request so the caller gets the error at pick time,
                        // not before the user has chosen anything.
                        let _ = req_rx.recv();
                        let _ = rep_tx.send(Reply::Failed(e));
                        return;
                    }
                };
                if req_rx.recv().is_err() {
                    return;
                }
                tokio::time::sleep(FOCUS_SETTLE).await;
                let reply = match paste(&session.0, &session.1).await {
                    Ok(()) => Reply::Pasted(session.2),
                    Err(e) => Reply::Failed(e),
                };
                let _ = rep_tx.send(reply);
            });
        });

        PasteAgent { tx: req_tx, rx: rep_rx }
    }

    /// Ask for the paste and block until it lands. Call only after the window is hidden.
    pub fn paste_now(&self, timeout: Duration) -> Reply {
        if self.tx.send(()).is_err() {
            return Reply::Failed("paste thread went away".into());
        }
        match self.rx.recv_timeout(timeout) {
            Ok(reply) => reply,
            Err(e) => Reply::Failed(e.to_string()),
        }
    }
}

type OpenSession = (
    RemoteDesktop,
    ashpd::desktop::Session<RemoteDesktop>,
    Option<String>,
);

async fn open_session(restore_token: Option<&str>) -> Result<OpenSession, String> {
    let proxy = RemoteDesktop::new().await.map_err(|e| e.to_string())?;
    let session = proxy
        .create_session(Default::default())
        .await
        .map_err(|e| e.to_string())?;

    proxy
        .select_devices(
            &session,
            SelectDevicesOptions::default()
                .set_devices(ashpd::enumflags2::BitFlags::from(DeviceType::Keyboard))
                // ExplicitlyRevoked plus the saved token means the KDE dialog appears
                // once, ever, and not on later runs.
                .set_persist_mode(PersistMode::ExplicitlyRevoked)
                .set_restore_token(restore_token),
        )
        .await
        .map_err(|e| e.to_string())?;

    let response = proxy
        .start(&session, None, Default::default())
        .await
        .map_err(|e| e.to_string())?
        .response()
        .map_err(|e| e.to_string())?;

    if !response.devices().contains(DeviceType::Keyboard) {
        return Err("portal did not grant keyboard access".into());
    }
    let token = response.restore_token().map(str::to_owned);
    Ok((proxy, session, token))
}

async fn paste(
    proxy: &RemoteDesktop,
    session: &ashpd::desktop::Session<RemoteDesktop>,
) -> Result<(), String> {
    for (code, state) in [
        (KEY_LEFTCTRL, KeyState::Pressed),
        (KEY_V, KeyState::Pressed),
        (KEY_V, KeyState::Released),
        (KEY_LEFTCTRL, KeyState::Released),
    ] {
        proxy
            .notify_keyboard_keycode(session, code, state, NotifyKeyboardKeycodeOptions::default())
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn notify(body: &str) {
    let _ = std::process::Command::new("notify-send")
        .args(["--app-name=Emoji Picker", "--icon=face-smile", "Emoji Picker", body])
        .spawn();
}
