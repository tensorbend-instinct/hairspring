//! Tool "read_image": see hs_loop::tools2.
include!("shared/sdk.rs");
fn main() {
    serve("read_image", "tool", &mut |method, params| match method {
        "tool.call" => {
            let wd = std::env::var("HS_TERM_WORKDIR").unwrap_or_else(|_| ".".into());
            hs_loop::tools2::read_image(std::path::Path::new(&wd), &params["args"])
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    });
}
