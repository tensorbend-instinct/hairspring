# Messages migration checkpoint

Local implementation uses https://api.deepseek.com/anthropic/v1/messages and deepseek-flash. Stock sse-codec handles event framing. Native thinking/signature/tool-use blocks are preserved through author log replay and critic history. Parallel tool results are grouped into the immediately following user message.

Observed RED regressions: initial transport/default route, native history, parallel results, author history and silent multi-call discard. Focused final batch 50 pass / 0 fail. Critic integration file then passes 2 tests including the added two-turn native parallel-history/served identity coverage.

Prior grouped build t1: verified, 56.0 seconds, 9 steps / 10 calls. Final build t1 verified in 42.3 seconds, 2 steps / 3 calls, served deepseek-flash in author/verifier/critic events, no HTTP400. Final full suite: 1143 pass / 7 fail / 1 ignored (1151 total). All seven failures are in critic_shell_red and repexec_sandbox_red and require real root; unverified here. First full suite 1139 pass / 7 fail / 1 ignored; seven need real root and are unverified. Mid-fix suite 1139 pass / 8 fail / 1 ignored picked up the new parallel-results RED test while running; final suite is on the grouped fix.

Not parity: exact dsh 24-tool semantics still missing; author accepts a single native tool call and now fails closed instead of silently dropping extra calls. No transport watchdog, critic cap or checker relaxation added. Native desktop and final six-task comparison remain outstanding. Returned served model propagates into author/verifier kernel log events and critic model_end trace; final live identity check pending.
