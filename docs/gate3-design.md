# Gate 3 design: inner loop with semantic feedback

Proof gate (spec 10, row 3): run a coding benchmark suite with feedback
injection on vs off (ablation): steps-to-pass drops with no regression in
pass rate. Measure the feedback-hiding coverage limit and publish it.

Spec 6 inner loop: observe -> drain_feedback -> assemble -> model.call ->
validate -> submit; feedback (compiler/test/LSP signal) is computed under
the model call and injected into the next step: zero extra model round
trips. context_inject events record what entered the window and why.

## Decisions

- hs-loop crate: InnerLoop drives kernel (gate 2) + log (gate 1). Nothing
  model-specific in the loop.
- World side at this gate: a checker plugin per benchmark family. After each
  write action the checker runs; its verdict is recorded as a feedback
  event and (in the ON arm) injected into the next prompt. In the OFF arm
  it is recorded but withheld from the model context - that single flag is
  the ablation.
- Benchmark: 24 deterministic coding tasks (hs-bench), each "produce
  answer file content X_i". 18 are feedback-repairable (the checker error
  names the needed correction), 6 are not (error carries no usable signal).
  The model plugin is scripted: it parses "expected token" feedback and
  repairs; blind it cycles a fixed candidate list that never contains the
  right answer for repairable tasks. This isolates the harness mechanism
  under test (drain -> inject -> repair in the next step) from model
  quality, which the spec says nothing about.
- Metrics (published in PROOF-gate3.txt): steps-to-pass per arm, pass rate
  per arm, coverage limit = fraction of failure classes feedback cannot
  hide (expected 6/24 = 25% for this suite).

## Cut

No compaction/window pressure, no K/skills, no prefetch (later gates). No
real LLM: no key exists and the spec claims harness-level deltas only.
