Run from the repository root, after building hs-repl:

HS_TUI=on target/debug/hs-repl --config artifacts/h2h/goal-ui/wrapup-rig.toml --dir /tmp/hs-wrapup-run --max-steps 3

Type /goal Inspect parser and verify tests. The fixture reads the actual get_goal result, then issues update_goal complete with the returned id and revision. It next emits prose, not an answer submission. The goal becomes complete and disarmed, but the mission remains not passed. /goal shows that terminal state. This tests deterministic control/rendering, not verified task completion or model speed.
