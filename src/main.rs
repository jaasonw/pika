mod emoji;
mod insert;
mod ipc;
mod store;
mod ui;

use gtk4 as gtk;
use gtk::prelude::*;
use gtk4::glib;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

const USAGE: &str = "\
emoji-picker - grid emoji picker for KDE Wayland

usage: emoji-picker [options]

  (no options)   show the picker once, insert the choice, exit
  --daemon       stay resident; later invocations pop the existing window instantly
  --no-paste     copy to the clipboard only, never synthesize Ctrl+V
  --print        write the chosen emoji to stdout as well
  -h, --help     this text
";

/// Upper bound on waiting for the portal, so a hung D-Bus call cannot wedge the app.
const PASTE_TIMEOUT: Duration = Duration::from_secs(20);

fn main() -> glib::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return glib::ExitCode::SUCCESS;
    }
    let daemon = args.iter().any(|a| a == "--daemon");
    let no_paste = args.iter().any(|a| a == "--no-paste");
    let print = args.iter().any(|a| a == "--print");

    // A running daemon owns the UI; hand off to it and exit.
    if !daemon && ipc::request_show() {
        return glib::ExitCode::SUCCESS;
    }

    let app = gtk::Application::builder()
        .application_id("dev.jason.EmojiPicker")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE | gtk::gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    // We parse argv ourselves; keep GApplication from rejecting our flags.
    app.connect_command_line(|app, _| {
        app.activate();
        glib::ExitCode::SUCCESS
    });

    app.connect_activate(move |app| {
        let st = Rc::new(RefCell::new(store::Store::load()));
        let agent = Rc::new(RefCell::new(if no_paste {
            None
        } else {
            Some(insert::PasteAgent::spawn(
                st.borrow().restore_token().map(str::to_owned),
            ))
        }));

        let picker: Rc<RefCell<Option<Rc<ui::Picker>>>> = Rc::new(RefCell::new(None));

        let on_pick = {
            let app = app.clone();
            let st = st.clone();
            let agent = agent.clone();
            let picker = picker.clone();
            move |ch: &str| {
                let ch = ch.to_string();
                if print {
                    println!("{ch}");
                }
                st.borrow_mut().record_use(&ch);

                if let Err(e) = insert::copy_to_clipboard(&ch) {
                    insert::notify(&format!("Could not set the clipboard: {e}"));
                }

                // Hide first: the paste has to land in whatever had focus before us.
                if let Some(p) = picker.borrow().as_ref() {
                    p.window.set_visible(false);
                }

                let app = app.clone();
                let st = st.clone();
                let agent = agent.clone();
                let picker = picker.clone();
                // Let the hide reach the compositor before any key event goes out.
                glib::timeout_add_local_once(Duration::from_millis(30), move || {
                    finish(&app, &st, &agent, &picker, daemon, no_paste);
                });
            }
        };

        let p = ui::Picker::new(app, st.borrow().recents().to_vec(), on_pick);
        *picker.borrow_mut() = Some(p.clone());

        if daemon {
            let picker = picker.clone();
            let st = st.clone();
            let _ = ipc::serve(move || {
                if let Some(p) = picker.borrow().as_ref() {
                    p.present(st.borrow().recents().to_vec());
                }
            });
            // Hold the process open while the window is hidden. The guard must outlive
            // the closure, and the daemon lives until the process exits anyway.
            std::mem::forget(app.hold());
        }

        p.present(st.borrow().recents().to_vec());

        // Closing the window without picking should still end a one-shot run.
        if !daemon {
            let app = app.clone();
            p.window.connect_close_request(move |_| {
                app.quit();
                glib::Propagation::Proceed
            });
        }
    });

    let code = app.run_with_args::<String>(&[]);
    if daemon {
        ipc::cleanup();
    }
    code
}

/// Runs after the window is hidden: paste if we can, then quit or go back to sleep.
fn finish(
    app: &gtk::Application,
    st: &Rc<RefCell<store::Store>>,
    agent: &Rc<RefCell<Option<insert::PasteAgent>>>,
    picker: &Rc<RefCell<Option<Rc<ui::Picker>>>>,
    daemon: bool,
    no_paste: bool,
) {
    let mut pasted = false;
    if let Some(a) = agent.borrow().as_ref() {
        match a.paste_now(PASTE_TIMEOUT) {
            insert::Reply::Pasted(token) => {
                let mut st = st.borrow_mut();
                st.set_restore_token(token);
                st.set_paste_denied(false);
                pasted = true;
            }
            insert::Reply::Failed(e) => {
                eprintln!("emoji-picker: paste failed: {e}");
                let mut st = st.borrow_mut();
                // Only explain the fallback the first time it happens.
                if !st.paste_denied() {
                    insert::notify("Copied. Press Ctrl+V to insert.\nAllow \"Remote Desktop\" to paste automatically.");
                    st.set_paste_denied(true);
                }
            }
        }
    }
    if !pasted && no_paste {
        insert::notify("Copied to clipboard.");
    }
    st.borrow().save();

    if daemon {
        // The session is single-use once started; get a fresh agent for the next pick.
        let token = st.borrow().restore_token().map(str::to_owned);
        *agent.borrow_mut() = if no_paste {
            None
        } else {
            Some(insert::PasteAgent::spawn(token))
        };
        let _ = picker;
    } else {
        app.quit();
    }
}
