//! Test fixture plugin for hs-kernel. Modes:
//!   echo-tool     tool `echo`: returns `args["text"]` unchanged
//!   fake-model    model "fake-v1": deterministic completion + token counts
//!   rail-a|b|c    rail: appends "<name>:<hook>" to $`RAIL_LOG_FILE`, returns {}
//!   rail-crash    rail: exits(1) on any rail.hook
//!   flaky-tool    tool "flaky": exits(42) on first tool.call, works after
//!   dies-always   tool: appends to state file (arg3) at startup, exits(1) on every tool.call
//!   dies-stderr   tool: eprints dying words at startup, exits(1) on every tool.call
//!   dies-unless-flag  tool: exits(1) on tool.call unless flag file (arg3) exists; then replies "revived"
//!   hang-tool     tool "sleeper": sleeps 60s on tool.call (lease tests)
//!   usage-error   tool "usageerr": well-formed {"error":...} on every tool.call (arg3: state file); process stays healthy - supervisor must NOT strike
//!   bogus         describe lies (claims different name than configured)
//!   shout-tool    describes as arg2 (default "echo") but uppercases the text - a
//!                 stand-in for "same name, DIFFERENT command" in reload tests
//!   costed-tool   tool: returns output + `cost_usd_micros` 42 (canonical-cost tests)
//!   heartbeat-tool tool: describes as arg2 (default "echo"); appends a beat to
//!                 arg3 every 100ms for its whole life - orphan-leak detection
//! Protocol: newline-delimited JSON, see `hs-kernel::protocol`.

use std::io::{BufRead, BufReader, Write};

