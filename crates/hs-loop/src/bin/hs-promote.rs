//! hs-promote: the out-of-mission promotion driver (spec gate 8 / D7,
//! Eric 2026-09-13). Takes a recorded policy proposal, benches it against
//! the parent through REAL missions on a held-out task set, promotes the
//! winner into the [prompts] overlay the live surfaces load (or journals
//! the rejection with its reason), and rewinds on regression.
//!
//!   hs-promote evaluate --proposals <policy_proposals.jsonl> [--version N]
//!       [--name swe-mission] --bench <file> --held-out <file>
//!       --runner fixture|swe
//!       [--overlay <path>] [--journal <path>]
//!       fixture runner: no extra args (gatemodel missions, zero cost -
//!       candidates pass only when they carry the gatemodel marker; this
//!       mode proves the driver machinery, not model quality)
//!       swe runner (PAID, real model):
//!       --instances-dir <dir> --runs-root <dir> --swe-driver <run_subset_par.py>
//!       --manifest <bench manifest.json>  (repo/base_commit/test_patch per instance)
//!       [--model deepseek] [--max-steps N] [--wall-secs N]
//!
//!   hs-promote rewind [--name swe-mission] [--overlay <path>] [--journal <path>]

use hs_loop::evolve::{self, BenchOutcome};
use hs_loop::promote;
use hs_loop::sweprompt::{self, PolicyOverlay};
use std::path::{Path, PathBuf};

const MARKER: &str = "EVO-PREFLIGHT-LAW";

fn sibling(name: &str) -> String {
    let exe = std::env::current_exe().expect("current exe");
    hs_loop::mcpbridge::resolve_plugin_bin(&exe, name)
        .unwrap_or_else(|e| fail(&format!("{name} sibling: {e}")))
        .display()
        .to_string()
}

fn arg(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn fail(msg: &str) -> ! {
    eprintln!("hs-promote: {msg}");
    std::process::exit(2);
}

/// Fixture runner: real InnerLoop missions on the gatemodel rig. The
/// marker makes prompt content load-bearing, so the driver's bench ->
/// held-out -> promote/rewind mechanics are exercised end to end at zero
/// cost. Not a model-quality signal.
fn fixture_runner(base: PathBuf) -> impl Fn(Option<&str>, &str) -> BenchOutcome {
    move |template, task| {
        let d = base.join(format!("{task}-{}", uuid::Uuid::new_v4()));
        let log = d.join("log");
        std::fs::create_dir_all(&log).expect("fixture dir");
        let config = d.join("hairspring.toml");
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

[[models]]
name = "gatemodel"
command = ["{gatemodel}"]
default = true
"#,
                answer = sibling("hs-plugin-answer"),
                checker = sibling("hs-plugin-checker"),
                gatemodel = sibling("hs-plugin-gatemodel"),
            ),
        )
        .expect("fixture config");
        let kernel = hs_kernel::Kernel::load(&config).expect("fixture kernel");
        let mut l = hs_loop::InnerLoop::new(kernel, &log, true, 3).expect("fixture loop");
        let stream = l.stream_id();
        let answer = log.join("work").join(task).join("answer.txt");
        let prompt = if let Some(t) = template {
            t.replace("{answer_path}", &answer.display().to_string())
        } else {
            let args = sweprompt::PromptArgs {
                ws: ".".into(),
                problem_statement: "p".into(),
                fail_to_pass: vec![],
                repo_layout: String::new(),
                nudge: String::new(),
                answer_path: answer.display().to_string(),
                orientation: String::new(),
                mcp_tools: String::new(),
            };
            sweprompt::build_mission_prompt(None, &args)
        };
        let r = l.run_mission_full(task, &prompt).expect("fixture mission");
        BenchOutcome {
            task: task.into(),
            passed: r.passed,
            steps: r.steps,
            cost_micros: 0,
            stream_id: stream,
        }
    }
}

