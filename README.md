# emoji-picker

A grid emoji picker for KDE on Wayland, in the style of the Windows and macOS pickers.
Press a hotkey, pick an emoji, and it appears in whatever you were typing in.

![every emoji in one scrolling list, recents pinned at the top]()

## Features

- **Everything in one list.** Recents pinned at the top, then every category behind its
  own header. ~3,900 emoji, straight from Unicode.
- **Search** by name and keyword — "cat", "kitten" and "smiling face" all land where you
  expect.
- **Category tabs** jump to a section, and follow along as you scroll.
- **Inserts directly** into the focused text field. No clipboard round trip, so your
  clipboard history stays clean. Apps that can't take a direct insert fall back to
  clipboard-and-paste automatically.
- **Nothing running in the background.** The hotkey starts it, picking an emoji ends it.
  A resident mode is available if you want the window to appear instantly.

## How it compares

Plenty of emoji pickers exist. What separates them on KDE Wayland is how they get the
character into your text field — and most either can't, or need privileges to do it.

| Picker | Insert mechanism | Extra setup it needs | Works on KDE Wayland |
| --- | --- | --- | --- |
| **this one** | Wayland input method, portal paste as fallback | none | yes |
| plasma-emojier (KDE's own) | clipboard only | none | you paste it yourself |
| [rofimoji](https://github.com/fdw/rofimoji) | `wtype` | a supported menu (rofi/wofi) | no — KWin has no virtual-keyboard protocol |
| [bemoji](https://github.com/marty-oehme/bemoji) | `wtype` | a supported menu | no, same reason |
| [jockel09/emoji-picker](https://github.com/jockel09/emoji-picker) | clipboard + `ydotool` Ctrl+V | ydotool daemon, your user in the `input` group | yes, at the cost of raw `/dev/uinput` access |
| [im-emoji-picker](https://github.com/GaZaTu/im-emoji-picker) | input method plugin | fcitx5 or ibus installed and configured | yes, if you run one |
| [Smile](https://github.com/mijorus/smile), Emote | clipboard, GNOME-oriented | none | you paste it yourself |

The `wtype` pickers are the trap: they install and run fine, then quietly do nothing,
because KWin does not implement the protocol they type through.

Where the others are ahead: `im-emoji-picker` and `jockel09/emoji-picker` both offer skin
tone and gender selectors, kaomoji, and favourites, and rofimoji covers arbitrary Unicode
characters, not just emoji. This one has none of those yet. It is also KDE-specific by
design, where rofimoji and bemoji run on anything with a dmenu-style launcher.

## Install

```sh
./contrib/install.sh
```

This builds the binary, puts it in `~/.local/bin`, and installs a hidden `.desktop`
entry. Requires a Rust toolchain and GTK 4.

### Bind a hotkey

System Settings → **Keyboard → Shortcuts → Add New → Command or Script**, enter the full
path `~/.local/bin/emoji-picker`, click the shortcut field and press your key. `Meta+.`
and `Meta+;` are both good choices.

> Add the shortcut through System Settings rather than by editing config files. KDE
> caches the command when your session starts, so a hand-edited `.desktop` file keeps
> launching whatever was there before until you log out.

## Usage

| Key | Action |
| --- | --- |
| type | search by name and keyword |
| ← ↑ ↓ → | move around the grid |
| Enter | insert the selected emoji |
| Tab / Shift-Tab | next / previous category |
| Esc, or click outside | cancel |
| click | insert |

Pressing the hotkey again while the picker is open closes it.

### Options

| Flag | Effect |
| --- | --- |
| `--no-insert` | copy to the clipboard only; insert into nothing |
| `--copy` | also put the emoji on the clipboard when it was inserted directly |
| `--print` | write the chosen emoji to stdout as well |
| `--daemon` | stay resident, so the window appears instantly |
| `--test-im`, `--test-paste` | check one insert route on its own, for debugging |

### Running it resident

By default nothing stays in memory between uses. If you would rather trade ~20 MB of RAM
for an instantly appearing window:

```sh
cp contrib/emoji-picker.service ~/.config/systemd/user/
systemctl --user enable --now emoji-picker
```

The same hotkey works either way — starting or stopping the service is the whole switch.

## Troubleshooting

**Nothing happens when I press the hotkey.** Run `~/.local/bin/emoji-picker` in a
terminal. If it works there, the shortcut is still bound to an old command; re-add it in
System Settings.

**The emoji lands on the clipboard but not in the field.** Some apps (generally older
X11 ones) can't take a direct insert, so the picker asks KDE for permission to paste on
your behalf. Approve the "Remote Desktop" dialog the first time and it will not ask
again. If you declined it, revoke and retry under System Settings → Applications →
Remote Desktop.

**KDE keeps telling me a remote control session started.** That notification comes from
the fallback paste route. `docs/` explains how to mute it, and why you might not want to.

**Emoji show as blank boxes.** Install a colour emoji font, e.g. `noto-fonts-emoji`.

## Configuration

Recents and the saved paste permission live in `~/.config/emoji-picker/state.json`.
Delete it to reset.

## Contributing

See [AGENTS.md](AGENTS.md) for the architecture, how emoji data is regenerated from
Unicode, and the Wayland and KDE specifics that shaped the implementation.

## Licence

Emoji names and keywords are Unicode data; see `data/LICENSE`.
