# HAIRSPRING

A self-improving agent harness, judged by its own history.

HAIRSPRING runs coding missions inside a strict rig: every model call, tool
call, and verdict lands in an append-only, hash-chained event log that can be
verified, rewound, and replayed. Tools and models are isolated plugin
processes speaking a small NDJSON protocol, so a misbehaving plugin cannot
touch the books. Missions close under a graded contract - the agent's own
declared checks must pass - under explicit dollar and wall-clock guards.
Sub-agent delegation is async: children run concurrently and the parent
joins them before it may pass.

## Install

```sh
git clone https://github.com/tensorbend-instinct/hairspring.git
cd hairspring
./install.sh
```

One command from a clone: it builds the release binaries, installs them to
`~/.local/share/hairspring/bin`, puts a `hairspring` command in
`~/.local/bin`, and writes a starter rig to
`~/.config/hairspring/hairspring.toml`. Rust is installed via rustup if
`cargo` is missing.

## Quickstart

```sh
export HS_DEEPSEEK_API_KEY=<your key>

# One-shot mission
hairspring run --goal "fix the off-by-one in src/parser.rs" \
    --config ~/.config/hairspring/hairspring.toml --dir /tmp/hs-run

# Interactive fullscreen TUI
hairspring --config ~/.config/hairspring/hairspring.toml --dir /tmp/hs-run
```

No API key? The shipped config includes an offline `scripted` model; make it
the default to try the loop with zero network. In the TUI, `:agents` shows
live sub-agent delegations mid-run.

## Architecture

| Crate | What it owns |
|-------|--------------|
| `hs-core` | Event schema, canonical encoding, hash chaining |
| `hs-log` | Per-stream segmented log, content-addressed payloads, fsync durability, torn-tail recovery, rewind/replay |
| `hs-kernel` | Plugin kernel: process isolation, NDJSON protocol, gated dispatch |
| `hs-loop` | The agent loop, REPL, fullscreen TUI, and the built-in tool/model plugins |
| `hs-swarm` | Async sub-agent delegation: concurrent children, atomic booking, join-at-close |
| `hs-goal` | Graded-mission rig: blind self-check, verifier, external grading |
| `hs-hashline` / `hs-applypatch` | Hash-anchored edit application |
| `hs-bench` / `hs-scorer` | Benchmarking and tiered scoring of the harness itself |
| `hs-selfmod` | Guarded self-modification loop |
| `hs-memory` / `hs-world` | Memory and world services |
| `hs-cli` | Log inspection (`hs-log-cli`) and test fixtures |

The event log is the spine: plugins never write it, the loop is its only
writer, and a crash at any point leaves consistent provenance on every
stream.

## Development

```sh
cargo build --workspace
cargo test --workspace --no-fail-fast
cargo clippy --workspace --all-targets   # held at zero warnings
```

The suite is strict RED-first: every behavioral change lands with a failing
test that pins it first. Test sessions, bench output, and proof captures are
runtime artifacts and are never committed.

## License

Proprietary; third-party notices in `THIRD_PARTY_NOTICES.md` and `LICENSES/`.
