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

    // Backend selection: default is "direct" (opens the atheneum DB via the
    // atheneum crate, no external services). Set ATHENEUM_BACKEND=http to use
    // the envoy HTTP bridge instead.
    let mode = std::env::var("ATHENEUM_BACKEND").unwrap_or_else(|_| "direct".to_string());

    let backend: Arc<dyn backend::Backend> = if mode == "http" {
        #[cfg(feature = "http")]
        {
            let base_url = std::env::var("ATHENEUM_URL")
                .unwrap_or_else(|_| "http://localhost:9876".to_string());
            tracing::info!("atheneum-mcp starting (backend: http {base_url})");
            Arc::new(backend::http::HttpBackend::new(base_url))
        }
        #[cfg(not(feature = "http"))]
        {
            anyhow::bail!("HTTP backend requested but compiled without --features http")
        }
    } else {
        #[cfg(feature = "direct")]
        {
            let db_path = std::env::var("ATHENEUM_DB")
                .unwrap_or_else(|_| "~/.magellan/atheneum/atheneum.db".to_string());
            let expanded = shellexpand::tilde(&db_path).to_string();
            let path = std::path::PathBuf::from(&expanded);
            tracing::info!(
                "atheneum-mcp starting (backend: direct, db: {})",
                path.display()
            );
            let graph = atheneum::AtheneumGraph::open(&path)?;
            Arc::new(backend::direct::direct_from_graph(graph))
        }
        #[cfg(not(feature = "direct"))]
        {
            anyhow::bail!("Direct backend requested but compiled without --features direct")
        }
    };

    let server = AtheneumMcpServer::new(backend);
    let service = server.serve(stdio()).await?;
    service.waiting().await?;

    Ok(())
}
