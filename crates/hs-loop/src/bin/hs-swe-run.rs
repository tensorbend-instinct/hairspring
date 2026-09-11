//! hs-swe-run: run ONE SWE-bench-Live instance as a hairspring mission
//! end to end. The workspace must already be prepped (base commit checked
//! out, `test_patch` applied and committed) - the launch script owns that.
//!
//! Usage:
//!   hs-swe-run --instance <json> --model glm|deepseek --feedback on|off
//!              --budget-micros N --max-steps N --run-dir <dir>
//!
//! Writes <run-dir>/result.json and appends one line to <run-dir>/ledger.txt:
//!   `model,arm,instance_id,passed,steps,model_calls,cost_micros,budget_killed`

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
    args.windows(2).find(|w| w[0] == name).map(|w| w[1].clone())
}

fn bin(name: &str) -> Result<String, Box<dyn std::error::Error>> {
    let exe = std::env::current_exe()?;
    let p = hs_loop::mcpbridge::resolve_plugin_bin(&exe, name)
        .map_err(|e| format!("hs-swe-run: {e}"))?;
    eprintln!("hs-swe-run: resolved {name} -> {}", p.display());
    Ok(p.display().to_string())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let instance_path = arg(&args, "--instance").expect("--instance");
    let model = arg(&args, "--model").expect("--model glm|deepseek");
    let feedback = arg(&args, "--feedback").expect("--feedback on|off") == "on";
    let budget_micros: u64 = arg(&args, "--budget-micros")
        .expect("--budget-micros")
        .parse()
        .map_err(|_| "--budget-micros must be an integer")?;
    let max_steps: u32 = arg(&args, "--max-steps")
        .unwrap_or("25".into())
        .parse()
        .map_err(|_| "--max-steps must be an integer")?;
    let wall_secs: Option<u64> = arg(&args, "--wall-secs")
        .or_else(|| std::env::var("HS_SUBSET_WALL_SECS").ok())
        .and_then(|v| v.parse().ok());
    let run_dir = PathBuf::from(arg(&args, "--run-dir").expect("--run-dir"));
    let blind = arg(&args, "--mode").as_deref() == Some("blind");
    // blind mode: ground truth enters ONLY here, as process memory - never
    // env (the repo.exec sandbox inherits the driver env), never the prompt,
    // never the mission streams. It grades after the mission, outside it.
    let grade_cmds: Vec<String> = args
        .windows(2)
        .filter(|w| w[0] == "--grade-cmd")
        .map(|w| w[1].clone())
        .collect();
    std::fs::create_dir_all(&run_dir)?;

    let raw = std::fs::read_to_string(&instance_path)?;
    let v: serde_json::Value = serde_json::from_str(&raw)?;
    let inst: Instance = serde_json::from_value(v.clone())?;
    let f2p = if inst.fail_to_pass.is_empty() {
        inst.fail_to_pass_caps
    } else {
        inst.fail_to_pass
    };

    // Goal evaluator f2p = the REAL acceptance command (HS_SWE_F2P, written
    // by the subset runner as "bash <run_dir>/f2p.sh"), never the
    // instance's fail_to_pass test IDs (forensic item 1, 2026-09-06: test IDs
    // ran as shell -> exit 127 -> env_limited in 100% of sessions).
    let goal_cmds: Vec<String> = if blind {
        Vec::new()
    } else {
        std::env::var("HS_SWE_F2P")
            .ok()
            .map(|s| {
                s.split(',')
                    .map(|x| x.trim().to_string())
                    .filter(|x| !x.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    };

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
    assert!(
        ws.join(".git").exists(),
        "workspace not prepped: {}",
        ws.display()
    );
    let log_root = run_dir.join("log");
    let answer_path = log_root
        .join("work")
        .join(&inst.instance_id)
        .join("answer.txt");

    // repo layout for grounding (paths the model may touch)
    let listing = std::process::Command::new("git")
        .args(["ls-files"])
        .current_dir(&ws)
        .output()?;
    let files: Vec<&str> = std::str::from_utf8(&listing.stdout)?.lines().collect();
    let mut layout = String::new();
    for f in files.iter().take(60) {
        layout.push_str(f);
        layout.push('\n');
    }

    let policy = match std::env::var("HS_POLICY_TOML") {
        Ok(p) => Some(
            hs_loop::sweprompt::load_policy_overlay(std::path::Path::new(&p)).unwrap_or_else(|e| {
                eprintln!("hs-swe-run: {e}");
                std::process::exit(2);
            }),
        ),
        Err(_) => None,
    };
    // MCP tool surface (the MCP adapter gate design): discover each server's
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
            let out = std::process::Command::new(bin("hs-plugin-mcpcall")?)
                .args([
                    "--config",
                    &servers_toml,
                    "--server",
                    &s.name,
                    "--list",
                    "--list-verbose",
                ])
                .output()
                .unwrap_or_else(|e| {
                    eprintln!("hs-swe-run: mcp discovery spawn: {e}");
                    std::process::exit(2);
                });
            if !out.status.success() {
                eprintln!(
                    "hs-swe-run: mcp discovery: {}",
                    String::from_utf8_lossy(&out.stderr)
                );
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
                    bin("hs-plugin-mcpcall")?,
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

    let prompt_args = hs_loop::sweprompt::PromptArgs {
        ws: ws.display().to_string(),
        problem_statement: inst.problem_statement.clone(),
        // Prompt carries the SANDBOX-resolved FAIL_TO_PASS command
        // (octodns-1298, 2026-09-07): the host f2p.sh path is hidden
        // from the repo.exec sandbox and the model burned steps hunting
        // the filesystem for it. Runner override: HS_SWE_F2P_DISPLAY.
        // Default: pytest node ids run with python3 (the mission venv
        // is first on the sandbox PATH).
        fail_to_pass: std::env::var("HS_SWE_F2P_DISPLAY").map_or_else(|_| {
                if f2p.is_empty() {
                    goal_cmds.clone()
                } else {
                    vec![format!("python3 -m pytest {} -x -q", f2p.join(" "))]
                }
            }, |d| vec![d]),
        repo_layout: layout.clone(),
        nudge: std::env::var("HS_SWE_PROMPT_NUDGE").unwrap_or_default(),
        answer_path: answer_path.display().to_string(),
        orientation: hs_loop::sweprompt::probe_orientation(),
        mcp_tools: String::new(),
    };
    let prompt = if blind {
        hs_loop::sweprompt::build_blind_mission_prompt(policy.as_ref(), &prompt_args)
    } else {
        hs_loop::sweprompt::build_mission_prompt(policy.as_ref(), &prompt_args)
    };
    std::fs::write(run_dir.join("mission_prompt.txt"), &prompt)?;
    // audit artifact: the exact native tool surface the model operates under
    std::fs::write(
        run_dir.join("tools.json"),
        serde_json::to_string_pretty(&native_tools).expect("json! values serialize"),
    )?;

    // Provider plugin resolution: a dedicated hs-plugin-<model> wins;
    // otherwise the generic OpenAI-compatible provider plugin resolves
    // the provider by name ([[providers]] TOML / builtin), the provider
    // name riding as argv[1].
    let model_cmd = match bin(&format!("hs-plugin-{model}")) {
        Ok(p) => format!("\"{p}\""),
        Err(_) => format!("\"{}\", \"{model}\"", bin("hs-plugin-provmodel")?),
    };
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
command = [{model_cmd}]
default = true
{mcp_tools}
"#,
            answersubmit = bin("hs-plugin-answersubmit")?,
            checker = bin(if blind {
                "hs-plugin-selfcheck"
            } else {
                "hs-plugin-swecheck"
            })?,
            fileread = bin("hs-plugin-fileread")?,
            reposearch = bin("hs-plugin-reposearch")?,
            repoexec = bin("hs-plugin-repoexec")?,
            policy = bin("hs-plugin-policy")?,
            editpatch = bin(if edit_path == "anchor" {
                "hs-plugin-editanchor"
            } else {
                "hs-plugin-applypatch"
            })?,
            edittoolname = if edit_path == "anchor" {
                "edit.anchor"
            } else {
                "edit.patch"
            },
            notescratch = bin("hs-plugin-notescratch")?,
            model = model,
            mcp_tools = mcp_tools,
        ),
    )?;

    let work_dir = log_root.join("work").join(&inst.instance_id);
    std::fs::create_dir_all(&work_dir).expect("work dir");
    // notes.scratch storage: per-mission, inherited by tool processes
    unsafe { std::env::set_var("HS_SCRATCH_FILE", work_dir.join("notes.md")) };

    let kernel = hs_loop::swe_kernel(&config, &log_root).expect("kernel load");
    hs_loop::require_visibility(&kernel).unwrap_or_else(|m| {
        eprintln!("STARTUP REFUSED: {m}");
        std::process::exit(2);
    });
    let mut l = hs_loop::InnerLoop::new(kernel, &log_root, feedback, max_steps).expect("loop");
    l.set_budget_micros(budget_micros);
    // B1 (v5 cut #10): when the bench attaches the K plane, the model
    // consults it through the memory.recall tool - no blind pre-pass.
    let memory_db_early = arg(&args, "--memory-db").map(std::path::PathBuf::from);
    if memory_db_early.is_some() {
        native_tools.push(hs_loop::toolschema::memory_recall_tool());
    }
    l.set_tools(serde_json::Value::Array(native_tools));
    if let Some(w) = wall_secs {
        l.set_wall_secs(w);
    }
    l.set_progress_path(&run_dir.join("progress.json"));
    let budget_tokens = arg(&args, "--context-budget-tokens")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or_else(|| hs_loop::default_budget_for_model(&model));
    l.set_context_budget_tokens(budget_tokens);
    if let Ok(ws) = std::env::var("HS_SWE_WORKSPACE")
        && !goal_cmds.is_empty() {
            l.set_goal_evaluator(std::path::Path::new(&ws), goal_cmds.clone());
        }
    let memory_db = memory_db_early;
    if let Some(db) = &memory_db {
        l.set_memory_db(db);
    }
    let started = std::time::Instant::now();
    let r = l
        .run_mission_full(&inst.instance_id, &prompt)
        .expect("mission run");
    let wall_secs = started.elapsed().as_secs();
    let cost = l.total_cost_micros();

    if let Some(db) = &memory_db
        && let Ok(store) = hs_memory::sqlite::SqliteMemoryStore::open(db).map_err(|e| e.to_string())
        {
            for rec in hs_memory::extract::extract_stream(
                &log_root,
                r.stream_id,
                &inst.instance_id,
                "operator",
            ) {
                if let Err(e) = store.put(rec) {
                    eprintln!("memory extract: {e}");
                }
            }
        }

    // final patch = last answer content (the loop re-runs checker each step;
    // the workspace holds the passing state on success)
    let answer = std::fs::read_to_string(&r.answer_path).unwrap_or_default();
    if let Some(patch) = hs_bench::extract_patch(&answer) {
        std::fs::write(run_dir.join("model_patch.diff"), patch)?;
    }
    // Blind-mode external grading (Eric 2026-09-07): ground truth touches
    // the run ONLY here - after the mission is over, writing a separate
    // grade.json that never enters any mission stream. This is the
    // SWE-bench-official shape: blind submission, external scoring.
    if !grade_cmds.is_empty() {
        let grade = grade_submission(&ws, &run_dir, &grade_cmds);
        std::fs::write(
            run_dir.join("grade.json"),
            serde_json::to_string_pretty(&grade).expect("json! values serialize"),
        )?;
        eprintln!(
            "grade: passed={} (external, post-mission)",
            grade["passed"].as_bool().unwrap_or(false)
        );
    }
    let result = serde_json::json!({
        "instance_id": inst.instance_id,
        "model": model,
        "mode": if blind { "blind" } else { "f2p" },
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
    println!(
        "{}",
        serde_json::to_string(&result).expect("json! values serialize")
    );
    let _ = Path::new("/"); // keep Path import
    Ok(())
}

/// Apply the submitted patch to a pristine clone of the base workspace and
/// run the ground-truth commands there. Process-memory only: `grade_cmds`
/// arrive as CLI args, never env (the agent's exec sandbox inherits env).
fn grade_submission(
    ws: &std::path::Path,
    run_dir: &std::path::Path,
    cmds: &[String],
) -> serde_json::Value {
    let gws = run_dir.join("grade-ws");
    let _ = std::fs::remove_dir_all(&gws);
    let clone = std::process::Command::new("git")
        .args([
            "clone",
            "-q",
            &ws.display().to_string(),
            &gws.display().to_string(),
        ])
        .output();
    match clone {
        Ok(o) if !o.status.success() => {
            return serde_json::json!({"graded": true, "passed": false, "error": format!("clone: {}", String::from_utf8_lossy(&o.stderr))});
        }
        Err(e) => {
            return serde_json::json!({"graded": true, "passed": false, "error": format!("clone spawn: {e}")});
        }
        _ => {}
    }
    let patch = run_dir.join("model_patch.diff");
    if patch.exists() {
        let apply = std::process::Command::new("git")
            .args(["apply", &patch.display().to_string()])
            .current_dir(&gws)
            .output();
        match apply {
            Ok(o) if !o.status.success() => {
                return serde_json::json!({"graded": true, "passed": false, "error": format!("patch apply: {}", String::from_utf8_lossy(&o.stderr))});
            }
            Err(e) => {
                return serde_json::json!({"graded": true, "passed": false, "error": format!("apply spawn: {e}")});
            }
            _ => {}
        }
    }
    let mut results = vec![];
    let mut passed = true;
    for c in cmds {
        match std::process::Command::new("sh")
            .args(["-c", c])
            .current_dir(&gws)
            .output()
        {
            Ok(o) => {
                let ok = o.status.success();
                if !ok {
                    passed = false;
                }
                let mut tail = String::from_utf8_lossy(&o.stdout).into_owned();
                tail.push_str(&String::from_utf8_lossy(&o.stderr));
                if tail.len() > 2000 {
                    tail = hs_loop::msgfmt::tail_bytes_safe(&tail, 2000);
                }
                results
                    .push(serde_json::json!({"cmd": c, "passed": ok, "output_tail": tail.trim()}));
            }
            Err(e) => {
                passed = false;
                results.push(serde_json::json!({"cmd": c, "passed": false, "output_tail": format!("spawn: {e}")}));
            }
        }
    }
    serde_json::json!({
        "graded": true,
        "passed": passed,
        "results": results,
        "note": "external post-mission grading - these commands never entered the mission",
    })
}
