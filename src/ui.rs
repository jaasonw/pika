use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{gdk, glib};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::emoji;

/// Fixed geometry. The list is virtualized, so the scroll math below has to be able to
/// work out where a section starts without having measured the widgets.
const COLUMNS: usize = 12;
const CELL: i32 = 44;
const HEADER_H: i32 = 30;

/// How many emoji rows to put in the list before handing control back to GTK. The card
/// shows about eight; the rest are appended from an idle callback, which keeps the
/// widget-building for ~170 rows off the path to the first frame.
const HEAD_ROWS: usize = 10;

/// Row payloads are encoded into a `StringList` rather than a custom GObject: one marker
/// byte says whether the row is a section header or a run of emoji, and emoji within a
/// row are separated by the unit separator.
const HEADER_TAG: char = '\u{1}';
const ROW_TAG: char = '\u{2}';
const SEP: char = '\u{1f}';

const CSS: &str = "
/* The surface covers the screen so clicks outside the card can dismiss it; only the
   card itself is painted. */
window.picker { background: transparent; }
.card { background: @theme_bg_color; border-radius: 12px; border: 1px solid alpha(@theme_fg_color, 0.15); }
/* Buttons carry a themed background-image as well as a colour, so both have to be
   cleared for the flat cell look and re-set for the selected one. */
button.emoji-cell { font-size: 24px; padding: 0; min-width: 0; min-height: 0;
  background: none; background-image: none; border: none; box-shadow: none;
  border-radius: 6px; }
button.emoji-cell:hover { background-color: alpha(@theme_fg_color, 0.10); }
button.emoji-cell.sel, button.emoji-cell.sel:hover {
  background-image: none;
  background-color: @theme_selected_bg_color;
  color: @theme_selected_fg_color;
  outline: 2px solid @theme_selected_bg_color; }
.section { font-size: 11px; font-weight: bold; opacity: 0.55; padding: 8px 4px 2px 4px; }
.tabs button { font-size: 17px; padding: 2px 5px; min-height: 0; min-width: 0; }
.footer { font-size: 12px; opacity: 0.7; padding: 2px 8px; }
.gear { background: none; background-image: none; border: none; box-shadow: none;
  opacity: 0.45; padding: 4px; min-width: 0; min-height: 0; }
.gear:hover { opacity: 1; background-color: alpha(@theme_fg_color, 0.10); }
";

/// One entry in the scrolling list. The header's text lives only in the model, since
/// nothing here needs to read it back — the view only cares how tall the item is and
/// whether it holds emoji.
enum Item {
    Header,
    Row(Vec<&'static str>),
}

struct View {
    items: Vec<Item>,
    /// Item index of each emoji row, so navigation can map a grid row to a list position.
    rows: Vec<usize>,
    /// Grid row of each item, or `None` for headers. The inverse of `rows`, kept so the
    /// bind handler does not scan `rows` for every row that scrolls past.
    row_of: Vec<Option<usize>>,
    /// Item index where each section header sits, in tab order. `None` when a section is
    /// absent (no recents yet, or a search is active).
    sections: Vec<Option<usize>>,
    /// Running pixel offset of every item, plus a final total. `offsets[i]` is where item
    /// `i` starts; the scroll handler reads this on every frame, so it is precomputed
    /// rather than summed on demand.
    offsets: Vec<f64>,
}

impl View {
    fn row_cells(&self, r: usize) -> &[&'static str] {
        match self.items.get(self.rows.get(r).copied().unwrap_or(usize::MAX)) {
            Some(Item::Row(cells)) => cells,
            _ => &[],
        }
    }

    /// Pixel offset of an item, given the fixed heights above.
    fn offset(&self, index: usize) -> f64 {
        self.offsets.get(index).copied().unwrap_or(0.0)
    }

