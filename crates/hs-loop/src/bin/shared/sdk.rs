// Minimal shared plugin loop (same wire protocol as gate-2 plugins).
use std::io::{BufRead, BufReader, Write};

// Each bin includes this file and uses serve or serve_ext (or both):
// whichever a given bin does not call is dead code there, not an error.
#[allow(dead_code)]
fn serve(
    name: &'static str,
    kind: &'static str,
    handler: &mut dyn FnMut(&str, serde_json::Value) -> serde_json::Value,
) {
    serve_ext(name, kind, &mut |m, p, _emit| handler(m, p));
}



/// serve + an emitter for interstitial frames (gap #3 streaming): the
/// handler may call emit({"delta": "..."}) any number of times BEFORE its
/// return value becomes the final response. Frames ride the same request
/// id; kernels without a delta sink skip them harmlessly. Stdout is locked
/// per write so emission mid-handler is safe.
// The handler type is inherently three-part (method, params, emitter);
// an alias would unify the emitter's lifetime with the handler's, which
// the borrow checker rejects - the inline HRTB form is the honest one.
#[allow(dead_code, clippy::type_complexity)]
fn serve_ext(
    name: &'static str,
    kind: &'static str,
    handler: &mut dyn FnMut(&str, serde_json::Value, &dyn Fn(serde_json::Value)) -> serde_json::Value,
) {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    for line in BufReader::new(stdin.lock()).lines() {
        let Ok(line) = line else { break };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
        let id = v["id"].clone();
        let method = v["method"].as_str().unwrap_or("");
        let write_frame = |frame: serde_json::Value| {
            let mut out = stdout.lock();
            writeln!(out, "{frame}").unwrap();
            out.flush().unwrap();
        };
        if method == "describe" {
            write_frame(serde_json::json!({"id": id, "result": {"name": name, "kind": kind, "version": "0.1.0"}}));
            continue;
        }
        let emit = |mut frame: serde_json::Value| {
            frame["id"] = id.clone();
            write_frame(frame);
        };
        let r = handler(method, v["params"].clone(), &emit);
        let resp = if let Some(e) = r.get("$error") {
            serde_json::json!({"id": id, "error": e})
        } else {
            serde_json::json!({"id": id, "result": r})
        };
        write_frame(resp);
    }
}
