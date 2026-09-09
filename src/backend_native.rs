//! Native Wayland backend using smithay-client-toolkit, Cairo, and Pango.
//!
//! Owns the layer surface, event loop, and input routing. Settings share the picker card
//! as a mode; insertion is handled by the `commit` module.

use crate::commit;
use crate::ipc;
use crate::picker::Picker;
use crate::picker::{Action, Mode};
use crate::render;
use crate::store::Store;
use crate::theme::Theme;
use crate::{Flags, since_start};
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::generic::Generic;
use smithay_client_toolkit::reexports::calloop::{
    EventLoop, Interest, Mode as PollMode, PostAction,
};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::keyboard::{
    KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers,
};
use smithay_client_toolkit::seat::pointer::{
    CursorIcon, PointerEvent, PointerEventKind, PointerHandler, ThemeSpec, ThemedPointer,
};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{delegate_registry, registry_handlers};
use std::cell::RefCell;
use std::process::ExitCode;
use std::rc::Rc;
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface};
use wayland_client::{Connection, QueueHandle};

/// Hand a URL to the desktop. Detached, so the picker exiting a moment later does not
/// take the browser with it.
fn open_url(url: &str) {
    use std::process::{Command, Stdio};
    let spawned = Command::new("xdg-open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Err(e) = spawned {
        eprintln!("pika: could not open {url} ({e})");
    }
}

/// The clipboard as text, for Ctrl+V into the search box. A missing or non-text clipboard
/// is not worth reporting: the paste simply does nothing.
fn clipboard_text() -> Option<String> {
    use std::io::Read;
    use wl_clipboard_rs::paste::{ClipboardType, MimeType, Seat, get_contents};
    let (mut pipe, _) =
        get_contents(ClipboardType::Regular, Seat::Unspecified, MimeType::Text).ok()?;
    let mut buf = String::new();
    pipe.read_to_string(&mut buf).ok()?;
    let line = buf.lines().next().unwrap_or_default().to_string();
    (!line.is_empty()).then_some(line)
}

/// Fallback surface size, used only if the compositor configures us with 0x0 - which it
/// should not, since we anchor to all four edges and it knows the output size.
const FALLBACK: (u32, u32) = (1920, 1080);

