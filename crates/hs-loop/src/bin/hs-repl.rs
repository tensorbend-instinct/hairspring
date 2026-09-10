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
    task_inbox: Option<PathBuf>,
    interrupt_file: Option<PathBuf>,
    resume: Option<String>,
    fork: Option<String>,
    project_dir: Option<PathBuf>,
}

fn parse_opts(args: &[String]) -> Result<Opts, Box<dyn std::error::Error>> {
    Ok(Opts {
        config: PathBuf::from(arg(args, "--config").expect("--config required")),
        dir: PathBuf::from(arg(args, "--dir").expect("--dir required")),
        feedback: arg(args, "--feedback").as_deref() == Some("on"),
        max_steps: arg(args, "--max-steps")
            .unwrap_or_else(|| hs_loop::DEFAULT_MISSION_MAX_STEPS.to_string())
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
        project_dir: arg(args, "--project-dir").map(PathBuf::from),
        steering_inbox: arg(args, "--steering-inbox").map(PathBuf::from),
        task_inbox: arg(args, "--task-inbox").map(PathBuf::from),
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
    let theme = hs_loop::uipaint::Theme::from_env();
    let md = std::sync::Arc::new(std::sync::Mutex::new(
        hs_loop::uipaint::MarkdownStreamer::with_theme(color, &theme),
    ));
    let md_push = md.clone();
    session.set_delta_sink(Box::new(move |d: &str| {
        let mut err = std::io::stderr();
        match md_push.lock() { Ok(mut s) => {
            s.push(d, &mut err);
        } _ => {
            eprint!("{d}");
        }}
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
        let mut p = hs_loop::uipaint::Painter::with_theme(&mut err, color, &theme);
        p.handle(&ev);
    }));
}

/// UI gap #7: one constructor for every mode - --resume and --fork
/// resolve through `hs_loop::repl::load_session`, so interactive and
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
    // B3 (v5 2.5): the task inbox completes the gateway trio - echo a
    // goal into <dir>/tasks.txt while a mission runs and it is booked
    // (Message, gateway traffic), queued, and run after the close.
    let tasks = opts
        .task_inbox
        .clone()
        .unwrap_or_else(|| opts.dir.join("tasks.txt"));
    session.set_task_inbox(&tasks);
}


/// UI gap #10: the full-screen surface. TTY stdin gets the ratatui
/// surface by default (`HS_TUI=off` falls back to line mode); piped stdin
/// always stays byte-plain line mode. The mission runner lives on a
/// worker thread that owns the session; the UI thread owns the
/// terminal and the `TuiState`.
fn run_fullscreen(
    session: ReplSession,
    opts: &Opts,
) -> Result<(), Box<dyn std::error::Error>> {
    use crossterm::event::{self, Event, MouseEventKind};
    use crossterm::execute;
    use crossterm::terminal::{
        disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
    };
    use hs_loop::tui::{self, TuiState};
    use hs_loop::uipaint::{Theme, UiEvent};
    use ratatui::backend::CrosstermBackend;
    use ratatui::Terminal;
    use std::io::stdout;
    use std::sync::mpsc;
    use std::time::Duration;

    enum TuiMsg {
        Ui(UiEvent),
        Delta(String),
        Done(Result<(hs_loop::MissionResult, u64), String>),
        Switched(Result<(String, String, uuid::Uuid), String>),
        ModelSet(Result<String, String>),
    }

    enum UiCmd {
        Goal(String),
        Switch(uuid::Uuid),
        SetModel(String),
    }

    let (tx, rx) = mpsc::channel::<TuiMsg>();
    let (goal_tx, goal_rx) = mpsc::channel::<UiCmd>();

    // Vitals snapshot before the session moves to the worker.
    let v0 = session.vitals();
    let theme = Theme::from_env();
    let mut st = TuiState {
        theme: theme.clone(),
        model_label: v0.model_label.clone(),
        missions_run: v0.missions_run,
        total_steps: v0.total_steps,
        total_model_calls: v0.total_model_calls,
        total_cost_micros: v0.total_cost_micros,
        stream_short: v0.stream_id.to_string().chars().take(8).collect(),
        ..Default::default()
    };

    let mut session = session;
    // Eric's five #4: the :models picker entries, captured before the
    // session moves to the worker thread.
    let model_entries: Vec<(String, bool)> = session.model_names();
    {
        let txu = tx.clone();
        session.set_ui_sink(Box::new(move |ev| {
            let _ = txu.send(TuiMsg::Ui(ev));
        }));
        let txd = tx.clone();
        session.set_delta_sink(Box::new(move |d: &str| {
            let _ = txd.send(TuiMsg::Delta(d.to_string()));
        }));
    }
    let wcfg = opts.config.clone();
    let wdir = opts.dir.clone();
    let wfeedback = opts.feedback;
    let wmax = opts.max_steps;
    std::thread::spawn(move || {
        let mut session = session;
        while let Ok(cmd) = goal_rx.recv() {
            match cmd {
                UiCmd::Goal(goal) => {
                    let r = session
                        .run_goal(&goal)
                        .map(|m| (m, session.total_cost_micros()))
                        .map_err(|e| e.to_string());
                    let _ = tx.send(TuiMsg::Done(r));
                    // B3: run gateway adds queued during that mission.
                    for queued in session.take_queued_goals() {
                        let r = session
                            .run_goal(&queued)
                            .map(|m| (m, session.total_cost_micros()))
                            .map_err(|e| e.to_string());
                        let _ = tx.send(TuiMsg::Done(r));
                    }
                }
                UiCmd::SetModel(name) => {
                    let r = session
                        .set_model_override(Some(name.clone()))
                        .map(|()| name)
                        .map_err(|e| e.to_string());
                    let _ = tx.send(TuiMsg::ModelSet(r));
                }
                UiCmd::Switch(id) => {
                    match hs_loop::repl::ReplSession::load_resume(
                        &wcfg, &wdir, wfeedback, wmax, id,
                    ) {
                        Ok(new_session) => {
                            let v = new_session.vitals();
                            let label = v.model_label.clone();
                            let short: String =
                                v.stream_id.to_string().chars().take(8).collect();
                            session = new_session;
                            let _ = tx.send(TuiMsg::Switched(Ok((label, short, id))));
                        }
                        Err(e) => {
                            let _ = tx.send(TuiMsg::Switched(Err(e.to_string())));
                        }
                    }
                }
            }
        }
    });

    // Terminal guard: always restore, even on panic unwind.
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = disable_raw_mode();
            let _ = execute!(stdout(), LeaveAlternateScreen);
        }
    }
    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;
    let _guard = Guard;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let mut running = false;

    // UI gap #7 on the full-screen surface: bare --resume opens the
    // picker overlay instead of the line-mode numbered prompt.
    // M16: the live stream, so the picker can exclude it.
    let mut current_stream = v0.stream_id;
    let mut resume_sessions: Vec<hs_loop::repl::SessionInfo> = Vec::new();
    let open_resume_picker = |st: &mut TuiState, current: uuid::Uuid| -> Vec<hs_loop::repl::SessionInfo> {
        let infos = hs_loop::repl::list_sessions_excluding(&opts.dir, current);
        if infos.is_empty() {
            st.push_transcript_line("no prior sessions in this dir to resume");
        } else {
            let entries: Vec<String> = infos
                .iter()
                .enumerate()
                .map(|(k, info)| hs_loop::repl::session_line(k + 1, info))
                .collect();
            st.open_picker(entries);
        }
        infos
    };
    if opts.resume.as_deref() == Some("") {
        resume_sessions = open_resume_picker(&mut st, current_stream);
    }
    let mut last_answer: Option<std::path::PathBuf> = None;
    loop {
        terminal.draw(|f| tui::render_skeleton(f, &st))?;
        while let Ok(msg) = rx.try_recv() {
            match msg {
                TuiMsg::Ui(ev) => st.on_ui_event(&ev),
                TuiMsg::Delta(d) => st.on_answer_delta(&d),
                TuiMsg::Done(r) => {
                    running = false;
                    match r {
                        Ok((m, cost_total)) => {
                            last_answer = Some(m.answer_path.clone());
                            // M13: calls were counted live via
                            // ModelCallEnd; adding m.model_calls here
                            // doubled the HUD count.
                            // M20: one method owns the sequencing -
                            // held answer commits, THEN the done line.
                            st.mission_done_report(
                                m.steps,
                                m.model_calls,
                                m.cost_micros,
                                cost_total,
                                &m.outcome,
                            );
                        }
                        Err(e) => st.push_transcript_line(&format!("mission failed: {e}")),
                    }
                    // Eric's five #1: the mission finished - run the
                    // head of the queued goals next, if any.
                    if let Some(next) = st.next_queued_goal() {
                        running = true;
                        st.push_goal_echo(&next);
                        let _ = goal_tx.send(UiCmd::Goal(next));
                    }
                }
                TuiMsg::Switched(r) => match r {
                    Ok((label, short, id)) => {
                        st.model_label = label;
                        st.stream_short = short.clone();
                        current_stream = id;
                        st.missions_run = 0;
                        st.total_steps = 0;
                        st.total_model_calls = 0;
                        st.total_cost_micros = 0;
                        // M14: restore the resumed session's visible
                        // history BEFORE the marker, so the screen
                        // reads like the session you picked.
                        tui::backfill_transcript(&mut st, &opts.dir, id);
                        st.push_transcript_line(&format!("\u{2500}\u{2500} resumed stream {short}"));
                    }
                    Err(e) => st.push_transcript_line(&format!("resume failed: {e}")),
                },
                TuiMsg::ModelSet(r) => match r {
                    Ok(name) => {
                        st.model_label = name.clone();
                        st.push_transcript_line(&format!(
                            "model › {name} (next mission onward)"
                        ));
                    }
                    Err(e) => st.push_transcript_line(&format!("model switch failed: {e}")),
                },
            }
        }
        if event::poll(Duration::from_millis(60))? {
            match event::read()? {
                Event::Key(k) => {
                    if k.kind != crossterm::event::KeyEventKind::Press {
                        continue;
                    }
                    match tui::handle_key(&mut st, k) {
                        tui::KeyAction::Continue | tui::KeyAction::ToggleAgents => {}
                        tui::KeyAction::ToggleLineage => {
                            st.lineage_view = hs_loop::tui_views::selfmod_view(&opts.dir);
                        }
                        tui::KeyAction::ToggleScorer => {
                            st.scorer_view = hs_loop::tui_views::scorer_view(&opts.dir);
                        }
                        tui::KeyAction::ToggleEvidence => {
                            st.evidence_view = hs_loop::tui_views::evidence_view(&opts.dir);
                        }
                        tui::KeyAction::ToggleTime => {
                            st.time_view = hs_log::StreamReader::open(&opts.dir, current_stream)
                                .ok()
                                .and_then(|r| {
                                    hs_loop::mission_time::MissionTime::decompose(&r).ok()
                                });
                        }
                        tui::KeyAction::Quit => break,
                        tui::KeyAction::Picked(tui::PickerKind::Models, choice) => {
                            let name = choice.split(' ').next().unwrap_or("").to_string();
                            if !name.is_empty() {
                                let _ = goal_tx.send(UiCmd::SetModel(name));
                            }
                        }
                        tui::KeyAction::Picked(tui::PickerKind::Themes, choice) => {
                            if let Some((n, th)) = hs_loop::uipaint::available_themes()
                                .into_iter()
                                .find(|(n, _)| *n == choice)
                            {
                                st.set_theme(n, th);
                            }
                        }
                        tui::KeyAction::Picked(tui::PickerKind::Resume, choice) => {
                            if let Some(id) =
                                resolve_resume_choice(&st, &resume_sessions, &choice)
                            {
                                if running {
                                    st.push_transcript_line(
                                        "(mission in flight - resume after it finishes)",
                                    );
                                } else {
                                    let _ = goal_tx.send(UiCmd::Switch(id));
                                }
                            }
                        }
                        tui::KeyAction::Submit(text) => {
                            let t = text.trim().to_string();
                            if t == ":resume" {
                                resume_sessions = open_resume_picker(&mut st, current_stream);
                            } else if t == ":help" {
                                // M26: the surface's OWN help - the
                                // line-mode REPL_HELP advertised
                                // commands that were dead ends here.
                                for line in hs_loop::tui::TUI_HELP.lines() {
                                    st.push_transcript_line(line);
                                }
                            } else if t == ":status" {
                                st.push_transcript_line(&st.status_line());
                            } else if t == ":history" {
                                let h = st.editor.history_entries();
                                if h.is_empty() {
                                    st.push_transcript_line("(no goals submitted yet)");
                                } else {
                                    for e in h {
                                        st.push_transcript_line(&format!("  {e}"));
                                    }
                                }
                            } else if t == ":models" {
                                let entries: Vec<String> = model_entries
                                    .iter()
                                    .map(|(n, d)| {
                                        let tag = if *n == st.model_label {
                                            " (current)"
                                        } else if *d {
                                            " (default)"
                                        } else {
                                            ""
                                        };
                                        format!("{n}{tag}")
                                    })
                                    .collect();
                                st.open_picker_kind(tui::PickerKind::Models, entries);
                            } else if t == ":theme" {
                                let entries: Vec<String> =
                                    hs_loop::uipaint::available_themes()
                                        .iter()
                                        .map(|(n, _)| n.to_string())
                                        .collect();
                                st.open_picker_kind(tui::PickerKind::Themes, entries);
                            } else if t == ":last" {
                                match &last_answer {
                                    Some(p) => {
                                        st.push_transcript_line(&format!(
                                            "\u{203a} {}",
                                            p.display()
                                        ));
                                        match std::fs::read_to_string(p) {
                                            Ok(body) => {
                                                let theme = st.theme.clone();
                                                st.push_transcript_markdown(&body, &theme);
                                            }
                                            Err(e) => st.push_transcript_line(&format!(
                                                "(unreadable: {e})"
                                            )),
                                        }
                                    }
                                    None => st
                                        .push_transcript_line("(no mission has finished yet)"),
                                }
                            } else if let Some(goal) = t.strip_prefix(":") {
                                st.push_transcript_line(&format!(
                                    "unknown command :{goal} (:help lists commands)"
                                ));
                            } else if !running {
                                running = true;
                                st.push_goal_echo(&t);
                                let _ = goal_tx.send(UiCmd::Goal(t));
                            } else {
                                // Eric's five #1: mid-mission goals
                                // QUEUE (FIFO) instead of dropping.
                                st.queue_goal(&t);
                            }
                        }
                    }
                }
                Event::Mouse(m) => match m.kind {
                    MouseEventKind::ScrollUp => st.transcript_wheel_up(3),
                    MouseEventKind::ScrollDown => st.transcript_wheel_down(3),
                    _ => {}
                },
                // Resize needs no redraw here; everything else is a no-op
                _ => {}
            }
        }
    }
    Ok(())
}


