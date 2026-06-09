use std::sync::Arc;

use atheneum_mcp::{backend, AtheneumMcpServer};
use rmcp::transport::stdio;
use rmcp::ServiceExt;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let base_url =
        std::env::var("ATHENEUM_URL").unwrap_or_else(|_| "http://localhost:9876".to_string());

    tracing::info!("atheneum-mcp starting (backend: {base_url})");

    let backend: Arc<dyn backend::Backend> = Arc::new(backend::http::HttpBackend::new(base_url));

    let server = AtheneumMcpServer::new(backend);

    let service = server.serve(stdio()).await?;
    service.waiting().await?;

    Ok(())
}
