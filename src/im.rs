//! Commit text through KWin's Wayland input-method interface.
//!
//! This avoids the clipboard and portal when the focused application supports
//! `zwp_text_input_v2/v3`. The interface is single-client and does not reach most
//! XWayland clients; callers fall back to the portal when it is unavailable.

use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};
use wayland_client::{Connection, Dispatch, QueueHandle, protocol::wl_registry};

/// Why the input-method route did not carry the emoji.
///
/// [`Error::NoFocus`] means the target does not speak `zwp_text_input`, so retrying this
/// route cannot help. [`Error::Unusable`] describes a failure of the route itself and allows
/// the caller to try the portal.
#[derive(Debug)]
pub enum Error {
    /// Nothing took input-method focus before the deadline: an X11 client, or Chromium
    /// without `--enable-wayland-ime`.
    NoFocus,
    /// The route is unavailable: the interface is missing, busy, or disconnected.
    Unusable(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NoFocus => f.write_str("no text field took input-method focus"),
            Error::Unusable(e) => f.write_str(e),
        }
    }
}

pub mod proto {
    #![allow(non_camel_case_types, clippy::all)]
    use wayland_client;
    use wayland_client::protocol::*;
    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/input-method-unstable-v1.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_client_code!("protocols/input-method-unstable-v1.xml");
}

use proto::zwp_input_method_context_v1::ZwpInputMethodContextV1;
use proto::zwp_input_method_v1::{self, ZwpInputMethodV1};

#[derive(Default)]
struct State {
    method: Option<ZwpInputMethodV1>,
    context: Option<ZwpInputMethodContextV1>,
    /// Latest serial the compositor sent for this context; commits must quote it.
    serial: u32,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
            && interface == "zwp_input_method_v1"
        {
            state.method =
                Some(registry.bind::<ZwpInputMethodV1, _, _>(name, version.min(1), qh, ()));
        }
    }
}

impl Dispatch<ZwpInputMethodV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwpInputMethodV1,
        event: zwp_input_method_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_input_method_v1::Event::Activate { id } => state.context = Some(id),
            zwp_input_method_v1::Event::Deactivate { context } => {
                context.destroy();
                state.context = None;
            }
        }
    }

    // `activate` carries a newly created context object, so the queue has to be told
    // how to construct it.
    wayland_client::event_created_child!(State, ZwpInputMethodV1, [
        zwp_input_method_v1::EVT_ACTIVATE_OPCODE => (ZwpInputMethodContextV1, ()),
    ]);
}

impl Dispatch<ZwpInputMethodContextV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwpInputMethodContextV1,
        event: proto::zwp_input_method_context_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let proto::zwp_input_method_context_v1::Event::CommitState { serial } = event {
            state.serial = serial;
        }
    }
}

/// Try to commit `text` into the focused text field.
///
/// Returns `Err` with the reason when this route is not usable, so the caller can fall
/// back to the portal. Call it only once the picker's own window is hidden - otherwise
/// the focused text input is ours.
pub fn commit(text: &str, wait: Duration) -> Result<(), Error> {
    let unusable = |e: &dyn std::fmt::Display| Error::Unusable(e.to_string());

    let conn = Connection::connect_to_env().map_err(|e| unusable(&e))?;
    let display = conn.display();
    let mut queue = conn.new_event_queue::<State>();
    let qh = queue.handle();
    display.get_registry(&qh, ());

    let mut state = State::default();
    queue.roundtrip(&mut state).map_err(|e| unusable(&e))?;
    if state.method.is_none() {
        return Err(Error::Unusable(
            "compositor does not offer zwp_input_method_v1".into(),
        ));
    }

    // Wait for the compositor to hand us a context for the focused field.
    //
    // This polls rather than calling `blocking_dispatch`, which waits on the socket with
    // no timeout. When the focused app speaks no text-input the compositor sends nothing
    // at all - not even a deactivate - so a blocking wait never returns and `wait` never
    // elapses. The picker would then sit here until some unrelated window took text
    // focus, and commit the emoji into that one instead.
    let deadline = Instant::now() + wait;
    loop {
        queue
            .dispatch_pending(&mut state)
            .map_err(|e| unusable(&e))?;
        if state.context.is_some() {
            break;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(Error::NoFocus);
        }
        conn.flush().map_err(|e| unusable(&e))?;
        // `None` means another thread is already reading, or events arrived between the
        // dispatch and now; either way go round and dispatch them rather than wait.
        let Some(guard) = queue.prepare_read() else {
            continue;
        };
        let mut pfd = libc::pollfd {
            fd: guard.connection_fd().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // At least 1ms, so a sub-millisecond remainder still gets a real poll.
        let ms = (left.as_millis().max(1).min(i32::MAX as u128)) as libc::c_int;
        match unsafe { libc::poll(&mut pfd, 1, ms) } {
            0 => return Err(Error::NoFocus),
            n if n < 0 => {
                let e = std::io::Error::last_os_error();
                if e.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(unusable(&e));
            }
            _ => {
                guard.read().map_err(|e| unusable(&e))?;
            }
        }
    }

    let context = state.context.clone().unwrap();
    context.commit_string(state.serial, text.to_string());
    conn.flush().map_err(|e| unusable(&e))?;
    queue.roundtrip(&mut state).map_err(|e| unusable(&e))?;
    Ok(())
}

/// `--test-im`: wait for you to focus a field, then commit a marker string.
pub fn test_commit() {
    eprintln!("focus a text field NOW, committing in 5s");
    std::thread::sleep(Duration::from_secs(5));
    match commit("IM-OK", Duration::from_secs(3)) {
        Ok(()) => eprintln!("committed"),
        Err(Error::NoFocus) => eprintln!("no text field took input-method focus in 3s"),
        Err(e) => eprintln!("input method unavailable: {e}"),
    }
}
