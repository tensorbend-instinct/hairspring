// Minimal shared plugin loop (same wire protocol as gate-2 plugins).
use std::io::{BufRead, BufReader, Write};

fn serve(
    name: &'static str,
    kind: &'static str,
    handler: &mut dyn FnMut(&str, serde_json::Value) -> serde_json::Value,
) {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in BufReader::new(stdin.lock()).lines() {
        let Ok(line) = line else { break };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
        let id = v["id"].clone();
        let method = v["method"].as_str().unwrap_or("");
        let resp = if method == "describe" {
            serde_json::json!({"id": id, "result": {"name": name, "kind": kind, "version": "0.1.0"}})
        } else {
            let r = handler(method, v["params"].clone());
            if let Some(e) = r.get("$error") {
                serde_json::json!({"id": id, "error": e})
            } else {
                serde_json::json!({"id": id, "result": r})
            }
        };
        writeln!(out, "{resp}").unwrap();
        out.flush().unwrap();
    }
}
