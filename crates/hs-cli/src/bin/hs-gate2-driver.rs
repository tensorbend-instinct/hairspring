//! hs-gate2-driver: a deliberately tiny harness used by the gate-2 proof.
//! It loads a plugin config, serves line commands on stdin, and records all
//! calls to the gate-1 log. It contains zero capability-specific code -
//! that is the point of the proof.
//!
//! Commands (one per line):
//!   tool <name> <text...>      -> "OK <name>: <output>" | "ERR <e>"
//!   model <name> <prompt...>   -> "OK <name>: <completion>" | "ERR <e>"
//!   reload                     -> "RELOADED true|false"
//! Prints READY once loaded.

use hs_kernel::Kernel;
use std::io::{BufRead, BufReader, Write};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let get = |f: &str| {
        args.iter()
            .position(|a| a == f)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let dir = std::path::PathBuf::from(get("--dir").expect("--dir"));
    let config = std::path::PathBuf::from(get("--config").expect("--config"));
    let mut kernel = Kernel::load_with_log(&config, &dir).expect("kernel load");
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    println!("READY");
    out.flush().unwrap();
    for line in BufReader::new(stdin.lock()).lines() {
        let Ok(line) = line else { break };
        let mut it = line.splitn(3, ' ');
        let resp = match (it.next(), it.next(), it.next()) {
            (Some("tool"), Some(name), Some(text)) => {
                match kernel.call_tool("operator", name, serde_json::json!({"text": text})) {
                    Ok(o) => format!("OK {name}: {}", o.output["output"].as_str().unwrap_or("")),
                    Err(e) => format!("ERR {e}"),
                }
            }
            (Some("model"), Some(name), Some(prompt)) => {
                match kernel.call_model("operator", Some(name), prompt) {
                    Ok(o) => format!("OK {name}: {}", o.completion),
                    Err(e) => format!("ERR {e}"),
                }
            }
            (Some("reload"), _, _) => match kernel.reload_if_changed() {
                Ok(changed) => format!("RELOADED {changed}"),
                Err(e) => format!("ERR {e}"),
            },
            _ => "ERR bad command".to_string(),
        };
        println!("{resp}");
        out.flush().unwrap();
    }
}
