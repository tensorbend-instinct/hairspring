# Real-model A/B plan (pre-registered, not yet run)

Status: NOT RUN. No key route exists yet, so there are no results. Nothing here supports a quality claim.

## Arms (matched)
- A: HAIRSPRING at main, model `deepseek-flash` (DeepSeek-V4.1-Flash), direct to api.deepseek.com through the admission-metered proxy (benchmarks/paired15/spend_proxy.py).
- B: OpenHands SDK 1.49.6 baseline, same model id, same proxy, same sandbox, same step/time policy.
- Optional strong-model reference: `deepseek-v4-pro` on both arms, only if budget remains after the Flash pass.
- Same task set, same hidden-test grader (benchmarks/paired15/score_native.py), same seeds, sequential runs.

## Tasks and seeds
- SWE-bench Verified, 15 difficulty-stratified instances locked at seed 20260926 (benchmarks/paired15/manifest.json).
- 3 repeats per (arm, task): 90 runs. Report solved/15 per repeat with mean and range, steps, wall time.
- Exact verifiers first: preflight.py, reference-patch positive grades and fail-closed controls (controls-final.json) must pass before any paid call.
- Limit: this is a native diagnostic grader, not the official Docker harness, and 15 instances cannot support a state-of-the-art claim. A SOTA claim needs the official evaluation on the full 500-instance set, which this 2-core / 1 GB / 29 GB box cannot run.

## Cost (prices: https://api-docs.deepseek.com/quick_start/pricing, checked 2026-10-01)
- deepseek-flash peak: $0.30 / M input (miss), $0.006 / M (cache hit), $1.20 / M output. Off-peak is half.
- deepseek-v4-pro peak: $1.32 / M input (miss), $0.044 / M (hit), $3.96 / M output.
- Assumed per run: 3-10 M input tokens at ~70% cache hit, 0.15-0.4 M output. About $0.5-$2.0 per Flash run.
- Flash pass, 90 runs: estimate $45-$180, central ~$90. Proxy admission cap per run $2.50, so worst case is bounded and the 90-run pass can be cut to 2 repeats (60 runs, ~$60) if the ledger runs hot.
- Budget: ceiling $150. benchmarks/paired15/ledger.json already books $10.587499 spent, so $139.41 remains. The README text says $19.93; the ledger file is the machine record and the two disagree, to be reconciled against the DeepSeek billing page before launch.

## Launch gate
Launch only when (1) a sanctioned key route exists (key never in Actions secrets or any store outside the vault, never on a command line), (2) Eric approves the estimate above, (3) preflight and controls are green.
