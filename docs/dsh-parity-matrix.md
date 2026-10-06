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
| grep, glob | repo.search (grep) + glob tool (glob_red 3/3) | proven (tests) |
| write, edit | edit.anchor, edit.patch | proven |
| read_image | read_image tool (tools2_red; PNG/JPEG/GIF dims + base64) | proven (tests) |
| web_fetch | web.fetch (SSRF-guarded, tested) | proven (tests) |
| web_search | web.search (Brave API) | proven (live call, 3 of 3 results) |
| todo_write | todo tool (tools2_red) | proven (tests) |
| skill | skill.list, skill.view | proven (tests) |
| subagent, list_agents, send_message, interrupt_agent | agent.spawn, agent.spawn_poll only | partial |
| subagent_fork | none | missing |
| create_goal, get_goal, update_goal | `goal` tool | proven (tests) |
| exit_plan_mode (plan mode) | `plan` tool + dispatcher gate | proven (tests) |
| ask_user_question | ask_user_question (file protocol; UI side not built) | partial |
| present | present tool (tools2_red; records .hs/presented.jsonl) | proven (tests) |
| job_kill, job_list, job_output (background jobs) | `jobs` tool: start/list/output/kill, confined like term.exec | proven (tests) |
| schedule_create/delete/list/update | schedule tool: create/list/update/delete/due (tools2_red; the harness must poll `due`) | partial (store proven; harness firing loop missing) |
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
| Compact older history | /compact + auto budget distillation | proven (command, budget fn); model distillation after /compact not run live |
| Export session log ZIP | hs_loop::export::export_zip + Engine::export_session (export_red 2/2, engine_red e4; python zipfile testzip verified) | proven |

## HAIRSPRING-only (not in dsh list)
Declared-check closing with independent critic, hash-chained log verify, held-out
promotion driver, memory scoring, world service: proven by tests (see audit report).

## Open
1. Cross-check dsh rows against dsh public docs/source.
2. Matched-task speed table (needs spend quote).
3. Close the "missing" rows that matter, tests first.
