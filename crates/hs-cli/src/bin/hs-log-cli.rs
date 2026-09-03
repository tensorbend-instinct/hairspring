//! hs-log-cli: inspect and verify HAIRSPRING event logs.
//!   hs-log-cli verify --dir D    verify every stream under D (chain + blobs)
//!   hs-log-cli dump  --dir D     print one line per event

use hs_log::*;
use std::path::PathBuf;
use uuid::Uuid;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("help");
    let dir = PathBuf::from(
        args.iter()
            .position(|a| a == "--dir")
            .and_then(|i| args.get(i + 1))
            .expect("--dir required"),
    );
    let streams: Vec<Uuid> = std::fs::read_dir(dir.join("streams"))
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| Uuid::parse_str(&e.file_name().into_string().unwrap()).ok())
                .collect()
        })
        .unwrap_or_default();
    match cmd {
        "verify" => {
            let mut bad = false;
            for s in &streams {
                match verify_stream(&dir, *s) {
                    Ok(r) => println!(
                        "stream {s}: OK ({} events, tip {})",
                        r.events,
                        hex(&r.last_hash)
                    ),
                    Err(c) => {
                        println!("stream {s}: CORRUPT at seq {} - {:?}", c.seq, c.kind);
                        bad = true;
                    }
                }
            }
            std::process::exit(if bad { 1 } else { 0 });
        }
        "dump" => {
            for s in &streams {
                let r = StreamReader::open(&dir, *s).unwrap();
                for e in r.events().unwrap() {
                    println!(
                        "{} seq={} {:?} prev={}.. hash={}..",
                        e.event_id,
                        e.seq,
                        e.kind,
                        hex(&e.prev_hash)[..8].to_string(),
                        hex(&e.hash)[..8].to_string()
                    );
                }
            }
        }
        _ => {
            eprintln!("usage: hs-log-cli verify|dump --dir D");
            std::process::exit(2);
        }
    }
}

fn hex(b: &[u8; 32]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
