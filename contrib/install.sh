#!/bin/sh
# Build, install the binary, and register the .desktop entry.
set -e
cd "$(dirname "$0")/.."
cargo build --release
mkdir -p "$HOME/.local/bin" "$HOME/.local/share/applications"
install -m755 target/release/pika "$HOME/.local/bin/pika"
install -m644 contrib/pika.desktop "$HOME/.local/share/applications/pika.desktop"
echo "installed to ~/.local/bin/pika"
echo "now bind a hotkey: see README.md"
