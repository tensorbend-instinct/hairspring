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
    . "${CARGO_HOME:-$HOME/.cargo}/env"
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

mkdir -p "$PREFIX" "$BINLINK_DIR" "$CONFIG_DIR"
# A managed upgrade is a clean replacement, not copies over an old tree:
# removed plugin names cannot survive and shadow the exact checkout being installed.
rm -rf "$PREFIX/bin.new"
mkdir -p "$PREFIX/bin.new"
for b in $BINS; do
    cp "$SRC/target/release/$b" "$PREFIX/bin.new/$b"
done
rm -rf "$PREFIX/bin.old"
if [ -d "$PREFIX/bin" ]; then mv "$PREFIX/bin" "$PREFIX/bin.old"; fi
mv "$PREFIX/bin.new" "$PREFIX/bin"
rm -rf "$PREFIX/bin.old"
cp "$SRC/examples/seqmodel-demo.jsonl" "$PREFIX/seqmodel-demo.jsonl"
ln -sf "$PREFIX/bin/hs-repl" "$BINLINK_DIR/hairspring"

# The binary must resolve on a normal login shell, not only in this one.
# When the binlink dir is not on PATH, add it to the login profile with an
# idempotent, append-only managed block; unrelated profile content and the
# user's shell setup are left untouched.
case ":$PATH:" in
    *":$BINLINK_DIR:"*) on_path=1 ;;
    *) on_path=0 ;;
esac
MARKER="hairspring PATH (managed by install.sh)"
add_path_block() {
    prof="$1"
    if ! grep -qF "$MARKER" "$prof" 2>/dev/null; then
        {
            printf '# >>> %s >>>\n' "$MARKER"
            printf 'export PATH="%s:$PATH"\n' "$BINLINK_DIR"
            printf '# <<< %s <<<\n' "$MARKER"
        } >> "$prof"
        echo "Added $BINLINK_DIR to PATH in $prof (managed block; open a new shell to pick it up)."
    fi
}
if [ "$on_path" -eq 0 ]; then
    add_path_block "$HOME/.profile"
    case "${SHELL:-}" in
        */zsh) add_path_block "$HOME/.zprofile" ;;
    esac
fi

if [ ! -x "$BINLINK_DIR/hairspring" ]; then
    echo "ERROR: installed $BINLINK_DIR/hairspring is missing or not executable" >&2
    exit 1
fi
if command -v bash >/dev/null 2>&1; then LOGIN_SH=bash; else LOGIN_SH=sh; fi
if [ "$on_path" -eq 1 ]; then
    resolved="$(command -v hairspring 2>/dev/null || true)"
else
    resolved="$("$LOGIN_SH" -lc 'command -v hairspring' 2>/dev/null | head -n 1 || true)"
fi
if [ "$resolved" != "$BINLINK_DIR/hairspring" ]; then
    echo "ERROR: installed $BINLINK_DIR/hairspring but a normal shell resolves ${resolved:-nothing}" >&2
    "$LOGIN_SH" -lc 'type -a hairspring' >&2 2>/dev/null || true
    exit 1
fi
printf 'Resolution:\n'
"$LOGIN_SH" -lc 'type -a hairspring' 2>/dev/null || command -V hairspring
installed_hash="$(sha256sum "$PREFIX/bin/hs-repl" | awk '{print $1}')"
source_commit="$(git -C "$SRC" rev-parse HEAD 2>/dev/null || echo unknown)"
printf 'Source commit: %s\nInstalled binary: %s\nInstalled sha256: %s\n' "$source_commit" "$PREFIX/bin/hs-repl" "$installed_hash"

if [ ! -f "$CONFIG_DIR/hairspring.toml" ]; then
    sed "s|@PREFIX@|$PREFIX|g" "$SRC/hairspring.example.toml" > "$CONFIG_DIR/hairspring.toml"
    echo "Wrote $CONFIG_DIR/hairspring.toml"
fi

cat <<MSG

Installed. Next:
  1. PATH is handled: $BINLINK_DIR was already on it, or a managed block
     was appended to your login profile (open a new shell to pick it up).
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
