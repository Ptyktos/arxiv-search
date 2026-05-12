mod fetch;
mod persistence;
mod rate_limit;
mod tool;

use anyhow::{Context as _, Result};
use clap::Parser;
use rmcp::ServiceExt as _;

use fetch::FetchClient;
use tool::ArxivServer;

#[derive(Parser)]
#[command(
    name = "arxiv-search-mcp",
    about = "arXiv Search MCP Server",
    long_about = "Exposes MCP tools for searching arXiv and retrieving prepared paper content. \
                  The native binary is for local MCP clients and Claude Desktop; the repo also \
                  includes a Cloudflare Worker entrypoint under crates/worker.\n\n\
                  Use --stdio for Claude Desktop and local MCP clients.\n\
                  Default: HTTP/SSE server.\n\n\
                  Optional env vars:\n\
                  SEMANTIC_SCHOLAR_API_KEY — raises Semantic Scholar rate limits."
)]
struct Cli {
    /// Use stdio transport (for Claude Desktop and local MCP clients)
    #[arg(long)]
    stdio: bool,

    /// Host to bind the HTTP server to
    #[arg(long, default_value = "127.0.0.1", env = "HOST")]
    host: String,

    /// Port to bind the HTTP server to
    #[arg(long, default_value = "3000", env = "PORT")]
    port: u16,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "arxiv_search_mcp=info,rmcp=warn".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    let ss_api_key = std::env::var("SEMANTIC_SCHOLAR_API_KEY").ok();
    let client = FetchClient::new(ss_api_key).await.context("failed to build HTTP client")?;
    let server = ArxivServer::new(client);

    if cli.stdio {
        tracing::info!("Starting in stdio mode");
        let service = server
            .serve(rmcp::transport::stdio())
            .await
            .context("Failed to initialise stdio transport")?;
        service
            .waiting()
            .await
            .context("stdio server exited with error")?;
    } else {
        let addr = format!("{}:{}", cli.host, cli.port);
        tracing::info!("Starting HTTP/SSE server on http://{addr}");
        run_sse_server(server, &addr).await?;
    }

    Ok(())
}

async fn run_sse_server(server: ArxivServer, addr: &str) -> Result<()> {
    use rmcp::transport::sse_server::{SseServer, SseServerConfig};
    use tokio_util::sync::CancellationToken;

    let bind: std::net::SocketAddr = addr.parse().context("Invalid bind address")?;
    let ct = CancellationToken::new();

    let config = SseServerConfig {
        bind,
        sse_path: "/sse".to_string(),
        post_path: "/message".to_string(),
        ct: ct.clone(),
    };

    let sse_server = SseServer::serve_with_config(config)
        .await
        .context("Failed to start SSE server")?;

    tracing::info!("Listening on http://{addr} — SSE: /sse, messages: /message");

    let _service_guard = sse_server.with_service(move || server.clone());

    tokio::signal::ctrl_c()
        .await
        .context("Failed to listen for ctrl-c")?;
    tracing::info!("Shutting down");
    ct.cancel();

    Ok(())
}