pub fn run(flags: Flags) -> ExitCode {
    let conn = match Connection::connect_to_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("pika: no Wayland display ({e})");
            return ExitCode::FAILURE;
        }
    };
    let (globals, queue) = match registry_queue_init(&conn) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("pika: registry init failed ({e})");
            return ExitCode::FAILURE;
        }
    };
    let qh = queue.handle();

    let compositor = match CompositorState::bind(&globals, &qh) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("pika: no wl_compositor ({e})");
            return ExitCode::FAILURE;
        }
    };
    let layer_shell = match LayerShell::bind(&globals, &qh) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("pika: compositor has no wlr-layer-shell ({e})");
            return ExitCode::FAILURE;
        }
    };
    let shm = match Shm::bind(&globals, &qh) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("pika: no wl_shm ({e})");
            return ExitCode::FAILURE;
        }
    };

    let surface = compositor.create_surface(&qh);
    let layer = layer_shell.create_layer_surface(&qh, surface, Layer::Overlay, Some("pika"), None);
    // A full-output surface receives outside-card clicks so it can dismiss the picker.
    layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
    // A hotkey-driven picker must receive keys without requiring a click first.
    layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
    layer.commit();

    // One slot, not two: the picker redraws on input, never continuously, so there is no
    // frame in flight to double-buffer against. At full-output size that is the difference
    // between ~9 MB and ~18 MB of shm. SlotPool grows on demand if that assumption breaks.
    let pool = match SlotPool::new(4, &shm) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("pika: shm pool failed ({e})");
            return ExitCode::FAILURE;
        }
    };

    let store = Rc::new(RefCell::new(Store::load()));
    let ui = {
        let st = store.borrow();
        Picker::new(st.recents().to_vec(), st.settings().skin_tone)
    };

    let mut app = App {
        registry: RegistryState::new(&globals),
        output: OutputState::new(&globals, &qh),
        seat: SeatState::new(&globals, &qh),
        shm,
        pool,
        layer,
        keyboard: None,
        pointer: None,
        compositor,
        cursor: CursorIcon::Default,
        width: FALLBACK.0,
        height: FALLBACK.1,
        scale: 1,
        theme: Theme::load(),
        ui,
        store: store.clone(),
        configured: false,
        reported_first_frame: !flags.time_launch,
        ctrl: false,
        picked: None,
        exit: false,
    };

    // Keep the socket and Wayland connection in one event loop.
    let mut event_loop: EventLoop<App> = match EventLoop::try_new() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("pika: event loop failed ({e})");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = WaylandSource::new(conn.clone(), queue).insert(event_loop.handle()) {
        eprintln!("pika: could not watch the wayland connection ({e})");
        return ExitCode::FAILURE;
    }

    // Serving the socket is what makes a second hotkey press close the picker rather than
    // start another one. A failure here is not fatal: the picker still works, it just
    // stops being single-instance.
    match ipc::bind().and_then(|l| l.set_nonblocking(true).map(|()| l)) {
        Ok(listener) => {
            let source = Generic::new(listener, Interest::READ, PollMode::Level);
            let registered = event_loop
                .handle()
                .insert_source(source, |_, listener, app| {
                    while let Ok((mut stream, _)) = listener.accept() {
                        ipc::ack(&mut stream);
                        app.exit = true;
                    }
                    Ok(PostAction::Continue)
                });
            if let Err(e) = registered {
                eprintln!("pika: could not watch the toggle socket ({e})");
            }
        }
        Err(e) => eprintln!("pika: single-instance socket unavailable ({e})"),
    }

    while !app.exit {
        if let Err(e) = event_loop.dispatch(None, &mut app) {
            eprintln!("pika: dispatch failed ({e})");
            return ExitCode::FAILURE;
        }
    }

    if let Some(k) = app.keyboard.take() {
        k.release();
    }
    if let Some(p) = app.pointer.take() {
        p.pointer().release();
    }

    let Some(ch) = app.picked.take() else {
        return ExitCode::SUCCESS;
    };
    if flags.print {
        println!("{ch}");
    }
    store.borrow_mut().record_use(ch);

    // Unmap before inserting. The keystrokes have to land in whatever had focus before us,
    // and while this surface is up it holds the keyboard exclusively.
    let surface = app.layer.wl_surface();
    surface.attach(None, 0, 0);
    surface.commit();
    let _ = conn.roundtrip();
    // The roundtrip only proves the compositor saw the unmap, not that it has moved focus
    // on and told the new client.
    std::thread::sleep(commit::HIDE_SETTLE);

    let (want_insert, want_copy) = {
        let st = store.borrow();
        (st.settings().insert, st.settings().always_copy)
    };
    commit::finish(commit::Ctx {
        ch,
        st: &store,
        no_paste: flags.no_paste || !want_insert,
        always_copy: flags.always_copy || want_copy,
    });
    ExitCode::SUCCESS
}

struct App {
    registry: RegistryState,
    output: OutputState,
    seat: SeatState,
    shm: Shm,
    pool: SlotPool,
    layer: LayerSurface,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<ThemedPointer>,
    /// Kept so the pointer can be given a cursor surface when a seat appears.
    compositor: CompositorState,
    /// Last cursor asked for, so an unchanged one is not re-sent on every motion event.
    cursor: CursorIcon,
    width: u32,
    height: u32,
    /// Integer output scale. Fractional scaling is Phase 3 follow-up work.
    scale: i32,
    theme: Theme,
    ui: Picker,
    store: Rc<RefCell<Store>>,
    configured: bool,
    reported_first_frame: bool,
    /// Ctrl held, tracked from modifier events so key handling can branch on it.
    ctrl: bool,
    /// Set when the user commits a choice, and consumed after the loop ends so the
    /// insert happens with the surface already unmapped.
    picked: Option<&'static str>,
    exit: bool,
}

