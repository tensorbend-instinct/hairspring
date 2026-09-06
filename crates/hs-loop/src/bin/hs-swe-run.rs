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

use hs_memory::MemoryStore;
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
    let exe = std::env::current_exe().unwrap();
    let p = hs_loop::mcpbridge::resolve_plugin_bin(&exe, name)
        .unwrap_or_else(|e| panic!("hs-swe-run: {e}"));
    eprintln!("hs-swe-run: resolved {name} -> {}", p.display());
    p.display().to_string()
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
    let wall_secs: Option<u64> = arg(&args, "--wall-secs")
        .or_else(|| std::env::var("HS_SUBSET_WALL_SECS").ok())
        .and_then(|v| v.parse().ok());
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

    // Goal evaluator f2p = the REAL acceptance command (HS_SWE_F2P, written
    // by ops/subset/run_subset_par.py as "bash <run_dir>/f2p.sh"), never the
    // instance's fail_to_pass test IDs (forensic item 1, 2026-09-06: test IDs
    // ran as shell -> exit 127 -> env_limited in 100% of sessions).
    let goal_cmds: Vec<String> = std::env::var("HS_SWE_F2P")
        .ok()
        .map(|s| s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect())
        .unwrap_or_default();

    // bake-off (2026-09-06): HS_SWE_EDIT_PATH=applypatch|anchor selects the
    // edit tool; anchor mode also puts repo.read into anchored mode (the
    // env is inherited by every plugin process the kernel spawns).
    let edit_path = std::env::var("HS_SWE_EDIT_PATH").unwrap_or_else(|_| "applypatch".to_string());
    unsafe {
        if edit_path == "anchor" {
            std::env::set_var("HS_SWE_READ_ANCHORS", "1");
        } else {
            std::env::remove_var("HS_SWE_READ_ANCHORS");
        }
    }

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

    let policy = match std::env::var("HS_POLICY_TOML") {
        Ok(p) => Some(
            hs_loop::sweprompt::load_policy_overlay(std::path::Path::new(&p))
                .unwrap_or_else(|e| {
                    eprintln!("hs-swe-run: {e}");
                    std::process::exit(2);
                }),
        ),
        Err(_) => None,
    };
    // MCP tool surface (docs/mcp-adapter-gate.md): discover each server's
    // tools through the bridge and register them namespaced. Discovery
    // failure is a hard error - a half-registered surface is worse than none.
    let mut mcp_tools = String::new();
    // Native tool delivery: builtin schemas + every discovered MCP tool
    // with its server-provided input schema (Eric 2026-09-05).
    let mut native_tools = hs_loop::toolschema::builtin_tools_with_edit(&edit_path);
    if let Ok(servers_toml) = std::env::var("HS_MCP_SERVERS") {
        let servers = hs_loop::mcpbridge::load_mcp_servers(std::path::Path::new(&servers_toml))
            .unwrap_or_else(|e| {
                eprintln!("hs-swe-run: {e}");
                std::process::exit(2);
            });
        for s in &servers {
            let out = std::process::Command::new(bin("hs-plugin-mcpcall"))
                .args(["--config", &servers_toml, "--server", &s.name, "--list", "--list-verbose"])
                .output()
                .unwrap_or_else(|e| {
                    eprintln!("hs-swe-run: mcp discovery spawn: {e}");
                    std::process::exit(2);
                });
            if !out.status.success() {
                eprintln!("hs-swe-run: mcp discovery: {}", String::from_utf8_lossy(&out.stderr));
                std::process::exit(2);
            }
            let discovered: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout)
                .unwrap_or_else(|e| {
                    eprintln!("hs-swe-run: mcp discovery parse: {e}");
                    std::process::exit(2);
                });
            for d in &discovered {
                let full = d["name"].as_str().unwrap_or("").to_string();
                let desc = d["description"].as_str().unwrap_or("").to_string();
                let tool = full
                    .strip_prefix(&format!("mcp.{}.", s.name))
                    .unwrap_or_else(|| {
                        eprintln!("hs-swe-run: unexpected tool name {full}");
                        std::process::exit(2);
                    })
                    .to_string();
                mcp_tools.push_str(&format!(
                    "\n[[tools]]\nname = \"{full}\"\ncommand = [\"{}\", \"--plugin\", \"--config\", \"{servers_toml}\", \"--server\", \"{}\", \"--tool\", \"{tool}\", \"--name\", \"{full}\"]\nsubjects = [\"*\"]\n",
                    bin("hs-plugin-mcpcall"),
                    s.name,
                ));
                native_tools.push(hs_loop::toolschema::mcp_tool(
                    &full,
                    &desc,
                    d.get("input_schema").cloned(),
                ));
            }
        }
    }

    let prompt = hs_loop::sweprompt::build_mission_prompt(
        policy.as_ref(),
        &hs_loop::sweprompt::PromptArgs {
            ws: ws.display().to_string(),
            problem_statement: inst.problem_statement.clone(),
            fail_to_pass: if !goal_cmds.is_empty() { goal_cmds.clone() } else { f2p.clone() },
            repo_layout: layout.clone(),
            nudge: std::env::var("HS_SWE_PROMPT_NUDGE").unwrap_or_default(),
            answer_path: answer_path.display().to_string(),
            orientation: hs_loop::sweprompt::probe_orientation(),
            mcp_tools: String::new(),
        },
    );
    std::fs::write(run_dir.join("mission_prompt.txt"), &prompt).unwrap();
    // audit artifact: the exact native tool surface the model operates under
    std::fs::write(
        run_dir.join("tools.json"),
        serde_json::to_string_pretty(&native_tools).unwrap(),
    )
    .unwrap();

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
name = "repo.exec"
command = ["{repoexec}"]
subjects = ["*"]

