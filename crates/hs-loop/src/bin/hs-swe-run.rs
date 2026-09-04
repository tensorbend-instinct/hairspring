//! hs-swe-run: run ONE SWE-bench-Live instance as a hairspring mission
//! end to end. The workspace must already be prepped (base commit checked
//! out, test_patch applied and committed) - the launch script owns that.
//!
//! Usage:
//!   hs-swe-run --instance <json> --model glm|deepseek --feedback on|off
//!              --budget-micros N --max-steps N --run-dir <dir>
//!
//! Writes <run-dir>/result.json and appends one line to <run-dir>/ledger.txt:
//!   model,arm,instance_id,passed,steps,model_calls,cost_micros,budget_killed

use std::path::{Path, PathBuf};

#[derive(serde::Deserialize)]
struct Instance {
    instance_id: String,
    problem_statement: String,
    #[serde(default)]
    fail_to_pass: Vec<String>,
    #[serde(default, rename = "FAIL_TO_PASS")]
    fail_to_pass_caps: Vec<String>,
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find(|w| w[0] == name)
        .map(|w| w[1].clone())
}

fn bin(name: &str) -> String {
    std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join(name)
        .display()
        .to_string()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let instance_path = arg(&args, "--instance").expect("--instance");
    let model = arg(&args, "--model").expect("--model glm|deepseek");
    let feedback = arg(&args, "--feedback").expect("--feedback on|off") == "on";
    let budget_micros: u64 = arg(&args, "--budget-micros")
        .expect("--budget-micros")
        .parse()
        .unwrap();
    let max_steps: u32 = arg(&args, "--max-steps").unwrap_or("25".into()).parse().unwrap();
    let run_dir = PathBuf::from(arg(&args, "--run-dir").expect("--run-dir"));
    std::fs::create_dir_all(&run_dir).unwrap();

    let raw = std::fs::read_to_string(&instance_path).unwrap();
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let inst: Instance = serde_json::from_value(v.clone()).unwrap();
    let f2p = if !inst.fail_to_pass.is_empty() {
        inst.fail_to_pass
    } else {
        inst.fail_to_pass_caps
    };

    let ws = run_dir.join("ws");
    assert!(ws.join(".git").exists(), "workspace not prepped: {}", ws.display());
    let log_root = run_dir.join("log");
    let answer_path = log_root.join("work").join(&inst.instance_id).join("answer.txt");

    // repo layout for grounding (paths the model may touch)
    let listing = std::process::Command::new("git")
        .args(["ls-files"])
        .current_dir(&ws)
        .output()
        .unwrap();
    let files: Vec<&str> = std::str::from_utf8(&listing.stdout)
        .unwrap()
        .lines()
        .collect();
    let mut layout = String::new();
    for f in files.iter().take(60) {
        layout.push_str(f);
        layout.push('\n');
    }

    let prompt = format!(
        "You are fixing a real bug in the repository checked out at {ws} (base commit, failing tests already added).\n\
         PROBLEM STATEMENT (from the issue tracker):\n{stmt}\n\n\
         The checker will apply your patch and run: {tests}\n\
         It also runs a set of PASS_TO_PASS regression tests; do not break existing behavior.\n\n\
         Repo files (partial listing):\n{layout}\n\
         TOOLS (one tool call per reply, exactly one JSON object, no prose):\n\
         1. {{\"tool\":\"repo.search\",\"args\":{{\"pattern\":\"<literal substring>\"}}}} - find code by substring; returns path:line hits (max 100).\n\
         2. {{\"tool\":\"repo.read\",\"args\":{{\"path\":\"<repo-relative path>\"}}}} - read a file (max 40KB, truncated flag if capped).\n\
         3. {{\"tool\":\"answer.write\",\"args\":{{\"path\":\"<ANSWER_PATH>\",\"content\":\"```diff\\n<one unified diff, paths a/... b/... relative to repo root>\\n```\"}}}} - submit your patch. Ground every hunk in code you actually read: correct file, correct current line numbers, exact context lines. The checker runs automatically after each answer.write and its verdict comes back as FEEDBACK.\n\
         WORKFLOW: search and read to locate the real code FIRST, then write a patch that applies cleanly. \
         The exact ANSWER_PATH value is given to you on the ANSWER_PATH line each attempt. \
         Do not include prose outside the JSON. If you get FEEDBACK, repair and continue.",
        ws = ws.display(),
        stmt = inst.problem_statement.trim(),
        tests = f2p.join(" ; "),
        layout = layout,
    );
    std::fs::write(run_dir.join("mission_prompt.txt"), &prompt).unwrap();

    let config = run_dir.join("hairspring.toml");
    std::fs::write(
        &config,
        format!(
            r#"
[[tools]]
name = "answer.write"
command = ["{answer}"]
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

[[models]]
name = "{model}"
command = ["{model_bin}"]
default = true
"#,
            answer = bin("hs-plugin-answer"),
            checker = bin("hs-plugin-swecheck"),
            fileread = bin("hs-plugin-fileread"),
            reposearch = bin("hs-plugin-reposearch"),
            model = model,
            model_bin = bin(&format!("hs-plugin-{model}")),
        ),
    )
    .unwrap();

    let kernel = hs_kernel::Kernel::load(&config).expect("kernel load");
    let mut l = hs_loop::InnerLoop::new(kernel, &log_root, feedback, max_steps).expect("loop");
    l.set_budget_micros(budget_micros);
    let started = std::time::Instant::now();
    let r = l
        .run_mission_full(&inst.instance_id, &prompt)
        .expect("mission run");
    let wall_secs = started.elapsed().as_secs();
    let cost = l.total_cost_micros();

    // final patch = last answer content (the loop re-runs checker each step;
    // the workspace holds the passing state on success)
    let answer = std::fs::read_to_string(&r.answer_path).unwrap_or_default();
    if let Some(patch) = hs_bench::extract_patch(&answer) {
        std::fs::write(run_dir.join("model_patch.diff"), patch).unwrap();
    }
    let result = serde_json::json!({
        "instance_id": inst.instance_id,
        "model": model,
        "feedback": feedback,
        "passed": r.passed,
        "steps": r.steps,
        "model_calls": r.model_calls,
        "cost_micros": cost,
        "budget_killed": r.budget_killed,
        "wall_secs": wall_secs,
    });
    std::fs::write(
        run_dir.join("result.json"),
        serde_json::to_string_pretty(&result).unwrap(),
    )
    .unwrap();
    let ledger = format!(
        "{model},{arm},{id},{passed},{steps},{calls},{cost},{killed},{wall}s\n",
        arm = if feedback { "system" } else { "baseline" },
        id = inst.instance_id,
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
    println!("{}", serde_json::to_string(&result).unwrap());
    let _ = Path::new("/"); // keep Path import
}