    /// Append one item, maintaining the indices that hang off it. Returns its position.
    ///
    /// `offsets` stays one longer than `items`, so the tail entry is always the total
    /// height and `offset(i)` needs no bounds special-casing.
    fn push(&mut self, item: Item) -> usize {
        let at = self.items.len();
        let h = match &item {
            Item::Header => {
                self.row_of.push(None);
                HEADER_H
            }
            Item::Row(_) => {
                self.row_of.push(Some(self.rows.len()));
                self.rows.push(at);
                CELL
            }
        };
        self.items.push(item);
        self.offsets.push(self.offsets[at] + h as f64);
        at
    }
}

/// One unit of list-filling work: a run of emoji, and the section title above it when
/// this is the start of a section rather than the continuation of one that was split to
/// keep the first screen small.
type Chunk = (Option<String>, Vec<&'static str>);

/// Append a run of emoji to the view and to the encoded rows that back it, returning the
/// item index of its header. Empty runs are skipped entirely, which is how "no recents
/// yet" reads as an absent tab rather than a blank one.
///
/// `encoded` is positional: entry `i` describes the item `encoded.len()` behind the end
/// of `view`, so a caller filling a tail can splice it in at the offset it started from.
fn push_section(
    view: &mut View,
    encoded: &mut Vec<String>,
    title: Option<&str>,
    cells: &[&'static str],
) -> Option<usize> {
    if cells.is_empty() {
        return None;
    }
    let at = title.map(|t| {
        let at = view.push(Item::Header);
        encoded.push(format!("{HEADER_TAG}{}", t.to_uppercase()));
        at
    });
    for chunk in cells.chunks(COLUMNS) {
        view.push(Item::Row(chunk.to_vec()));
        encoded.push(format!("{ROW_TAG}{}", chunk.join(&SEP.to_string())));
    }
    at
}

/// Fill from `queue` until `max_rows` emoji rows are in, splitting the run in progress if
/// it would overshoot. Section starts are recorded in `view.sections`; a continuation of
/// a split run records nothing, since it carries no header and no tab points at it.
fn fill(view: &mut View, encoded: &mut Vec<String>, queue: &mut Vec<Chunk>, max_rows: usize) {
    while view.rows.len() < max_rows {
        if queue.is_empty() {
            return;
        }
        let (title, cells) = queue.remove(0);
        // At least one row of room, since the loop guard just passed.
        let room = (max_rows - view.rows.len()) * COLUMNS;
        if cells.len() > room {
            let at = push_section(view, encoded, title.as_deref(), &cells[..room]);
            if title.is_some() {
                view.sections.push(at);
            }
            queue.insert(0, (None, cells[room..].to_vec()));
            return;
        }
        let at = push_section(view, encoded, title.as_deref(), &cells);
        if title.is_some() {
            view.sections.push(at);
        }
    }
}

impl Default for View {
    fn default() -> Self {
        View {
            items: Vec::new(),
            rows: Vec::new(),
            row_of: Vec::new(),
            sections: Vec::new(),
            // The offset of item 0; every push appends the offset of the item after it.
            offsets: vec![0.0],
        }
    }
}

pub struct Picker {
    pub window: gtk::Window,
    entry: gtk::SearchEntry,
    model: gtk::StringList,
    list: gtk::ListView,
    footer: gtk::Label,
    view: RefCell<View>,
    /// Selected cell as (grid row, column).
    sel: Cell<(usize, usize)>,
    recents: RefCell<Vec<String>>,
    /// Fitzpatrick tone applied to emoji that take one; 0 is the default yellow.
    tone: Cell<u8>,
    tabs: RefCell<Vec<gtk::ToggleButton>>,
    /// Set while a tab click is driving the scroll, so the scroll handler does not fight
    /// the button it just activated.
    scrolling: Cell<bool>,
    /// Bumped on every rebuild, so a queued tail fill can tell it has been outrun by a
    /// newer one — a keystroke landing before the idle callback runs.
    fill: Cell<u32>,
}

impl Picker {
    pub fn new(
        app: &gtk::Application,
        recents: Vec<String>,
        on_pick: impl Fn(&str) + 'static,
        on_dismiss: impl Fn() + 'static,
        on_settings: impl Fn() + 'static,
    ) -> Rc<Self> {
        let provider = gtk::CssProvider::new();
        provider.load_from_data(CSS);
        if let Some(display) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }

        let window = gtk::Window::builder()
            .application(app)
            .decorated(false)
            .css_classes(vec!["picker".to_string()])
            .build();

        window.init_layer_shell();
        window.set_layer(Layer::Overlay);
        window.set_keyboard_mode(KeyboardMode::Exclusive);
        window.set_namespace(Some("emoji-picker"));
        // Anchored to every edge, so the surface spans the output: a click landing
        // outside the card is the only way the compositor will tell us the user meant
        // to dismiss us.
        for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            window.set_anchor(edge, true);
        }

        let entry = gtk::SearchEntry::builder()
            .placeholder_text("Search emoji")
            .hexpand(true)
            .build();
        // GtkSearchEntry waits 150ms after the last keystroke before it emits
        // search-changed. Searching costs well under a millisecond, so that delay was the
        // whole of the perceived lag.
        entry.set_search_delay(30);

        let gear = gtk::Button::builder()
            .icon_name("configure")
            .tooltip_text("Settings")
            .focusable(false)
            .valign(gtk::Align::Center)
            .css_classes(vec!["gear".to_string()])
            .build();

        let search_row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .margin_top(8)
            .margin_start(8)
            .margin_end(8)
            .build();
        search_row.append(&entry);
        search_row.append(&gear);

        let tab_bar = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(2)
            .halign(gtk::Align::Center)
            .margin_top(6)
            .css_classes(vec!["tabs".to_string(), "linked".to_string()])
            .build();

        let model = gtk::StringList::new(&[]);
        let selection = gtk::NoSelection::new(Some(model.clone()));
        let list = gtk::ListView::builder()
            .model(&selection)
            .single_click_activate(false)
            .build();

        let scroller = gtk::ScrolledWindow::builder()
            .child(&list)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .margin_start(8)
            .margin_end(8)
            .margin_top(2)
            .build();

        let footer = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(vec!["footer".to_string()])
            .build();

        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(vec!["card".to_string()])
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .width_request(COLUMNS as i32 * CELL + 24)
            .height_request(440)
            .overflow(gtk::Overflow::Hidden)
            .build();
        card.append(&search_row);
        card.append(&tab_bar);
        card.append(&scroller);
        card.append(&footer);
        window.set_child(Some(&card));

        let picker = Rc::new(Picker {
            window: window.clone(),
            entry: entry.clone(),
            model: model.clone(),
            list: list.clone(),
            footer: footer.clone(),
            view: RefCell::new(View::default()),
            sel: Cell::new((0, 0)),
            recents: RefCell::new(recents),
            tone: Cell::new(0),
            tabs: RefCell::new(Vec::new()),
            scrolling: Cell::new(false),
            fill: Cell::new(0),
        });

        let pick = Rc::new(on_pick);
        let dismiss = Rc::new(on_dismiss);

        {
            let p = picker.clone();
            gear.connect_clicked(move |_| {
                // Hide first: the settings window takes the keyboard grab.
                p.window.set_visible(false);
                on_settings();
            });
        }

        // Each list row is either a section header or a strip of emoji buttons. The
        // buttons are built once and rebound as rows scroll past.
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup({
            let p = picker.clone();
            let pick = pick.clone();
            move |_, item| {
                let item = item.downcast_ref::<gtk::ListItem>().unwrap();
                let stack = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                let header = gtk::Label::builder()
                    .xalign(0.0)
                    .height_request(HEADER_H)
                    .css_classes(vec!["section".to_string()])
                    .build();
                let row = gtk::Box::builder()
                    .orientation(gtk::Orientation::Horizontal)
                    .height_request(CELL)
                    .build();
                for _ in 0..COLUMNS {
                    let b = gtk::Button::builder()
                        .width_request(CELL)
                        .height_request(CELL)
                        .css_classes(vec!["emoji-cell".to_string()])
                        .build();
                    let p = p.clone();
                    let pick = pick.clone();
                    b.connect_clicked(move |b| {
                        // The button's label is the emoji itself.
                        let ch = b.label().unwrap_or_default().to_string();
                        if !ch.is_empty() {
                            p.record_selection(&ch);
                            pick(&ch);
                        }
                    });
                    row.append(&b);
                }
                stack.append(&header);
                stack.append(&row);
                item.set_child(Some(&stack));
                item.set_activatable(false);
                item.set_selectable(false);
            }
        });

        factory.connect_bind({
            let p = picker.clone();
            move |_, item| {
                let item = item.downcast_ref::<gtk::ListItem>().unwrap();
                let pos = item.position() as usize;
                let text = item
                    .item()
                    .and_downcast::<gtk::StringObject>()
                    .map(|s| s.string().to_string())
                    .unwrap_or_default();
                let Some(stack) = item.child().and_downcast::<gtk::Box>() else { return };
                let header = stack.first_child().and_downcast::<gtk::Label>().unwrap();
                let row = stack.last_child().and_downcast::<gtk::Box>().unwrap();

                let mut chars = text.chars();
                match chars.next() {
                    Some(HEADER_TAG) => {
                        header.set_label(chars.as_str());
                        header.set_visible(true);
                        row.set_visible(false);
                    }
                    _ => {
                        header.set_visible(false);
                        row.set_visible(true);
                        let cells: Vec<&str> = chars.as_str().split(SEP).collect();
                        // Straight lookup rather than a scan of every row: this runs for
                        // each row that scrolls into view.
                        let grid_row = p.view.borrow().row_of.get(pos).copied().flatten();
                        let mut child = row.first_child();
                        for col in 0..COLUMNS {
                            let Some(b) = child.clone().and_downcast::<gtk::Button>() else { break };
                            child = b.next_sibling();
                            match cells.get(col) {
                                Some(ch) if !ch.is_empty() => {
                                    b.set_label(ch);
                                    b.set_visible(true);
                                }
                                _ => {
                                    b.set_label("");
                                    b.set_visible(false);
                                }
                            }
                            let selected = grid_row == Some(p.sel.get().0) && col == p.sel.get().1;
                            if selected {
                                b.add_css_class("sel");
                            } else {
                                b.remove_css_class("sel");
                            }
                        }
                    }
                }
            }
        });

        list.set_factory(Some(&factory));

        // Tabs jump to a section rather than filtering: the list always holds everything.
        let mut first: Option<gtk::ToggleButton> = None;
        for (i, (label, tip)) in std::iter::once(("\u{1f550}", "Recently used"))
            .chain(emoji::GROUPS.iter().map(|g| (group_icon(g), *g)))
            .enumerate()
        {
            let b = gtk::ToggleButton::builder()
                .label(label)
                .focusable(false)
                .tooltip_text(tip)
                .build();
            match &first {
                Some(f) => b.set_group(Some(f)),
                None => first = Some(b.clone()),
            }
            let p = picker.clone();
            b.connect_clicked(move |_| p.scroll_to_section(i));
            tab_bar.append(&b);
            picker.tabs.borrow_mut().push(b);
        }

        // Scrolling updates which tab reads as active.
        {
            let p = picker.clone();
            scroller.vadjustment().connect_value_changed(move |adj| {
                if !p.scrolling.get() {
                    p.sync_active_tab(adj.value());
                }
            });
        }

        {
            let p = picker.clone();
            entry.connect_search_changed(move |_| p.rebuild());
        }

        // The entry keeps focus so typing always filters; navigation keys are
        // intercepted here and applied to the selection by hand.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let p = picker.clone();
            let pick = pick.clone();
            let dismiss = dismiss.clone();
            keys.connect_key_pressed(move |_, key, _, state| {
                let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
                let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
                match key {
                    gdk::Key::Escape => {
                        dismiss();
                        glib::Propagation::Stop
                    }
                    gdk::Key::Return | gdk::Key::KP_Enter => {
                        if let Some(ch) = p.selected_char() {
                            p.record_selection(&ch);
                            pick(&ch);
                        }
                        glib::Propagation::Stop
                    }
                    gdk::Key::Tab | gdk::Key::ISO_Left_Tab => {
                        p.cycle_section(shift);
                        glib::Propagation::Stop
                    }
                    gdk::Key::Left => p.move_by(0, -1),
                    gdk::Key::Right => p.move_by(0, 1),
                    gdk::Key::Up => p.move_by(-1, 0),
                    gdk::Key::Down => p.move_by(1, 0),
                    gdk::Key::Page_Up => p.move_by(-6, 0),
                    gdk::Key::Page_Down => p.move_by(6, 0),
                    gdk::Key::Home if ctrl => p.move_to((0, 0)),
                    gdk::Key::End if ctrl => {
                        let last = p.view.borrow().rows.len().saturating_sub(1);
                        p.move_to((last, 0))
                    }
                    _ => glib::Propagation::Proceed,
                }
            });
        }
        window.add_controller(keys);

        // Click outside the card dismisses, the way a menu does.
        let click = gtk::GestureClick::new();
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let card = card.clone();
            let window = window.clone();
            click.connect_pressed(move |_, _, x, y| {
                if let Some(bounds) = card.compute_bounds(&window) {
                    if !bounds.contains_point(&gtk::graphene::Point::new(x as f32, y as f32)) {
                        dismiss();
                    }
                }
            });
        }
        window.add_controller(click);