struct SweCfg {
    instances_dir: PathBuf,
    runs_root: PathBuf,
    driver: PathBuf,
    model: String,
    max_steps: Option<String>,
    wall_secs: Option<String>,
    swe_run_bin: PathBuf,
    current_overlay: Option<PolicyOverlay>,
    prompt_name: String,
    /// Full SWE-bench metadata (repo, base_commit, test_patch) per instance,
    /// from the bench manifest - the per-instance json is prompt-only.
    manifest: Vec<serde_json::Value>,
    /// Eric's approved cycle spend, cumulative across EVERY paid mission of
    /// the cycle (proposal mission included via --spent-micros-so-far).
    /// Checked before each new mission starts; hitting it aborts the cycle
    /// instead of starting another mission.
    cycle_cap_micros: u64,
    spent: std::cell::Cell<u64>,
}

/// SWE runner (PAID): one real hs-swe-run mission per (template, task)
/// through the subset driver's own machinery (ws prep, venv, f2p,
/// preflight). Both arms run under an EXPLICIT overlay so a mid-run
/// canonical-overlay change can never contaminate the comparison: the
/// candidate arm gets current+candidate, the parent arm gets the current
/// overlay verbatim (or an empty overlay when the parent is builtin).
fn swe_runner(cfg: SweCfg) -> impl Fn(Option<&str>, &str) -> BenchOutcome {
    move |template, task| {
        let arm = if template.is_some() { "candidate" } else { "parent" };
        let dir = cfg.runs_root.join(format!("{arm}-{task}"));
        let result_path = dir.join("runs").join(task).join("result.json");
        if !result_path.exists() {
            // The cumulative kill switch: Eric's number, never crossed.
            let spent = cfg.spent.get();
            if spent >= cfg.cycle_cap_micros {
                fail(&format!(
                    "cycle spend cap reached: ${:.2} of ${:.2} spent - aborting before mission {task}",
                    spent as f64 / 1e6,
                    cfg.cycle_cap_micros as f64 / 1e6
                ));
            }
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("instances")).expect("arm dir");
            // Venvs are worker-private (PAR=1 here) and arms run
            // sequentially, so one shared venv cache per runs-root is safe
            // and saves a multi-minute pip build per arm.
            #[cfg(unix)]
            {
                let shared = cfg.runs_root.join("shared-venvs");
                std::fs::create_dir_all(&shared).expect("shared venvs");
                let _ = std::fs::remove_file(dir.join("venvs"));
                std::os::unix::fs::symlink(&shared, dir.join("venvs"))
                    .expect("venvs symlink");
            }
            let src = cfg.instances_dir.join(format!("{task}.json"));
            let inst = std::fs::read_to_string(&src)
                .unwrap_or_else(|e| fail(&format!("instance {}: {e}", src.display())));
            std::fs::write(dir.join("instances").join(format!("{task}.json")), &inst)
                .expect("instance copy");
            let entry = cfg
                .manifest
                .iter()
                .find(|e| e["instance_id"].as_str() == Some(task))
                .unwrap_or_else(|| fail(&format!("{task}: not in the bench manifest")));
            std::fs::write(
                dir.join("manifest.json"),
                serde_json::to_string(&vec![entry]).expect("manifest entry"),
            )
            .expect("manifest");
            let overlay_text = match template {
                Some(t) => promote::render_overlay(cfg.current_overlay.as_ref(), &cfg.prompt_name, t),
                // Builtin parent: an explicit empty overlay so the run is
                // deterministic even if a canonical overlay appears mid-run.
                None => "[prompts]\n".to_string(),
            };
            let overlay_path = dir.join("arm-policy.toml");
            std::fs::write(&overlay_path, overlay_text).expect("arm overlay");
            let mut cmd = std::process::Command::new("python3");
            cmd.arg(&cfg.driver)
                .env("S50", &dir)
                .env("PAR", "1")
                .env("HS_SWE_RUN_BIN", &cfg.swe_run_bin)
                .env("HS_SUBSET_MODEL", &cfg.model)
                .env("HS_POLICY_TOML", &overlay_path);
            if let Some(s) = &cfg.max_steps {
                cmd.env("HS_SUBSET_MAX_STEPS", s);
            }
            if let Some(s) = &cfg.wall_secs {
                cmd.env("HS_SUBSET_WALL_SECS", s);
            }
            let out = cmd.output().expect("driver spawn");
            std::fs::write(
                dir.join("driver.out"),
                format!(
                    "rc={:?}\n{}\n--- STDERR ---\n{}",
                    out.status.code(),
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                ),
            )
            .expect("driver out");
        }
        let text = std::fs::read_to_string(&result_path)
            .unwrap_or_else(|e| fail(&format!("{}: no result.json ({e}); see {}", task, dir.join("driver.out").display())));
        let v: serde_json::Value = serde_json::from_str(&text).expect("result.json parses");
        let cost = v["cost_micros"].as_u64().unwrap_or(0);
        let now = cfg.spent.get() + cost;
        cfg.spent.set(now);
        eprintln!(
            "hs-promote: ${:.2} of ${:.2} spent (mission {task}: ${:.2})",
            now as f64 / 1e6,
            cfg.cycle_cap_micros as f64 / 1e6,
            cost as f64 / 1e6
        );
        // The mission stream id for lineage traces, when the run dir keeps one.
        let stream_id = std::fs::read_dir(dir.join("runs").join(task).join("log").join("streams"))
            .ok()
            .and_then(|mut it| it.next())
            .and_then(|e| e.ok())
            .and_then(|e| {
                e.path()
                    .file_stem()
                    .and_then(|s| s.to_str().map(str::to_owned))
            })
            .and_then(|s| uuid::Uuid::parse_str(&s).ok())
            .unwrap_or_else(uuid::Uuid::nil);
        BenchOutcome {
            task: task.into(),
            passed: v["passed"].as_bool().unwrap_or(false),
            steps: v["steps"].as_u64().unwrap_or(0) as u32,
            cost_micros: v["cost_micros"].as_u64().unwrap_or(0),
            stream_id,
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("");
    let name = arg(&args, "--name").unwrap_or_else(|| "swe-mission".to_string());
    let overlay = arg(&args, "--overlay")
        .map(PathBuf::from)
        .unwrap_or_else(sweprompt::default_policy_overlay_path);
    let journal = arg(&args, "--journal")
        .map(PathBuf::from)
        .unwrap_or_else(promote::default_journal_path);
    match mode {
        "rewind" => match evolve::rewind_named(&journal, &overlay, &name) {
            Ok(()) => println!(
                "{}",
                serde_json::json!({"rewound": true, "name": name, "overlay": overlay.display().to_string(), "journal": journal.display().to_string()})
            ),
            Err(e) => fail(&e),
        },
        "evaluate" => {
            let proposals = arg(&args, "--proposals").unwrap_or_else(|| fail("--proposals required"));
            let cand = match arg(&args, "--version") {
                Some(v) => promote::proposal_at(
                    Path::new(&proposals),
                    v.parse().unwrap_or_else(|_| fail("--version must be a number")),
                ),
                None => promote::latest_proposal(Path::new(&proposals)),
            }
            .unwrap_or_else(|e| fail(&e));
            let bench = promote::load_task_list(Path::new(
                &arg(&args, "--bench").unwrap_or_else(|| fail("--bench required")),
            ))
            .unwrap_or_else(|e| fail(&e));
            let held = promote::load_task_list(Path::new(
                &arg(&args, "--held-out").unwrap_or_else(|| fail("--held-out required")),
            ))
            .unwrap_or_else(|e| fail(&e));
            let parent = promote::parent_template(&overlay, &name).unwrap_or_else(|e| fail(&e));
            let current_overlay = if overlay.exists() {
                Some(sweprompt::load_policy_overlay(&overlay).unwrap_or_else(|e| fail(&e)))
            } else {
                None
            };
            let runner_kind = arg(&args, "--runner").unwrap_or_else(|| fail("--runner fixture|swe required"));
            let eval = match runner_kind.as_str() {
                "fixture" => {
                    let base = std::env::temp_dir().join(format!("hs-promote-fixture-{}", uuid::Uuid::new_v4()));
                    let runner = fixture_runner(base);
                    evolve::evaluate_candidate_named(
                        &runner,
                        parent,
                        cand.text.clone(),
                        &bench,
                        &held,
                        &overlay,
                        &journal,
                        &name,
                    )
                }
                "swe" => {
                    let exe = std::env::current_exe().expect("current exe");
                    let swe_run_bin = hs_loop::mcpbridge::resolve_plugin_bin(&exe, "hs-swe-run")
                        .unwrap_or_else(|e| fail(&format!("hs-swe-run sibling: {e}")));
                    let cfg = SweCfg {
                        instances_dir: PathBuf::from(
                            arg(&args, "--instances-dir").unwrap_or_else(|| fail("--instances-dir required (swe runner)")),
                        ),
                        runs_root: PathBuf::from(
                            arg(&args, "--runs-root").unwrap_or_else(|| fail("--runs-root required (swe runner)")),
                        ),
                        driver: PathBuf::from(
                            arg(&args, "--swe-driver").unwrap_or_else(|| fail("--swe-driver required (swe runner)")),
                        ),
                        model: arg(&args, "--model").unwrap_or_else(|| "deepseek".to_string()),
                        max_steps: arg(&args, "--max-steps"),
                        wall_secs: arg(&args, "--wall-secs"),
                        swe_run_bin,
                        current_overlay,
                        prompt_name: name.clone(),
                        manifest: {
                            let mp = arg(&args, "--manifest")
                                .unwrap_or_else(|| fail("--manifest <bench manifest.json> required for --runner swe"));
                            let text = std::fs::read_to_string(&mp)
                                .unwrap_or_else(|e| fail(&format!("manifest {mp}: {e}")));
                            serde_json::from_str(&text)
                                .unwrap_or_else(|e| fail(&format!("manifest {mp} parses: {e}")))
                        },
                        cycle_cap_micros: arg(&args, "--cycle-cap-micros")
                            .map(|v| v.parse().unwrap_or_else(|_| fail("--cycle-cap-micros must be a number")))
                            .unwrap_or(30_000_000),
                        spent: std::cell::Cell::new(
                            arg(&args, "--spent-micros-so-far")
                                .map(|v| v.parse().unwrap_or_else(|_| fail("--spent-micros-so-far must be a number")))
                                .unwrap_or(0),
                        ),
                    };
                    let runner = swe_runner(cfg);
                    evolve::evaluate_candidate_named(
                        &runner,
                        parent,
                        cand.text.clone(),
                        &bench,
                        &held,
                        &overlay,
                        &journal,
                        &name,
                    )
                }
                other => fail(&format!("unknown runner {other:?}: fixture|swe")),
            };
            let cost: u64 = eval
                .bench
                .iter()
                .chain(&eval.held_out_parent)
                .chain(&eval.held_out_candidate)
                .map(|o| o.cost_micros)
                .sum();
            println!(
                "{}",
                serde_json::json!({
                    "decision": format!("{:?}", eval.decision),
                    "parent_hash": eval.parent_hash,
                    "candidate_hash": eval.candidate_hash,
                    "candidate_version": cand.version,
                    "name": name,
                    "overlay": overlay.display().to_string(),
                    "journal": journal.display().to_string(),
                    "cost_usd": cost as f64 / 1e6,
                    "note": format!("fixture marker law: candidates pass only with {MARKER}"),
                })
            );
        }
        _ => {
            eprintln!("usage: hs-promote evaluate|rewind (see file header)");
            std::process::exit(2);
        }
    }
}
