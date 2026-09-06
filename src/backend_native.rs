//! The native Wayland backend: smithay-client-toolkit + cairo + pangocairo, no GTK.
//!
//! Phase 1 skeleton. It brings up the two subsystems the rest of the migration hangs off -
//! the Wayland connection and a pangocairo text context - so that the dependency gating is
//! proven and the library/memory floor can be measured, but it does not draw yet.
//! Phases 2-6 fill this in; see plans/wayland-native-migration.md.

use crate::theme::Theme;
use crate::{Flags, since_start};
use std::process::ExitCode;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::{Connection, Dispatch, QueueHandle};

pub fn run(_flags: Flags) -> ExitCode {
    let conn = match Connection::connect_to_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("emoji-picker: no Wayland display ({e})");
            return ExitCode::FAILURE;
        }
    };
    let (globals, _queue) = match registry_queue_init::<NoopState>(&conn) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("emoji-picker: registry init failed ({e})");
            return ExitCode::FAILURE;
        }
    };

    // Touch the text stack too: font discovery is a real share of cold start, and it is
    // what the first-frame budget in the plan has to account for.
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 1, 1).unwrap();
    let cr = cairo::Context::new(&surface).unwrap();
    let layout = pangocairo::functions::create_layout(&cr);
    layout.set_font_description(Some(&pango::FontDescription::from_string(
        "Noto Color Emoji 24",
    )));
    layout.set_text("\u{1F600}");
    let (w, h) = layout.pixel_size();

    let theme = Theme::load();

    eprintln!(
        "emoji-picker: native backend not implemented yet \
         (wayland globals={}, emoji layout={w}x{h}, {} theme bg={:?}, {:?} elapsed)",
        globals.contents().clone_list().len(),
        if theme.is_dark() { "dark" } else { "light" },
        theme.window_bg,
        since_start(),
    );
    ExitCode::FAILURE
}

/// The registry needs a dispatch target even though this skeleton handles no events yet.
/// Phase 3 replaces this with the real SCTK state, which delegates registry handling to
/// `RegistryState`.
struct NoopState;

impl Dispatch<WlRegistry, GlobalListContents> for NoopState {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as wayland_client::Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
