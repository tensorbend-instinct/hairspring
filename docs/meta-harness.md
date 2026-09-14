# Meta-Harness in HAIRSPRING

HAIRSPRING includes a filesystem-native outer loop based on Lee et al.,
[Meta-Harness](https://arxiv.org/abs/2603.28052) and its
[reference implementation](https://github.com/stanford-iris-lab/meta-harness).

`hs-meta-harness` keeps the proposer interface deliberately simple. A proposer
program receives the iteration number, the history root, and an output path. It
can inspect every earlier candidate's source, raw trial traces, external scores,
and reflections with ordinary filesystem tools. An evaluator program receives
the frozen candidate source root, task, trial number, and output path. Evaluator
results, including failures, remain outside the candidate's control.

```text
hs-meta-harness run \
  --root ./meta-history \
  --iterations 5 \
  --trials 2 \
  --tasks ./search-tasks.txt \
  --baseline current \
  --proposer ./propose-candidate \
  --evaluator ./evaluate-candidate
```

The history is append-only by iteration:

```text
iterations/0001/candidates/<name>/
  source/             frozen candidate files
  proposal.json       parent and hypothesis
  reflection.md       proposer diagnosis
  trials/<task>/0001/
    trace.log          raw execution trace
    result.json        normalized external verdict
  score.json           aggregate external score
frontier.json
evolution_summary.jsonl
search_config.json
```

Invalid or incomplete trial evidence counts as zero rather than disappearing.
Re-running against the same root resumes at the next iteration and preserves all
prior source and traces. Search-task scores choose the frontier; keep a separate
held-out or final evaluation outside this search loop before shipping a winner.
