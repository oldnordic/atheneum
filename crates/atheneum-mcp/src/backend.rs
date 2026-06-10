//! Backend abstraction for Atheneum operations.
//!
//! The MCP server is decoupled from Atheneum itself via the [`Backend`] trait.
//! Two implementations are provided:
//! - **HTTP**: calls an envoy/Atheneum HTTP bridge (default)
//! - **Direct**: links against the `atheneum` crate directly (feature `direct`)

use anyhow::Result;
use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde_json::Value;

// ---------------------------------------------------------------------------
// Parameter types
// ---------------------------------------------------------------------------

/// Parameters for storing a discovery.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StoreDiscoveryParams {
    pub target: String,
    pub observation: String,
    pub confidence: f64,
    pub tags: Vec<String>,
    pub project: Option<String>,
}

/// Parameters for storing a memory entry.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StoreMemoryParams {
    pub content: String,
    pub tags: Vec<String>,
    pub importance: i64,
}

// ---------------------------------------------------------------------------
// Backend trait
// ---------------------------------------------------------------------------

#[async_trait]
pub trait Backend: Send + Sync + 'static {
    async fn store_discovery(&self, params: StoreDiscoveryParams) -> Result<Value>;
    async fn query_knowledge(&self, target: &str, project: Option<&str>) -> Result<Value>;
    async fn search(&self, query: &str, k: usize, project: Option<&str>) -> Result<Value>;
    async fn store_memory(&self, params: StoreMemoryParams) -> Result<Value>;
    async fn query_memory(&self, query: &str, k: usize) -> Result<Value>;
    async fn list_sessions(&self, limit: i64) -> Result<Value>;
    async fn list_events(&self, limit: i64) -> Result<Value>;
    async fn navigate(&self, query: &str, k: usize, depth: u32) -> Result<Value>;
    async fn graph_stats(&self) -> Result<Value>;
}

// ---------------------------------------------------------------------------
// HTTP backend
// ---------------------------------------------------------------------------

#[cfg(feature = "http")]
pub mod http {
    use super::*;
    use reqwest::Client;

    pub struct HttpBackend {
        client: Client,
        base_url: String,
    }

    impl HttpBackend {
        pub fn new(base_url: impl Into<String>) -> Self {
            Self {
                client: Client::new(),
                base_url: base_url.into(),
            }
        }

        async fn post_json<P: serde::Serialize, R: DeserializeOwned>(
            &self,
            path: &str,
            payload: &P,
        ) -> Result<R> {
            let url = format!("{}{}", self.base_url, path);
            let resp = self.client.post(&url).json(payload).send().await?;
            if !resp.status().is_success() {
                let text = resp.text().await.unwrap_or_default();
                return Err(anyhow::anyhow!("HTTP error: {text}"));
            }
            Ok(resp.json().await?)
        }

        async fn get_json<R: DeserializeOwned>(&self, path: &str) -> Result<R> {
            let url = format!("{}{}", self.base_url, path);
            let resp = self.client.get(&url).send().await?;
            if !resp.status().is_success() {
                let text = resp.text().await.unwrap_or_default();
                return Err(anyhow::anyhow!("HTTP error: {text}"));
            }
            Ok(resp.json().await?)
        }
    }

    #[async_trait]
    impl Backend for HttpBackend {
        async fn store_discovery(&self, params: StoreDiscoveryParams) -> Result<Value> {
            let agent =
                std::env::var("ATHENEUM_AGENT").unwrap_or_else(|_| "mcp-client".to_string());
            let payload = serde_json::json!({
                "agent": agent,
                "discovery_type": "observation",
                "target": params.target,
                "project_id": params.project,
                "metadata": {
                    "observation": params.observation,
                    "confidence": params.confidence,
                    "tags": params.tags,
                }
            });
            self.post_json("/atheneum/discoveries", &payload).await
        }

        async fn query_knowledge(&self, target: &str, project: Option<&str>) -> Result<Value> {
            let mut path = format!("/atheneum/knowledge?target={}", encode(target));
            if let Some(p) = project {
                path.push_str(&format!("&project={}", encode(p)));
            }
            self.get_json(&path).await
        }

        async fn search(&self, query: &str, k: usize, project: Option<&str>) -> Result<Value> {
            let mut path = format!("/atheneum/search?q={}&k={}", encode(query), k);
            if let Some(p) = project {
                path.push_str(&format!("&project={}", encode(p)));
            }
            self.get_json(&path).await
        }

        async fn store_memory(&self, _params: StoreMemoryParams) -> Result<Value> {
            Err(anyhow::anyhow!(
                "store_memory requires direct backend (memory has no HTTP endpoint in envoy). \
                 Run with --features direct or set ATHENEUM_DIRECT=1"
            ))
        }

        async fn query_memory(&self, _query: &str, _k: usize) -> Result<Value> {
            Err(anyhow::anyhow!(
                "query_memory requires direct backend (memory has no HTTP endpoint in envoy). \
                 Run with --features direct or set ATHENEUM_DIRECT=1"
            ))
        }

        async fn list_sessions(&self, limit: i64) -> Result<Value> {
            let path = format!("/atheneum/sessions?last={limit}");
            self.get_json(&path).await
        }

        async fn list_events(&self, limit: i64) -> Result<Value> {
            let path = format!("/atheneum/events?limit={limit}");
            self.get_json(&path).await
        }

        async fn navigate(&self, query: &str, k: usize, depth: u32) -> Result<Value> {
            let path = format!(
                "/atheneum/graph/navigate?q={}&k={}&depth={depth}",
                encode(query),
                k
            );
            self.get_json(&path).await
        }

