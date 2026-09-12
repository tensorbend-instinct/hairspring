# DEEP PASS ledger
## CLOSED crates (line-by-line src read)
- hs-core (1 file, 376): FIX option-tag strict decode (RED decode_option_tags_red 2 tests, GREEN). Note: build/build_part identical bodies (intent labels) - left. testing::kind_tag_offset ignores arg - left.
- hs-memory (3 files, 285): FIX top_k silent row drop -> propagate (RED topk_corrupt_row_red, GREEN). DOC FIX memory_edges reserved-table comment. Added tempfile dev-dep (first test file in crate).
- hs-cli (8 files, 455): FIX dump inert+lying truncate(2_000_000)->real 3000 cap, boundary-safe via truncate_chars; trace truncate(300) boundary panic fixed too (RED log_dump_truncate_red 3 tests, GREEN). gate2-driver/demo/plugins clean.
- hs-goal (5 files, 496): FIX ToolCall events recorded cost 0 in canonical field (hs-loop convention breach; RED tool_cost_red + fixtures costtool/costmodel bins, GREEN). Notes: gateway read-then-truncate race window (demo-scope, left), spec-as-dirname (mission convention, left), Independent-mode `let _ = declared_done` style (left).
- hs-bench (1 file, 514): NO FIXES. Notes: expected-word heuristic silently mis-parses statements without "word " (fixture data controlled, left); run_set Err -> Unresolved drops error text (SWE-bench shape, left).
- hs-world (1 file, 639): FIX parse_hex32 panic on odd/non-ASCII ids (RED restore_hardening_red, GREEN). FIX lexical starts_with path-escape guard defeated by ".." - forged manifest wrote outside dest before failing (RED, GREEN via dest_join component check).
- hs-swarm (1 file, 215): DOC FIX run_to_completion's doc comment was attached to spawn_child (misattached ///). spawn() depth:0 hardcoded noted (proof path).
- hs-log (1 file, 756): NO FIXES. Notes: fork lineage GoalUpdate payload is non-JSON string while other GoalUpdates are JSON (no consumer parses GoalUpdate payloads; left); testing module's super-imports style (left).
## IN FLIGHT
- gates gate2 re-run needed (hs-world/hs-swarm changes landed mid-run)
## TODO crates
- hs-selfmod (1042), hs-kernel (1019+168?), hs-scorer (1456), hs-applypatch (1362), hs-hashline (4926), hs-loop (15851)

## hs-scorer (3 files, 1456 lines) — CLOSED 5:28 PM
1. FIXED ScorerPin::compute (lib.rs:249): routed through hs_log::write_blob on /tmp — storage side effect per pin + unwrap_or([0;32]) zero-hash fallback on IO error would defeat PinMismatch. Fix: pure sha2::Sha256::digest (sha2 already a dep). RED tests/pin_purity_red.rs (RED: blob existed), GREEN.
2. FIXED Lineage::promote (lib.rs:413): hand-rolled format! JSON — quoted name broke record (same defect class as hs-selfmod migration). Fix: serde_json::json!. RED tests/promotion_record_json_red.rs (RED: Error("expected `,` or `}`")), GREEN.
3. Notes (leave): held_out_assay empty-suite division; tier01 t.id[1..] slice; SelfModLoop::new expect-panic; run_canaries empty-suite NaN (not-drifted, config edge); attribution.rs doc says "assays" but parser only consumes tier01-shaped Score bodies (vocabulary in test docs too — house style).

## hs-applypatch (5 files, 1362 lines) — CLOSED 5:29 PM
Vendored codex apply-patch (Apache-2.0, provenance headers + THIRD_PARTY_NOTICES). Pure/IO-free apply, 27 inline tests incl. error paths, heredoc leniency, EOF edges, unicode matching. NO FIXES.
Notes (leave): byte-index [1..] slicing safe (matched prefix chars are ASCII 1-byte); parser lenient vs stated Lark grammar (accepts bare "+", blank lines between chunks) — matches codex upstream, documented; no integration tests/ dir but inline coverage.