impl App {
    fn draw(&mut self) {
        // Buffer dimensions are physical; the Cairo transform below turns the rest of the
        // drawing code back into logical pixels.
        let (w, h) = (
            self.width as i32 * self.scale,
            self.height as i32 * self.scale,
        );
        let stride = w * 4;

        let Ok((buffer, canvas)) = self
            .pool
            .create_buffer(w, h, stride, wl_shm::Format::Argb8888)
        else {
            eprintln!("pika: could not allocate a {w}x{h} buffer");
            self.exit = true;
            return;
        };

        {
            // SAFETY: `canvas` is a mutable borrow of the pool's mapping, valid for at
            // least this block, and the surface and context are both dropped before it
            // ends. wl_shm's Argb8888 is premultiplied little-endian, which is exactly
            // Cairo's ARgb32 on this target.
            let surface = unsafe {
                cairo::ImageSurface::create_for_data_unsafe(
                    canvas.as_mut_ptr(),
                    cairo::Format::ARgb32,
                    w,
                    h,
                    stride,
                )
            };
            let Ok(surface) = surface else {
                eprintln!("pika: cairo surface creation failed");
                self.exit = true;
                return;
            };
            let cr = cairo::Context::new(&surface).expect("cairo context");
            cr.scale(self.scale as f64, self.scale as f64);
            render::frame(
                &cr,
                &self.theme,
                &self.ui,
                self.store.borrow().settings(),
                self.width as f64,
                self.height as f64,
            );
            drop(cr);
            surface.finish();
        }

        let wl_surface = self.layer.wl_surface();
        wl_surface.set_buffer_scale(self.scale);
        // Only the card changed; damaging just that rect keeps the compositor from
        // reblending the whole screen for every keystroke.
        let (cx, cy) = render::card_origin(self.width as f64, self.height as f64);
        wl_surface.damage_buffer(
            cx as i32 * self.scale,
            cy as i32 * self.scale,
            render::CARD_W as i32 * self.scale,
            render::CARD_H as i32 * self.scale,
        );
        if buffer.attach_to(wl_surface).is_err() {
            eprintln!("pika: buffer attach failed");
            self.exit = true;
            return;
        }
        self.layer.commit();

        if !self.reported_first_frame {
            self.reported_first_frame = true;
            println!("first frame at {:?}", since_start());
            self.exit = true;
        }
    }
}

impl App {
    /// Which cursor belongs over a surface-relative point. Anything that responds to a
    /// click gets the hand; everywhere else, including outside the card, gets the arrow.
    fn cursor_for(&self, px: f64, py: f64) -> CursorIcon {
        let (w, h) = (self.width as f64, self.height as f64);
        let (cx, cy) = render::card_origin(w, h);
        let (x, y) = (px - cx, py - cy);
        if x < 0.0 || y < 0.0 || x >= render::CARD_W || y >= render::CARD_H {
            return CursorIcon::Default;
        }
        let over = match self.ui.mode {
            Mode::Settings => {
                render::donate_hit(x, y)
                    || render::tone_at(x, y).is_some()
                    || render::stepper_at(x, y).is_some()
                    || render::setting_at(x, y).is_some()
            }
            Mode::Browse if render::search_hit(x, y) => return CursorIcon::Text,
            Mode::Browse => {
                let (vx, vy) = render::viewport_origin(w, h);
                render::gear_hit(x, y)
                    || render::tab_at(x, y, self.ui.grid.sections.len())
                        .is_some_and(|i| self.ui.grid.sections[i].is_some())
                    || self.ui.cell_at(px - vx, py - vy).is_some()
            }
        };
        if over {
            CursorIcon::Pointer
        } else {
            CursorIcon::Default
        }
    }

    /// Ask for a cursor, skipping the request when it has not changed - motion events
    /// arrive far faster than the image needs to.
    fn set_cursor(&mut self, conn: &Connection, icon: CursorIcon) {
        if self.cursor == icon {
            return;
        }
        self.cursor = icon;
        if let Some(p) = self.pointer.as_ref() {
            let _ = p.set_cursor(conn, icon);
        }
    }

    /// Key handling for the settings mode. Escape and the Back row return to the picker;
    /// everything else edits a setting and saves it.
    fn settings_key(&mut self, key: Keysym) {
        let (tone, limit) = {
            let st = self.store.borrow();
            (st.settings().skin_tone, st.settings().recent_limit)
        };
        let action = match key {
            Keysym::Escape => Some(Action::Close),
            Keysym::Up => {
                self.ui.step_setting(-1);
                None
            }
            Keysym::Down => {
                self.ui.step_setting(1);
                None
            }
            Keysym::Left => self.ui.adjust_setting(-1, tone, limit),
            Keysym::Right => self.ui.adjust_setting(1, tone, limit),
            Keysym::Return | Keysym::KP_Enter | Keysym::space => self.ui.activate_setting(),
            _ => None,
        };
        let Some(action) = action else { return };
        self.apply(action);
    }

    fn apply(&mut self, action: Action) {
        {
            let mut st = self.store.borrow_mut();
            match action {
                Action::ToggleInsert => {
                    let v = st.settings().insert;
                    st.settings_mut().insert = !v;
                }
                Action::ToggleAlwaysCopy => {
                    let v = st.settings().always_copy;
                    st.settings_mut().always_copy = !v;
                }
                Action::SetTone(t) => st.settings_mut().skin_tone = t,
                Action::SetRecentLimit(n) => st.set_recent_limit(n),
                Action::ClearRecents => {
                    st.clear_recents();
                    self.ui.cleared_recents = true;
                }
                Action::ResetPaste => {
                    st.clear_restore_token();
                    self.ui.reset_paste = true;
                }
                Action::Close => {}
            }
            // Settings save on change; there is no OK button to hang the write off.
            st.save();
        }
        if action == Action::Close {
            let (recents, tone) = {
                let st = self.store.borrow();
                (st.recents().to_vec(), st.settings().skin_tone)
            };
            self.ui.close_settings(recents, tone);
        }
    }
}

