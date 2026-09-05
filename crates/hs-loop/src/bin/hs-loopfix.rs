//! Loop-test fixture: dies-always tool for supervisor abort tests.
//! Optional arg2 = state file: every spawn appends one line (attempt accounting).
use std::io::{BufRead, BufReader, Write};

fn main() {
    let name = std::env::args().nth(1).unwrap_or_else(|| "zombie".to_string());
    if let Some(state) = std::env::args().nth(2) {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(state)
            .unwrap();
        writeln!(f, "spawn").unwrap();
    }
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in BufReader::new(stdin.lock()).lines() {
        let line = line.unwrap();
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        let id = v["id"].clone();
        let resp = match v["method"].as_str().unwrap() {
            "describe" => {
                serde_json::json!({"id": id, "result": {"name": name, "kind": "tool", "version": "0.1.0"}})
            }
            "tool.call" => std::process::exit(1),
            _ => serde_json::json!({"id": id, "error": "unknown method"}),
        };
        writeln!(out, "{}", resp).unwrap();
        out.flush().unwrap();
    }
}
