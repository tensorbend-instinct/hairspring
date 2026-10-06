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
| subagent, list_agents, send_message, interrupt_agent | agent.spawn/spawn_poll + agent.list/send/interrupt over the swarm registry (agentctl_red 4/4, agent_control_red 2/2: interrupt and steering inbox reach a live child) | proven |
| subagent_fork | agent.fork rewritten to agent.spawn with ledger context (fork_red 2/2: live child spawned and finished; context formatter unit-tested) | proven |
| create_goal, get_goal, update_goal | `goal` tool | proven (tests) |
| exit_plan_mode (plan mode) | `plan` tool + dispatcher gate | proven (tests) |
| ask_user_question | ask_user_question tool + appback pending/answer + app question card (appback_red a1; live in app: card shown, click answered, tool returned) | proven |
| present | present tool (tools2_red; records .hs/presented.jsonl) | proven (tests) |
| job_kill, job_list, job_output (background jobs) | `jobs` tool: start/list/output/kill, confined like term.exec | proven (tests) |
| schedule_create/delete/list/update | schedule tool + Engine::fire_due_schedules firing loop (tools2_red 5/5, engine_red e5: due fires once, advances, refires next period) | proven |
| workflow | workflow tool (define/get/list/delete) + Engine::run_workflow (tools2_red t6, engine_red e6) | proven |

## Surface
| dsh feature | HAIRSPRING | Status |
|---|---|---|
| Workspaces sidebar, per-workspace sessions | Engine::sessions + app sidebar grouped by workspace (engine_red e2; app screenshot) | proven |
| Session auto-titles, spinner, age | titles from the goal, age_secs, footer working ticker (appback_red a3; app screenshot) | proven |
| Live footer: steps, tok/s, tok, cache hit, context % | built (8ba58bd, 05e5fdc) | proven (tests) |
| "Completed in Ns" folding, working ticker | built (b011d21) | proven (tests) |
| Folded reasoning | built | proven (tests) |
| Trajectory tab (Duration/Turns/Calls bars) | Engine::trajectory + app tab with Duration/Model/Tools bars and Turns/Calls/Cost (appback_red a2; app screenshot) | proven |
| Slash menu: Goal, Plan, Feedback, Compact, Permission, Model, Export, Add File | appback slash_commands/filter/run_slash, app menu as you type (appback_red a4; app screenshot of /co) | proven |
| Queue-while-busy | app submit queues behind a running mission and drains in order (appback_red a5; app screenshot: B and C queued while A ran) | proven |
| Modes Standard / PTC / Minimal / Creator | hs_loop::modes + InnerLoop/Engine::set_mode narrowing the offered tools, answer.submit always kept (engine_red e7) | proven |
| Plugins (Agent Teams, Auth Review, Dev Tools, Voice, Shell limits, loop dispatch, subagent limits, Web search) | app Plugins panel lists configured tools and models (appback_red a6); Voice = `speak` tool via stock piper TTS (tools2_red t7 fake command, t8 live piper: 1.82s WAV) | proven |
| Settings UI (loopback only) | app Settings: mode (applied live), max_steps (applied at open), permission ask/auto enforced by the dispatcher (permission_red 2/2: allow runs, deny refuses, timeout denies) | proven |
| Desktop/web app | Tauri v2 app on the in-process Engine (apps/desktop): setup, chat with fold rows, sessions, trajectory, slash menu, queue, question card, plugins, settings; release build run under Xvfb with a scripted model; not rebuilt from a fresh clone | proven (scripted model; no live-model run) |
| Compact older history | /compact + auto budget distillation | proven (command, budget fn); model distillation after /compact not run live |
| Export session log ZIP | hs_loop::export::export_zip + Engine::export_session (export_red 2/2, engine_red e4; python zipfile testzip verified) | proven |

## HAIRSPRING-only (not in dsh list)
Declared-check closing with independent critic, hash-chained log verify, held-out
promotion driver, memory scoring, world service: proven by tests (see audit report).

## Open
1. Cross-check dsh rows against dsh public docs/source.
2. Matched-task speed table (needs spend quote).
3. Close the "missing" rows that matter, tests first.
