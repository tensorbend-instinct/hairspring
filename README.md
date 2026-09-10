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

Missions confine every tool call with bubblewrap (Linux) - install it
first (`apt install bubblewrap` / `dnf install bubblewrap` /
`pacman -S bubblewrap`); the startup preflight checks it and says so if
it is missing. On macOS everything builds and starts, but missions need
a Linux host until a seatbelt backend lands.

## Quickstart

```sh
export HS_DEEPSEEK_API_KEY=<your key>

# One-shot mission
hairspring run --goal "fix the off-by-one in src/parser.rs" \
    --config ~/.config/hairspring/hairspring.toml --dir /tmp/hs-run

# Interactive fullscreen TUI
hairspring --config ~/.config/hairspring/hairspring.toml --dir /tmp/hs-run
```

In the TUI, `:agents` shows live sub-agent delegations mid-run.

### Offline trial (no API key)

The install ships a scripted model that replays a fixed two-step demo
mission with zero network. Point it at the installed script and make it
the default:

```sh
export HS_SEQMODEL_SCRIPT=~/.local/share/hairspring/seqmodel-demo.jsonl
export HS_SCRIPTED_PROMPT_AWARE=1   # scripted model answers the verifier
                                    # audit honestly, not just replay lines
export HS_CRITIC_SCRIPT='tool:grep -q hello hello.txt|clean'
                                    # a deterministic stand-in for the
                                    # checker's phase-2 critic: it really
                                    # probes the submission and reports
                                    # clean only when the probe passes
# in ~/.config/hairspring/hairspring.toml: comment `default = true` on the
# deepseek model, uncomment it on the scripted model (line ready)

hairspring run --goal "write hello.txt containing hello" \
    --config ~/.config/hairspring/hairspring.toml --dir /tmp/hs-demo
```

The demo writes `hello.txt` under `/tmp/hs-demo/work`, declares its own
check, submits, and closes `verified` (checker green + verifier audit) - a
full graded mission with no provider. The critic stand-in is for this
demo only: live missions leave `HS_CRITIC_SCRIPT` unset so the critic
resolves from `HS_CRITIC_MODEL` (deepseek or glm) with a real provider
key, and every abnormal critic exit fails closed. Without
`HS_SEQMODEL_SCRIPT` the scripted model stays inert: missions that call it
get an error naming the variable, and live models are unaffected. A missing
or wrong live key fails the mission with a message naming the key env var -
check the run's `stderr/` logs for a plugin's own dying words.

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

A note on the comments: doc comments cite the project's internal design
spec by gate and section ("gate 3", "spec section 10 row 7") and live
incidents by date. The spec itself is not in this repo; the anchors are
kept so every fix stays traceable to the incident and design section that
motivated it.

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
