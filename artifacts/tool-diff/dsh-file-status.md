# Native file tools and streaming completion checkpoint

Exact captured dsh read/write/edit names, descriptions and argument schemas are offered in both fresh and resumed REPL sessions. Execution is session-owned, confined to the project root, logged with kernel internal scopes, and gated by permission/plan mode. Legacy file tools remain offered for compatibility during the wider rollout.

Observed RED then GREEN: schema/window/caps, EOF, create/overwrite/version/literal edit/CRLF/mode/symlink guards, output before/after, mutation gate classification, dispatcher and model-visible wire/ledger evidence, missing-parent create. Focused batch 32 pass. Native live read/edit verified in 23.2s, 5 steps / 6 calls, served deepseek-flash across roles. Final note exactly goodbye world plus newline.

Protocol bug: Messages SSE loop awaited HTTP EOF even after message_stop. Hermetic chunked-server test keeps HTTP open after stop: RED on old loop, GREEN on protocol completion boundary. Fourteen transport tests pass. No runtime timeout/watchdog. Original live socket attribution remains partial because original message_stop was not captured. 308 one-second heartbeat samples had max gap 1.0465s while socket counters stayed flat. Stock curl identical reconstructed HTTP2 request completed12.255s; HTTP1.1 completed42.091s; trivial completed0.863s. No provider-wide outage; request/connection cause not observable.

Initial file suite1163 pass /9 fail /1 ignored. Seven need real root and remain unverified; one obsolete internals expectation corrected (3/3 pass), one vendor SSL reset passed unchanged recheck(1/1). Subsequent file suite1165/7/1. Combined completion-fix suite 1167 pass /7 root-dependent fail /1 ignored (1175 total).

Partial: sandbox escalation arguments fail safely rather than implementing approval/retry. Atomic replacement uses process-local mutation serialization, matching a local service locking scope; outside-process races are not ruled out. Full remaining21 dsh tools, author parallel execution, final six-task retiming and native desktop parity are outstanding. No completion claim for the whole adoption.