impl LayerShellHandler for App {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.exit = true;
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        let (w, h) = configure.new_size;
        self.width = if w == 0 { FALLBACK.0 } else { w };
        self.height = if h == 0 { FALLBACK.1 } else { h };
        // Later configures are resizes; both cases want a redraw at the new size.
        self.configured = true;
        self.draw();
    }
}

impl CompositorHandler for App {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        factor: i32,
    ) {
        self.scale = factor.max(1);
        if self.configured {
            self.draw();
        }
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl KeyboardHandler for App {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
    }

    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
    ) {
    }

    fn press_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        let ctrl = self.ctrl;
        if self.ui.mode == Mode::Settings {
            self.settings_key(event.keysym);
            self.draw();
            return;
        }
        match event.keysym {
            Keysym::Escape => {
                self.exit = true;
                return;
            }
            Keysym::comma if ctrl => self.ui.open_settings(),
            Keysym::Return | Keysym::KP_Enter => {
                self.picked = self.ui.selected();
                self.exit = true;
                return;
            }
            // Text editing. Plain arrows stay with the grid - this is a picker first - so
            // the text cursor is reached with Ctrl+arrows, Home and End.
            Keysym::BackSpace if ctrl => self.ui.edit(|q| q.delete_word_back()),
            Keysym::BackSpace => self.ui.edit(|q| q.backspace()),
            Keysym::Delete if ctrl => self.ui.edit(|q| q.delete_word_forward()),
            Keysym::Delete => self.ui.edit(|q| q.delete()),
            Keysym::Home => self.ui.edit(|q| {
                q.home();
                false
            }),
            Keysym::End => self.ui.edit(|q| {
                q.end();
                false
            }),
            Keysym::Left if ctrl => self.ui.edit(|q| {
                q.word_left();
                false
            }),
            Keysym::Right if ctrl => self.ui.edit(|q| {
                q.word_right();
                false
            }),
            Keysym::Left => self.ui.step(0, -1),
            Keysym::Right => self.ui.step(0, 1),
            Keysym::Up => self.ui.step(-1, 0),
            Keysym::Down => self.ui.step(1, 0),
            Keysym::Page_Up => self.ui.page(false),
            Keysym::Page_Down => self.ui.page(true),
            Keysym::Tab => self.ui.cycle_section(false),
            Keysym::ISO_Left_Tab => self.ui.cycle_section(true),
            Keysym::u if ctrl => self.ui.edit(|q| q.clear()),
            Keysym::w if ctrl => self.ui.edit(|q| q.delete_word_back()),
            Keysym::v if ctrl => match clipboard_text() {
                Some(t) => self.ui.insert(&t),
                None => return,
            },
            _ => {
                if ctrl {
                    return;
                }
                // `utf8` is what xkb composed for this key, so dead keys and compose
                // sequences arrive here already resolved.
                match event.utf8.as_deref() {
                    Some(t) if !t.is_empty() && !t.chars().any(char::is_control) => {
                        self.ui.insert(t)
                    }
                    _ => return,
                }
            }
        }
        self.draw();
    }

    /// SCTK drives repeat from its own timer, so held keys reach us here rather than as a
    /// stream of presses. Phase 5 routes both through the same handler.
    fn repeat_key(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        kbd: &wl_keyboard::WlKeyboard,
        serial: u32,
        event: KeyEvent,
    ) {
        self.press_key(conn, qh, kbd, serial, event);
    }

    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: KeyEvent,
    ) {
    }

    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        modifiers: Modifiers,
        _: RawModifiers,
        _: u32,
    ) {
        self.ctrl = modifiers.ctrl;
    }
}

