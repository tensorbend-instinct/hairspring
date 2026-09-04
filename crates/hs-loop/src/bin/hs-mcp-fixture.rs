//! In-tree MCP fixture server (rmcp over stdio) for bridge contract tests.
//! One tool: fixture.echo {text} -> text. Not shipped config; test-only.
use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
};

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct EchoRequest {
    #[schemars(description = "text to echo back")]
    pub text: String,
}

#[derive(Debug, Clone)]
struct Fixture {
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl Fixture {
    #[tool(description = "Echo the text back")]
    fn echo(&self, Parameters(EchoRequest { text }): Parameters<EchoRequest>) -> String {
        text
    }
}

#[tool_handler]
impl ServerHandler for Fixture {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some("hairspring MCP fixture".into()),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = rmcp::serve_server(
        Fixture { tool_router: Fixture::tool_router() },
        rmcp::transport::stdio(),
    )
    .await?;
    server.waiting().await?;
    Ok(())
}
