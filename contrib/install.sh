#!/bin/sh
# Build, install the binary, and register the .desktop entry.
set -e
cd "$(dirname "$0")/.."
cargo build --release
mkdir -p "$HOME/.local/bin" "$HOME/.local/share/applications"
install -m755 target/release/emoji-picker "$HOME/.local/bin/emoji-picker"
install -m644 contrib/emoji-picker.desktop "$HOME/.local/share/applications/emoji-picker.desktop"
echo "installed to ~/.local/bin/emoji-picker"
echo "now bind a hotkey: see README.md"
