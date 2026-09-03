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

## Gate 2 (tag gate-2): plugin kernel + Rails

- `crates/hs-kernel` - everything-is-a-plugin kernel (DSH): tools, models,
  and rails are executables declared in one TOML config, speaking
  newline-delimited JSON over stdio. Describe handshake validates identity
  and kind. Crash isolation with one respawn retry. Hot reload on config
  mtime: adding a capability never restarts the harness process.
- Rails (openJiuwen): lifecycle hooks (call.pre_tool/post_tool/pre_model/
  post_model), priority-ordered dispatch with name tie-break, visibility
  gating per subject, rail failures contained and logged.
- Every tool/model call lands on the gate-1 log (kind, latency, cost).

### Gate-2 proof (spec section 10, row 2)

`cargo test -p hs-cli --test gate2_proof -- --nocapture`: a running harness
process gains a brand-new tool and a brand-new model after a config-file-only
change - same pid before and after, harness binaries untouched, all calls on
the verified chain.

## Gate 3 (tag gate-3): inner loop with semantic feedback

- `crates/hs-loop` - the spec section 6 step: observe -> drain_feedback ->
  assemble -> model.call -> validate -> submit -> checker verdict. The
  verdict is recorded as a feedback event in both ablation arms; in the ON
  arm it is injected into the next step (recorded as context_inject: what
  entered the window and why). Feedback costs zero extra model round trips.
- Bench family: 24 deterministic coding tasks, 18 feedback-repairable,
  6 not (checker verdict carries no usable signal). Scripted deterministic
  model: no API spend, and the harness mechanism is what is under test.

### Gate-3 proof (spec section 10, row 3)

`cargo test -p hs-loop --test gate3_proof -- --nocapture`: steps-to-pass ON
3.00 vs OFF 6.00; pass rate 75% vs 0% (no regression); coverage limit 25%
of failure classes carry no injectable fix. Published in PROOF-gate3.txt.

## Gate 4 (tag gate-4): outer loop - Goal Mode, budgets, gateway

- `crates/hs-goal` - Goal records (spec section 5) with completion modes
  self / independent / hybrid; checkers are the only completion authority
  outside self mode; checker triggers are artifact-changed or done-declared,
  not every step. Budgets (steps + cost) read the log's own latency/cost
  records; exceed = budget_update + breakpoint, honest stop. Gateway = async
  append-only inbox drained at step boundaries; cancel/redirect/add_task
  land mid-run without touching progress.
- Every model/tool call is recorded on the mission stream with latency and
  cost (substrate completeness fix applied to gate 3's loop too).

### Gate-4 proof (spec section 10, row 4)

`cargo test -p hs-goal --test gate4_proof -- --nocapture`: 8 planted tasks,
4 of them false completions (visible spec met, hidden test fails, model
declares done). Independent and hybrid modes: 0 false passes (100% caught).
Self mode: 4/4 false plants falsely passed - miss rate measured and
published, not assumed. Gateway cancel/redirect proofs in
`tests/outer_loop.rs` (chains verify, history unbroken, work intact).

## Gate 5 (tag gate-5): sub-agent spawner + swarm operators

- `crates/hs-swarm` - Spawner: a delegated subtask runs the SAME substrate
  as a child stream (same log root, same kernel config). The parent stream
  records a `spawn` event naming the child stream_id; the child runs the
  gate-3 inner loop on its own verifiable stream. Failed children return an
  honest passed=false report; all chains verify.
- `InnerLoop::with_stream` (hs-loop) adopts a pre-created child stream.

### Gate-5 proof (spec section 10, row 5)

`cargo test -p hs-swarm --test gate5_proof -- --nocapture`: 8 delegated
subtasks run as child streams in the parent's log root, every chain
verifies, spawn events name every child, delegation overhead measured:
median 1.9ms (min 1.4, max 4.6) - milliseconds, not deployment.

## Layout for later gates

One crate per spec component: substrate daemon, executor, world service,
scorer service, gateway. See workspace `Cargo.toml` comments.
