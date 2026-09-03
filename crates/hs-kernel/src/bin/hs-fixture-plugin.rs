//! Test fixture plugin for hs-kernel. Modes:
//!   echo-tool     tool "echo": returns args["text"] unchanged
//!   fake-model    model "fake-v1": deterministic completion + token counts
//!   rail-a|b|c    rail: appends "<name>:<hook>" to $RAIL_LOG_FILE, returns {}
//!   rail-crash    rail: exits(1) on any rail.hook
//!   flaky-tool    tool "flaky": exits(42) on first tool.call, works after
//!   bogus         describe lies (claims different name than configured)
//! Protocol: newline-delimited JSON, see hs-kernel::protocol.

use std::io::{BufRead, BufReader, Write};

fn main() {
    let mode = std::env::args().nth(1).expect("mode arg");
    let name_override = std::env::args().nth(2);
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in BufReader::new(stdin.lock()).lines() {
        let line = line.unwrap();
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        let id = v["id"].clone();
        let method = v["method"].as_str().unwrap();
        let resp = match (mode.as_str(), method) {
            (_, "describe") => match mode.as_str() {
                "echo-tool" => {
                    serde_json::json!({"id": id, "result": {"name": name_override.clone().unwrap_or("echo".into()), "kind": "tool", "version": "0.1.0"}})
                }
                "flaky-tool" => {
                    serde_json::json!({"id": id, "result": {"name": "flaky", "kind": "tool", "version": "0.1.0"}})
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
