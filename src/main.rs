mod emoji;
mod im;
mod insert;
mod ipc;
mod settings;
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
  --no-insert    copy to the clipboard only, never insert into the focused field
  --copy         also put the emoji on the clipboard when it was inserted directly
  --print        write the chosen emoji to stdout as well
  --test-im      try the input-method insert on its own and report
  --test-paste   try the portal paste path on its own and report each step
  -h, --help     this text
";

/// Upper bound on waiting for the portal, so a hung D-Bus call cannot wedge the app.
const PASTE_TIMEOUT: Duration = Duration::from_secs(20);
/// How long to wait for a text field to offer us input-method focus before deciding this
/// app does not speak text-input and falling back to the portal.
const IM_WAIT: Duration = Duration::from_millis(400);

fn main() -> glib::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return glib::ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "--test-im") {
        im::test_commit();
        return glib::ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "--test-paste") {
        insert::test_paste(20, 500);
        return glib::ExitCode::SUCCESS;
    }
    let daemon = args.iter().any(|a| a == "--daemon");
    let no_paste = args.iter().any(|a| a == "--no-insert" || a == "--no-paste");
    let always_copy = args.iter().any(|a| a == "--copy");
    let print = args.iter().any(|a| a == "--print");

    // Another instance already owns the window: tell it to toggle and get out of the
    // way, so a second hotkey press closes the picker rather than opening a second one.
    if !daemon && ipc::request_toggle() {
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
        // The portal agent is spawned only if the input-method route fails: opening a
        // RemoteDesktop session is what raises KDE's permission dialog and its
        // "remote control" notification, and most inserts never need it.
        let agent: Rc<RefCell<Option<insert::PasteAgent>>> = Rc::new(RefCell::new(None));

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

                // Hide first: the insert has to land in whatever had focus before us.
                if let Some(p) = picker.borrow().as_ref() {
                    p.window.set_visible(false);
                }

                let app = app.clone();
                let st = st.clone();
                let agent = agent.clone();
                let picker = picker.clone();
                // Let the hide reach the compositor before any key event goes out.
                glib::timeout_add_local_once(Duration::from_millis(30), move || {
                    let prefs = {
                        let st = st.borrow();
                        (st.settings().insert, st.settings().always_copy)
                    };
                    finish(Ctx {
                        ch: &ch,
                        app: &app,
                        st: &st,
                        agent: &agent,
                        picker: &picker,
                        daemon,
                        // Flags win for a single run; otherwise the saved settings do.
                        no_paste: no_paste || !prefs.0,
                        always_copy: always_copy || prefs.1,
                    });
                });
            }
        };

        let on_dismiss = {
            let app = app.clone();
            let picker = picker.clone();
            move || {
                if let Some(p) = picker.borrow().as_ref() {
                    p.window.set_visible(false);
                }
                if !daemon {
                    app.quit();
                }
            }
        };

        let on_settings = {
            let app = app.clone();
            let st = st.clone();
            let picker = picker.clone();
            move || {
                let st_inner = st.clone();
                let picker = picker.clone();
                settings::present(&app, st.clone(), move || {
                    // Back to the picker, with any cleared recents reflected.
                    if let Some(p) = picker.borrow().as_ref() {
                        p.present(st_inner.borrow().recents().to_vec());
                    }
                });
            }
        };

        let p = ui::Picker::new(
            app,
            st.borrow().recents().to_vec(),
            on_pick,
            on_dismiss,
            on_settings,
        );
        *picker.borrow_mut() = Some(p.clone());

        // Both modes serve the socket, so the hotkey toggles in either one.
        {
            let picker = picker.clone();
            let st = st.clone();
            let app = app.clone();
            let _ = ipc::serve(move || {
                let Some(p) = picker.borrow().clone() else { return };
                if p.window.is_visible() {
                    p.window.set_visible(false);
                    if !daemon {
                        app.quit();
                    }
                } else {
                    p.present(st.borrow().recents().to_vec());
                }
            });
        }

        if daemon {
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
    // We owned the socket in either mode; leaving it behind would make the next
    // invocation think an instance is still up.
    ipc::cleanup();
    code
}

struct Ctx<'a> {
    ch: &'a str,
    app: &'a gtk::Application,
    st: &'a Rc<RefCell<store::Store>>,
    agent: &'a Rc<RefCell<Option<insert::PasteAgent>>>,
    picker: &'a Rc<RefCell<Option<Rc<ui::Picker>>>>,
    daemon: bool,
    no_paste: bool,
    always_copy: bool,
}

/// Runs after the window is hidden: insert the emoji, then quit or go back to sleep.
///
/// Two routes, in order of preference:
/// 1. Commit as an input method. Direct, invisible, needs no permission - but only
///    reaches apps speaking zwp_text_input, so XWayland clients miss it.
/// 2. Clipboard plus a portal-synthesized Ctrl+V. Works anywhere, at the cost of a
///    one-time permission dialog.
fn finish(cx: Ctx) {
    let mut inserted = false;

    if !cx.no_paste {
        match im::commit(cx.ch, IM_WAIT) {
            Ok(()) => inserted = true,
            Err(e) => eprintln!("emoji-picker: input method unavailable ({e}), using portal"),
        }
    }

    if !inserted || cx.always_copy || cx.no_paste {
        if let Err(e) = insert::copy_to_clipboard(cx.ch) {
            eprintln!("emoji-picker: clipboard failed: {e}");
        }
    }

    if !inserted && !cx.no_paste {
        // Only now is a portal session worth its permission prompt.
        if cx.agent.borrow().is_none() {
            let token = cx.st.borrow().restore_token().map(str::to_owned);
            *cx.agent.borrow_mut() = Some(insert::PasteAgent::spawn(token));
        }
        let reply = cx
            .agent
            .borrow()
            .as_ref()
            .map(|a| a.paste_now(PASTE_TIMEOUT));
        match reply {
            Some(insert::Reply::Pasted(token)) => {
                let mut st = cx.st.borrow_mut();
                st.set_restore_token(token);
                st.set_paste_denied(false);
                inserted = true;
            }
            Some(insert::Reply::Failed(e)) => {
                eprintln!("emoji-picker: paste failed: {e}");
                let mut st = cx.st.borrow_mut();
                // Only explain the fallback the first time it happens.
                if !st.paste_denied() {
                    insert::notify(
                        "Copied. Press Ctrl+V to insert.\nAllow \"Remote Desktop\" to paste automatically.",
                    );
                    st.set_paste_denied(true);
                }
            }
            None => {}
        }
        // The session is single-use once started; drop it so the next pick gets a fresh
        // one rather than a spent handle.
        *cx.agent.borrow_mut() = None;
    }

    if !inserted && cx.no_paste {
        insert::notify("Copied to clipboard.");
    }
    cx.st.borrow().save();

    if cx.daemon {
        let _ = cx.picker;
    } else {
        cx.app.quit();
    }
}