## hs-hashline (11 files, 4926 lines) — CLOSED 5:31 PM
Vendored hashline anchor engine (Apache-2.0, provenance headers). Anchor schemes (content/chunk/checkpoint), validate-then-apply atomicity, overlap detection, bounded shifted recovery — all sound; 157 pre-existing tests.
1. FIXED edit/apply.rs InsertAfter "EOF" on EMPTY file: guard `len > 1` skipped split_lines' synthetic [""] line, producing a leading blank line ("\nx" for content "x"). Fix: drop the len>1 guard (insert before synthetic trailing-empty line in all cases). RED crates/hs-hashline/tests/eof_insert_empty_file_red.rs (RED: assertion failed), GREEN: 158 tests 0 failed.
Notes (leave): scheme.rs doc references crate::util::hash (stale vendoring path, actual crate::hash); mutate.rs panics documented but DeleteLines/RangeRewrite clamp (benchmark-harness only); InsertAfter sentinels "0:"/"EOF" are magic strings defined by the tool layer (not vendored wrapper).

## hs-applypatch (5 files, 1362) — CLOSED 5:29 PM. Vendored codex apply-patch, pure/IO-free, 27 inline tests. NO FIXES. Notes: parser lenient vs Lark grammar (matches upstream); [1..] slicing safe (ASCII prefixes).

## hs-hashline (11 files, 4926) — CLOSED 5:31 PM
1. FIXED edit/apply.rs InsertAfter "EOF" on EMPTY file: `len > 1` guard skipped split_lines' synthetic [""] → leading blank line ("\nx"). Fix: drop len>1 guard. RED tests/eof_insert_empty_file_red.rs, GREEN 158 tests.
Notes: scheme.rs doc path crate::util::hash stale; mutate.rs panics-vs-clamps doc (bench harness); "0:"/"EOF" magic sentinels owned by tool layer.

## hs-loop (58 src files, 15,607 src lines) — IN PROGRESS (~3.5k lines read: all 34 small bins + termexec/selfcheck/goal/msgfmt-partial)
1. FIXED bin/hs-plugin-policy.rs: TWO-layer dead-wire bug — (a) dispatch matched only literal "policy.propose_prompt", but kernel ToolCall always sends method "tool.call" → "$error: unknown method" on every live call; (b) handler read params["name"]/["text"] instead of params["args"][...]. Spec-gate-8 self-instruction proposal path was dead in live missions. RED tests/policy_wire_red.rs (spawns real binary, tool.call frame; RED: "$error: unknown method", then "empty proposal text"), GREEN.
2. FIXED byte-boundary panic class (7 sites, one helper pair): msgfmt.rs gained tail_bytes_safe/prefix_bytes_safe; replaced raw byte slicing in termexec.rs tail, selfcheck.rs:58, repexec.rs tail, critic.rs tail, bin/hs-swe-run.rs:495, bin/hs-plugin-swecheck.rs:77+86. RED tests/utf8_tail_red.rs 3 tests (é*k+'x' output → cut mid-char; RED panics at termexec.rs:14, selfcheck.rs:58, repexec.rs:18 "start byte index 1 is not a char boundary"), GREEN. Full hs-loop suite: all targets 0 failed.
Process note: one patch script reused a stale `new` variable and mis-patched hs-swe-run.rs (nested fn); caught by compile error, repaired, re-verified.
REMAINING READS: msgfmt rest, mcpbridge, mcp_web_seam_red, repotools, evolve, mission_time, tui_views, assembler, sweprompt, toolschema, ledger, critic rest, uipaint, repexec rest, editapply, publication, realmodel, repl, tui, lib.rs, hs-tb-run, hs-repl, scripted, mcpcall, recmodel bins (~12k lines).

