//! Tool "glob": see hs_loop::tools2::glob.
include!("shared/sdk.rs");
fn main() {
    serve("glob", "tool", &mut |method, params| match method {
        "tool.call" => {
            let wd = std::env::var("HS_TERM_WORKDIR").or_else(|_| std::env::var("HS_SWE_WORKSPACE")).unwrap_or_else(|_| ".".into());
            hs_loop::tools2::glob(std::path::Path::new(&wd), &params["args"])
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    });
}
