# Working on emoji-picker

Context for anyone — human or agent — changing this code. The README is for people who
just want to use it; everything below is why the implementation looks the way it does.

Target environment: KDE Plasma 6.7 on Wayland (KWin), Rust.

The picker is a native Wayland client: `smithay-client-toolkit` for the protocol, `cairo`
and `pangocairo` for drawing, no widget toolkit. It was GTK 4 until the migration recorded
in `plans/wayland-native-migration.md`, and the GTK implementation is still buildable as a
reference — see [Two backends](#two-backends).

## Layout

| Path | What it holds |
| --- | --- |
| `src/main.rs` | CLI, flag parsing, backend dispatch |
| `src/commit.rs` | `finish()` — the insert decision, shared by both backends |
| `src/backend_native.rs` | the native backend: layer surface, calloop loop, key and pointer routing |
| `src/render.rs` | all cairo drawing, in logical pixels |
| `src/grid.rs` | the list and its geometry: rows, offsets, hit testing, navigation |
| `src/picker.rs` | UI state: query, selection, scroll, hover, settings mode |
| `src/theme.rs` | desktop colours, read from `kdeglobals` |
| `src/im.rs` | insert route 1, as a Wayland input method |
| `src/insert.rs` | insert route 2, clipboard + RemoteDesktop portal |
| `src/store.rs` | recents, settings, and the portal restore token |
| `src/ipc.rs` | single-instance socket |
| `src/emoji.rs` | the compiled-in emoji table and search |
| `src/backend_gtk.rs`, `src/ui.rs`, `src/settings.rs` | the GTK 4 backend, kept as a fallback |
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

## Two backends

`cargo build` gives the native client. The GTK 4 original still builds:

```sh
cargo build --release                                     # native (default)
cargo build --release --no-default-features --features gtk # GTK 4
```

Everything outside the UI is shared: `emoji.rs`, `store.rs`, `im.rs`, `insert.rs`,
`ipc.rs` and `commit.rs` know nothing about either toolkit. `commit::Ctx` takes a
`quit: &dyn Fn()` rather than a `gtk::Application` for exactly this reason, and
`ipc::bind`/`ipc::ack` are split from `ipc::serve` because the two loops pump the socket
differently — glib can await a channel but not poll a raw fd, calloop does the opposite.

Keep the GTK build compiling until it is deleted outright. It is the fallback if a KWin
update breaks something in the layer-shell path.

## The window (`src/backend_native.rs`, `src/render.rs`)

- **Layer-shell, anchored to all four edges,** as the GTK build was and for the same
  reason: a card-sized surface never receives clicks that land outside it, so
  click-to-dismiss would be impossible. The trade-off is unchanged — a click meant for
  another window dismisses the picker instead of reaching it.
- **The shm pool holds one slot, not two.** The picker redraws on input and never
  continuously, so there is no frame in flight to double-buffer against. At full-output
  size that is ~9 MB rather than ~18 MB. `SlotPool` allocates a second slot on demand if
  that assumption ever breaks, so it degrades rather than tears.
- **Cairo draws straight into the shm slot** through `create_for_data_unsafe`. `wl_shm`'s
  `Argb8888` is premultiplied little-endian, bit-identical to Cairo's `ARgb32` here, so no
  conversion is needed. Drawing into an owned surface and copying would cost a second
  full-size buffer.
- **Damage is limited to the card rect.** Without it the compositor reblends the whole
  screen on every keystroke.
- **The event loop is calloop,** with the Wayland connection and the toggle socket as two
  sources. `blocking_dispatch` would have needed a second thread for the socket.
- **Layout is arithmetic, not measurement.** `grid.rs` knows every row's height by
  construction, so `offsets` is a prefix sum and `row_at` is a binary search. The GTK build
  could not do this: `GtkListView` builds a widget per item up to a working set of ~205, the
  browse list is 177 rows, so a full splice built ~2,600 widgets at 40-60 ms. That forced a
  head-then-tail fill, a generation counter to drop a stale tail, and a `measure` pass to
  learn what GTK had actually laid out. None of that survives — 160 rows of `&'static str`
  build in microseconds.
- **Only visible rows are drawn.** `Grid::row_at(scroll)` finds the first, and the loop
  stops at the bottom of the viewport.
- **`Fonts` is built once per frame, not once per cell.** Parsing
  `FontDescription::from_string` for each of ~100 cells dominated a redraw.
- **The client draws its own cursor.** A plain `wl_pointer` never sets one, so the image
  stays whatever the previously focused surface left behind. `ThemedPointer` plus
  `set_cursor` fixes it; the request needs the latest enter serial, so the first image can
  only be asked for from the `Enter` event, and it is only re-sent when the icon actually
  changes — motion arrives far faster than the cursor needs updating.
- **Alpha overlays are pre-blended.** `Rgb::blend` composites the tints the GTK stylesheet
  wrote as `alpha(@theme_fg_color, 0.10)` against the known card background, so hover and
  border tints cost no Cairo group.

### Colours (`src/theme.rs`)

There is no CSS to hang `@theme_bg_color` off, so five colours are read from
`~/.config/kdeglobals`: the window background and foreground, the view background, and the
selection pair.

The plan originally called for the `org.freedesktop.appearance` portal first, to avoid
hardcoding Plasma. It is the other way round, deliberately: the portal is a D-Bus round
trip on a path budgeted at tens of milliseconds and reports only a light/dark preference
and an accent colour, where kdeglobals is one file read carrying the whole palette. **This
means non-KDE desktops get Breeze Light regardless of their actual theme.** The narrow fix,
if it ever matters, is to ask the portal for `color-scheme` only when kdeglobals is absent.

### Scaling

Integer output scale is handled: `scale_factor_changed` sets `wl_surface.set_buffer_scale`
and the Cairo transform, and the buffer is allocated at physical size. `wp_fractional_scale_v1`
is **not** bound, so at a fractional display scale KWin reports scale 2, the picker renders
at 2x, and the compositor downsamples. It looks right; the cost is a buffer bigger than the
display needs. Measured on a 1920x1200 panel: 37.4 MB RSS at 100%, 45.0 MB at 150%. True
fractional support would save about 7 MB while at a fractional scale, which is why it has
not been built.

## Settings

A **second mode on the same card**, not a second surface. The GTK build had no choice: only
one surface can hold the keyboard grab, so it hid the picker, showed a second layer surface,
and re-presented the picker on close. Flipping `Picker::mode` removes that entirely.

Rows carry real controls — a switch, a stepper, a strip of swatches — rather than a word
naming the value. "On"/"Off" text states what a setting is but not that it can be changed,
and the recents cap had no mouse affordance at all before the stepper.

A focused row is marked with a bar down its left edge rather than a flooded background, so
every control keeps one background colour to sit on; pointer hover is a separate, fainter
tint, since the keyboard and the mouse each need their own position.

`Picker` cannot reach the store, so a row names a `picker::Action` and
`App::apply` performs it. That keeps every store mutation in one function and leaves the
settings behaviour testable without a compositor.

Settings save on change and feed `commit::finish`; command-line flags override them for a
single run.

The recents cap goes through `Store::set_recent_limit`, which trims the list immediately;
assigning `settings.recent_limit` directly skips the trim. Its bounds come from
`store::RECENT_LIMIT_RANGE` so the row cannot offer a value the store would clamp away.

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
  holding only a string, such as the six sample hands in the settings tone row.
- `find()` binary-searches `BY_CH`, a build-time index sorted by sequence, rather than
  scanning all 3,944 entries — it runs per arrow key and once per recent.
- The tone is applied when the grid is built (`Grid::browse`), not stored in the data.

Sequences mixing two different tones (couples, handshakes) have no single tone to key
them under and are skipped, so they appear only in base form. Supporting them means a
per-person picker; no competitor does it either.

Recents store what was actually picked, so changing the tone does not rewrite history.

## Single instance (`src/ipc.rs`)

Whichever process owns the window binds `$XDG_RUNTIME_DIR/emoji-picker.sock`. A later
invocation finds it, sends `toggle`, and exits, so a second hotkey press closes the
picker instead of opening another. Stale sockets are probed and removed before binding.
Two presses inside the ~100 ms before the socket is bound can still start two processes;
not worth a lock file so far.

The native backend hands the listener to calloop as a `Generic` source, level-triggered
and drained on each wake. A toggle sets `exit`, because one-shot has nothing to toggle
back to. `--daemon` is **not implemented on the native backend** — it says so on stderr
rather than accepting the flag and running once anyway. The mode exists to hide cold start,
and cold start went from 165-172 ms to 65-100 ms, so it is on hold rather than pending. If
it is ever built, the work is unmapping and remapping the surface on toggle instead of
exiting.

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

- `--bench` times the table and search paths without opening a window.
- `--time-launch` reports time to first frame, then exits.

Measured on a 1920x1200 panel at scale 1, release builds, both backends on the same
machine:

| | GTK 4 | native |
| --- | --- | --- |
| first frame | 165-172 ms | 65-100 ms |
| RSS | 112.5 MB | 37.4 MB |
| private (USS) | 51.6 MB | 9.1 MB |
| shared objects mapped | 126 | 41 |

**Measure before believing a launch optimisation, and measure RSS rather than projecting
it.** Two projections made during the migration were wrong by 3-5x in both directions, and
`/proc/<pid>/smaps_rollup` settled each one in seconds. Things that turned out not to be
the problem: `GSK_RENDERER=cairo` and `GDK_DISABLE=gl,vulkan` together saved 23 ms and
**zero bytes**, so the GPU stack was never the cost. Things that turned out to be the cost:
GTK's own widget and type machinery. The emoji table and every Nucleo index together are
0.5 MB — `--help` peaks at 25.1 MB and `--bench` at 25.6 MB.

What remains in the native build is mostly not ours either: ~14.4 MB is `NotoColorEmoji`
mmapped, and ~9 MB is the full-output shm buffer.

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
4. `pgrep emoji-picker` is empty between uses.
5. `Ctrl+,` or the gear opens settings; each row responds to arrows, Enter and a click;
   changing the tone and going back re-tones the grid.
6. Build the other backend and repeat 1-4, so it does not rot.

**There is no way to drive the picker from a script on this desktop.** `wtype` needs
`zwp_virtual_keyboard_manager_v1`, which KWin does not implement — the same gap that
motivated the whole project. A uinput device is not a workaround either: the compositor
does not route to it while a layer surface holds an exclusive keyboard grab. Anything
involving a keystroke into the picker is a human test.