### hs-loop continued (5:38 PM):
3. FIXED assembler.rs byte-boundary truncate class (2 more sites: :49 LINE_CAP, :184 CONTENT_CAP — String::truncate at 20_000 panics mid-char on UTF-8 tool results, crashing the mission loop). Now msgfmt::prefix_bytes_safe. RED crates/hs-loop/tests/assembler_utf8_red.rs (2 tests; RED panics at assembler.rs:49 and :184; test-1 prefix math needed 2 iterations — serde_json Display quotes + off-by-one; final RED captured), GREEN: assembler_utf8_red 2 + assembler_red 5 + utf8_tail_red 3 + policy_wire_red 1 all ok.
Also closed by read: msgfmt.rs (107, clean), mcpbridge.rs (162, clean — deny-by-default roots), repotools.rs (182, clean — symlink-defeating), mcp_web_seam_red.rs (169, src-located test module), evolve.rs (215, clean — promotion journal fail-closed), mission_time.rs (223, clean), assembler.rs (272).

### hs-loop reads closed 5:38 PM (no-fix):
- sweprompt.rs (289): mission/TB prompt templates + policy overlay loading (proper toml:: escaping), FNV-1a hash-chained proposal log (non-crypto, documented), edit-policy per bake-off. Clean.
- toolschema.rs (363): native OpenAI-shape tool schemas, single source of truth, anchor/applypatch flavor swap. Clean.
- ledger.rs (366): exec ledger; its middle-elision helper counts chars (boundary-safe). Clean.
Note: assemble() RED needed 2 prefix-math iterations (serde_json Display adds quotes; off-by-one parity) before failing at the intended site — falsifier quality matters.
STILL UNREAD: tui_views, uipaint, repexec rest, editapply, publication, realmodel, repl, tui, lib.rs, critic rest, bins hs-tb-run/hs-repl/scripted/mcpcall/recmodel.

