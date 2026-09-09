//! Insert through the RemoteDesktop portal.
//!
//! KWin does not expose `zwp_virtual_keyboard_manager_v1`, so `wtype`-based tools do not
//! work here. The fallback copies the emoji and asks the portal to synthesize Ctrl+V after
//! the picker releases focus.

use ashpd::desktop::PersistMode;
use ashpd::desktop::remote_desktop::{
    DeviceType, KeyState, NotifyKeyboardKeycodeOptions, RemoteDesktop, SelectDevicesOptions,
};
use std::time::Duration;

/// Linux evdev key codes, as the portal expects.
const KEY_LEFTCTRL: i32 = 29;
const KEY_V: i32 = 47;

/// How long to wait after hiding the window before typing. The picker holds an exclusive
/// keyboard grab on its layer surface, and the compositor needs a moment to drop it and
/// hand focus back to the original text field.
const FOCUS_SETTLE: Duration = Duration::from_millis(150);
/// Gap between individual key events, so the modifier is unambiguously down before V.
const KEY_GAP: Duration = Duration::from_millis(20);
/// The portal session must outlive the events: closing it immediately (which process
/// exit does) drops keys that KWin has not delivered yet.
const HOLD_AFTER: Duration = Duration::from_millis(300);

pub fn copy_to_clipboard(text: &str) -> Result<(), String> {
    use wl_clipboard_rs::copy::{MimeType, Options, Source};
    let mut opts = Options::new();
    // Fork a server for the selection: a Wayland clipboard needs a live owner, and this
    // process is about to exit.
    opts.foreground(false);
    opts.copy(Source::Bytes(text.as_bytes().into()), MimeType::Text)
        .map_err(|e| e.to_string())
}

pub enum Reply {
    /// Portal session is up; Ctrl+V was delivered. Carries a refreshed restore token.
    Pasted(Option<String>),
    /// Nothing was typed; the emoji is still on the clipboard.
    Failed(String),
}

/// Open a portal session and synthesize Ctrl+V into whatever holds focus. Blocks for the
/// D-Bus round trips (and, first run only, the permission dialog) plus the paste itself,
/// so call only after the window is hidden and there is nothing left to overlap it with.
pub fn paste_once(restore_token: Option<String>, timeout: Duration) -> Reply {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => return Reply::Failed(e.to_string()),
    };
    rt.block_on(async move {
        let fut = async {
            let (proxy, session, token) = open_session(restore_token.as_deref()).await?;
            tokio::time::sleep(FOCUS_SETTLE).await;
            paste(&proxy, &session).await?;
            Ok(token)
        };
        match tokio::time::timeout(timeout, fut).await {
            Ok(Ok(token)) => Reply::Pasted(token),
            Ok(Err(e)) => Reply::Failed(e),
            Err(_) => Reply::Failed("timed out".into()),
        }
    })
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
                // Reuse the saved token after the first approval.
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
            .notify_keyboard_keycode(
                session,
                code,
                state,
                NotifyKeyboardKeycodeOptions::default(),
            )
            .await
            .map_err(|e| e.to_string())?;
        tokio::time::sleep(KEY_GAP).await;
    }
    tokio::time::sleep(HOLD_AFTER).await;
    Ok(())
}

/// `--test-paste`: open a session, wait for you to focus a field, send Ctrl+V, report.
pub fn test_paste() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    rt.block_on(async move {
        if let Err(e) = copy_to_clipboard("PASTE-OK") {
            eprintln!("clipboard: {e}");
        }
        eprintln!("opening portal session...");
        let (proxy, session, token) = match open_session(None).await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("session failed: {e}");
                return;
            }
        };
        eprintln!("session up (token {token:?}); focus a text field NOW, sending in 5s");
        tokio::time::sleep(Duration::from_secs(5)).await;

        match paste(&proxy, &session).await {
            Ok(()) => eprintln!("sent"),
            Err(e) => eprintln!("send failed: {e}"),
        }
    });
}

pub fn notify(body: &str) {
    let _ = std::process::Command::new("notify-send")
        .args(["--app-name=Pika", "--icon=face-smile", "Pika", body])
        .spawn();
}
