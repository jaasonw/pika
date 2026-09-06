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
| `src/store.rs` | recents, settings, and the portal restore token |
| `src/settings.rs` | the settings window |
| `src/ipc.rs` | single-instance socket |
| `src/emoji.rs` | the compiled-in emoji table and search |
| `build.rs` | turns `data/emoji.tsv` into static tables: the emoji, their tones, group ranges, search masks, and a lookup index |
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
- **One `GtkListView`** holds recents plus every category, so the ~3,900 emoji themselves
  cost nothing — but the rows very much do, see below.
- **Rows are encoded into a `StringList`** with a marker char for header-vs-emoji-row and
  a unit separator between cells, rather than a custom GObject. Pragmatic; revisit if row
  content grows structure.
- **`GtkListView` is not as lazy as it looks, and this dominated launch.** It builds a
  widget for every item handed to it, up to a working set of roughly 205 — measured, and
  reproducible with a standalone GTK program: 1,000-row and 4,000-row models both produce
  exactly 205 factory `setup` calls. The browse list is 177 rows, *under* that cap, so a
  full splice built every row: 12 `GtkButton`s each, ~2,600 widgets, 40-60 ms. It was
  happening twice per launch. Things that do **not** fix it, all tested: `hscrollbar-policy`,
  making row heights uniform, and waiting for the viewport to be allocated first.
- **So the list is filled head-then-tail.** `rebuild` puts `HEAD_ROWS` (10) rows in
  synchronously and appends the rest from `glib::idle_add_local_once`. Appending does not
  rebuild the rows already there, so the tail is genuinely free of the first frame.
  `Picker::fill` is a generation counter: a keystroke landing before the idle runs bumps
  it and the stale tail drops itself. A section longer than the head budget is split
  mid-run, which is what keeps a 500-hit search from building 42 rows on the keystroke.
- **`Picker::new` must not populate the list.** Every caller presents the window straight
  after, and `present` fills it; doing both cost an entire extra splice.
- **`View` precomputes what the scroll path reads.** `offsets` is a prefix sum so
  `offset()` is an index rather than a walk (`sync_active_tab` calls it on every scroll
  tick), and `row_of` inverts `rows` so the bind handler does not scan for each row that
  scrolls past.
- **Section jumps read `offsets`, which is built from measured heights.** Virtualized rows
  may not exist when a jump is computed, so the offsets cannot be read off the widgets
  each time — but they no longer have to match the CSS by hand either. `Picker::measure`
  takes the height of one laid-out header and one laid-out emoji row from the first frame,
  caches them in `Picker::metrics`, and `View::rescale` restates every offset from them;
  `View::new` starts later rebuilds from the cached pair.
  - `CELL` and `HEADER_H` are only what the widgets *request*, plus the guess the first
    view is built with before that frame lands. They are not what GTK lays out: the
    `.section` padding and the item widget's own box put a header at 42px and a row at
    56px against requests of 30 and 44. Summing the constants gave 7,648px for the 177-row
    browse list where GTK's `adjustment().upper()` was 9,772 — 28% short, so tab jumps
    under-scrolled further the lower the section sat, and `sync_active_tab` lit the wrong
    tab for the same reason.
  - Measure with `--time-launch`: it prints `height=` (what `offsets` says) next to
    `upper=` (what GTK laid out) once the list has settled. **Those two must agree.** They
    are the check that replaces keeping the constants in sync with the CSS — restyling row
    padding is now free, but a change that breaks the measurement shows up as a gap here.
  - `measure` identifies an item by which of the factory box's two children has a nonzero
    height, since GTK does not allocate the hidden one. It reads `compute_bounds`, not
    `height()`: the latter is the content box without the padding, which is exactly the
    number the constants already got wrong.
- **Selection repaint goes through the model.** Splicing a row's own text back over
  itself makes `StringList` emit items-changed, so the factory rebinds and repaints.
  Tracking bound row widgets in a map looked simpler but went stale as the list recycled
  them: the footer updated while the highlight stayed put.

## Skin tones

The generated table lists every tone variant as its own entry, which is why People & Body
holds ~2,400 of the ~3,900 rows. Showing them all would fill the grid with near-identical
hands and bury real hits in search, so:

- **The table is laid out base-first.** `build.rs` emits the 1,914 base emoji in tab-group
  order, then every tone variant, and publishes `BASE_COUNT` and `GROUP_RANGES` alongside.
  `by_group()` is therefore a slice and `search()` a prefix — neither filters. This
  replaced a `has_tone()` call per entry per group, which was ~35,000 char scans a rebuild.
- **`Emoji::toned()` is an array index.** Each entry carries `tones: [&str; 5]`, with
  emoji that take no tone repeating themselves, so there is no branch and no lookup. The
  old `with_tone()` built a `HashMap` on first use, at a cost of ~3,900 `String`
  allocations on the launch path. The free `with_tone(ch, tone)` remains for callers
  holding only a string, such as the sample hand in the settings window.
- `find()` binary-searches `BY_CH`, a build-time index sorted by sequence, rather than
  scanning all 3,944 entries — it runs per arrow key and once per recent.
- The tone is applied at render time in `ui.rs`, not stored in the data.

Sequences mixing two different tones (couples, handshakes) have no single tone to key
them under and are skipped, so they appear only in base form. Supporting them means a
per-person picker; no competitor does it either.

Recents store what was actually picked, so changing the tone does not rewrite history.

## Settings (`src/settings.rs`)

A **second layer-shell surface**, not a plain toplevel: the picker sits on the overlay
layer and an ordinary window renders underneath it. Only one surface can hold the
keyboard grab, so the picker hides while settings are open and is re-presented on close.

Settings save on change and feed `finish()`; command-line flags override them for a
single run.

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
matched dozens of emoji before the cat. The tiers are exact name, name prefix, exact
keyword, then name substring.

Before the fuzzy matcher runs, a **skip mask** rejects most of the table: `build.rs` emits
a 32-bit set of the letters in each entry's name and keywords, and a candidate whose mask
lacks a letter the query needs cannot match. Bit 27 marks a field holding non-ASCII (76
rows, all curly apostrophes) and is never rejected, since `Normalization::Smart` folds
characters the mask cannot model. `the_skip_mask_never_hides_a_real_hit` guards this by
brute-forcing the unfiltered matcher and comparing counts — **keep that test** if you
touch the masks.

## Measuring

Two flags exist because both of these were guessed wrong before they were measured:

- `--bench` times the table and search paths without starting GTK.
- `--time-launch` reports time to first frame and the settled list state, then exits. A
  short `model=` count means the idle tail fill never landed.

Cold start is roughly 170 ms, of which ~125 ms is process start, dynamic linking (114
shared objects) and GTK init — none of it ours. Measure to first frame before believing
any launch optimisation. `GSK_RENDERER` was tried and makes no difference here: cairo ties
the default and GL is worse.

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
