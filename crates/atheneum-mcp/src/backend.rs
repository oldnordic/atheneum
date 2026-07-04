//! Backend abstraction for Atheneum operations.
//!
//! The MCP server is decoupled from Atheneum itself via the [`Backend`] trait.
//! Two implementations are provided:
//! - **HTTP**: calls an envoy/Atheneum HTTP bridge (default)
//! - **Direct**: links against the `atheneum` crate directly (feature `direct`)

use anyhow::Result;
use async_trait::async_trait;
#[cfg(feature = "http")]
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
    pub key: Option<String>,
    pub content: String,
    pub tags: Vec<String>,
    pub importance: i64,
    pub scope: Option<String>,
    pub project: Option<String>,
}

/// Parameters for querying a memory entry by key.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QueryMemoryParams {
    pub key: String,
    pub scope: Option<String>,
    pub project: Option<String>,
    pub k: usize,
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
    async fn query_memory(&self, params: QueryMemoryParams) -> Result<Value>;
    async fn list_sessions(&self, limit: i64) -> Result<Value>;
    async fn list_events(&self, limit: i64) -> Result<Value>;
    async fn navigate(&self, query: &str, k: usize, depth: u32) -> Result<Value>;
    async fn graph_stats(&self) -> Result<Value>;
    // --- Phase 2 additions ---
    async fn search_memory(&self, query: &str, k: usize, project: Option<&str>) -> Result<Value>;
    async fn list_memory(
        &self,
        scope: Option<&str>,
        project: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> Result<Value>;
    async fn memory_bootstrap(
        &self,
        project: Option<&str>,
        tokens: usize,
        last_sessions: i64,
    ) -> Result<Value>;
    async fn query_wiki(&self, path: &str) -> Result<Value>;
    async fn wiki_search(&self, query: &str, project: Option<&str>, limit: usize) -> Result<Value>;
    async fn discoveries_recent(
        &self,
        project: Option<&str>,
        agent: Option<&str>,
        session: Option<&str>,
        dtype: Option<&str>,
        limit: i64,
    ) -> Result<Value>;
    async fn decision_search(
        &self,
        query: &str,
        project: Option<&str>,
        limit: i64,
    ) -> Result<Value>;
    async fn thread(
        &self,
        query: &str,
        k: usize,
        depth: u32,
        project: Option<&str>,
        tokens: usize,
    ) -> Result<Value>;
    async fn session_digest(&self, project: Option<&str>, last_sessions: i64) -> Result<Value>;
    async fn get_entity(&self, id: i64) -> Result<Value>;
    async fn get_neighbors(&self, id: i64) -> Result<Value>;
    async fn dream(
        &self,
        scope: Option<&str>,
        project: Option<&str>,
        dry_run: bool,
    ) -> Result<Value>;
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

        async fn query_memory(&self, _params: QueryMemoryParams) -> Result<Value> {
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

        // Phase 2 additions — not available via HTTP bridge.
        async fn search_memory(&self, _q: &str, _k: usize, _p: Option<&str>) -> Result<Value> {
            Err(not_direct("search_memory"))
        }
        async fn list_memory(
            &self,
            _s: Option<&str>,
            _p: Option<&str>,
            _o: usize,
            _l: usize,
        ) -> Result<Value> {
            Err(not_direct("list_memory"))
        }
        async fn memory_bootstrap(&self, _p: Option<&str>, _t: usize, _l: i64) -> Result<Value> {
            Err(not_direct("memory_bootstrap"))
        }
        async fn query_wiki(&self, _path: &str) -> Result<Value> {
            Err(not_direct("query_wiki"))
        }
        async fn wiki_search(&self, _q: &str, _p: Option<&str>, _l: usize) -> Result<Value> {
            Err(not_direct("wiki_search"))
        }
        async fn discoveries_recent(
            &self,
            _p: Option<&str>,
            _a: Option<&str>,
            _s: Option<&str>,
            _t: Option<&str>,
            _l: i64,
        ) -> Result<Value> {
            Err(not_direct("discoveries_recent"))
        }
        async fn decision_search(&self, _q: &str, _p: Option<&str>, _l: i64) -> Result<Value> {
            Err(not_direct("decision_search"))
        }
        async fn thread(
            &self,
            _q: &str,
            _k: usize,
            _d: u32,
            _p: Option<&str>,
            _t: usize,
        ) -> Result<Value> {
            Err(not_direct("thread"))
        }
        async fn session_digest(&self, _p: Option<&str>, _l: i64) -> Result<Value> {
            Err(not_direct("session_digest"))
        }
        async fn get_entity(&self, _id: i64) -> Result<Value> {
            Err(not_direct("get_entity"))
        }
        async fn get_neighbors(&self, _id: i64) -> Result<Value> {
            Err(not_direct("get_neighbors"))
        }
        async fn dream(&self, _s: Option<&str>, _p: Option<&str>, _d: bool) -> Result<Value> {
            Err(not_direct("dream"))
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

    /// Convenience: wrap a raw AtheneumGraph into a DirectBackend usable as
    /// a `dyn Backend` trait object (used by main.rs).
    pub fn direct_from_graph(graph: AtheneumGraph) -> DirectBackend {
        DirectBackend::new(Arc::new(tokio::sync::Mutex::new(graph)))
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
            // Preserve legacy behavior when callers omit `key`: derive a stable
            // exact-lookup key from the content.
            let key = params.key.unwrap_or_else(|| {
                if params.content.len() > 64 {
                    format!("{}…", &params.content[..63])
                } else {
                    params.content.clone()
                }
            });
            let scope = params.scope.unwrap_or_else(|| "agent".to_string());
            let confidence = (params.importance as f64 / 10.0).clamp(0.0, 1.0);
            let graph = self.graph.lock().await;
            tokio::task::block_in_place(|| {
                let tags = if params.tags.is_empty() {
                    None
                } else {
                    Some(params.tags.as_slice())
                };
                let id = graph.store_memory(
                    &key,
                    &params.content,
                    &scope,
                    confidence,
                    params.project.as_deref(),
                    tags,
                )?;
                Ok(json!({
                    "memory_id": id,
                    "key": key,
                    "scope": scope,
                    "confidence": confidence,
                    "project": params.project,
                    "tags": params.tags,
                }))
            })
        }

        async fn query_memory(&self, params: QueryMemoryParams) -> Result<Value> {
            let graph = self.graph.lock().await;
            tokio::task::block_in_place(|| {
                let results = graph.query_memory(
                    &params.key,
                    params.scope.as_deref(),
                    params.project.as_deref(),
                )?;
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
                        // Filter metadata edges and cap output size.
                        // Metadata edges (belongs_to_project, accessed, modified, observed_in)
                        // are bookkeeping, not navigational signal. They dominate the
                        // edge dump and obscure real relationships.
                        const METADATA_EDGE_TYPES: &[&str] = &[
                            "belongs_to_project",
                            "accessed",
                            "modified",
                            "observed_in",
                            "created_in_session",
                        ];
                        let filtered_edges: Vec<Value> = v
                            .edges
                            .into_iter()
                            .filter(|e| !METADATA_EDGE_TYPES.contains(&e.edge_type.as_str()))
                            .map(|e| {
                                json!({
                                    "type": e.edge_type,
                                    "from_id": e.from_id,
                                    "to_id": e.to_id,
                                })
                            })
                            .collect();

                        // Summarize entities: just kind + name, not full data blob
                        let entity_summary: Vec<Value> = v
                            .entities
                            .into_iter()
                            .map(|e| {
                                json!({
                                    "kind": e.kind,
                                    "name": e.name,
                                })
                            })
                            .collect();

                        json!({
                            "entry": {
                                "kind": v.entry.kind,
                                "name": v.entry.name,
                            },
                            "depth": v.depth,
                            "entities": entity_summary,
                            "edges": filtered_edges,
                            "metadata_edges_filtered": true,
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

        async fn search_memory(
            &self,
            query: &str,
            k: usize,
            project: Option<&str>,
        ) -> Result<Value> {
            let graph = self.graph.lock().await;
            tokio::task::block_in_place(|| {
                let results = graph.lexical_search(query, k, project, Some("Memory"), None)?;
                Ok(serde_json::to_value(results)?)
            })
        }

        async fn list_memory(
            &self,
            scope: Option<&str>,
            project: Option<&str>,
            offset: usize,
            limit: usize,
        ) -> Result<Value> {
            let graph = self.graph.lock().await;
            tokio::task::block_in_place(|| {
                let results = graph.list_memory_page(scope, project, offset, limit)?;
                Ok(serde_json::to_value(results)?)
            })
        }

        async fn memory_bootstrap(
            &self,
            project: Option<&str>,
            tokens: usize,
            last_sessions: i64,
        ) -> Result<Value> {
            let graph = self.graph.lock().await;
            tokio::task::block_in_place(|| {
                graph.compose_memory_bootstrap(project, tokens, last_sessions)
            })
        }

        async fn query_wiki(&self, path: &str) -> Result<Value> {
            let graph = self.graph.lock().await;
            tokio::task::block_in_place(|| match graph.get_wiki_page(path)? {
                Some(page) => Ok(serde_json::to_value(page)?),
                None => Ok(json!({ "found": false, "path": path })),
            })
        }

        async fn wiki_search(
            &self,
            query: &str,
            project: Option<&str>,
            limit: usize,
        ) -> Result<Value> {
            let graph = self.graph.lock().await;
            tokio::task::block_in_place(|| {
                let results = graph.search_wiki_pages(query, project, 0, limit)?;
                Ok(serde_json::to_value(results)?)
            })
        }

        async fn discoveries_recent(
            &self,
            project: Option<&str>,
            agent: Option<&str>,
            session: Option<&str>,
            dtype: Option<&str>,
            limit: i64,
        ) -> Result<Value> {
            let graph = self.graph.lock().await;
            tokio::task::block_in_place(|| {
                let results = graph.recent_discoveries(project, agent, session, dtype, limit)?;
                Ok(serde_json::to_value(results)?)
            })
        }

        async fn decision_search(
            &self,
            query: &str,
            project: Option<&str>,
            limit: i64,
        ) -> Result<Value> {
            let graph = self.graph.lock().await;
            tokio::task::block_in_place(|| {
                let results = graph.search_decisions(query, project, limit)?;
                Ok(serde_json::to_value(results)?)
            })
        }

        async fn thread(
            &self,
            query: &str,
            k: usize,
            depth: u32,
            project: Option<&str>,
            tokens: usize,
        ) -> Result<Value> {
            let graph = self.graph.lock().await;
            tokio::task::block_in_place(|| {
                let views = graph.thread_query(query, k, depth, project, tokens)?;
                let out: Vec<Value> = views
                    .iter()
                    .map(|v| {
                        json!({
                            "entry": v.entry,
                            "depth": v.depth,
                            "entities": v.entities,
                            "edges": v.edges,
                        })
                    })
                    .collect();
                Ok(Value::Array(out))
            })
        }

        async fn session_digest(&self, project: Option<&str>, last_sessions: i64) -> Result<Value> {
            let graph = self.graph.lock().await;
            tokio::task::block_in_place(|| graph.compose_digest_json(project, last_sessions))
        }

        async fn get_entity(&self, id: i64) -> Result<Value> {
            let graph = self.graph.lock().await;
            tokio::task::block_in_place(|| {
                let entity = graph.get_entity(id)?;
                Ok(serde_json::to_value(entity)?)
            })
        }

        async fn get_neighbors(&self, id: i64) -> Result<Value> {
            let graph = self.graph.lock().await;
            tokio::task::block_in_place(|| {
                let outgoing = graph.outgoing_edges(id)?;
                let incoming = graph.incoming_edges(id)?;
                Ok(json!({
                    "outgoing": outgoing,
                    "incoming": incoming,
                }))
            })
        }

        async fn dream(
            &self,
            scope: Option<&str>,
            project: Option<&str>,
            dry_run: bool,
        ) -> Result<Value> {
            let graph = self.graph.lock().await;
            tokio::task::block_in_place(|| {
                let mode = if dry_run {
                    atheneum::DreamMode::DryRun
                } else {
                    atheneum::DreamMode::AutoMerge
                };
                let config = atheneum::DreamConfig::default();
                let report = graph.dream_pass(mode, scope, project, &config)?;
                Ok(serde_json::to_value(report)?)
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Helper: minimal percent-encoding for query strings
// ---------------------------------------------------------------------------

#[cfg(feature = "http")]
fn not_direct(op: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "{op} requires the direct backend (set ATHENEUM_BACKEND=direct or compile with --features direct)"
    )
}

#[cfg(feature = "http")]
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
