# Working on emoji-picker

Context for anyone — human or agent — changing this code. The README is for people who
just want to use it; everything below is why the implementation looks the way it does.

Target environment: KDE Plasma 6.7 on Wayland (KWin), Rust + GTK 4.

## Layout

| Path | What it holds |
| --- | --- |
| `src/main.rs` | CLI, app lifecycle, and `finish()` — the insert decision |
| `src/ui.rs` | the window: layer-shell surface, list, search, tabs, keys |
| `src/im.rs` | insert route 1, as a Wayland input method |
| `src/insert.rs` | insert route 2, clipboard + RemoteDesktop portal |
| `src/store.rs` | recents and the portal restore token |
| `src/ipc.rs` | single-instance socket |
| `src/emoji.rs` | the compiled-in emoji table and search |
| `build.rs` | turns `data/emoji.tsv` into a static Rust array |
| `tools/update-emoji.py` | regenerates `data/emoji.tsv` from Unicode |
| `protocols/` | vendored `input-method-unstable-v1.xml` |

## Inserting text on KDE Wayland

This is the whole problem the project exists to solve, and most of the surprises live
here.

**Why not `wtype`?** KWin exposes no `zwp_virtual_keyboard_manager_v1` — check with
`wayland-info`. `wtype` needs it, and `rofimoji`, `rofi-emoji` and similar pickers all
insert via `wtype`, so on this desktop they pick an emoji and then silently do nothing.
That is the bug that started this project.

**Why not do what Klipper does?** Klipper pastes directly because it lives inside
plasmashell, which KWin privileges with fake-input access. No third-party binary can get
that.

### Route 1: input method (`src/im.rs`)

KWin *does* expose `zwp_input_method_v1`. An input method commits text into whatever
holds the text-input focus rather than synthesizing keys, which avoids the portal, the
clipboard, and all timing concerns. This is the default path.

- No crate ships client bindings for this old unstable protocol, so the XML is vendored
  in `protocols/` and `wayland_scanner::generate_client_code!` builds bindings at compile
  time. `wayland-backend` must be a direct dependency for the generated code to compile.
- `activate` creates a context object, which the queue can only construct if the
  `event_created_child!` macro sits **inside** the `Dispatch` impl for
  `ZwpInputMethodV1`. Placing it at module scope compiles but panics at runtime.
- Commit only after the picker's window is hidden, or the focused text input is our own
  search box.
- Expect failure and fall back: only one client may bind the interface (fcitx5, ibus or a
  virtual keyboard would already hold it), and it reaches only apps speaking
  `zwp_text_input_v2/v3`, so XWayland clients generally miss out.

### Route 2: clipboard + portal (`src/insert.rs`)

`xdg-desktop-portal-kde` implements RemoteDesktop, so the emoji goes on the clipboard and
the portal synthesizes Ctrl+V. Works anywhere, but costs a permission dialog, raises a
"remote control session started" notification, and is fussy about timing.

The session is created **lazily**, only after route 1 fails — creating it eagerly is what
put a permission prompt and a notification in front of every insert.

Three constants had to be right before this worked at all:

- `FOCUS_SETTLE` (150 ms) — the picker holds an exclusive keyboard grab on its layer
  surface; the compositor needs a moment to drop it and restore focus.
- `KEY_GAP` (20 ms) — so Ctrl is unambiguously down before V.
- `HOLD_AFTER` (300 ms) — **the subtle one.** Every `notify_keyboard_keycode` call
  returned success and nothing was typed, because the process exited immediately after,
  closing the session before KWin had delivered the events.

`persist_mode = ExplicitlyRevoked` plus the token in `store.rs` keeps the dialog to one
appearance, ever.

### Debugging either route

`--test-im` and `--test-paste` drive one route in isolation, print each step, and give
you five seconds to focus a target window. Reach for these before changing timings: they
separate "the mechanism is broken" from "the app's own sequencing is wrong".

## The window (`src/ui.rs`)

- **Layer-shell, anchored to all four edges.** The surface spans the output and paints
  only a centred card. It has to: a card-sized surface never receives clicks that land
  outside it, so click-to-dismiss was impossible before this. The trade-off is that a
  click meant for another window dismisses the picker instead of reaching it.
- **Key controller in the capture phase.** `GtkSearchEntry` swallows Escape for its own
  clear-search behaviour, so bubble-phase handling never sees it.
