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

use hs_loop::repl::{parse_command, ReplCommand, ReplSession, REPL_HELP};
use std::io::{BufRead, Write};
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
        resume: arg(args, "--resume"),
    })
}

fn apply_streaming(session: &mut ReplSession) {
    // Gap #3: stream model output to stderr as it arrives (stdout stays
    // clean for the result JSON).
    session.set_delta_sink(Box::new(|d: &str| {
        eprint!("{d}");
        let _ = std::io::Write::flush(&mut std::io::stderr());
    }));
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

fn print_result(r: &hs_loop::MissionResult) {
    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "passed": r.passed,
            "steps": r.steps,
            "model_calls": r.model_calls,
            "outcome": r.outcome,
            "budget_killed": r.budget_killed,
            "harness_error": r.harness_error,
            "stream_id": r.stream_id.to_string(),
            "answer_path": r.answer_path.display().to_string(),
        }))
        .expect("json! values serialize")
    );
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let one_shot_goal = if args.get(1).map(|s| s.as_str()) == Some("run") {
        Some(arg(&args, "--goal").expect("--goal required in run mode"))
    } else {
        None
    };
    let opts = parse_opts(&args)?;
    std::fs::create_dir_all(&opts.dir)?;

    match one_shot_goal {
        Some(goal) => {
            // Gap #4: --resume <stream-id> continues a prior session's
            // stream (history replays from the log); default opens fresh.
            let mut session = match &opts.resume {
                Some(id) => {
                    let stream_id = uuid::Uuid::parse_str(id)
                        .map_err(|e| format!("--resume needs a stream uuid: {e}"))?;
                    ReplSession::load_resume(
                        &opts.config,
                        &opts.dir,
                        opts.feedback,
                        opts.max_steps,
                        stream_id,
                    )
                    .map_err(|e| format!("session resume: {e}"))?
                }
                None => ReplSession::load(&opts.config, &opts.dir, opts.feedback, opts.max_steps)
                    .map_err(|e| format!("session load: {e}"))?,
            };
            apply_guards(&mut session, &opts);
            apply_streaming(&mut session);
            let r = session
                .run_goal(&goal)
                .map_err(|e| format!("mission: {e}"))?;
            print_result(&r);
        }
        None => {
            eprintln!("hairspring repl (:help for commands, :quit to exit)");
            let mut session =
                ReplSession::load(&opts.config, &opts.dir, opts.feedback, opts.max_steps)
                    .map_err(|e| format!("session load: {e}"))?;
            apply_guards(&mut session, &opts);
            apply_streaming(&mut session);
            let stdin = std::io::stdin();
            let mut out = std::io::stdout();
            loop {
                eprint!("hs> ");
                out.flush()?;
                let mut line = String::new();
                if stdin.lock().read_line(&mut line)? == 0 {
                    break; // EOF
                }
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }
                match parse_command(&line) {
                    ReplCommand::Quit => break,
                    ReplCommand::Help => eprintln!("{REPL_HELP}"),
                    ReplCommand::Status => {
                        println!(
                            "{}",
                            serde_json::to_string(&serde_json::json!({
                                "stream_id": session.stream_id().to_string(),
                                "cost_usd_micros": session.total_cost_micros(),
                            }))
                            .expect("json! values serialize")
                        );
                    }
                    ReplCommand::LastAnswer => match session.last_answer() {
                        Some(a) => println!("{a}"),
                        None => eprintln!("no mission has run yet"),
                    },
                    ReplCommand::Unknown(c) => {
                        eprintln!("unknown command {c} (:help lists commands)")
                    }
                    ReplCommand::Goal(goal) => match session.run_goal(&goal) {
                        Ok(r) => print_result(&r),
                        Err(e) => eprintln!("mission failed: {e}"),
                    },
                }
            }
        }
    }
    Ok(())
}
