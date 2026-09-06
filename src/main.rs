mod commit;
mod emoji;
mod im;
mod insert;
mod ipc;
mod store;

#[cfg(feature = "gtk")]
mod backend_gtk;
#[cfg(feature = "gtk")]
mod settings;
#[cfg(feature = "gtk")]
mod ui;

#[cfg(feature = "native")]
mod backend_native;
#[cfg(feature = "native")]
mod theme;

#[cfg(not(any(feature = "gtk", feature = "native")))]
compile_error!("enable exactly one UI backend: --features gtk (default) or --features native");

use std::process::ExitCode;
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
  --bench        time the emoji table and search, then exit
  --time-launch  report time to first frame, then exit
  -h, --help     this text
";

/// Runtime flags, parsed once and handed to whichever UI backend is compiled in.
#[derive(Clone, Copy)]
pub struct Flags {
    pub daemon: bool,
    pub no_paste: bool,
    pub always_copy: bool,
    pub print: bool,
    pub time_launch: bool,
}

/// Process start, for `--time-launch`. Taken before anything else runs.
static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

pub fn since_start() -> Duration {
    START.get().map(|t| t.elapsed()).unwrap_or_default()
}

fn main() -> ExitCode {
    let _ = START.set(std::time::Instant::now());
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |f: &str| args.iter().any(|a| a == f);

    if has("-h") || has("--help") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if has("--test-im") {
        im::test_commit();
        return ExitCode::SUCCESS;
    }
    if has("--test-paste") {
        insert::test_paste(20, 500);
        return ExitCode::SUCCESS;
    }
    if has("--bench") {
        emoji::bench();
        return ExitCode::SUCCESS;
    }

    let flags = Flags {
        daemon: has("--daemon"),
        no_paste: has("--no-insert") || has("--no-paste"),
        always_copy: has("--copy"),
        print: has("--print"),
        // Report how long the window took to reach the screen, then leave. Cold start is
        // mostly toolkit, Wayland and font setup rather than our own work, so the only way
        // to tell an optimisation from a placebo here is to measure to first frame.
        time_launch: has("--time-launch"),
    };

    // Another instance already owns the window: tell it to toggle and get out of the
    // way, so a second hotkey press closes the picker rather than opening a second one.
    if !flags.daemon && ipc::request_toggle() {
        return ExitCode::SUCCESS;
    }

    // While both backends are compiled in, the native one wins - it is the one being
    // brought up, and building with both is how the two get compared.
    #[cfg(feature = "native")]
    let code = backend_native::run(flags);
    #[cfg(all(feature = "gtk", not(feature = "native")))]
    let code = backend_gtk::run(flags);

    // We owned the socket in either mode; leaving it behind would make the next
    // invocation think an instance is still up.
    ipc::cleanup();
    code
}
