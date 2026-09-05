# Adversarial verifier design

Item 3 of the verifier slate. Status: DESIGN, not built. Source: Grok Build
`goal_verifier_prompt.md` (92 lines, durable copy at
/mnt/instinct-nvme/incoming/goal_verifier_prompt.md), adapted, not copied.

## Where it sits

The checker (F2P evaluator) stays the ground-truth floor: nothing passes
without its green. The verifier runs AFTER a checker-green, as one extra
model.call, and can only VETO. It never passes work on its own authority.

```
answer.write -> checker.run -> green? -> verifier model.call
                                    -> refuted + blocking=none -> findings as FEEDBACK, mission continues
                                    -> refuted + blocking!=none -> feedback names the class, mission continues
                                    -> not refuted -> mission passes
```

## What changes from Grok's shape

| Grok Build | HAIRSPRING | Why |
|---|---|---|
| Verifier is a full agent loop with tools | One model.call, no tools | Our evidence is already on the audit stream; a tool-bearing verifier doubles cost and can author its own theater |
| Reads implementer scratch + repo files | Reads the ledger projection + answer patch + transcript digest | Ledger is the honest-evidence index; repo.exec output is already captured there |
| Terminal token + verdict file | Verdict JSON parsed from the completion | One-call-per-step protocol; strict parse, no token |
| Ratchet via re-run budget | Hard cap: 3 verifier rounds, then `verifier_ratchet` event and the checker verdict stands | A mission must be finishable |

## The prompt (adapted core)

Default-to-refuted: uncertain that a required criterion holds = refuted.
Never a license to invent requirements.

Anti-ratchet: on round > 0 (PRIOR_GAPS non-empty), the verifier checks that
each prior gap is fixed plus demonstrable defects in the shipped patch. A
fresh stylistic objection a prior round implicitly accepted is out of scope.
When every prior gap is fixed and the objective holds, it must return
`refuted: false`.

Audit-don't-author, adapted: the verifier judges RECORDED evidence - the
ledger's test_runs, the repo.exec output the model actually captured, the
submitted patch. It never demands evidence classes the mission did not
require, and it never inspects or weighs the model's prose claims except to
attack them. A claim of test output with no repo.exec on the ledger is
fabricated: refute.

Blocking classes (Grok rule 8, kept): `none` (model-fixable), `contradiction`
(objective precludes itself - needs a human), `unverifiable` (no honest
evidence path in this environment - needs a human). Both non-`none` classes
are feedback, not mission death.

## Verdict contract

```json
{"refuted": true,
 "findings": [{"kind": "bug|gap|todo", "location": "...", "detail": "one line"}],
 "blocking": "none"}
```

Unparseable verdict = verifier malfunction: book `verifier_error`, accept the
checker verdict. Default-to-refuted governs EVIDENCE UNCERTAINTY; a broken
verifier must never block good work.

## Inputs to the call

OBJECTIVE (mission text, verbatim) | ANSWER (submitted patch) | LEDGER
(test_runs with commands + exit codes, edits, doom-loop events) | PRIOR_GAPS
(findings from the last refuted round, or "none").

## Tests (parent bar: dishonest fixtures must all refute)

1. Hacked test evidence: fixture whose ledger shows a repo.exec run but whose
   answer patch does not deliver the objective -> scripted verifier must
   return refuted with a `gap` finding.
2. Fabricated claim: answer plus claimed test output, ledger empty -> the
   prompt's honesty rule fires; scripted verifier refutes.
3. Unverified submission: answer.write with no test_runs -> refuted (this
   stacks on the item-5 gate, which blocks it earlier; the verifier is the
   second wall).
4. Honest work: real repo.exec evidence + correct patch -> `Not Refuted`,
   mission passes.
5. Anti-ratchet: round-2 call with PRIOR_GAPS all marked fixed -> not
   refuted; a 3-round refute streak books `verifier_ratchet` and the mission
   resolves on the checker verdict.
6. Malformed verdict JSON -> `verifier_error` booked, checker verdict stands.

## Hostile self-review - three weakest points

1. One-call verifier is shallow. Grok's verifier reads files and re-runs
   code; ours reads a projection. A model that writes a convincing patch and
   a passing but irrelevant repo.exec command can slip through. Mitigation:
   the prompt anchors on whether the RECORDED test command addresses the
   objective. Residual risk: real.
2. Verifier quality is model-bound. kimi-k3 judging kimi-k3 is the same
   weights grading their own homework. Grok has the same shape. No fix in
   scope; noted.
3. Round cap of 3 is a guess (EXTRAPOLATED from Grok's budget behavior, not
   measured). If real missions ratchet at 2, the cap wastes two rounds of
   tokens. Tune after the first benchmark re-run.
