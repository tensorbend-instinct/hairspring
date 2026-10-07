From the repository root, build hs-repl and hs-plugin-scripted, then:

mkdir -p artifacts/h2h/skill-ui/proj/.git
HS_TUI=on HS_SEQMODEL_SCRIPT=$PWD/artifacts/h2h/skill-ui/script.jsonl target/debug/hs-repl --config artifacts/h2h/skill-ui/rig.toml --dir /tmp/hs-skill-fixture --project-dir $PWD/artifacts/h2h/skill-ui/proj --max-steps 1

Type Load parser instructions. The scripted model loads the fixture parser skill. The .git directory intentionally supplies a project boundary; without it nearest enclosing git root is the HAIRSPRING repository, not this fixture directory. The mission remains not passed. This is a load/rendering fixture, not completed parser work or measured model throughput.
