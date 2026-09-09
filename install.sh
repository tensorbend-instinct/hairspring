#!/bin/sh
# HAIRSPRING one-command install.
#
#   ./install.sh            (from a clone)
#
# Layout it creates:
#   ~/.local/share/hairspring/bin   the hs-repl binary + runtime plugins
#   ~/.local/bin/hairspring         the command on your PATH
#   ~/.config/hairspring/hairspring.toml  your rig (created once, then yours)
set -eu

PREFIX="${HS_PREFIX:-$HOME/.local/share/hairspring}"
BINLINK_DIR="${HS_BINLINK_DIR:-$HOME/.local/bin}"
CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/hairspring"
SRC="$(cd "$(dirname "$0")" && pwd)"

if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo not found - installing Rust via rustup..."
    curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal
    . "$HOME/.cargo/env"
fi

echo "Building HAIRSPRING (release, locked)..."
(cd "$SRC" && cargo build --release --locked --workspace --bins)

BINS="hs-repl hs-log-cli \
hs-plugin-answer hs-plugin-answersubmit hs-plugin-selfcheck \
hs-plugin-fileread hs-plugin-reposearch hs-plugin-repoexec \
hs-plugin-editapply hs-plugin-notescratch hs-plugin-termexec \
hs-plugin-swarm hs-plugin-scripted hs-plugin-deepseek"

mkdir -p "$PREFIX/bin" "$BINLINK_DIR" "$CONFIG_DIR"
for b in $BINS; do
    cp "$SRC/target/release/$b" "$PREFIX/bin/$b"
done
ln -sf "$PREFIX/bin/hs-repl" "$BINLINK_DIR/hairspring"

if [ ! -f "$CONFIG_DIR/hairspring.toml" ]; then
    sed "s|@PREFIX@|$PREFIX|g" "$SRC/hairspring.example.toml" > "$CONFIG_DIR/hairspring.toml"
    echo "Wrote $CONFIG_DIR/hairspring.toml"
fi

cat <<MSG

Installed. Next:
  1. Make sure $BINLINK_DIR is on your PATH.
  2. Set your model key:  export HS_DEEPSEEK_API_KEY=<key>
     (or point HS_DEEPSEEK_API_KEY_FILE at a file holding it)
  3. Run a mission:       hairspring run --goal "your goal" --config $CONFIG_DIR/hairspring.toml --dir /tmp/hs-run
     Interactive TUI:     hairspring --config $CONFIG_DIR/hairspring.toml --dir /tmp/hs-run

No API key? The config ships an offline "scripted" model - swap
default = true onto it to try the loop with zero network.
MSG
