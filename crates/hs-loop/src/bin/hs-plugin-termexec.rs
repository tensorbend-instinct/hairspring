//! Terminal-bench mission tool "term.exec": direct bash exec in the live task
//! container (see `hs_loop::termexec`). Env: `HS_TERM_WORKDIR` (default /app),
//! `HS_TERM_EXEC_TIMEOUT_SECS` (default 120).
include!("shared/sdk.rs");
fn main() {
    serve("term.exec", "tool", &mut |method, params| match method {
        "tool.call" => {
            let cmd = params["args"]["command"].as_str().unwrap_or("");
            let wd = std::env::var("HS_TERM_WORKDIR").unwrap_or_else(|_| "/app".into());
            let timeout: u64 = std::env::var("HS_TERM_EXEC_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(120);
            hs_loop::termexec::run(std::path::Path::new(&wd), cmd, timeout)
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    });
}
