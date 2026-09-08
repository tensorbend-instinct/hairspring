//! hs-repl: run the HAIRSPRING harness on a goal, end to end, from the CLI.
//!
//! One-shot:
//!   hs-repl run --goal "fix the parser bug" --config hairspring.toml --dir /path/run
//!       [--feedback on|off] [--max-steps N] [--budget-micros N] [--wall-secs N]
//!   Runs the goal as one mission and prints the result as JSON.
//!
//! Interactive:
//!   hs-repl --config hairspring.toml --dir /path/run [--feedback on|off] [--max-steps N]
//!   One goal per line; :help lists the commands.

use hs_loop::repl::ReplSession;
use std::path::PathBuf;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.windows(2).find(|w| w[0] == name).map(|w| w[1].clone())
}

struct Opts {
    config: PathBuf,
    dir: PathBuf,
    feedback: bool,
    max_steps: u32,
    budget_micros: Option<u64>,
    wall_secs: Option<u64>,
    steering_inbox: Option<PathBuf>,
    interrupt_file: Option<PathBuf>,
    resume: Option<String>,
    fork: Option<String>,
}

fn parse_opts(args: &[String]) -> Result<Opts, Box<dyn std::error::Error>> {
    Ok(Opts {
        config: PathBuf::from(arg(args, "--config").expect("--config required")),
        dir: PathBuf::from(arg(args, "--dir").expect("--dir required")),
        feedback: arg(args, "--feedback").as_deref() == Some("on"),
        max_steps: arg(args, "--max-steps")
            .unwrap_or("25".into())
            .parse()
            .map_err(|_| "--max-steps must be an integer")?,
        budget_micros: arg(args, "--budget-micros")
            .map(|v| v.parse())
            .transpose()
            .map_err(|_| "--budget-micros must be an integer")?,
        wall_secs: arg(args, "--wall-secs")
            .map(|v| v.parse())
            .transpose()
            .map_err(|_| "--wall-secs must be an integer")?,
        steering_inbox: arg(args, "--steering-inbox").map(PathBuf::from),
        interrupt_file: arg(args, "--interrupt-file").map(PathBuf::from),
        resume: args
            .iter()
            .position(|a| a == "--resume")
            .map(|i| {
                // UI gap #7: bare "--resume" (no uuid) opens the picker.
                args.get(i + 1)
                    .filter(|v| !v.starts_with("--"))
                    .cloned()
                    .unwrap_or_default()
            }),
        fork: arg(args, "--fork"),
    })
}

fn apply_streaming(session: &mut ReplSession) {
    // Gap #3: stream model output to stderr as it arrives (stdout stays
    // clean for the result JSON). UI gap #4: prose renders as MARKDOWN
    // while it streams - the streamer is shared so UI events and the
    // post-mission flush keep output ordered (prose tail first).
    use std::io::IsTerminal;
    let color = std::io::stderr().is_terminal();
    let md = std::sync::Arc::new(std::sync::Mutex::new(
        hs_loop::uipaint::MarkdownStreamer::new(color),
    ));
    let md_push = md.clone();
    session.set_delta_sink(Box::new(move |d: &str| {
        let mut err = std::io::stderr();
        if let Ok(mut s) = md_push.lock() {
            s.push(d, &mut err);
        } else {
            eprint!("{d}");
        }
        let _ = std::io::Write::flush(&mut std::io::stderr());
    }));
    let md_flush = md.clone();
    session.set_ui_flush(Box::new(move || {
        let mut err = std::io::stderr();
        if let Ok(mut s) = md_flush.lock() {
            s.finish(&mut err);
        }
    }));
    let md_events = md;
    session.set_ui_sink(Box::new(move |ev| {
        let mut err = std::io::stderr();
        if let Ok(mut s) = md_events.lock() {
            s.finish(&mut err); // prose tail lands before the card
        }
        let mut p = hs_loop::uipaint::Painter::new(&mut err, color);
        p.handle(&ev);
    }));
}

