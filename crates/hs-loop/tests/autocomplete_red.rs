//! REPL UI gap #6 (Eric 2026-09-08: "just fix the gaps", SOTA-REPL UI
//! investigation): pi/omp tab-complete their commands at the prompt;
//! hs-repl makes the operator type :commands from memory.
//!
//! Contract: the REPL owns a command completer - a pure completion
//! function over :commands plus a rustyline Completer wired into the
//! TTY editor, so Tab at the prompt offers :help, :status, etc.

use hs_loop::repl::{command_completions, CommandCompleter};
use rustyline::completion::Completer;
use rustyline::history::DefaultHistory;
use rustyline::Context;

// R1: prefix completion over the REPL's :commands
#[test]
fn r1_command_prefix_completions() {
    assert_eq!(command_completions(":h"), vec![":help", ":history"]);
    assert_eq!(command_completions(":q"), vec![":quit"]);
    assert_eq!(command_completions(":st"), vec![":status"]);
    assert_eq!(command_completions(":l"), vec![":last"]);
}

// R2: a bare colon offers every command; non-command text offers none
#[test]
fn r2_bare_colon_and_non_commands() {
    let all = command_completions(":");
    for c in [":help", ":status", ":history", ":last", ":quit"] {
        assert!(all.contains(&c.to_string()), "bare colon offers {c}");
    }
    assert!(command_completions("fix the parser").is_empty());
    assert!(command_completions("").is_empty());
}

// R3: the rustyline Completer replaces from the colon, so Tab on
// ":st" yields ":status" in place (not an append at the cursor).
#[test]
fn r3_rustyline_completer_replaces_from_colon() {
    let c = CommandCompleter;
    let hist = DefaultHistory::new();
    let ctx = Context::new(&hist);
    let (start, pairs) = c.complete(":st", 3, &ctx).expect("completion runs");
    assert_eq!(start, 0, "replacement spans the whole line");
    let replacements: Vec<&str> = pairs.iter().map(|p| p.replacement.as_str()).collect();
    assert_eq!(replacements, vec![":status"]);
    // no completion mid-goal
    let (_, pairs) = c.complete("fix it", 6, &ctx).expect("completion runs");
    assert!(pairs.is_empty());
}
