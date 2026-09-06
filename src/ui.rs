use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{gdk, glib};
use gtk4_layer_shell::{KeyboardMode, Layer, LayerShell};
use std::cell::RefCell;
use std::rc::Rc;

use crate::emoji;

/// Fixed column count: the arrow-key math below needs to know the grid width, and
/// GridView does not report the count it settled on.
const COLUMNS: u32 = 12;
const CELL: i32 = 44;

const CSS: &str = "
window.picker { background: @theme_bg_color; border-radius: 12px; }
.emoji-cell { font-size: 24px; padding: 4px; }
.tabs button { font-size: 18px; padding: 2px 6px; min-height: 0; min-width: 0; }
.footer { font-size: 12px; opacity: 0.7; padding: 2px 8px; }
";

pub struct Picker {
    pub window: gtk::Window,
    entry: gtk::SearchEntry,
    model: gtk::StringList,
    selection: gtk::SingleSelection,
    /// None = the Recents tab.
    category: Rc<RefCell<Option<&'static str>>>,
    recents: Rc<RefCell<Vec<String>>>,
}

impl Picker {
    pub fn new(app: &gtk::Application, recents: Vec<String>, on_pick: impl Fn(&str) + 'static) -> Rc<Self> {
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
            .default_width(COLUMNS as i32 * CELL + 24)
            .default_height(400)
            .resizable(false)
            .css_classes(vec!["picker".to_string()])
            .build();

        window.init_layer_shell();
        window.set_layer(Layer::Overlay);
        window.set_keyboard_mode(KeyboardMode::Exclusive);
        window.set_namespace(Some("emoji-picker"));
        // No anchors set, so the compositor centers the surface.

        let entry = gtk::SearchEntry::builder()
            .placeholder_text("Search emoji")
            .margin_top(8)
            .margin_start(8)
            .margin_end(8)
            .build();

        let tabs = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(2)
            .halign(gtk::Align::Center)
            .margin_top(6)
            .css_classes(vec!["tabs".to_string(), "linked".to_string()])
            .build();

        let model = gtk::StringList::new(&[]);
        let selection = gtk::SingleSelection::new(Some(model.clone()));
        selection.set_autoselect(true);

        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let label = gtk::Label::builder()
                .css_classes(vec!["emoji-cell".to_string()])
                .width_request(CELL)
                .height_request(CELL)
                .build();
            item.downcast_ref::<gtk::ListItem>().unwrap().set_child(Some(&label));
        });
        factory.connect_bind(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let ch = item
                .item()
                .and_downcast::<gtk::StringObject>()
                .map(|s| s.string().to_string())
                .unwrap_or_default();
            if let Some(label) = item.child().and_downcast::<gtk::Label>() {
                label.set_label(&ch);
                label.set_tooltip_text(emoji::find(&ch).map(|e| e.name));
            }
        });

        let grid = gtk::GridView::builder()
            .model(&selection)
            .factory(&factory)
            .min_columns(COLUMNS)
            .max_columns(COLUMNS)
            .single_click_activate(true)
            .build();

        let scroller = gtk::ScrolledWindow::builder()
            .child(&grid)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .margin_start(8)
            .margin_end(8)
            .margin_top(6)
            .build();

        let footer = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(vec!["footer".to_string()])
            .build();

        let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
        vbox.append(&entry);
        vbox.append(&tabs);
        vbox.append(&scroller);
        vbox.append(&footer);
        window.set_child(Some(&vbox));

        let picker = Rc::new(Picker {
            window: window.clone(),
            entry: entry.clone(),
            model: model.clone(),
            selection: selection.clone(),
            category: Rc::new(RefCell::new(None)),
            recents: Rc::new(RefCell::new(recents)),
        });

        // Tab bar: one leading Recents tab, then the Unicode groups.
        let buttons: Rc<RefCell<Vec<gtk::ToggleButton>>> = Rc::new(RefCell::new(Vec::new()));
        let mut first: Option<gtk::ToggleButton> = None;
        for (i, (label, group)) in std::iter::once(("\u{1f550}", None))
            .chain(emoji::GROUPS.iter().map(|g| (group_icon(g), Some(*g))))
            .enumerate()
        {
            let b = gtk::ToggleButton::builder()
                .label(label)
                .focusable(false)
                .tooltip_text(group.unwrap_or("Recently used"))
                .build();
            if let Some(f) = &first {
                b.set_group(Some(f));
            } else {
                first = Some(b.clone());
            }
            // Open on Recents, unless there are none yet.
            if i == usize::from(picker.recents.borrow().is_empty()) {
                b.set_active(true);
            }
            let p = picker.clone();
            b.connect_toggled(move |b| {
                if b.is_active() {
                    *p.category.borrow_mut() = group;
                    p.entry.set_text("");
                    p.refresh();
                }
            });
            tabs.append(&b);
            buttons.borrow_mut().push(b);
        }

        {
            let p = picker.clone();
            entry.connect_search_changed(move |_| p.refresh());
        }

        // Selection drives the footer caption.
        {
            let footer = footer.clone();
            selection.connect_selected_item_notify(move |sel| {
                let text = sel
                    .selected_item()
                    .and_downcast::<gtk::StringObject>()
                    .and_then(|s| emoji::find(&s.string()))
                    .map(|e| format!("{}  {}", e.ch, e.name))
                    .unwrap_or_default();
                footer.set_label(&text);
            });
        }

        let pick = Rc::new(on_pick);

        {
            let p = picker.clone();
            let pick = pick.clone();
            grid.connect_activate(move |_, pos| {
                if let Some(ch) = p.item_at(pos) {
                    pick(&ch);
                }
            });
        }

        // The entry keeps focus so typing always filters; navigation keys are
        // intercepted here and applied to the grid selection by hand.
        let keys = gtk::EventControllerKey::new();
        {
            let p = picker.clone();
            let pick = pick.clone();
            let buttons = buttons.clone();
            keys.connect_key_pressed(move |_, key, _, state| {
                let n = p.model.n_items();
                let cur = p.selection.selected();
                let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
                let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
                match key {
                    gdk::Key::Escape => {
                        p.window.close();
                        glib::Propagation::Stop
                    }
                    gdk::Key::Return | gdk::Key::KP_Enter => {
                        if let Some(ch) = p.item_at(cur) {
                            pick(&ch);
                        }
                        glib::Propagation::Stop
                    }
                    gdk::Key::Tab | gdk::Key::ISO_Left_Tab => {
                        cycle_tabs(&buttons.borrow(), shift);
                        glib::Propagation::Stop
                    }
                    _ if n == 0 => glib::Propagation::Proceed,
                    gdk::Key::Left => p.select(cur.saturating_sub(1)),
                    gdk::Key::Right => p.select((cur + 1).min(n - 1)),
                    gdk::Key::Up => p.select(cur.saturating_sub(COLUMNS)),
                    gdk::Key::Down => p.select((cur + COLUMNS).min(n - 1)),
                    gdk::Key::Page_Up => p.select(cur.saturating_sub(COLUMNS * 5)),
                    gdk::Key::Page_Down => p.select((cur + COLUMNS * 5).min(n - 1)),
                    gdk::Key::Home if ctrl => p.select(0),
                    gdk::Key::End if ctrl => p.select(n - 1),
                    _ => glib::Propagation::Proceed,
                }
            });
        }
        window.add_controller(keys);

        picker.refresh();
        picker
    }

    /// Bring the window up ready for input. Also used by the daemon on each show.
    pub fn present(&self, recents: Vec<String>) {
        *self.recents.borrow_mut() = recents;
        self.entry.set_text("");
        self.refresh();
        self.window.present();
        self.entry.grab_focus();
    }

    fn item_at(&self, pos: u32) -> Option<String> {
        if pos == gtk::INVALID_LIST_POSITION {
            return None;
        }
        self.model.string(pos).map(|s| s.to_string())
    }

    fn select(&self, pos: u32) -> glib::Propagation {
        self.selection.set_selected(pos);
        glib::Propagation::Stop
    }

    /// Rebuild the visible set from the search text, or from the active tab when the
    /// search box is empty.
    fn refresh(&self) {
        let query = self.entry.text().to_string();
        let items: Vec<String> = if !query.trim().is_empty() {
            emoji::search(query.trim()).iter().map(|e| e.ch.to_string()).collect()
        } else if let Some(group) = *self.category.borrow() {
            emoji::by_group(group).map(|e| e.ch.to_string()).collect()
        } else {
            self.recents.borrow().clone()
        };

        let refs: Vec<&str> = items.iter().map(|s| s.as_str()).collect();
        self.model.splice(0, self.model.n_items(), &refs);
        if !refs.is_empty() {
            self.selection.set_selected(0);
        }
    }
}

fn cycle_tabs(buttons: &[gtk::ToggleButton], back: bool) {
    let Some(i) = buttons.iter().position(|b| b.is_active()) else { return };
    let n = buttons.len();
    let next = if back { (i + n - 1) % n } else { (i + 1) % n };
    buttons[next].set_active(true);
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