/// UI gap #7: one constructor for every mode - --resume and --fork
/// resolve through hs_loop::repl::load_session, so interactive and
/// one-shot behave identically.
fn build_session(opts: &Opts) -> Result<ReplSession, Box<dyn std::error::Error>> {
    let parse = |v: &Option<String>, flag: &str| -> Result<Option<uuid::Uuid>, Box<dyn std::error::Error>> {
        v.as_ref()
            .map(|s| {
                uuid::Uuid::parse_str(s).map_err(|e| format!("{flag} needs a stream uuid: {e}").into())
            })
            .transpose()
    };
    Ok(hs_loop::repl::load_session(
        &opts.config,
        &opts.dir,
        opts.feedback,
        opts.max_steps,
        parse(&opts.resume, "--resume")?,
        parse(&opts.fork, "--fork")?,
    )?)
}

fn apply_guards(session: &mut ReplSession, opts: &Opts) {
    if let Some(b) = opts.budget_micros {
        session.set_budget_micros(b);
    }
    if let Some(w) = opts.wall_secs {
        session.set_wall_secs(w);
    }
    // Gap #2: every hs-repl mission carries an operator channel by default
    // - echo a line into <dir>/steering.txt to steer the next step,
    // touch <dir>/interrupt to stop cleanly. Flags override the paths.
    let steering = opts
        .steering_inbox
        .clone()
        .unwrap_or_else(|| opts.dir.join("steering.txt"));
    let interrupt = opts
        .interrupt_file
        .clone()
        .unwrap_or_else(|| opts.dir.join("interrupt"));
    session.set_steering_inbox(&steering);
    session.set_interrupt_file(&interrupt);
}


fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let one_shot_goal = if args.get(1).map(|s| s.as_str()) == Some("run") {
        Some(arg(&args, "--goal").expect("--goal required in run mode"))
    } else {
        None
    };
    let mut opts = parse_opts(&args)?;
    std::fs::create_dir_all(&opts.dir)?;

    // UI gap #7: `--resume` with no id lists prior sessions and lets the
    // operator pick one instead of pasting a raw stream uuid.
    if let Some(r) = &opts.resume {
        if r.is_empty() {
            use std::io::IsTerminal;
            if !std::io::stdin().is_terminal() {
                return Err("--resume without an id opens the picker, which needs a TTY; piped mode wants --resume <uuid>".into());
            }
            let sessions = hs_loop::repl::list_sessions(&opts.dir);
            if sessions.is_empty() {
                return Err("no prior sessions in this dir to resume".into());
            }
            eprintln!("prior sessions (newest first):");
            for (i, s) in sessions.iter().enumerate() {
                eprintln!("  {}", hs_loop::repl::session_line(i + 1, s));
            }
            eprint!("resume which? [1-{}] ", sessions.len());
            let mut line = String::new();
            std::io::stdin().read_line(&mut line)?;
            let id = hs_loop::repl::pick_session(&sessions, &line)
                .ok_or("invalid selection")?;
            opts.resume = Some(id.to_string());
        }
    }

    match one_shot_goal {
        Some(goal) => {
            // Gap #4: --resume <stream-id> continues a prior session's
            // stream (history replays from the log); default opens fresh.
            let mut session = build_session(&opts)?;
            apply_guards(&mut session, &opts);
            apply_streaming(&mut session);
            let r = session
                .run_goal(&goal)
                .map_err(|e| format!("mission: {e}"))?;
            hs_loop::repl::print_result(&r);
        }
        None => {
            eprintln!("hairspring repl (:help for commands, :quit to exit)");
            // UI gap #7: interactive honors --resume/--fork like one-shot
            // (the picker resolves to a uuid above; previously the
            // interactive arm ignored it and opened a fresh stream).
            let mut session = build_session(&opts)?;
            apply_guards(&mut session, &opts);
            apply_streaming(&mut session);
            use std::io::IsTerminal;
            if std::io::stdin().is_terminal() {
                let mut ed = hs_loop::repl::RustylineEditor::new(&opts.dir)
                    .map_err(|e| format!("line editor: {e}"))?;
                hs_loop::repl::run_interactive(&mut session, &mut ed)?;
            } else {
                let stdin = std::io::stdin();
                let mut ed = hs_loop::repl::StdinEditor::new(&opts.dir, stdin.lock());
                hs_loop::repl::run_interactive(&mut session, &mut ed)?;
            }
        }
    }
    Ok(())
}
