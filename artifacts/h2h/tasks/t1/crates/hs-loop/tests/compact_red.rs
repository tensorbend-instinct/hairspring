use hs_loop::{compact_budget, InnerLoop};

#[test]
fn c1_forced_compact_shrinks_budget_unforced_does_not() {
    assert_eq!(compact_budget(400_000, false), 400_000);
    assert_eq!(compact_budget(400_000, true), 50_000);
    assert_eq!(compact_budget(8_000, true), 2_000, "floor");
    assert_eq!(compact_budget(1_000, true), 1_000, "never above the window");
}

#[test]
fn c2_request_is_one_shot_state() {
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("hairspring.toml"), &format!("[[models]]\nname = \"seqmodel\"\ncommand = [\"{}\"]\ndefault = true\n", env!("CARGO_BIN_EXE_hs-plugin-seqmodel"))).unwrap();
    let k = hs_kernel::Kernel::load(&dir.path().join("hairspring.toml")).unwrap();
    let mut l = InnerLoop::new(k, log.path(), true, 3).unwrap();
    assert!(!l.compact_pending());
    l.request_compact();
    assert!(l.compact_pending());
}

#[test]
fn c3_compact_is_a_repl_command() {
    assert!(matches!(hs_loop::repl::parse_command("/compact"), hs_loop::repl::ReplCommand::Compact));
    assert!(matches!(hs_loop::repl::parse_command(":compact"), hs_loop::repl::ReplCommand::Compact));
    assert!(hs_loop::repl::command_completions("/comp").contains(&"/compact".to_string()));
}
