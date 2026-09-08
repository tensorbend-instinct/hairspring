//! hs-tb-run: run ONE terminal-bench task as a blind hairspring mission
//! inside the task container. The deliverable is the LIVE container state,
//! not a patch; official tests never enter the mission (Harbor's separate
//! verifier grades after the agent container is torn down). The mission's
//! only stop authority is the agent's own .hs/checks (selfcheck direct mode).
//!
//! Usage:
//!   hs-tb-run --instruction <file> --task-id <id> --model <m>
//!             --feedback on|off --budget-micros N --max-steps N
//!             --run-dir <dir> [--workdir /app] [--wall-secs N]
//!
//! Writes <run-dir>/result.json + mission_prompt.txt + tools.json + ledger.txt.

use std::path::PathBuf;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.windows(2).find(|w| w[0] == name).map(|w| w[1].clone())
}

fn bin(name: &str) -> Result<String, Box<dyn std::error::Error>> {
    let exe = std::env::current_exe()?;
    let p = hs_loop::mcpbridge::resolve_plugin_bin(&exe, name)
        .map_err(|e| format!("hs-tb-run: {e}"))?;
    eprintln!("hs-tb-run: resolved {name} -> {}", p.display());
    Ok(p.display().to_string())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let instruction_path = arg(&args, "--instruction").expect("--instruction");
    let task_id = arg(&args, "--task-id").expect("--task-id");
    let model = arg(&args, "--model").expect("--model");
    let feedback = arg(&args, "--feedback").expect("--feedback on|off") == "on";
    let budget_micros: u64 = arg(&args, "--budget-micros")
        .expect("--budget-micros")
        .parse()
        .map_err(|_| "--budget-micros must be an integer")?;
    let max_steps: u32 = arg(&args, "--max-steps")
        .unwrap_or("200".into())
        .parse()
        .map_err(|_| "--max-steps must be an integer")?;
    let wall_secs: Option<u64> = arg(&args, "--wall-secs").and_then(|v| v.parse().ok());
    let run_dir = PathBuf::from(arg(&args, "--run-dir").expect("--run-dir"));
    let workdir = arg(&args, "--workdir").unwrap_or_else(|| "/app".into());
    // --critic on: the stop authority gains a second gate - an independent
    // critic context that tries to REFUTE the submission after the agent's
    // own checks go green (Eric 2026-09-07). Default off: blind selfcheck.
    let critic = arg(&args, "--critic").as_deref() == Some("on");
    std::fs::create_dir_all(&run_dir)?;

    let instruction = std::fs::read_to_string(&instruction_path)?;
    let log_root = run_dir.join("log");
    let answer_path = log_root.join("work").join(&task_id).join("answer.txt");

    // Every plugin the kernel spawns inherits this env: the workdir is the
    // live task container root, checks run in place, submissions are
    // summaries (the deliverable is machine state, never a patch).
    unsafe {
        std::env::set_var("HS_SWE_WORKSPACE", &workdir);
        std::env::set_var("HS_TERM_WORKDIR", &workdir);
        std::env::set_var("HS_SELFCHECK_DIRECT", "1");
        std::env::set_var("HS_ANSWER_RAW", "1");
        if critic {
            std::env::set_var("HS_TB_INSTRUCTION_FILE", &instruction_path);
            std::env::set_var("HS_TB_ANSWER_FILE", &answer_path);
            std::env::set_var("HS_CRITIC_TRACE", run_dir.join("critic-trace.jsonl"));
        }
    }

    let prompt_args = hs_loop::sweprompt::TbPromptArgs {
        workdir: workdir.clone(),
        instruction: instruction.clone(),
        answer_path: answer_path.display().to_string(),
    };
    let prompt = if critic {
        hs_loop::sweprompt::build_tb_mission_prompt_critic(&prompt_args)
    } else {
        hs_loop::sweprompt::build_tb_mission_prompt(&prompt_args)
    };
    std::fs::write(run_dir.join("mission_prompt.txt"), &prompt)?;
    let mut native_tools = hs_loop::toolschema::tb_tools();
    // MCP tool seam (Eric 2026-09-07 web-tooling order; mirrors hs-swe-run):
    // HS_MCP_SERVERS points at a [[mcp_servers]] TOML; discovered tools
    // register namespaced (mcp.<server>.<tool>) in the kernel config AND the
    // native schema. Discovery failure is a hard error - a half-registered
    // surface is worse than none.
    let mut mcp_fragment = String::new();
    if let Ok(servers_toml) = std::env::var("HS_MCP_SERVERS") {
        let (fragment, mcp_native) =
            hs_loop::mcpbridge::discover_mcp_tools(std::path::Path::new(&servers_toml))
                .unwrap_or_else(|e| {
                    eprintln!("hs-tb-run: {e}");
                    std::process::exit(2);
                });
        mcp_fragment = fragment;
        native_tools.extend(mcp_native);
    }
    std::fs::write(
        run_dir.join("tools.json"),
        serde_json::to_string_pretty(&native_tools).expect("json! values serialize"),
    )?;

    let config = run_dir.join("hairspring.toml");
    std::fs::write(
        &config,
        format!(
            r#"
[[tools]]
name = "answer.submit"
command = ["{answersubmit}"]
subjects = ["*"]

[[tools]]
name = "checker.run"
command = ["{checker}"]
subjects = ["*"]

[[tools]]
name = "repo.read"
command = ["{fileread}"]
subjects = ["*"]

[[tools]]
name = "repo.search"
command = ["{reposearch}"]
subjects = ["*"]

[[tools]]
name = "term.exec"
command = ["{termexec}"]
subjects = ["*"]

[[tools]]
name = "notes.scratch"
command = ["{notescratch}"]
subjects = ["*"]

[[models]]
name = "{model}"
command = ["{model_bin}"]
default = true
"#,
            answersubmit = bin("hs-plugin-answersubmit")?,
            checker = bin(if critic { "hs-plugin-critic" } else { "hs-plugin-selfcheck" })?,
            fileread = bin("hs-plugin-fileread")?,
            reposearch = bin("hs-plugin-reposearch")?,
            termexec = bin("hs-plugin-termexec")?,
            notescratch = bin("hs-plugin-notescratch")?,
            model = model,
            model_bin = bin(&format!("hs-plugin-{model}"))?,
        ),
    ) + &mcp_fragment)?;

    let work_dir = log_root.join("work").join(&task_id);
    std::fs::create_dir_all(&work_dir).expect("work dir");
    unsafe { std::env::set_var("HS_SCRATCH_FILE", work_dir.join("notes.md")) };

    let kernel = hs_loop::swe_kernel(&config, &log_root).expect("kernel load");
    hs_loop::require_visibility(&kernel).unwrap_or_else(|m| {
        eprintln!("STARTUP REFUSED: {m}");
        std::process::exit(2);
    });
    let mut l = hs_loop::InnerLoop::new(kernel, &log_root, feedback, max_steps).expect("loop");
    l.set_budget_micros(budget_micros);
    l.set_tools(serde_json::Value::Array(native_tools));
    if let Some(w) = wall_secs {
        l.set_wall_secs(w);
    }
    l.set_progress_path(&run_dir.join("progress.json"));

    let started = std::time::Instant::now();
    let r = l
        .run_mission_full(&task_id, &prompt)
        .expect("mission run");
    let wall_secs = started.elapsed().as_secs();
    let cost = l.total_cost_micros();

    let result = serde_json::json!({
        "task_id": task_id,
        "model": model,
        "mode": if critic { "tb-critic" } else { "tb-blind" },
        "critic": critic,
        "feedback": feedback,
        "passed": r.passed,
        "steps": r.steps,
        "model_calls": r.model_calls,
        "cost_micros": cost,
        "budget_killed": r.budget_killed,
        "harness_error": r.harness_error,
        "outcome": r.outcome,
        "wall_secs": wall_secs,
    });
    std::fs::write(
        run_dir.join("result.json"),
        serde_json::to_string_pretty(&result).expect("json! values serialize"),
    )?;
    let ledger = format!(
        "{model},{arm},{id},{passed},{steps},{calls},{cost},{killed},{wall}s\n",
        model = model,
        arm = if feedback { "system" } else { "baseline" },
        id = task_id,
        passed = r.passed,
        steps = r.steps,
        calls = r.model_calls,
        cost = cost,
        killed = r.budget_killed,
        wall = wall_secs,
    );
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(run_dir.join("ledger.txt"))
        .map(|mut f| {
            use std::io::Write;
            f.write_all(ledger.as_bytes())
        });
    println!(
        "{}",
        serde_json::to_string(&result).expect("json! values serialize")
    );
    Ok(())
}