const USAGE: &str = "hairspring - the HAIRSPRING loop: one-shot missions and the interactive REPL

USAGE:
  hairspring run --goal \"<goal>\" --config <rig.toml> --dir <run-dir> [flags]
  hairspring --config <rig.toml> --dir <run-dir> [flags]     (interactive REPL)

FLAGS:
  --config <path>        rig config (tools + models); install.sh wrote one to
                         ~/.config/hairspring/hairspring.toml
  --dir <path>           run directory (streams/, work/, stderr/ plugin logs)
  --goal <text>          one-shot mission (run mode); omit for the REPL
  --feedback on          mission memory feedback (default off)
  --max-steps <n>        step cap per mission (default 50)
  --project-dir <path>   project directory missions are confined to (default: <dir>/work)
  --budget-micros <n>    per-mission spend cap in USD micros
  --wall-secs <n>        wall-clock cap per mission
  --resume [stream-id]   resume a prior session (bare: pick from a list)
  --fork <stream-id>     fork a prior session
  -h, --help             print this text

Live models need their key env (e.g. HS_DEEPSEEK_API_KEY). For a
zero-network trial run the scripted model instead - see the README's
offline quickstart (HS_SEQMODEL_SCRIPT + examples/seqmodel-demo.jsonl).
";

fn main() {
    // Operator-facing errors print as prose via Display, never Rust
    // Debug - the Debug wrapper leaked escaped quotes to the user's
    // terminal on the first-run failure path (2026-09-10).
    if let Err(e) = run() {
        eprintln!("hairspring: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    // Stranger-path burn (2026-09-09): `--help` must not fall into
    // parse_opts and panic on `--config required` - the first command a
    // new user runs prints usage and exits 0.
    if args.iter().skip(1).any(|a| a == "--help" || a == "-h") {
        print!("{USAGE}");
        return Ok(());
    }
    let one_shot_goal = if args.get(1).map(std::string::String::as_str) == Some("run") {
        Some(arg(&args, "--goal").expect("--goal required in run mode"))
    } else {
        None
    };
    let mut opts = parse_opts(&args)?;
    // Mission isolation: the project directory is a mission-start input.
    // Validate + canonicalize NOW (startup), then export so the session
    // loader (work anchor) and every spawned plugin (bwrap confinement,
    // answer-write guard) see the same root. A bad path is a startup
    // error, never a silent fall-back to an unconfined anchor.
    if let Some(dir) = &opts.project_dir {
        let canonical = std::fs::canonicalize(dir)
            .map_err(|e| format!("--project-dir {}: {e}", dir.display()))?;
        if !canonical.is_dir() {
            return Err(format!("--project-dir {} is not a directory", canonical.display()).into());
        }
        unsafe { std::env::set_var("HS_PROJECT_ROOT", &canonical) };
        println!("project root: {} (missions confined to this directory)", canonical.display());
    }
    std::fs::create_dir_all(&opts.dir)?;
    // Stranger-path burn (2026-09-10): a relative --dir leaked the
    // relative anchor downstream - the reported answer_path went out as
    // "relrun/work/..." and on the live surface world artifact proposals
    // stayed relative, drawing "world_path must be absolute" rejections.
    // Canonicalize once at startup so the work anchor, the swarm env, and
    // every reported path share one absolute root.
    opts.dir = std::fs::canonicalize(&opts.dir)
        .map_err(|e| format!("--dir {}: {e}", opts.dir.display()))?;
    // Eric's five #5: the agent.spawn tool plugin learns the session's
    // log root + kernel config from the environment (plugin processes
    // only see env + args; the loop injects the per-call parent
    // stream id itself).
    // SAFETY: all four set_var calls run here at the top of main, before
    // build_session spawns any plugin process and before any thread
    // exists - the process env is only ever read afterwards.
    unsafe {
        std::env::set_var("HS_SWARM_LOG_ROOT", &opts.dir);
        std::env::set_var("HS_SWARM_CONFIG", &opts.config);
        std::env::set_var("HS_SWARM_FEEDBACK", if opts.feedback { "1" } else { "0" });
        std::env::set_var("HS_SWARM_MAX_STEPS", opts.max_steps.to_string());
    }

    // UI gap #7: `--resume` with no id lists prior sessions and lets the
    // operator pick one instead of pasting a raw stream uuid.
    let tui_active = {
        use std::io::IsTerminal;
        std::io::stdin().is_terminal() && std::env::var("HS_TUI").as_deref() != Ok("off")
    };
    if let Some(r) = &opts.resume
        && r.is_empty() && !tui_active {
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

    if let Some(goal) = one_shot_goal {
        // Gap #4: --resume <stream-id> continues a prior session's
        // stream (history replays from the log); default opens fresh.
        let mut session = build_session(&opts)?;
        apply_guards(&mut session, &opts);
        apply_streaming(&mut session);
        let r = session
            .run_goal(&goal)
            .map_err(|e| format!("mission: {e}"))?;
        hs_loop::repl::print_result(&r);
        let mut all_passed = r.passed;
        // B3: gateway adds queued mid-run execute after the close.
        for queued in session.take_queued_goals() {
            let r = session
                .run_goal(&queued)
                .map_err(|e| format!("queued mission: {e}"))?;
            hs_loop::repl::print_result(&r);
            all_passed &= r.passed;
        }
        // T7: the exit code IS the mission contract (Codex/Claude
        // convention) - 0 iff every mission passed, so scripts can rely
        // on `hairspring run ... && next-step`.
        if !all_passed {
            std::process::exit(1);
        }
    } else {
        eprintln!("hairspring repl (:help for commands, :quit to exit)");
        // UI gap #7: interactive honors --resume/--fork like one-shot
        // (the picker resolves to a uuid above; previously the
        // interactive arm ignored it and opened a fresh stream).
        let mut session = build_session(&opts)?;
        apply_guards(&mut session, &opts);
        apply_streaming(&mut session);
        use std::io::IsTerminal;
        if tui_active {
            run_fullscreen(session, &opts)?;
        } else if std::io::stdin().is_terminal() {
            let mut ed = hs_loop::repl::RustylineEditor::new(&opts.dir)
                .map_err(|e| format!("line editor: {e}"))?;
            hs_loop::repl::run_interactive(&mut session, &mut ed)?;
        } else {
            let stdin = std::io::stdin();
            let mut ed = hs_loop::repl::StdinEditor::new(&opts.dir, stdin.lock());
            hs_loop::repl::run_interactive(&mut session, &mut ed)?;
        }
    }
    Ok(())
}


/// Map a picked resume entry back to its stream id: the entry's leading
/// "N)" number indexes the session list shown when the picker opened.
fn resolve_resume_choice(
    _st: &hs_loop::tui::TuiState,
    sessions: &[hs_loop::repl::SessionInfo],
    choice: &str,
) -> Option<uuid::Uuid> {
    let num = choice.split(')').next().unwrap_or("");
    hs_loop::repl::pick_session(sessions, num)
}

#[cfg(test)]
mod tui_bin_tests {
    #[test]
    fn resume_choice_maps_entry_to_stream() {
        let dir = std::env::temp_dir().join("tui-bin-resume-map");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // No sessions recorded: any choice resolves to nothing.
        let infos = hs_loop::repl::list_sessions(&dir);
        assert!(infos.is_empty());
        assert_eq!(
            hs_loop::repl::pick_session(&infos, "1"),
            None,
            "empty session list never resolves"
        );
    }
}
