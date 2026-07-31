use std::sync::Arc;

use atheneum_mcp::{backend, AtheneumMcpServer};
use rmcp::transport::stdio;
use rmcp::ServiceExt;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
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
                .unwrap_or_else(|_| "~/.hermes/atheneum/atheneum.db".to_string());
            let expanded = shellexpand::tilde(&db_path).to_string();
            let path = std::path::PathBuf::from(&expanded);
            tracing::info!(
                "atheneum-mcp starting (backend: direct, db: {})",
                path.display()
            );
            let graph = atheneum::AtheneumGraph::open(&path)?;
            let graph = Arc::new(tokio::sync::Mutex::new(graph));
            // Wire the code side of the unified tool API: with a CrossRouter
            // configured, code_query/refresh and search/navigate kind=code|all
            // reach magellan/llmgrep/mirage via meta.db. Without it every
            // code-side call degrades to BACKEND_UNAVAILABLE.
            match atheneum::CrossRouter::open() {
                Ok(cross) => {
                    let cross = cross.with_central_knowledge_db(path.clone());
                    tracing::info!(
                        "CrossRouter configured (meta.db open); code-side tools enabled"
                    );
                    Arc::new(backend::direct::DirectBackend::with_cross_router(
                        graph, cross,
                    ))
                }
                Err(err) => {
                    tracing::warn!(
                        "CrossRouter unavailable ({err}); falling back to graph-only \
                         backend — code-side tools will return BACKEND_UNAVAILABLE"
                    );
                    Arc::new(backend::direct::DirectBackend::new(graph))
                }
            }
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