        // No rebuild here: every caller presents the window straight afterwards, and
        // present() fills the list itself. Doing it twice cost a full model splice.
        picker
    }

    /// Bring the window up ready for input. Also used by the daemon on each show.
    pub fn present(self: &Rc<Self>, recents: Vec<String>, tone: u8) {
        self.tone.set(tone);
        *self.recents.borrow_mut() = recents;
        // Clearing the box emits search-changed, which rebuilds on its own; only pay for
        // that when there is actually something to clear.
        if self.entry.text().is_empty() {
            self.rebuild();
        } else {
            self.entry.set_text("");
        }
        self.window.present();
        self.entry.grab_focus();
    }

    /// A one-line summary of the filled list, for `--time-launch`.
    pub fn debug_state(&self) -> String {
        let view = self.view.borrow();
        let sections: Vec<String> = view
            .sections
            .iter()
            .map(|s| s.map_or("-".into(), |i| i.to_string()))
            .collect();
        format!(
            "model={} items={} rows={} height={} sections=[{}]",
            self.model.n_items(),
            view.items.len(),
            view.rows.len(),
            view.offset(view.items.len()),
            sections.join(","),
        )
    }

    fn record_selection(&self, ch: &str) {
        // Keep the in-memory recents current so a daemon's next show is right even
        // before the store is reloaded.
        let mut r = self.recents.borrow_mut();
        r.retain(|x| x != ch);
        r.insert(0, ch.to_string());
    }

    fn selected_char(&self) -> Option<String> {
        let (r, c) = self.sel.get();
        self.view.borrow().row_cells(r).get(c).map(|s| s.to_string())
    }

    /// Rebuild the list: recents pinned first, then every category in order, or the
    /// search results when the box has text.
    ///
    /// A `GtkListView` builds a widget for every item it is handed, up to a working set
    /// of roughly 200 — and the browse list is only 177 rows, so a full splice builds all
    /// of them. Each row is a dozen cells, which is why that splice costs tens of
    /// milliseconds. So the list is filled in two parts: enough to cover the viewport
    /// now, and the remainder from an idle callback once the window is up.
    fn rebuild(self: &Rc<Self>) {
        let query = self.entry.text().trim().to_string();
        let tone = self.tone.get();
        let searching = !query.is_empty();

        // Anything an earlier fill queued belongs to a list that no longer exists.
        self.fill.set(self.fill.get().wrapping_add(1));
        let generation = self.fill.get();

        // Each section, in tab order, as a title and the cells under it.
        let mut queue: Vec<Chunk> = Vec::new();
        if searching {
            let hits = emoji::search(&query).iter().map(|e| e.toned(tone)).collect();
            queue.push((Some("results".into()), hits));
        } else {
            let recents = self
                .recents
                .borrow()
                .iter()
                .filter_map(|ch| emoji::find(ch).map(|e| e.ch))
                .collect();
            queue.push((Some("recents".into()), recents));
            for group in emoji::GROUPS {
                let cells = emoji::by_group(group).map(|e| e.toned(tone)).collect();
                queue.push((Some(group.to_string()), cells));
            }
        }

        // Enough to cover the viewport now; the rest waits for an idle turn.
        let mut view = View::default();
        let mut encoded: Vec<String> = Vec::new();
        fill(&mut view, &mut encoded, &mut queue, HEAD_ROWS);

        // A search has no meaningful sections, but the tab bar still expects one slot per
        // tab, so none of them light up.
        if searching {
            view.sections = vec![None; emoji::GROUPS.len() + 1];
        }

        self.sel.set((0, 0));
        let next = encoded.len() as u32;
        *self.view.borrow_mut() = view;

        let refs: Vec<&str> = encoded.iter().map(|s| s.as_str()).collect();
        self.model.splice(0, self.model.n_items(), &refs);
        self.update_footer();

        if queue.is_empty() {
            return;
        }

        // The rest goes on once the window has had its chance to draw. Appending leaves
        // the rows already built alone, so this costs nothing that was paid up front.
        let p = self.clone();
        glib::idle_add_local_once(move || {
            if p.fill.get() != generation {
                return;
            }
            let mut encoded: Vec<String> = Vec::new();
            // Continuations carry no title, so this appends rows without touching
            // `sections` — the head already recorded every one that a tab points at.
            fill(&mut p.view.borrow_mut(), &mut encoded, &mut queue, usize::MAX);
            let refs: Vec<&str> = encoded.iter().map(|s| s.as_str()).collect();
            p.model.splice(next, 0, &refs);
        });
    }

    fn move_by(&self, dr: i32, dc: i32) -> glib::Propagation {
        let (r, c) = self.sel.get();
        let view = self.view.borrow();
        let rows = view.rows.len();
        if rows == 0 {
            return glib::Propagation::Stop;
        }

        let (mut r, mut c) = (r as i32 + dr, c as i32 + dc);
        // Horizontal movement wraps across row boundaries, so holding Right walks the
        // whole grid the way it reads.
        if c < 0 {
            r -= 1;
            c = COLUMNS as i32 - 1;
        } else if c >= COLUMNS as i32 {
            r += 1;
            c = 0;
        }
        let r = r.clamp(0, rows as i32 - 1) as usize;
        let len = view.row_cells(r).len() as i32;
        let c = c.clamp(0, (len - 1).max(0)) as usize;
        drop(view);
        self.move_to((r, c))
    }

    fn move_to(&self, next: (usize, usize)) -> glib::Propagation {
        let prev = self.sel.get();
        if prev == next {
            return glib::Propagation::Stop;
        }
        self.sel.set(next);
        self.refresh_row(prev.0);
        if next.0 != prev.0 {
            self.refresh_row(next.0);
        }
        self.scroll_into_view(next.0);
        self.update_footer();
        glib::Propagation::Stop
    }

    /// Re-emit one row so the factory rebinds it and repaints the highlight.
    ///
    /// Splicing the row's own text back over itself is the only way to make a
    /// `StringList` say items-changed; tracking the live widgets by hand instead went
    /// stale as soon as the list recycled one.
    fn refresh_row(&self, r: usize) {
        let view = self.view.borrow();
        let Some(&item) = view.rows.get(r) else { return };
        drop(view);
        let pos = item as u32;
        let Some(text) = self.model.string(pos) else { return };
        self.model.splice(pos, 1, &[text.as_str()]);
    }

    fn update_footer(&self) {
        let text = self
            .selected_char()
            .and_then(|ch| emoji::find(&ch))
            .map(|e| format!("{}  {}", e.ch, e.name))
            .unwrap_or_default();
        self.footer.set_label(&text);
    }

    fn adjustment(&self) -> Option<gtk::Adjustment> {
        self.list
            .parent()
            .and_downcast::<gtk::ScrolledWindow>()
            .map(|s| s.vadjustment())
    }

    /// Keep the selected row on screen without disturbing the scroll otherwise.
    fn scroll_into_view(&self, r: usize) {
        let Some(adj) = self.adjustment() else { return };
        let view = self.view.borrow();
        let Some(&item) = view.rows.get(r) else { return };
        let top = view.offset(item);
        let bottom = top + CELL as f64;
        // A row at the top of a section should show its header too.
        let target = if top < adj.value() {
            (top - HEADER_H as f64).max(0.0)
        } else if bottom > adj.value() + adj.page_size() {
            bottom - adj.page_size()
        } else {
            return;
        };
        adj.set_value(target);
    }

    fn scroll_to_section(&self, index: usize) {
        let Some(adj) = self.adjustment() else { return };
        let at = self.view.borrow().sections.get(index).copied().flatten();
        let Some(at) = at else { return };
        let target = self.view.borrow().offset(at);
        self.scrolling.set(true);
        adj.set_value(target.min(adj.upper() - adj.page_size()));
        self.scrolling.set(false);
    }

    fn cycle_section(&self, back: bool) {
        let tabs = self.tabs.borrow();
        let cur = tabs.iter().position(|b| b.is_active()).unwrap_or(0);
        let present: Vec<usize> = self
            .view
            .borrow()
            .sections
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.map(|_| i))
            .collect();
        if present.is_empty() {
            return;
        }
        let here = present.iter().position(|&i| i == cur).unwrap_or(0);
        let next = if back {
            (here + present.len() - 1) % present.len()
        } else {
            (here + 1) % present.len()
        };
        let target = present[next];
        drop(tabs);
        if let Some(b) = self.tabs.borrow().get(target) {
            b.set_active(true);
        }
        self.scroll_to_section(target);
    }

    /// Light up whichever tab owns the section currently at the top of the viewport.
    fn sync_active_tab(&self, value: f64) {
        let view = self.view.borrow();
        let mut active = None;
        for (i, at) in view.sections.iter().enumerate() {
            let Some(at) = at else { continue };
            if view.offset(*at) <= value + 1.0 {
                active = Some(i);
            }
        }
        drop(view);
        if let Some(i) = active {
            if let Some(b) = self.tabs.borrow().get(i) {
                if !b.is_active() {
                    b.set_active(true);
                }
            }
        }
    }
}

fn group_icon(group: &str) -> &'static str {
    match group {
        "Smileys & Emotion" => "\u{1f600}",
        "People & Body" => "\u{1f44b}",
        "Animals & Nature" => "\u{1f43b}",
        "Food & Drink" => "\u{1f34e}",
        "Travel & Places" => "\u{2708}\u{fe0f}",
        "Activities" => "\u{26bd}",
        "Objects" => "\u{1f4a1}",
        "Symbols" => "\u{1f523}",
        "Flags" => "\u{1f6a9}",
        _ => "\u{2753}",
    }
}
