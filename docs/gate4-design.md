# Gate 4 design: outer loop - Goal Mode, budgets, gateway

Proof gate (spec 10, row 4): plant false-completion cases (artifact exists
but fails hidden tests): independent/hybrid modes catch 100% of them; self
mode's miss rate is measured and reported. Gateway events mid-run leave
progress intact.

Spec 5 Goal record: completion_mode in {self, independent, hybrid},
checkers are plugin refs, budget {wall_time_s, tokens, cost_usd,
iterations, evolution_steps}. Spec 6: goal checks on checker triggers, not
every step; budget exceed -> checkpoint + hand to outer; gateway input is
async, incorporated at execution boundaries.

## Decisions

- hs-goal crate: OuterLoop over kernel + log. Goal = spec + mode +
  checkers + budget. Completion authority: self = model's say-so; independent
  = checkers only, say-so ignored; hybrid = say-so triggers immediate
  checker verification, checker decides.
- Checker trigger (not every step): the artifact changed this step, or the
  model declared done. Independent mode checks on both triggers; that is
  what makes false completion unreachable in that mode.
- Budgets at this gate: max_steps + max_cost_usd_micros, fed by the log's
  own latency/cost fields. Exceed -> budget_update event + breakpoint
  (Decision) event -> BudgetExceeded outcome. wall_time/tokens/
  evolution_steps land with their later gates.
- Gateway: an append-only JSONL inbox file the outer loop drains between
  steps (async, off the hot path; spec 9.6). Events: redirect{new_spec},
  cancel, add_task{spec}. Handled at step boundaries only; progress =
  log + artifact, never destroyed. cancel/redirect recorded as message and
  goal_update events.
- Plants: 8 tasks; the model declares done at attempt 2 on all of them.
  Even-numbered plants are honest (artifact passes hidden tests by then);
  odd-numbered are false completions (visible spec met, hidden test
  fails). Self mode's measured miss rate = 4/8 = 50%; independent and
  hybrid must catch 4/4 = 100% (never report a false pass).

## Cut

No planner/decomposition, no sub-agent spawn (gate 5), no distillation
(gate 7+), no wall_time/tokens budgets yet, no gateway *service* (the
file inbox stands in for ingress; the service is the gate-4 gateway
component productized later).
