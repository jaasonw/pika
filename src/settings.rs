//! The settings window.
//!
//! A second layer-shell surface rather than a plain toplevel: the picker sits on the
//! overlay layer, and an ordinary window would be painted underneath it.

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::glib;
use gtk4_layer_shell::{KeyboardMode, Layer, LayerShell};
use std::cell::RefCell;
use std::rc::Rc;

use crate::store::{RECENT_LIMIT_RANGE, Store};

const CSS: &str = "
.settings-card { background: @theme_bg_color; border-radius: 12px;
  border: 1px solid alpha(@theme_fg_color, 0.15); padding: 18px; }
.settings-title { font-size: 15px; font-weight: bold; }
.settings-group { font-size: 11px; font-weight: bold; opacity: 0.55; padding-top: 10px; }
.settings-hint { font-size: 11px; opacity: 0.6; }
";

/// Build and show the settings window. `on_close` runs when it is dismissed, so the
/// caller can bring the picker back.
pub fn present(
    app: &gtk::Application,
    store: Rc<RefCell<Store>>,
    on_close: impl Fn() + 'static,
) {
    let provider = gtk::CssProvider::new();
    provider.load_from_data(CSS);
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }

    let window = gtk::Window::builder().application(app).decorated(false).build();
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_keyboard_mode(KeyboardMode::Exclusive);
    window.set_namespace(Some("emoji-picker-settings"));

    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .css_classes(vec!["settings-card".to_string()])
        .width_request(420)
        .spacing(4)
        .build();

    let title = gtk::Label::builder()
        .label("Emoji Picker Settings")
        .xalign(0.0)
        .css_classes(vec!["settings-title".to_string()])
        .build();
    card.append(&title);

    card.append(&group_label("Insertion"));

    let insert = switch_row(
        "Insert into the focused field",
        "Off means the emoji is only copied, for you to paste yourself.",
        store.borrow().settings().insert,
    );
    let copy = switch_row(
        "Always copy to the clipboard too",
        "On by necessity when a direct insert is not possible.",
        store.borrow().settings().always_copy,
    );
    card.append(&insert.0);
    card.append(&copy.0);

    {
        let store = store.clone();
        insert.1.connect_active_notify(move |s| {
            let mut st = store.borrow_mut();
            st.settings_mut().insert = s.is_active();
            st.save();
        });
    }
    {
        let store = store.clone();
        copy.1.connect_active_notify(move |s| {
            let mut st = store.borrow_mut();
            st.settings_mut().always_copy = s.is_active();
            st.save();
        });
    }

    card.append(&group_label("Recents"));

    let limit_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .margin_top(6)
        .build();
    let limit_text = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .build();
    limit_text.append(&gtk::Label::builder().label("How many to remember").xalign(0.0).build());
    // Step by a full row, so the section never ends in a ragged half row.
    let limit = gtk::SpinButton::with_range(
        RECENT_LIMIT_RANGE.0 as f64,
        RECENT_LIMIT_RANGE.1 as f64,
        12.0,
    );
    limit.set_value(store.borrow().settings().recent_limit as f64);
    limit.set_valign(gtk::Align::Center);
    limit_row.append(&limit_text);
    limit_row.append(&limit);
    card.append(&limit_row);

    {
        let store = store.clone();
        limit.connect_value_changed(move |sb| {
            let mut st = store.borrow_mut();
            st.set_recent_limit(sb.value() as usize);
            st.save();
        });
    }

    let clear = gtk::Button::with_label("Clear Recent");
    clear.set_halign(gtk::Align::Start);
    {
        let store = store.clone();
        let clear_btn = clear.clone();
        clear.connect_clicked(move |_| {
            let mut st = store.borrow_mut();
            st.clear_recents();
            st.save();
            clear_btn.set_label("Recents cleared");
            clear_btn.set_sensitive(false);
        });
    }
    card.append(&clear);

    card.append(&group_label("Paste permission"));

    let reset = gtk::Button::with_label("Reset paste permission");
    reset.set_halign(gtk::Align::Start);
    reset.set_margin_top(6);
    {
        let store = store.clone();
        let reset_btn = reset.clone();
        reset.connect_clicked(move |_| {
            let mut st = store.borrow_mut();
            st.clear_restore_token();
            st.save();
            reset_btn.set_label("KDE will ask again on the next fallback paste");
            reset_btn.set_sensitive(false);
        });
    }
    card.append(&reset);
    card.append(
        &gtk::Label::builder()
            .label("Only used for apps that cannot take a direct insert.")
            .xalign(0.0)
            .wrap(true)
            .css_classes(vec!["settings-hint".to_string()])
            .build(),
    );

    let close = gtk::Button::with_label("Close");
    close.set_halign(gtk::Align::End);
    close.set_margin_top(16);
    card.append(&close);
    window.set_child(Some(&card));

    let on_close = Rc::new(on_close);
    let shut = {
        let window = window.clone();
        let on_close = on_close.clone();
        move || {
            window.set_visible(false);
            on_close();
        }
    };

    {
        let shut = shut.clone();
        close.connect_clicked(move |_| shut());
    }

    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let shut = shut.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                shut();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
    }
    window.add_controller(keys);

    window.present();
}

fn group_label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text.to_uppercase())
        .xalign(0.0)
        .css_classes(vec!["settings-group".to_string()])
        .build()
}

/// A label-and-hint block with a switch on the right.
fn switch_row(title: &str, hint: &str, active: bool) -> (gtk::Box, gtk::Switch) {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .margin_top(6)
        .build();
    let text = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .build();
    text.append(&gtk::Label::builder().label(title).xalign(0.0).build());
    text.append(
        &gtk::Label::builder()
            .label(hint)
            .xalign(0.0)
            .wrap(true)
            .css_classes(vec!["settings-hint".to_string()])
            .build(),
    );
    let sw = gtk::Switch::builder().active(active).valign(gtk::Align::Center).build();
    row.append(&text);
    row.append(&sw);
    (row, sw)
}
