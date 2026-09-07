//! What happens after the user picks an emoji: the window hides, then hands off to
//! [`finish`].

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
    // The focused app speaks no text-input at all, so the portal has to carry this pick.
    // Worth saying out loud: it is the difference between an instant insert and a slow one
    // behind a permission dialog, and the app is going to do it on every pick.
    let mut no_focus = false;

    if !cx.no_paste {
        match im::commit(cx.ch, IM_WAIT) {
            Ok(()) => inserted = true,
            Err(im::Error::NoFocus) => no_focus = true,
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
        let token = cx.st.borrow().restore_token().map(str::to_owned);
        match insert::paste_once(token, PASTE_TIMEOUT) {
            insert::Reply::Pasted(token) => {
                let mut st = cx.st.borrow_mut();
                st.set_restore_token(token);
                st.set_paste_denied(false);
                inserted = true;
            }
            insert::Reply::Failed(e) => {
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
        }
    }

    // Only when the portal actually covered for the missing text field. A failed paste
    // has already said something more useful about how to fix it, and two notifications
    // for one pick is one too many.
    if no_focus && inserted {
        insert::notify("No text field found. Copied and pasted instead.");
    }

    if !inserted && cx.no_paste {
        insert::notify("Copied to clipboard.");
    }
    cx.st.borrow().save();
}