fn main() {
    let mode = std::env::args().nth(1).expect("mode arg");
    let name_override = std::env::args().nth(2);
    if mode == "stderr-spew" {
        eprintln!("fixture-stderr-marker: spew plugin starting");
    }
    if mode == "dies-stderr" {
        eprintln!("dying-words-marker: HS_DYING_API_KEY not set (fixture standing in for a real plugin's missing-key death)");
    }
    if mode == "usage-error"
        && let Some(state) = std::env::args().nth(2) {
            use std::io::Write as _;
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(state)
                .unwrap();
            writeln!(f, "spawn").unwrap();
        }
    if mode == "dies-always"
        && let Some(state) = std::env::args().nth(3) {
            use std::io::Write as _;
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(state)
                .unwrap();
            writeln!(f, "spawn").unwrap();
        }
    if mode == "heartbeat-tool"
        && let Some(state) = std::env::args().nth(3) {
            std::thread::spawn(move || loop {
                use std::io::Write as _;
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&state)
                {
                    let _ = writeln!(f, "beat");
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            });
        }
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in BufReader::new(stdin.lock()).lines() {
        let line = line.unwrap();
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        let id = v["id"].clone();
        let method = v["method"].as_str().unwrap();
        let resp = match (mode.as_str(), method) {
            (_, "describe") => match mode.as_str() {
                "echo-tool" | "shout-tool" | "costed-tool" | "heartbeat-tool" | "params-echo-tool" => {
                    serde_json::json!({"id": id, "result": {"name": name_override.clone().unwrap_or("echo".into()), "kind": "tool", "version": "0.1.0"}})
                }
                "flaky-tool" => {
                    serde_json::json!({"id": id, "result": {"name": "flaky", "kind": "tool", "version": "0.1.0"}})
                }
                "dies-always" => {
                    serde_json::json!({"id": id, "result": {"name": "zombie", "kind": "tool", "version": "0.1.0"}})
                }
                "dies-stderr" => {
                    serde_json::json!({"id": id, "result": {"name": "dying", "kind": "tool", "version": "0.1.0"}})
                }
                "dies-unless-flag" => {
                    serde_json::json!({"id": id, "result": {"name": "revivable", "kind": "tool", "version": "0.1.0"}})
                }
                "hang-tool" => {
                    serde_json::json!({"id": id, "result": {"name": "sleeper", "kind": "tool", "version": "0.1.0"}})
                }
                "stderr-spew" => {
                    serde_json::json!({"id": id, "result": {"name": "spew", "kind": "tool", "version": "0.1.0"}})
                }
                "usage-error" => {
                    serde_json::json!({"id": id, "result": {"name": "usageerr", "kind": "tool", "version": "0.1.0"}})
                }
                "fake-model" => {
                    serde_json::json!({"id": id, "result": {"name": name_override.clone().unwrap_or("fake-v1".into()), "kind": "model", "version": "0.1.0"}})
                }
                "bogus" => {
                    serde_json::json!({"id": id, "result": {"name": "not-what-you-configured", "kind": "tool", "version": "0.1.0"}})
                }
                m if m.starts_with("rail-") => {
                    serde_json::json!({"id": id, "result": {"name": m, "kind": "rail", "version": "0.1.0"}})
                }
                _ => serde_json::json!({"id": id, "error": "bad mode"}),
            },
            ("echo-tool", "tool.call") => {
                let text = v["params"]["args"]["text"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                serde_json::json!({"id": id, "result": {"output": text}})
            }
            ("params-echo-tool", "tool.call") => {
                serde_json::json!({"id": id, "result": {"params": v["params"].clone()}})
            }
            ("shout-tool", "tool.call") => {
                let text = v["params"]["args"]["text"]
                    .as_str()
                    .unwrap_or("")
                    .to_uppercase();
                serde_json::json!({"id": id, "result": {"output": text}})
            }
            ("costed-tool", "tool.call") => {
                let text = v["params"]["args"]["text"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                serde_json::json!({"id": id, "result": {"output": text, "cost_usd_micros": 42}})
            }
            ("heartbeat-tool", "tool.call") => {
                serde_json::json!({"id": id, "result": {"output": "beat-ok"}})
            }
            ("dies-always" | "dies-stderr", "tool.call") => {
                std::process::exit(1);
            }
            ("dies-unless-flag", "tool.call") => {
                let flag = std::env::args().nth(3).expect("flag path arg");
                if !std::path::Path::new(&flag).exists() {
                    std::process::exit(1);
                }
                serde_json::json!({"id": id, "result": {"output": "revived"}})
            }
            ("usage-error", "tool.call") => {
                // a LIVE process reporting an application-level error
                if let Some(state) = std::env::args().nth(2) {
                    use std::io::Write as _;
                    let mut f = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(state)
                        .unwrap();
                    writeln!(f, "call").unwrap();
                }
                serde_json::json!({"id": id, "error": "usage: pass args.diff (inline unified diff) or args.path (the ANSWER_PATH)"})
            }
            ("stderr-spew", "tool.call") => {
                serde_json::json!({"id": id, "result": {"output": "spew-ok"}})
            }
            ("hang-tool", "tool.call") => {
                std::thread::sleep(std::time::Duration::from_secs(60));
                serde_json::json!({"id": id, "result": {"output": "slept"}})
            }
            ("flaky-tool", "tool.call") => {
                let flag = std::env::temp_dir().join("hs-fixture-flaky-once");
                if !flag.exists() {
                    std::fs::write(&flag, b"x").unwrap();
                    std::process::exit(42);
                }
                serde_json::json!({"id": id, "result": {"output": "flaky-ok"}})
            }
            ("fake-model", "model.call") => {
                let prompt = v["params"]["prompt"].as_str().unwrap_or("");
                let completion = format!(
                    "fake-completion:{}",
                    prompt.chars().rev().collect::<String>()
                );
                serde_json::json!({"id": id, "result": {
                    "completion": completion,
                    "input_tokens": prompt.len() / 4 + 1,
                    "output_tokens": 7,
                    "cost_usd_micros": 1300
                }})
            }
            (m, "rail.hook") if m.starts_with("rail-") => {
                if m == "rail-crash" {
                    std::process::exit(1);
                }
                let hook = v["params"]["hook"].as_str().unwrap_or("?");
                let f = std::env::var("RAIL_LOG_FILE").unwrap();
                use std::fs::OpenOptions;
                let mut lf = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(f)
                    .unwrap();
                writeln!(lf, "{m}:{hook}").unwrap();
                serde_json::json!({"id": id, "result": {}})
            }
            _ => serde_json::json!({"id": id, "error": "unknown method/mode"}),
        };
        writeln!(out, "{}", serde_json::to_string(&resp).unwrap()).unwrap();
        out.flush().unwrap();
    }
}
