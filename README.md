# emoji-picker

A grid emoji picker for KDE on Wayland, in the style of the Windows and macOS pickers.
Press a hotkey, pick an emoji, and it appears in whatever you were typing in.

![every emoji in one scrolling list, recents pinned at the top]()

## Features

- **Everything in one list.** Recents pinned at the top, then every category behind its
  own header. ~3,900 emoji, straight from Unicode.
- **Search** by name and keyword — "cat", "kitten" and "smiling face" all land where you
  expect.
- **Skin tone** applied across the grid, chosen once in settings.
- **Category tabs** jump to a section, and follow along as you scroll.
- **Inserts directly** into the focused text field. No clipboard round trip, so your
  clipboard history stays clean. Apps that can't take a direct insert fall back to
  clipboard-and-paste automatically.
- **Nothing running in the background.** The hotkey starts it, picking an emoji ends it.
  It opens in well under a tenth of a second, so there is nothing to keep resident.
- **Small.** No widget toolkit: a native Wayland client drawing with cairo. ~37 MB
  resident, of which 14 MB is the colour emoji font itself.

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

Where the others are ahead: `im-emoji-picker` and `jockel09/emoji-picker` both offer a
gender selector, kaomoji and favourites, and rofimoji covers arbitrary Unicode
characters, not just emoji. This one has none of those, and it targets KDE, where
rofimoji and bemoji run on anything with a dmenu-style launcher.

### Other Wayland desktops

Only KDE is tested. Two protocols decide what happens elsewhere:

| | Window opens | Inserts directly |
| --- | --- | --- |
| KDE Plasma | yes | yes |
| Hyprland | yes | via the portal, so a permission dialog each session |
| sway, river, Wayfire | yes | no — clipboard only |
| GNOME | **no** | — |

The window needs `zwlr_layer_shell_v1`, which every wlroots compositor has and Mutter
does not, so on GNOME the picker exits rather than starting. Direct insertion needs
`zwp_input_method_v1`; wlroots compositors implement **v2** instead, so they fall back to
the portal where one is available and to the clipboard where it is not.

## Install

```sh
./contrib/install.sh
```

This builds the binary, puts it in `~/.local/bin`, and installs a hidden `.desktop`
entry. Requires a Rust toolchain, and cairo and pango at runtime — both of which a KDE
desktop already has.

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
| Page Up / Page Down | scroll a screenful |
| Enter | insert the selected emoji |
| Tab / Shift-Tab | next / previous category |
| Ctrl-, | open settings |
| Esc, or click outside | cancel |
| click | insert; a category tab jumps to it |

In the search box:

| Key | Action |
| --- | --- |
| Ctrl-← / Ctrl-→ | move the cursor a word |
| Home / End | start / end of the line |
| Backspace / Delete | delete either side of the cursor |
| Ctrl-Backspace, Ctrl-W | delete the word before the cursor |
| Ctrl-Delete | delete the word after it |
| Ctrl-U | clear |
| Ctrl-V | paste |
| click | put the cursor where you clicked |

Plain ← and → stay with the grid, so the text cursor moves by word rather than by
character.

Pressing the hotkey again while the picker is open closes it.

### Options

| Flag | Effect |
| --- | --- |
| `--no-insert` | copy to the clipboard only; insert into nothing |
| `--copy` | also put the emoji on the clipboard when it was inserted directly |
| `--print` | write the chosen emoji to stdout as well |
| `--test-im`, `--test-paste` | check one insert route on its own, for debugging |
| `--bench`, `--time-launch` | time the search table, or time to first frame |

## Troubleshooting

**Nothing happens when I press the hotkey.** Run `~/.local/bin/emoji-picker` in a
terminal. If it works there, the shortcut is still bound to an old command; re-add it in
System Settings. If it prints `compositor has no wlr-layer-shell`, you are on a desktop
this cannot draw on — see [Other Wayland desktops](#other-wayland-desktops).

**The emoji lands on the clipboard but not in the field.** Some apps (generally older
X11 ones) can't take a direct insert, so the picker asks KDE for permission to paste on
your behalf. Approve the "Remote Desktop" dialog the first time and it will not ask
again. If you declined it, revoke and retry under System Settings → Applications →
Remote Desktop.

**KDE keeps telling me a remote control session started.** That notification comes from
the fallback paste route. [AGENTS.md](AGENTS.md) explains how to mute it, and why you
might not want to.

**Emoji show as blank boxes.** Install a colour emoji font, e.g. `noto-fonts-emoji`.

## Configuration

The gear beside the search box — or `Ctrl-,` — opens the settings: whether to insert or
only copy, whether to always copy as well, the skin tone, how many recents to keep, and a
reset for the saved paste permission. Arrows move and change, Enter activates, Escape goes
back.

Everything lives in `~/.config/emoji-picker/state.json`. Delete it to start over.

## Contributing

See [AGENTS.md](AGENTS.md) for the architecture, how emoji data is regenerated from
Unicode, and the Wayland and KDE specifics that shaped the implementation.

## Licence

MIT — see [LICENSE](LICENSE).

Emoji names and keywords are Unicode data, under their own terms; see `data/LICENSE`.
