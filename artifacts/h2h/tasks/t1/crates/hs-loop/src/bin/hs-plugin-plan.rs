//! Tool "plan": enter/exit plan mode and save the plan under HS_TERM_WORKDIR.
include!("shared/sdk.rs");
fn main() {
    serve("plan", "tool", &mut |method, params| match method {
        "tool.call" => {
            let wd = std::env::var("HS_TERM_WORKDIR").unwrap_or_else(|_| ".".into());
            hs_loop::plan_mode::call(std::path::Path::new(&wd), &params["args"])
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    });
}
