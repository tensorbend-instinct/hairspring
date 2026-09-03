# Gate 5 design: sub-agent spawner + swarm operators

Spec row (section 10): "A delegated subtask runs the same substrate as a
child stream; delegation overhead measured in milliseconds, not deployment."
Falsifiable: if the child run is not a verifiable child stream on the same
log, or delegation overhead is not measured and published in ms, the gate
fails.

## What "same substrate" means here

- One log root. A spawn creates a CHILD STREAM in the same hs-log store as
  the parent stream (segmented per stream, per gate 1). The parent stream
  records `spawn` (EventKind::Spawn, reserved in gate 1's schema: "sub-agent
  creation + mission ref") with { child_stream_id, mission, budget }.
- Same kernel configuration: the child loop loads the same plugin set; the
  Nth agent costs a spawn call, not a new engine (spec: fan-out, PROVEN in
  openJiuwen).
- Child runs the gate-3 InnerLoop (observe/drain/assemble/call/validate/
  submit) writing its own ModelCall/ToolCall/Feedback events to its own
  stream; on completion it emits a subagent_report event the parent can
  collect.

## Swarm operators (gate scope; keep minimal, cut list applies)

- `spawn(parent_log_root, parent_stream, mission, budget) -> child handle`
  synchronous spawn of a child loop on a child stream; records the Spawn
  event on the parent stream with the child stream_id.
- `run_to_completion(child)` - drives the child inner loop.
- `collect(child) -> report` - reads the child's outcome (pass/fail, steps,
  cost) from its stream; parent records subagent_report.
- Parallel fan-out: N children on threads, each with its own stream and
  work dir; they never share mutable state except the append-only log
  (separate streams; the log's per-stream locking from gate 1 covers it).
- Budget propagation: child budget comes from the parent's goal budget
  allocation (spec pseudocode: spawn(subtask, budget=allocate(goal,
  subtask))); child steps+cost read from its own stream records.

## What gets measured

- Delegation overhead: wall time from the parent's spawn decision to the
  child's first appended event, in milliseconds. Published per-spawn and as
  a distribution over N=8 spawns. Assert median < 1000ms and publish actuals
  (expected: single-digit ms; the claim is "not deployment").
- Child chain verifies independently (hs-log verifier per stream).
- Parent chain contains the spawn events linking to each child stream_id.

## Proof test shape (tests/gate5_proof.rs)

1. Parent loop on task family A spawns 8 children (parallelizable subtasks,
   scripted model, answer/checker plugins from gate 3).
2. Assert: 8 child streams in the SAME log root, each chain verifies, each
   child passed its subtask; parent stream has 8 Spawn events whose payloads
   name the 8 child stream_ids; 8 subagent_report events back.
3. Assert: delegation overhead measured, median under threshold, published.
4. Adversarial: kill a child mid-run (drop its writer) - parent still
   collects the remaining reports and records the failure honestly.

## Cut list for gate 5

No gateway/goal integration (gate 4 owns it; spawner is used by the outer
loop in gate 6+), no distillation, no prefetch, no world observation
(gate 6), no scorer involvement (gate 7).