        async fn graph_stats(&self) -> Result<Value> {
            self.get_json("/atheneum/graph/stats").await
        }
    }
}

// ---------------------------------------------------------------------------
// Direct backend (links against atheneum crate)
// ---------------------------------------------------------------------------

#[cfg(feature = "direct")]
pub mod direct {
    use super::*;
    use atheneum::AtheneumGraph;
    use serde_json::json;
    use std::sync::Arc;

    pub struct DirectBackend {
        graph: Arc<tokio::sync::Mutex<AtheneumGraph>>,
    }

    impl DirectBackend {
        pub fn new(graph: Arc<tokio::sync::Mutex<AtheneumGraph>>) -> Self {
            Self { graph }
        }
    }

    #[async_trait]
    impl Backend for DirectBackend {
        async fn store_discovery(&self, params: StoreDiscoveryParams) -> Result<Value> {
            let agent =
                std::env::var("ATHENEUM_AGENT").unwrap_or_else(|_| "mcp-client".to_string());
            let metadata = serde_json::json!({
                "observation": params.observation,
                "confidence": params.confidence,
                "tags": params.tags,
            });
            let graph = self.graph.lock().await;
            tokio::task::block_in_place(|| {
                let id = graph.store_discovery_in_project(
                    &agent,
                    "observation",
                    &params.target,
                    params.project.as_deref(),
                    metadata,
                )?;
                Ok(json!({ "discovery_id": id }))
            })
        }

        async fn query_knowledge(&self, target: &str, project: Option<&str>) -> Result<Value> {
            let graph = self.graph.lock().await;
            tokio::task::block_in_place(|| {
                if let Some(p) = project {
                    graph.query_knowledge_in_project(target, Some(p), None)
                } else {
                    graph.query_knowledge(target, None)
                }
            })
        }

        async fn search(&self, query: &str, k: usize, project: Option<&str>) -> Result<Value> {
            let graph = self.graph.lock().await;
            tokio::task::block_in_place(|| {
                let results = graph.lexical_search(query, k, project, None, None)?;
                Ok(serde_json::to_value(results)?)
            })
        }

        async fn store_memory(&self, params: StoreMemoryParams) -> Result<Value> {
            // Derive a stable key from content since the tool schema does not expose
            // an explicit key field. Truncate to 64 chars to keep keys readable.
            let key = if params.content.len() > 64 {
                format!("{}…", &params.content[..63])
            } else {
                params.content.clone()
            };
            let scope = "agent";
            let confidence = (params.importance as f64 / 10.0).clamp(0.0, 1.0);
            let graph = self.graph.lock().await;
            tokio::task::block_in_place(|| {
                let tags = if params.tags.is_empty() {
                    None
                } else {
                    Some(params.tags.as_slice())
                };
                let id =
                    graph.store_memory(&key, &params.content, scope, confidence, None, tags)?;
                Ok(json!({
                    "memory_id": id,
                    "key": key,
                    "scope": scope,
                    "confidence": confidence,
                    "tags": params.tags,
                }))
            })
        }

        async fn query_memory(&self, query: &str, _k: usize) -> Result<Value> {
            let graph = self.graph.lock().await;
            tokio::task::block_in_place(|| {
                let results = graph.query_memory(query, None, None)?;
                Ok(serde_json::to_value(results)?)
            })
        }

        async fn list_sessions(&self, limit: i64) -> Result<Value> {
            let graph = self.graph.lock().await;
            tokio::task::block_in_place(|| {
                let results = graph.query_sessions(None, limit, None)?;
                Ok(serde_json::to_value(results)?)
            })
        }

        async fn list_events(&self, limit: i64) -> Result<Value> {
            let graph = self.graph.lock().await;
            tokio::task::block_in_place(|| {
                let results = graph.query_events(None, None, limit as usize)?;
                Ok(serde_json::to_value(results)?)
            })
        }

        async fn navigate(&self, query: &str, k: usize, depth: u32) -> Result<Value> {
            let graph = self.graph.lock().await;
            tokio::task::block_in_place(|| {
                let results = graph.navigate(query, k, depth, None, None, None)?;
                let views: Vec<Value> = results
                    .into_iter()
                    .map(|v| {
                        json!({
                            "entry": v.entry,
                            "depth": v.depth,
                            "entities": v.entities,
                            "edges": v.edges,
                        })
                    })
                    .collect();
                Ok(Value::Array(views))
            })
        }

        async fn graph_stats(&self) -> Result<Value> {
            let graph = self.graph.lock().await;
            tokio::task::block_in_place(|| {
                let entity_counts = graph.count_entities_by_kind()?;
                let edge_counts = graph.count_edges_by_type()?;
                let total_entities: i64 = entity_counts.iter().map(|(_, c)| c).sum();
                let total_edges: i64 = edge_counts.iter().map(|(_, c)| c).sum();
                Ok(json!({
                    "entity_count": total_entities,
                    "edge_count": total_edges,
                    "entity_counts": entity_counts,
                    "edge_counts": edge_counts,
                }))
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Helper: minimal percent-encoding for query strings
// ---------------------------------------------------------------------------

fn encode(s: &str) -> String {
    let needs_encoding = s
        .bytes()
        .any(|b| !b.is_ascii_alphanumeric() && !b"-_.~".contains(&b));
    if !needs_encoding {
        return s.to_string();
    }
    s.bytes()
        .map(|b| match b {
            b'-' | b'_' | b'.' | b'~' => String::from_utf8_lossy(&[b]).into_owned(),
            b if b.is_ascii_alphanumeric() => String::from_utf8_lossy(&[b]).into_owned(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}
