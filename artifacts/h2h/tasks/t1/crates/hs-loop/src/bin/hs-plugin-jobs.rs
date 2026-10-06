//! Tool "jobs": background jobs (start/list/output/kill) under HS_TERM_WORKDIR.
include!("shared/sdk.rs");
fn main() {
    serve("jobs", "tool", &mut |method, params| match method {
        "tool.call" => {
            let wd = std::env::var("HS_TERM_WORKDIR").unwrap_or_else(|_| ".".into());
            hs_loop::jobs::call(std::path::Path::new(&wd), &params["args"])
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    });
}
