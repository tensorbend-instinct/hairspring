//! SWE-bench checker plugin "swecheck": ground truth for repo+patch
//! missions. The model's answer file holds a unified diff (possibly fenced -
//! `hs_bench::extract_patch` normalizes); the checker applies it in the
//! mission workspace and runs `FAIL_TO_PASS/PASS_TO_PASS` commands. Failure
//! feedback names the failing tests and the apply/test output tail - the
//! loop's feedback channel turns that into repairs.
//!
//! Per-mission config via env (the runner spawns one mission at a time):
//!   `HS_SWE_WORKSPACE`  - checked-out repo dir
//!   `HS_SWE_F2P`        - comma-separated `FAIL_TO_PASS` test commands
//!   `HS_SWE_P2P`        - comma-separated `PASS_TO_PASS` test commands (may be empty)
//!   `HS_SWE_SETUP`      - optional command run before tests (e.g. install)
include!("shared/sdk.rs");

fn run_cmd(dir: &std::path::Path, cmd: &str) -> (bool, String) {
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(dir)
        .output();
    match out {
        Ok(o) => (
            o.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            ),
        ),
        Err(e) => (false, format!("spawn failed: {e}")),
    }
}

fn reset_to_base(ws: &std::path::Path) {
    // every judgement starts from the committed base state: a prior green
    // verdict left the patch applied (3862 wedge: re-judging the same answer
    // flipped green->red), and a failed one can leave untracked residue that
    // poisons the next apply (8609: "already exists in working directory")
    // stdout/stderr nulled: the plugin stdout is the JSON-RPC wire - a
    // chatty git ("Removing ..." from clean) corrupts the stream
    let _ = std::process::Command::new("git")
        .args(["checkout", "--", "."])
        .current_dir(ws)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    let _ = std::process::Command::new("git")
        .args(["clean", "-fd"])
        .current_dir(ws)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

fn main() {
    serve("checker.run", "tool", &mut |method, params| match method {
        "tool.call" => {
            let a = &params["args"];
            let path = a["path"].as_str().unwrap_or("");
            let ws = match std::env::var("HS_SWE_WORKSPACE") {
                Ok(w) => std::path::PathBuf::from(w),
                Err(_) => return serde_json::json!({"$error": "HS_SWE_WORKSPACE not set"}),
            };
            let raw = std::fs::read_to_string(path).unwrap_or_default();
            let Some(patch) = hs_bench::extract_patch(&raw) else {
                return serde_json::json!({"passed": false,
                    "error": "no unified diff in your answer file; respond with a JSON tool call whose content is one ```diff fenced unified diff (paths a/... b/...)".to_string()});
            };
            reset_to_base(&ws);
            match hs_bench::apply_model_patch(&ws, &patch) {
                Err(e) => serde_json::json!({"$error": format!("apply machinery: {e:?}")}),
                Ok(hs_bench::ApplyResult::NoApply(msg)) => {
                    // reset any partial application, then feed the error back

                    reset_to_base(&ws);
                    serde_json::json!({"passed": false,
                        "error": format!("patch did not apply: {}", hs_loop::msgfmt::prefix_bytes_safe(&msg, 600))})
                }
                Ok(hs_bench::ApplyResult::Applied) => {
                    let mut failures: Vec<String> = vec![];
                    if let Ok(setup) = std::env::var("HS_SWE_SETUP")
                        && !setup.trim().is_empty() {
                            let (ok, log) = run_cmd(&ws, &setup);
                            if !ok {
                                failures
                                    .push(format!("setup failed: {}", hs_loop::msgfmt::prefix_bytes_safe(&log, 400)));
                            }
                        }
                    let f2p = std::env::var("HS_SWE_F2P").unwrap_or_default();
                    let p2p = std::env::var("HS_SWE_P2P").unwrap_or_default();
                    for cmd in f2p
                        .split(',')
                        .chain(p2p.split(','))
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        let (ok, log) = run_cmd(&ws, cmd);
                        if !ok {
                            let tail: String = log
                                .lines()
                                .rev()
                                .take(12)
                                .collect::<Vec<_>>()
                                .into_iter()
                                .rev()
                                .collect::<Vec<_>>()
                                .join("\n");
                            failures.push(format!("{cmd} FAILED:\n{tail}"));
                        }
                    }
                    // a patch that fails tests must not linger in the tree:
                    // the next attempt starts from the base commit state
                    if failures.is_empty() {
                        serde_json::json!({"passed": true})
                    } else {
                        reset_to_base(&ws);
                        serde_json::json!({"passed": false,
                            "error": failures.join("\n---\n")})
                    }
                }
            }
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    });
}