[[tools]]
name = "policy.propose_prompt"
command = ["{policy}"]
subjects = ["*"]

[[tools]]
name = "{edittoolname}"
command = ["{editpatch}"]
subjects = ["*"]

[[tools]]
name = "notes.scratch"
command = ["{notescratch}"]
subjects = ["*"]

[[models]]
name = "{model}"
command = ["{model_bin}"]
default = true
{mcp_tools}
"#,
            answersubmit = bin("hs-plugin-answersubmit"),
            checker = bin("hs-plugin-swecheck"),
            fileread = bin("hs-plugin-fileread"),
            reposearch = bin("hs-plugin-reposearch"),
            repoexec = bin("hs-plugin-repoexec"),
            policy = bin("hs-plugin-policy"),
            editpatch = bin(if edit_path == "anchor" { "hs-plugin-editanchor" } else { "hs-plugin-applypatch" }),
            edittoolname = if edit_path == "anchor" { "edit.anchor" } else { "edit.patch" },
            notescratch = bin("hs-plugin-notescratch"),
            model = model,
            model_bin = bin(&format!("hs-plugin-{model}")),
            mcp_tools = mcp_tools,
        ),
    )
    .unwrap();

    let work_dir = log_root.join("work").join(&inst.instance_id);
    std::fs::create_dir_all(&work_dir).expect("work dir");
    // notes.scratch storage: per-mission, inherited by tool processes
    unsafe { std::env::set_var("HS_SCRATCH_FILE", work_dir.join("notes.md")) };

    let kernel = hs_kernel::Kernel::load(&config).expect("kernel load");
    let mut l = hs_loop::InnerLoop::new(kernel, &log_root, feedback, max_steps).expect("loop");
    // Gate-8 async verifier seam (promotion-gated): HS_ASYNC_VERIFY=1 turns
    // on speculative continuation + the verdict cache. Off = today's
    // synchronous baseline, untouched.
    if std::env::var("HS_ASYNC_VERIFY").as_deref() == Ok("1") {
        let ws = std::env::var("HS_SWE_WORKSPACE").expect("HS_SWE_WORKSPACE for async verify");
        l.set_async_verify(std::path::Path::new(&ws));
    }
    l.set_budget_micros(budget_micros);
    l.set_tools(serde_json::Value::Array(native_tools));
    if let Some(w) = wall_secs {
        l.set_wall_secs(w);
    }
    l.set_progress_path(&run_dir.join("progress.json"));
    let budget_tokens = arg(&args, "--context-budget-tokens")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or_else(|| hs_loop::default_budget_for_model(&model));
    l.set_context_budget_tokens(budget_tokens);
    if let Ok(ws) = std::env::var("HS_SWE_WORKSPACE") {
        if !goal_cmds.is_empty() {
            l.set_goal_evaluator(std::path::Path::new(&ws), goal_cmds.clone());
        }
    }
    let memory_db = arg(&args, "--memory-db").map(std::path::PathBuf::from);
    if let Some(db) = &memory_db {
        l.set_memory_db(db);
    }
    let started = std::time::Instant::now();
    let r = l
        .run_mission_full(&inst.instance_id, &prompt)
        .expect("mission run");
    let wall_secs = started.elapsed().as_secs();
    let cost = l.total_cost_micros();

    if let Some(db) = &memory_db {
        if let Ok(store) = hs_memory::sqlite::SqliteMemoryStore::open(db).map_err(|e| e.to_string()) {
            for rec in hs_memory::extract::extract_stream(&log_root, r.stream_id, &inst.instance_id, "operator") {
                if let Err(e) = store.put(rec) {
                    eprintln!("memory extract: {e}");
                }
            }
        }
    }

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
        "harness_error": r.harness_error,
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
