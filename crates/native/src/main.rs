use anyhow::{Context as _, Result};
use clap::Parser;
use rmcp::ServiceExt as _;

use arxiv_search_rs_mcp_native::fetch::FetchClient;
use arxiv_search_rs_mcp_native::tool::ArxivServer;

#[derive(Parser)]
#[command(
    name = "arxiv-search-mcp",
    about = "arXiv Search MCP Server",
    long_about = "Exposes MCP tools for searching arXiv and retrieving prepared paper content. \
                  The native binary is for local MCP clients and Claude Desktop; the repo also \
                  includes a Cloudflare Worker entrypoint under crates/worker.\n\n\
                  Use --stdio for Claude Desktop and local MCP clients.\n\
                  Default: Streamable HTTP server (MCP 2025-03-26) on POST /mcp.\n\n\
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
    let client = FetchClient::new(ss_api_key)
        .await
        .context("failed to build HTTP client")?;
    let server = ArxivServer::new(client);

    if cli.stdio {
        tracing::info!("Starting in stdio mode");
        let service = server
            .serve(rmcp::transport::io::stdio())
            .await
            .context("Failed to initialise stdio transport")?;
        service
            .waiting()
            .await
            .context("stdio server exited with error")?;
    } else {
        let addr = format!("{}:{}", cli.host, cli.port);
        tracing::info!("Starting Streamable HTTP server on http://{addr}/mcp");
        run_http_server(server, &addr).await?;
    }

    Ok(())
}

async fn run_http_server(server: ArxivServer, addr: &str) -> Result<()> {
    use rmcp::transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    };
    use tokio_util::sync::CancellationToken;

    let bind: std::net::SocketAddr = addr.parse().context("Invalid bind address")?;
    let ct = CancellationToken::new();

    let config = StreamableHttpServerConfig::default().with_cancellation_token(ct.child_token());

    let service: StreamableHttpService<ArxivServer, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok(server.clone()),
            std::sync::Arc::default(),
            config,
        );

    let router = axum::Router::new().nest_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .context("Failed to bind TCP listener")?;

    tracing::info!("Listening on http://{addr}/mcp");

    axum::serve(listener, router)
        .with_graceful_shutdown(async move { ct.cancelled_owned().await })
        .await
        .context("HTTP server error")?;

    Ok(())
}
