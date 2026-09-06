//! What happens after the user picks an emoji, independent of which UI drew the grid.
//!
//! This is deliberately free of any toolkit types: both the GTK backend and the native
//! Wayland one hide their window, then hand off to [`finish`].

use crate::{im, insert, store};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

/// Upper bound on waiting for the portal, so a hung D-Bus call cannot wedge the app.
pub const PASTE_TIMEOUT: Duration = Duration::from_secs(20);
/// How long to wait for a text field to offer us input-method focus before deciding this
/// app does not speak text-input and falling back to the portal.
pub const IM_WAIT: Duration = Duration::from_millis(400);
/// How long to let the window hide reach the compositor before the insert goes out. The
/// keystroke has to land in whatever had focus before us, not in the picker.
pub const HIDE_SETTLE: Duration = Duration::from_millis(30);

pub struct Ctx<'a> {
    pub ch: &'a str,
    pub st: &'a Rc<RefCell<store::Store>>,
    pub agent: &'a Rc<RefCell<Option<insert::PasteAgent>>>,
    /// Ends the process. Called only for one-shot runs; a daemon stays up.
    pub quit: &'a dyn Fn(),
    pub daemon: bool,
    pub no_paste: bool,
    pub always_copy: bool,
}

/// Runs after the window is hidden: insert the emoji, then quit or go back to sleep.
///
/// Two routes, in order of preference:
/// 1. Commit as an input method. Direct, invisible, needs no permission - but only
///    reaches apps speaking zwp_text_input, so XWayland clients miss it.
/// 2. Clipboard plus a portal-synthesized Ctrl+V. Works anywhere, at the cost of a
///    one-time permission dialog.
pub fn finish(cx: Ctx) {
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

    if !cx.daemon {
        (cx.quit)();
    }
}
