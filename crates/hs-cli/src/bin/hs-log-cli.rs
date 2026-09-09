//! hs-log-cli: inspect and verify HAIRSPRING event logs.
//!   hs-log-cli verify --dir D    verify every stream under D (chain + blobs)
//!   hs-log-cli dump  --dir D     print one line per event
//!   hs-log-cli trace --dir D [--follow]  print `ModelCall` reasoning traces (live with --follow)

use hs_core::Payload;
use hs_log::{verify_stream, StreamReader, read_blob};
use std::path::PathBuf;
use uuid::Uuid;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map_or("help", std::string::String::as_str);
    let dir = PathBuf::from(
        args.iter()
            .position(|a| a == "--dir")
            .and_then(|i| args.get(i + 1))
            .expect("--dir required"),
    );
    let streams: Vec<Uuid> = std::fs::read_dir(dir.join("streams"))
        .map(|rd| {
            rd.filter_map(std::result::Result::ok)
                .filter_map(|e| Uuid::parse_str(&e.file_name().to_string_lossy()).ok())
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
            std::process::exit(i32::from(bad));
        }
        "dump" => {
            let with_payloads = args.iter().any(|a| a == "--payloads");
            for s in &streams {
                let r = StreamReader::open(&dir, *s)?;
                for e in r.events()? {
                    // time-audit fields: ts_wall_ms + payload size make
                    // inter-event gaps (harness overhead) measurable
                    let payload_len = match &e.payload {
                        Payload::Inline(b) => b.len(),
                        Payload::BlobRef { len, .. } => *len as usize,
                        _ => 0,
                    };
                    println!(
                        "{} seq={} {:?} ts={} lat={}ms plen={} prev={}.. hash={}..",
                        e.event_id,
                        e.seq,
                        e.kind,
                        e.ts_wall_ms,
                        e.latency_ms,
                        payload_len,
                        &hex(&e.prev_hash)[..8],
                        &hex(&e.hash)[..8]
                    );
                    if with_payloads {
                        let bytes: Option<Vec<u8>> = match &e.payload {
                            Payload::Inline(b) => Some(b.clone()),
                            Payload::BlobRef { hash, .. } => read_blob(&dir, hash).ok(),
                            _ => None,
                        };
                        if let Some(b) = bytes {
                            let mut t = String::from_utf8_lossy(&b).into_owned();
                            if t.len() > 3000 {
                                truncate_chars(&mut t, 3000);
                                t.push_str("\n...[truncated]");
                            }
                            println!("  payload: {t}");
                        }
                    }
                }
            }
        }
        "trace" => {
            // Live-readable reasoning traces (Eric 2026-09-06): every
            // ModelCall payload already carries reasoning_content; this
            // prints it in seq order, and --follow keeps emitting as new
            // events land so a run's thinking can be watched live.
            let follow = args.iter().any(|a| a == "--follow");
            let mut known: Vec<Uuid> = streams.clone();
            let mut seen: std::collections::HashMap<Uuid, u64> = std::collections::HashMap::new();
            loop {
                if follow
                    && let Ok(rd) = std::fs::read_dir(dir.join("streams")) {
                        for e in rd.filter_map(std::result::Result::ok) {
                            if let Ok(u) = Uuid::parse_str(&e.file_name().to_string_lossy())
                                && !known.contains(&u) {
                                    known.push(u);
                                }
                        }
                    }
                for s in &known {
                    let from: Option<u64> = seen.get(s).copied();
                    let r = match StreamReader::open(&dir, *s) {
                        Ok(r) => r,
                        Err(_) => continue,
                    };
                    let events = match r.events() {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    for e in events {
                        if let Some(f) = from
                            && e.seq <= f {
                                continue;
                            }
                        seen.insert(*s, e.seq);
                        if e.kind != hs_core::EventKind::ModelCall {
                            continue;
                        }
                        let bytes: Option<Vec<u8>> = match &e.payload {
                            Payload::Inline(b) => Some(b.clone()),
                            Payload::BlobRef { hash, .. } => read_blob(&dir, hash).ok(),
                            _ => None,
                        };
                        let Some(b) = bytes else { continue };
                        let v: serde_json::Value = serde_json::from_slice(&b).unwrap_or_default();
                        let model = v["model"].as_str().unwrap_or("?");
                        let role = v["role"].as_str().unwrap_or("agent");
                        let rtok = v["reasoning_tokens"].as_u64().unwrap_or(0);
                        let thinking = v["reasoning_content"].as_str().unwrap_or("");
                        let completion = v["completion"].as_str().unwrap_or("");
                        let mut act = completion.to_string();
                        if act.len() > 300 {
                            truncate_chars(&mut act, 300);
                            act.push_str("...[truncated]");
                        }
                        println!(
                            "seq={} ts={} lat={}ms role={} model={} reasoning_tokens={}",
                            e.seq, e.ts_wall_ms, e.latency_ms, role, model, rtok
                        );
                        println!("THINK: {thinking}");
                        println!("ACT: {act}");
                        use std::io::Write;
                        std::io::stdout().flush().ok();
                    }
                }
                if !follow {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        }
        _ => {
            eprintln!("usage: hs-log-cli verify|dump|trace --dir D [--payloads|--follow]");
            std::process::exit(2);
        }
    }
    Ok(())
}

fn hex(b: &[u8; 32]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Truncate to at most `max` bytes without splitting a multi-byte char
/// (`String::truncate` panics on a non-boundary).
fn truncate_chars(t: &mut String, max: usize) {
    let mut end = max.min(t.len());
    while !t.is_char_boundary(end) {
        end -= 1;
    }
    t.truncate(end);
}