- **One virtualized `GtkListView`** holds recents plus every category. Rows are built on
  demand, so ~3,900 emoji cost nothing at startup; building real widgets for all of them
  does not scale.
- **Rows are encoded into a `StringList`** with a marker char for header-vs-emoji-row and
  a unit separator between cells, rather than a custom GObject. Pragmatic; revisit if row
  content grows structure.
- **Section jumps compute pixel offsets** from `CELL` and `HEADER_H` instead of measuring
  widgets, since virtualized rows may not exist yet. **Those constants must match the
  CSS** — restyling row padding without updating them breaks tab navigation silently.
- **Selection repaint goes through the model.** Splicing a row's own text back over
  itself makes `StringList` emit items-changed, so the factory rebinds and repaints.
  Tracking bound row widgets in a map looked simpler but went stale as the list recycled
  them: the footer updated while the highlight stayed put.

## Single instance (`src/ipc.rs`)

Whichever process owns the window binds `$XDG_RUNTIME_DIR/emoji-picker.sock`. A later
invocation finds it, sends `toggle`, and exits, so a second hotkey press closes the
picker instead of opening another. Both one-shot and `--daemon` serve it. Stale sockets
are probed and removed before binding. Two presses inside the ~100 ms before the socket
is bound can still start two processes; not worth a lock file so far.

## Emoji data

`data/emoji.tsv` is generated and checked in; `build.rs` compiles it into a static array,
so startup does no IO and the build needs no network.

There is no emoji API to poll — Unicode ships static files. `tools/update-emoji.py`
merges two of them:

- `emoji-test.txt` — the authoritative list of fully-qualified emoji, official display
  order, group and subgroup.
- CLDR `annotations` and `annotationsDerived` — names and keywords, the latter covering
  skin-tone and gender variants.

```sh
python3 tools/update-emoji.py                                   # track latest
python3 tools/update-emoji.py --emoji-version 16.0 --cldr-ref release-46
python3 tools/update-emoji.py --locale de                       # any locale CLDR has
cargo test                                                      # sanity-check the table
```

CLDR keys sequences **without** the U+FE0F variation selector that `emoji-test.txt`
carries, so lookups retry with it stripped; without that, ☺️, ❤️‍🔥 and ~1,180 others
lose their names. The script reports the count delta and warns about any fallback names —
a non-zero warning count means a source changed shape.

The count dropped 5,042 → 3,944 when the data moved off the `rofi-emoji` file, which
carried non-fully-qualified duplicates. `src/emoji.rs` asserts a floor of 3,500.

Search ranks whole-word hits above fuzzy subsequence matches; without that, "kitten"
matched dozens of emoji before the cat.

## KDE gotchas that cost real time

- **kglobalaccel runs inside `kwin_wayland`** on Plasma 6.7 and caches each shortcut's
  command at session start. Editing `~/.local/share/applications/*.desktop` or
  `kglobalshortcutsrc` changes nothing until logout — the old command keeps launching.
  `kbuildsycoca6` does not help. Register through System Settings, which pushes to the
  running compositor. There is no way to restart kglobalaccel alone; restarting the
  Wayland compositor ends the session.
- **`kwriteconfig6` mangles group names containing brackets**, such as
  `[services][foo.desktop]`, writing an escaped `\x5d` group that KDE ignores.
- **System Settings rewrites `kglobalshortcutsrc` when it closes**, so config edits made
  while it is open get reverted. Close it first.
- **Muting the portal notification** means `~/.config/xdg-desktop-portal-kde.notifyrc`
  with `Action=` under `[Event/remotedesktopstarted]`. It takes effect on next login.
  Note what this costs: that notification is the desktop reporting that *something*
  gained control of input, and muting it hides the warning for every application, not
  just this one. With route 1 working it should rarely fire, so prefer fixing the
  fallback over muting the warning.

## Testing

`cargo test` covers the emoji table and search ranking — the parts that can be checked
without a compositor. Everything else needs a real session:

1. `cargo run` — grid appears centred, typing filters, Enter inserts.
2. Insert into three toolkits: a Qt app, a GTK app, and an XWayland one. The last should
   fall back to the portal; watch stderr to confirm which route ran.
3. Second hotkey press closes rather than opening a second window.
4. `pgrep emoji-picker` is empty between uses without `--daemon`.
