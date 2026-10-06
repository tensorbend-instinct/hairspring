# dsh (DeepSeek Harness) vs HAIRSPRING - parity matrix

Source for dsh rows: five dsh UI screenshots and a tool/plugin list captured by an
earlier dsh test agent (not re-verified against dsh source in this pass). Public-docs
cross-check: NOT done yet. HAIRSPRING status comes from this repo at 8f1c785.
Status: proven (test or live run) / partial / missing. "Matched-task speed" is
unmeasured except t1 (dsh 112s vs HAIRSPRING 742s before fix, 102s after; n=1 each,
rebuilt task, different model runs).

## Tools (30 in dsh)
| dsh tool | HAIRSPRING | Status |
|---|---|---|
| bash | term.exec (confined, model-shell env) | proven |
| read | repo.read | proven |
| grep, glob | repo.search (one tool; glob not separate - verify) | partial |
| write, edit | edit.anchor, edit.patch | proven |
| read_image | none | missing |
| web_fetch, web_search | MCP bridge exists; no shipped web tools | missing |
| todo_write | notes.scratch (notes, not a task list UI) | partial |
| skill | skill.list, skill.view | proven (tests) |
| subagent, list_agents, send_message, interrupt_agent | agent.spawn, agent.spawn_poll only | partial |
| subagent_fork | none | missing |
| create_goal, get_goal, update_goal | hs-goal crate; not exposed as model tools | partial |
| exit_plan_mode (plan mode) | none | missing |
| ask_user_question | none | missing |
| present | none | missing |
| job_kill, job_list, job_output (background jobs) | none | missing |
| schedule_create/delete/list/update | none | missing |
| workflow | none | missing |

## Surface
| dsh feature | HAIRSPRING | Status |
|---|---|---|
| Workspaces sidebar, per-workspace sessions | TUI /sessions grouped by workspace | partial (TUI only) |
| Session auto-titles, spinner, age | /name, titles partial | partial |
| Live footer: steps, tok/s, tok, cache hit, context % | built (8ba58bd, 05e5fdc) | proven (tests) |
| "Completed in Ns" folding, working ticker | built (b011d21) | proven (tests) |
| Folded reasoning | built | proven (tests) |
| Trajectory tab (Duration/Turns/Calls bars) | hash-chained log + hs-log-cli; no view | partial |
| Slash menu: Goal, Plan, Feedback, Compact, Permission, Model, Export, Add File | /caps /history /name /reasoning /models; no goal/plan/compact/export/permission | partial |
| Queue-while-busy | queue_goal in TUI | partial |
| Modes Standard / PTC / Minimal / Creator | none | missing |
| Plugins (Agent Teams, Auth Review, Dev Tools, Voice, Shell limits, loop dispatch, subagent limits, Web search) | plugin system exists (hs-plugin-*); those plugins do not | partial |
| Settings UI (loopback only) | config file + `setup` | partial |
| Desktop/web app | wrapper scaffold only | missing |
| Compact older history | none | missing |
| Export session log ZIP | none | missing |

## HAIRSPRING-only (not in dsh list)
Declared-check closing with independent critic, hash-chained log verify, held-out
promotion driver, memory scoring, world service: proven by tests (see audit report).

## Open
1. Cross-check dsh rows against dsh public docs/source.
2. Matched-task speed table (needs spend quote).
3. Close the "missing" rows that matter, tests first.