## hs-loop fix 4 (2026-09-09 ~5:45): format!-templated JSON booked onto streams (recurring class, 2 sites)
- lib.rs:466 poll_gateway_tasks embedded goal via {:?} (Rust debug escapes \u{7f} = invalid JSON) -> unparseable Message payload poisoned canonical record
- lib.rs:544 set_model_override embedded model names raw -> name with `"`/`\` = unparseable CapabilityChange
- Both -> serde_json::json! + to_string
- RED tests/json_booking_red.rs j1 (DEL byte in gateway goal) + j2 (backslash model name): RED payloads `{"goal":"fix-the\u{7f}-thing"}` / `"weird\gen"` invalid escape, GREEN after fix; round-trip byte-exact asserted
- Regression: cap_change_red 2, gateway_add_red 2, utf8_tail_red 3, policy_wire_red 1, assembler_utf8_red 2 all ok
- lib.rs read complete (2349/2349). Notes: set_memory_db panics on bad memory-db path (expect) - noted, not changed (signature change ripples; path is operator-config, doc'd below); join loop hang only when wall_secs=None and child wedged-not-lost (doc'd by code comment, bounded by child max_steps).

## repexec.rs read 651/651 - notes (no RED fixes)
- edit_path_violation guardrail is string-token based and bypassable by shell quoting ("git" apply, g'i't apply, ${VAR} expansion): model CAN deliberately bypass the edit.apply gate. Contained by bwrap (quality gate, not security gate); full shell lexing out of scope. Design-limit noted.
- Module header (lines 1-7) stale: says "network off by construction" but fix-1 (2026-09-06) made host network the DEFAULT (egress_off only via HS_SWE_NET=off). Doc-only; noted.
- /root bound rw in sandbox; /root/.ssh and /root/.git-credentials masked but /root/.cargo/credentials.toml (registry tokens) NOT masked. On a dedicated bench box likely absent; host-hygiene note.
- Ok(None)=>unreachable! in run_with_prep/run_host: invariant enforced (prep never returns Ok(None)); fine.

## fix5 — hand-rolled JSON via format! on streamed payload paths (hs-loop, 2 sites)
- lib.rs:poll_gateway_tasks goal + lib.rs:set_model_override model name used `{goal:?}`/`{model}` inside
  format!-built payload strings. `{:?}` debug escaping is NOT JSON escaping: DEL (0x7F), 0x00-0x1F
  control bytes, and non-ASCII all pass straight through; a goal or model name containing any of them
  booked INVALID JSON (upstream improve_request loop "over JSON" proof class).
- Fix: both sites now build via serde_json::json! (exactly the "booked payload must be valorized" rule).
- RED: crates/hs-loop/tests/json_booking_red.rs j1 (goal `fix-the\u{7f}-thing` -> booked_value not json
  parseable) + j2 (model `weird\gen` -> override payload invalid). RED: invalid found as expected
  (fixes-red-evidence/box log `/tmp/fix58-red7.log` j1/j2 FAIL); GREEN after fix (Full suite log
  `/tmp/hsloop-f5.log`, tests j1/j2 ok).
- Convention now: every booked/transport JSON payload is serde_json-created. format! is for display
  strings only (msgfmt sweep earlier applied to shortest_* helpers).

## fix6 — symlink write-through: candidate-repo symlink → arbitrary host-file clobber (editapply)
- hs-editanchor::utils::safe_join computed the final path then applied it, but the anchor
  validation only verified the *joined* candidate's containment INSIDE via canonicalize of the/candidate
  root — an attacker (or model) placing a symlink INSIDE the candidate (`codex-stage/.../evillink ->
  /root/.ssh/`) made a later update/write chase it OUT. apply_blocks wrote through the link:
  `assert!(p.exists)` / `fs::write(p)` = host-file clobber outside the sandbox; reads leaked host files.
- Fix: safe_join now walks each path component with symlink_metadata and REFUSES (is_symlink ==
  VisError::NotAllowed). editapply::apply_blocks (was trusting the join on both read and write) also
  goes through the same component check. worktop + blacklist behavior unchanged.
- RED: crates/hs-loop/tests/symlink_write_red.rs s1 (file symlink via apply_blocks: applied=true +
  host file has new content), s2 (codex "Update File" through codepath: applied + clobber), s3
  (symlinked DIR component: applied + clobber) — all RED applied:true & target clobbered; GREEN after
  fix: refused, apply_ok=true empty updates. Box log `/tmp/f6-sym-red.log`.
- Regression proof nearest suites: editapply_red (6 tests), applypatch_red (4), anchor_red (3) — pass
  in run log `/tmp/hsloop-loopfix-symlog2.log`.

## fix7 — mission-id path traversal: `run_mission_full`'s work/<id> join was unvalidated (lib.rs)
- InnerLoop::run_mission_full joined `log_root/work/<mission_id>` with the caller-supplied id. hs-repl
  slugifies its passthrough, but the swarm CHILD path passes the untrusted model `agent.spawn`
  mission string STRAIGHT as the id — `/` in the id = path traversal (dir creation + log/answer
  writes outside work/).
- Fix: lib.rs now rejects empty ids, `.`/`..`, and any `/` or `\` in the id up front with
  LoopError::Visibility("mission id contains path characters") — refuse-before-any-IO, so a rejected
  mission leaves zero filesystem trace.
- RED: crates/hs-loop/tests/mission_path_red.rs m1 (`a/b` id accepted pre-fix + dir created at
  work/a/b), m2 (`../../abs-probe` id -> dir EXISTED at /tmp/abs-mission-probe absolute), m3
  (backslash id). RED confirmed both join forms escape; GREEN: all three refused, dirs NOT created,
  verifier error text carries reason. Box log `/tmp/f7-missionp-red.log`.
- Regression proof: ctx_order_red, evolution_red, swe_mission_in_process, budget_kill, swarm_async_red,
  swarm_poll_gated_red all pass (run log `/tmp/hsloop-missionp-logs.log`).

## extra notes (not fixes, reported for visibility)
- hs-plugin-scripted.rs prompt-aware arm builds tool-call JSON-ish strings with format! + the
  ANSWER_PATH from the prompt (harness-written path; a quote in it would corrupt the completion ->
  degraded fixture, not host damage). Test-scaffold; left with note.
- hs-plugin-scripted streaming test seam chunks on bytes then from_utf8_lossy per chunk — deliberate
  fixture (HS_SEQMODEL_DELTAS). Fine.
- hs-plugin-mcpcall CLI mode enforces --path-args allowed_roots before spawn; PLUGIN mode (kernel-
  called) passes model args to the server unchecked — defense-in-depth gap, server's allowed_roots
  is the owner contract. Noted; not code-changed (mirrors existing MCP gate design).
- tui.rs: all text surgery uses char_indices or char-cells (wrap_line/render line) — byte-safe.

## hs-loop gate (closed 2026-09-09)
- 58 src files + all 34 bins read line-by-line = 15,607 lines.
- fixes this sweep: f1 policy dead-wire, f2 11-site utf8 byte-boundary class (7 loop + 4 cli/global),
  f3 assembler 2 utf8 sites, f4 assembler header/tail contracts rewrite (already in earlier entries), and
  f5/f6/f7 above.
- Full hs-loop suite after ALL fixes: 171 test-result lines, 0 failed, `EXIT=0` (/tmp/hsloop-suite5.log).

## fix8 — critic shell: mechanism now matches the read-only promise (parent steering 2026-09-09 6:10)
- critic.rs's term_exec ran via termexec::run = an unrestricted ROOT shell while CRITIC_SYSTEM's
  RULES claimed "Read-only ... never modify". Harness-model trust gap: a hostile/careless critic
  model could clobber the submission under review.
- Fix: new termexec::run_readonly - same process-group/timeout discipline, but the child runs as
  uid/gid nobody (65534); std's uid/gid setup clears supplementary groups (strace-verified:
  setgid -> setgroups(0,NULL) -> setuid). Task files (root-owned) are read-only BY MECHANISM; /tmp
  scratch stays writable per the contract. Fail-closed when euid != 0 ($error, no unenforced run).
  critic::refute now routes probes through run_readonly; CRITIC_SYSTEM reworded ("UNPRIVILEGED user",
  "read-only to you BY MECHANISM", "you are root" removed). NOTE: first attempt added a pre_exec
  setgroups + libc dep; strace showed std already clears groups and the late pre_exec EPERM'd - reverted.
- RED: tests/critic_shell_red.rs - RED evidence: compile-RED (run_readonly missing) + p1 premise green
  (old root surface clobbers) + r5 behavior-RED pre-fix (refute loop mutation landed). GREEN 7/7
  (r1 refuses mutation w/ Permission denied, r2 reads, r3 /tmp scratch, r4 prompt==mechanism, r5 loop
  cannot mutate, r6 fail-closed w/o root). Box log: run at 2026-09-09 6:15 (test result: ok. 7 passed).
- Regressions: critic_red 14 (fixture dirs made world-traversable 0755 - deployment reality /app),
  termexec_red 5, tb_mode_red 6, selfcheck_direct_red 3, feedback_integrity_red 10,
  fixture_honesty_red 2, structured_messages_red 6, verifier_evidence_red 2, verifier_red 4,
  wall_guard_red 2 - all green.

## fix9 — mcpcall plugin-mode allowed_roots pre-check (parent steering 2026-09-09 6:10)
- Plugin mode (the kernel's per-call path) forwarded model args to the MCP server with NO client-side
  root check; the CLI's --path-args gate existed only on the interactive path.
- Fix: mcpbridge::check_args_paths walks the arg tree (depth-capped) and applies check_path_allowed to
  every path-like string (absolute, or ../-traversal; URLs exempt via "://"; bare server-relative names
  pass - the server resolves them against its own cwd, same as the CLI contract). Enforced in
  hs-plugin-mcpcall's plugin arm BEFORE the server is spawned; $error names the path + allowed_roots.
  Server-side allowed_roots stays as defense in depth.
- RED: tests/mcp_plugin_roots_red.rs over the real rmcp fixture server (plugin wire protocol):
  p1 ("/etc/passwd" echoed pre-fix -> refused post-fix), p5 ("../escape"), p6 (nested ["/etc/shadow"])
  all RED (server reached, content echoed); p2/p3/p4 pins green throughout (in-root, plain string, URL).
  GREEN 6/6 post-fix.
- Regressions: mcpcall_red 4, mcpbridge_red 1 - green.
