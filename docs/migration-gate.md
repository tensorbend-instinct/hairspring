# Gate 9b - migration transactions + fencing (spec v5)

Imports (Zhao & Zhao, arXiv:2609.00546; v1 preprint, self-reported numbers -
imported as design rules): single-authority fencing and swaps-as-transactions.

## What exists

`hs_selfmod::migration`:
- `Binding { kind: Model|Harness|Executor|Host, reference }` - a replaceable
  substrate binding. The continuity substrate (identity, memory, policy, log)
  is what survives a swap.
- `ContinuityAuthority` - at most one binding holds continuation authority
  over a stream. A second claimant is `FencingViolation`, never a tolerated
  race. Re-claim by the holder is idempotent.
- `Migration` - the swap transaction: quiesce -> checkpoint -> validate ->
  bind -> rehydrate -> resume. `bind` is the single promotion point:
  authority moves old -> new there and nowhere else, exactly once.
  `validate(false)` emits `validate_failed` and forces `abort`, which leaves
  the old variant in authority. Every step lands on the canonical log as a
  `capability_change` event (kind 67) with `old_binding`, `new_binding`,
  `step` - a swap is never invisible to the scorer.
- `Migration::recover` rebuilds an in-flight transaction from the log alone
  (read path; no side store), resuming the protocol where the log left off.

## Proof

`crates/hs-selfmod/tests/migration_red.rs` (red-first): happy-path event
order + single promotion point, failed validation retaining old authority,
second-claimant fencing violation, and kill-mid-transaction recovery.
