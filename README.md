# emoji-picker

A grid emoji picker for KDE on Wayland, in the style of the Windows and macOS pickers.
Search, category tabs, recents. Picking an emoji copies it **and** pastes it into the
field you were typing in.

## Why another one

On this desktop KWin exposes no `zwp_virtual_keyboard_manager_v1`. That is the protocol
`wtype` uses, and `wtype` is how `rofimoji`, `rofi-emoji` and friends insert their
result — so on KDE Wayland they pick an emoji and then do nothing at all.

This picker inserts through the **RemoteDesktop portal** instead, which
`xdg-desktop-portal-kde` does implement: it puts the emoji on the clipboard, closes its
window so focus returns to your text field, then has the portal synthesize Ctrl+V.

The first insert raises one KDE permission dialog. Approving it stores a restore token in
`~/.config/emoji-picker/state.json`, and the dialog never appears again. If you deny it,
the emoji is still on your clipboard and you press Ctrl+V yourself.

## Install

```sh
./contrib/install.sh
```

Puts the binary in `~/.local/bin` and a hidden `.desktop` entry in
`~/.local/share/applications`.

### Bind a hotkey

System Settings → **Keyboard → Shortcuts → Add New → Command or Script**, enter
`~/.local/bin/emoji-picker` (write the path out in full), click the shortcut field and
press your key. `Meta+.` and `Meta+;` are both good choices.

> **Add it through System Settings, not by editing config files.** On Plasma 6.7
> kglobalaccel is part of the `kwin_wayland` process, and it caches each shortcut's
> command in memory when the session starts. Editing
> `~/.local/share/applications/*.desktop` or `~/.config/kglobalshortcutsrc` by hand
> changes nothing until you log out — the old command keeps launching, and
> `kbuildsycoca6` does not help. Registering through System Settings pushes the new
> command to the running compositor immediately.

## Usage

| Key | Action |
| --- | --- |
| type | filter by name and keywords |
| ← ↑ ↓ → | move around the grid |
| Enter | insert |
| Tab / Shift-Tab | next / previous category |
| Esc | cancel |
| click | insert |

The window opens on **Recents** once you have picked anything. Pressing the hotkey again
while the picker is up closes it: an invocation that finds a running instance tells it to
toggle and then exits, so the shortcut never stacks up windows.

### Flags

- `--no-paste` — copy only, never synthesize Ctrl+V.
- `--print` — also write the chosen emoji to stdout.
- `--daemon` — see below.

## Silencing the "Remote control session started" popup

Every insert opens a portal session, and KDE announces that with a notification. To mute
just that one event, create `~/.config/xdg-desktop-portal-kde.notifyrc`:

```ini
[Event/remotedesktopstarted]
Action=
```

Log out and back in for it to take effect.

**Understand the trade-off first.** That notification is the desktop telling you
something has gained control of your input. Muting it hides the warning for *every*
application that starts a remote-control session, not only this one. The portal
permission itself is untouched — you can still see and revoke it under System Settings →
Applications → Remote Desktop. Delete the file to get the warning back.

Running with `--no-paste` avoids the portal, and therefore the notification, entirely;
you press Ctrl+V yourself.

## Troubleshooting

**The emoji is on the clipboard but nothing was typed.** The portal was denied or timed
out. Check `System Settings → Applications → Remote Desktop`; if this app is listed,
remove it and pick again to get a fresh permission prompt.

**Nothing happens when I press the hotkey.** Run `~/.local/bin/emoji-picker` from a
terminal to see the error. If it works there but not from the hotkey, the shortcut is
still bound to its old command — see the note under *Bind a hotkey*.

**Nothing is typed, but no error appears.** Three timings in `src/insert.rs` govern this,
and all three had to be right before inserts worked reliably here:

- `FOCUS_SETTLE` (150 ms) - how long to wait after hiding the window. The picker holds an
  exclusive keyboard grab on its layer surface, and the compositor needs a moment to drop
  it and give focus back to your text field.
- `KEY_GAP` (20 ms) - spacing between key events, so Ctrl is unambiguously down before V.
- `HOLD_AFTER` (300 ms) - how long the portal session stays open after the last key.
  Without it the process exits, the session closes, and KWin drops keys it has not
  delivered yet. This was the one that made inserts silently do nothing.

Raise them on a loaded machine. `emoji-picker --test-paste` exercises the portal path on
its own: it copies `PASTE-OK`, gives you five seconds to focus a field, sends Ctrl+V, and
reports each step - useful for telling a portal problem apart from a timing one.

## Resource use

By default there is no background process at all: the hotkey launches the binary, it
shows the grid, inserts, and exits. Nothing is resident between uses.

If you would rather trade ~20 MB of RAM for an instantly appearing window:

```sh
systemctl --user enable --now emoji-picker   # after copying contrib/emoji-picker.service
```

Whichever process owns the window listens on `$XDG_RUNTIME_DIR/emoji-picker.sock`, in
both modes. A plain `emoji-picker` call that finds the socket asks the existing instance
to toggle, so the same hotkey works either way — starting or stopping the service is the
whole switch.

## Data

`data/emoji.tsv` is generated from Unicode's own published data and compiled into the
binary by `build.rs`, so there is no data file to read at startup and no dependency on
any other package.

There is no emoji "API" to poll - Unicode ships static files, which suits us better,
since the table is checked in and builds stay offline. Two sources are merged:

- [`emoji-test.txt`](https://unicode.org/Public/emoji/latest/emoji-test.txt) - the
  authoritative list of fully-qualified emoji, in the official display order, with the
  group and subgroup each belongs to.
- [CLDR annotations](https://github.com/unicode-org/cldr/tree/main/common/annotations) -
  short names and search keywords, and the `annotationsDerived` companion covering skin
  tone and gender variants.

To refresh after a new Emoji release (roughly annually):

```sh
python3 tools/update-emoji.py     # rewrites data/emoji.tsv
cargo test                        # sanity-checks the table and search ranking
```

Pin a release rather than tracking `main` if you prefer:

```sh
python3 tools/update-emoji.py --emoji-version 16.0 --cldr-ref release-46
```

`--locale` takes any locale CLDR annotates, so a non-English build is
`--locale de`. The script reports how the emoji count moved and warns if any entry
had to fall back to its `emoji-test.txt` name. Licensing is in `data/LICENSE`.
