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

System Settings → Keyboard → Shortcuts → Add New → Application → *Emoji Picker*, then set
the key. `Meta+.` is a good choice; it currently belongs to Plasma's own emoji picker, so
clear that one first.

## Usage

| Key | Action |
| --- | --- |
| type | filter by name and keywords |
| ← ↑ ↓ → | move around the grid |
| Enter | insert |
| Tab / Shift-Tab | next / previous category |
| Esc | cancel |
| click | insert |

The window opens on **Recents** once you have picked anything.

### Flags

- `--no-paste` — copy only, never synthesize Ctrl+V.
- `--print` — also write the chosen emoji to stdout.
- `--daemon` — see below.

## Resource use

By default there is no background process at all: the hotkey launches the binary, it
shows the grid, inserts, and exits. Nothing is resident between uses.

If you would rather trade ~20 MB of RAM for an instantly appearing window:

```sh
systemctl --user enable --now emoji-picker   # after copying contrib/emoji-picker.service
```

The daemon listens on `$XDG_RUNTIME_DIR/emoji-picker.sock`. A plain `emoji-picker` call
detects the socket and asks the daemon to show its window, so the same hotkey works in
either mode — starting or stopping the service is the whole switch.

## Data

Emoji names and keywords come from the `rofi-emoji` data file (CDDL/CC-BY, see
`data/LICENSE`), compiled into the binary at build time. There is no runtime dependency
on that package and no data file to read at startup.
