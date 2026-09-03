# HAIRSPRING

A self-improving agent harness, judged by its own history.

Rust implementation of `hairspring_engineering_design_v4` (spec in the
closed-loop-signal-lab Drive folder). The build order's 8 proof gates are the
plan; each gate is a commit + tag (`gate-1`, `gate-2`, ...). Nothing is built
ahead of its gate - the spec's cut list applies to this repo.

## Gate 1 (this tag): canonical append-only event log

- `crates/hs-core` - event schema (spec section 3), canonical deterministic
  encoding, hash chaining, event kinds. Kind space reserves
  `capability_delta` / `fitness_delta` / `regression` for gates 7-8
  (runtime-independent fencing + harness-of-harness requirements, 2026-09-02).
- `crates/hs-log` - per-stream segmented log, payload-by-hash blob store,
  fsync-before-ack durability, torn-tail recovery, full-chain verifier,
  rewind/replay.
- `crates/hs-cli` - `hs-demo`: a deterministic killable executor used by the
  gate-1 proof tests, and `hs-log-cli` for verify/replay inspection.

### Gate-1 proof (spec section 10, row 1)

1. Kill the executor mid-run (SIGKILL); resume from the log with zero state
   loss inside the tier-A budget (< 5 s warm).
2. Rewind to an arbitrary seq and replay to identical state.
3. Corrupt a byte in the log; the hash chain catches it at the exact seq.

Run: `cargo test -p hs-log --test gate1_proof -- --nocapture`

## Layout for later gates

One crate per spec component: substrate daemon, executor, world service,
scorer service, gateway. See workspace `Cargo.toml` comments.
