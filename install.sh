#!/bin/sh
# HAIRSPRING one-command install.
#
#   ./install.sh            (from a clone)
#
# Layout it creates:
#   ~/.local/share/hairspring/bin   the hs-repl binary + runtime plugins
#   ~/.local/share/hairspring/seqmodel-demo.jsonl  the offline demo script
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
# Pin the target dir: the cp below reads $SRC/target/release, so the build
# must land there even when the operator has CARGO_TARGET_DIR exported
# (stranger burn 2026-09-11: an exported target dir stranded the binaries
# and the install died on the cp).
(cd "$SRC" && CARGO_TARGET_DIR="$SRC/target" cargo build --release --locked --workspace --bins)

BINS="hs-repl hs-log-cli \
hs-plugin-answer hs-plugin-answersubmit hs-plugin-selfcheck hs-plugin-critic \
hs-plugin-fileread hs-plugin-reposearch hs-plugin-repoexec \
hs-plugin-editapply hs-plugin-notescratch hs-plugin-termexec \
hs-plugin-swarm hs-plugin-policy hs-plugin-scripted hs-plugin-deepseek hs-plugin-provmodel hs-promote hs-plugin-gatemodel hs-plugin-checker"

mkdir -p "$PREFIX/bin" "$BINLINK_DIR" "$CONFIG_DIR"
for b in $BINS; do
    cp "$SRC/target/release/$b" "$PREFIX/bin/$b"
done
cp "$SRC/examples/seqmodel-demo.jsonl" "$PREFIX/seqmodel-demo.jsonl"
ln -sf "$PREFIX/bin/hs-repl" "$BINLINK_DIR/hairspring"

if [ ! -f "$CONFIG_DIR/hairspring.toml" ]; then
    sed "s|@PREFIX@|$PREFIX|g" "$SRC/hairspring.example.toml" > "$CONFIG_DIR/hairspring.toml"
    echo "Wrote $CONFIG_DIR/hairspring.toml"
fi

cat <<MSG

Installed. Next:
  1. Make sure $BINLINK_DIR is on your PATH.
  2. Add your model key:  hairspring setup
     (guided: checks what is configured, stores the key owner-only under
     $CONFIG_DIR/keys/, validates it, and prints the next command)
  3. Run a mission in your project:
                          hairspring run --goal "your goal" --config $CONFIG_DIR/hairspring.toml --dir ./hs-run --project-dir .
     Interactive TUI:     hairspring --config $CONFIG_DIR/hairspring.toml --dir ./hs-run
     (--dir holds run state; missions are confined to --project-dir,
      default <dir>/work - the TUI asks once, and the root always prints)

No API key? Offline demo mission (zero network):
  export HS_SEQMODEL_SCRIPT=$PREFIX/seqmodel-demo.jsonl
  export HS_SCRIPTED_PROMPT_AWARE=1
  In $CONFIG_DIR/hairspring.toml, move default = true from the
  deepseek model to the scripted model (it has a commented line ready),
  then:
  hairspring run --goal "write hello.txt containing hello" --config $CONFIG_DIR/hairspring.toml --dir ./hs-demo
MSG