impl PointerHandler for App {
    fn pointer_frame(
        &mut self,
        conn: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        let (w, h) = (self.width as f64, self.height as f64);
        let (vx, vy) = render::viewport_origin(w, h);
        let mut dirty = false;

        for event in events {
            let (px, py) = event.position;
            match event.kind {
                PointerEventKind::Enter { .. } => {
                    // The enter serial is required for the first cursor request.
                    let icon = self.cursor_for(px, py);
                    self.cursor = CursorIcon::Default;
                    self.set_cursor(conn, icon);
                }
                PointerEventKind::Motion { .. } => {
                    let icon = self.cursor_for(px, py);
                    self.set_cursor(conn, icon);
                    if self.ui.mode == Mode::Settings {
                        let (cx, cy) = render::card_origin(w, h);
                        let row = render::setting_at(px - cx, py - cy);
                        if row != self.ui.hover_setting {
                            self.ui.hover_setting = row;
                            dirty = true;
                        }
                        continue;
                    }
                    let hover = self.ui.cell_at(px - vx, py - vy);
                    if hover != self.ui.hover {
                        self.ui.hover = hover;
                        dirty = true;
                    }
                }
                PointerEventKind::Leave { .. } => {
                    if self.ui.hover.take().is_some() || self.ui.hover_setting.take().is_some() {
                        dirty = true;
                    }
                }
                PointerEventKind::Press { .. } => {
                    let (cx, cy) = render::card_origin(w, h);
                    let outside = px < cx
                        || py < cy
                        || px >= cx + render::CARD_W
                        || py >= cy + render::CARD_H;
                    if outside {
                        self.exit = true;
                        return;
                    }
                    if self.ui.mode == Mode::Settings {
                        if render::donate_hit(px - cx, py - cy) {
                            open_url(render::DONATE_URL);
                            continue;
                        }
                        if let Some(d) = render::stepper_at(px - cx, py - cy) {
                            let (tone, limit) = {
                                let st = self.store.borrow();
                                (st.settings().skin_tone, st.settings().recent_limit)
                            };
                            self.ui.setting =
                                render::setting_at(px - cx, py - cy).unwrap_or(self.ui.setting);
                            if let Some(action) = self.ui.adjust_setting(d, tone, limit) {
                                self.apply(action);
                            }
                            dirty = true;
                            continue;
                        }
                        if let Some(tone) = render::tone_at(px - cx, py - cy) {
                            self.ui.setting = render::tone_row();
                            self.apply(Action::SetTone(tone));
                            dirty = true;
                            continue;
                        }
                        if let Some(i) = render::setting_at(px - cx, py - cy) {
                            self.ui.setting = i;
                            if let Some(action) = self.ui.activate_setting() {
                                self.apply(action);
                            }
                            dirty = true;
                        }
                        continue;
                    }
                    if render::gear_hit(px - cx, py - cy) {
                        self.ui.open_settings();
                        dirty = true;
                        continue;
                    }
                    if render::search_hit(px - cx, py - cy) {
                        let at = render::search_index_at(self.ui.query.text(), px - cx);
                        self.ui.edit(|q| {
                            q.set_cursor(at);
                            false
                        });
                        dirty = true;
                        continue;
                    }
                    if let Some(i) = render::tab_at(px - cx, py - cy, self.ui.grid.sections.len()) {
                        self.ui.goto_section(i);
                        dirty = true;
                        continue;
                    }
                    if let Some(cell) = self.ui.cell_at(px - vx, py - vy) {
                        self.ui.sel = cell;
                        self.picked = self.ui.selected();
                        self.exit = true;
                        return;
                    }
                }
                PointerEventKind::Axis { vertical, .. } if vertical.absolute != 0.0 => {
                    self.ui.scroll_by(vertical.absolute);
                    dirty = true;
                }
                _ => {}
            }
        }
        if dirty {
            self.draw();
        }
    }
}

impl SeatHandler for App {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        match capability {
            Capability::Keyboard if self.keyboard.is_none() => {
                match self.seat.get_keyboard(qh, &seat, None) {
                    Ok(k) => self.keyboard = Some(k),
                    Err(e) => eprintln!("pika: no keyboard ({e})"),
                }
            }
            Capability::Pointer if self.pointer.is_none() => {
                // A Wayland client draws its own cursor. Without a themed pointer the
                // image is whatever the previously focused surface left behind.
                let surface = self.compositor.create_surface(qh);
                let shm = self.shm.wl_shm().clone();
                match self.seat.get_pointer_with_theme::<_, ()>(
                    qh,
                    &seat,
                    &shm,
                    surface,
                    ThemeSpec::default(),
                ) {
                    Ok(p) => self.pointer = Some(p),
                    Err(e) => eprintln!("pika: no pointer ({e})"),
                }
            }
            _ => {}
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
        match capability {
            Capability::Keyboard => {
                if let Some(k) = self.keyboard.take() {
                    k.release();
                }
            }
            Capability::Pointer => {
                if let Some(p) = self.pointer.take() {
                    p.pointer().release();
                }
            }
            _ => {}
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_registry!(App);

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState, SeatState];
}

smithay_client_toolkit::delegate_dispatch2!(App);
