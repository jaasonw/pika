//! The GTK 4 UI backend: the original implementation, unchanged in behaviour and moved out
//! of `main.rs` so a second backend can sit beside it during the migration.

use crate::{Flags, commit, insert, ipc, settings, since_start, store, ui};
use gtk::prelude::*;
use gtk4 as gtk;
use gtk4::glib;
use std::cell::RefCell;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Duration;

pub fn run(flags: Flags) -> ExitCode {
    let Flags {
        daemon,
        no_paste,
        always_copy,
        print,
        time_launch,
    } = flags;

    let app = gtk::Application::builder()
        .application_id("dev.jason.EmojiPicker")
        .flags(
            gtk::gio::ApplicationFlags::NON_UNIQUE
                | gtk::gio::ApplicationFlags::HANDLES_COMMAND_LINE,
        )
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
                // Let the hide reach the compositor before any key event goes out.
                glib::timeout_add_local_once(commit::HIDE_SETTLE, move || {
                    let prefs = {
                        let st = st.borrow();
                        (st.settings().insert, st.settings().always_copy)
                    };
                    let quit = {
                        let app = app.clone();
                        move || app.quit()
                    };
                    commit::finish(commit::Ctx {
                        ch: &ch,
                        st: &st,
                        agent: &agent,
                        quit: &quit,
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
                        let (recents, tone) = {
                            let st = st_inner.borrow();
                            (st.recents().to_vec(), st.settings().skin_tone)
                        };
                        p.present(recents, tone);
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
        if let Ok(toggles) = ipc::serve() {
            let picker = picker.clone();
            let st = st.clone();
            let app = app.clone();
            glib::spawn_future_local(async move {
                while toggles.recv().await.is_ok() {
                    let Some(p) = picker.borrow().clone() else {
                        return;
                    };
                    if p.window.is_visible() {
                        p.window.set_visible(false);
                        if !daemon {
                            app.quit();
                        }
                    } else {
                        let (recents, tone) = {
                            let st = st.borrow();
                            (st.recents().to_vec(), st.settings().skin_tone)
                        };
                        p.present(recents, tone);
                    }
                }
            });
        }

        if daemon {
            // Hold the process open while the window is hidden. The guard must outlive
            // the closure, and the daemon lives until the process exits anyway.
            std::mem::forget(app.hold());
        }

        let (recents, tone) = {
            let st = st.borrow();
            (st.recents().to_vec(), st.settings().skin_tone)
        };
        p.present(recents, tone);

        if time_launch {
            // The first frame is the number that matters: everything before it is
            // invisible to the user, and everything after is already interactive.
            let app = app.clone();
            let p = p.clone();
            let window = p.window.clone();
            window.add_tick_callback(move |_, _| {
                println!("first frame at {:?}", since_start());
                // The list finishes filling from an idle callback, so report the settled
                // state too: a short count here means the tail never landed.
                let (app, p) = (app.clone(), p.clone());
                glib::timeout_add_local_once(Duration::from_millis(500), move || {
                    println!("settled at {:?}, {}", since_start(), p.debug_state());
                    app.quit();
                });
                glib::ControlFlow::Break
            });
        }

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
    if code == glib::ExitCode::SUCCESS {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
