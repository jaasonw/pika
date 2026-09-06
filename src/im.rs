//! Direct text insertion as a Wayland input method.
//!
//! KWin exposes `zwp_input_method_v1`. An input method does not synthesize keystrokes:
//! it commits text straight into whatever holds the text-input focus. That avoids the
//! portal entirely - no permission prompt, no "remote control" notification, no
//! clipboard round trip, and none of the timing the portal path needs.
//!
//! It does not always apply. Only one client may bind the interface (a virtual keyboard
//! or fcitx5 would already hold it), and it only reaches apps that speak
//! `zwp_text_input_v2/v3` - so XWayland clients generally miss out. Every failure here
//! is expected to fall back to the portal.

use std::time::{Duration, Instant};
use wayland_client::{Connection, Dispatch, QueueHandle, protocol::wl_registry};

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
    committed: bool,
    /// The compositor never offered the interface at all.
    unavailable: bool,
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
        if let wl_registry::Event::Global { name, interface, version } = event {
            if interface == "zwp_input_method_v1" {
                state.method =
                    Some(registry.bind::<ZwpInputMethodV1, _, _>(name, version.min(1), qh, ()));
            }
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
            // The compositor hands us a context whenever a text field takes focus.
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
pub fn commit(text: &str, wait: Duration) -> Result<(), String> {
    let conn = Connection::connect_to_env().map_err(|e| e.to_string())?;
    let display = conn.display();
    let mut queue = conn.new_event_queue::<State>();
    let qh = queue.handle();
    display.get_registry(&qh, ());

    let mut state = State::default();
    queue.roundtrip(&mut state).map_err(|e| e.to_string())?;
    if state.method.is_none() {
        state.unavailable = true;
        return Err("compositor does not offer zwp_input_method_v1".into());
    }

    // Wait for the compositor to hand us a context for the focused field.
    let deadline = Instant::now() + wait;
    while state.context.is_none() {
        if Instant::now() >= deadline {
            return Err("no text field took input-method focus".into());
        }
        queue
            .blocking_dispatch(&mut state)
            .map_err(|e| e.to_string())?;
    }

    let context = state.context.clone().unwrap();
    context.commit_string(state.serial, text.to_string());
    state.committed = true;
    conn.flush().map_err(|e| e.to_string())?;
    queue.roundtrip(&mut state).map_err(|e| e.to_string())?;
    Ok(())
}

/// `--test-im`: wait for you to focus a field, then commit a marker string.
pub fn test_commit() {
    eprintln!("focus a text field NOW, committing in 5s");
    std::thread::sleep(Duration::from_secs(5));
    match commit("IM-OK", Duration::from_secs(3)) {
        Ok(()) => eprintln!("committed"),
        Err(e) => eprintln!("input method unavailable: {e}"),
    }
}
