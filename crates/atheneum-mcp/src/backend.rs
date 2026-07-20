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
#[cfg(any(feature = "direct", test))]
use serde_json::json;
use serde_json::Value;
#[cfg(any(feature = "direct", test))]
use std::collections::HashSet;

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
    pub include_superseded: Option<bool>,
}

/// Parameters for patching a memory entry in place. Mirrors the underlying
/// `atheneum::MemoryPatch`. `id` is required; all other fields optional.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UpdateMemoryParams {
    pub id: i64,
    pub content: Option<String>,
    pub importance: Option<i64>,
    pub tags: Option<Vec<String>>,
    pub replace_tags: Option<bool>,
}

/// Parameters for adding/upserting a memory entry by concept.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AddMemoryParams {
    pub concept: String,
    pub body_patch: String,
    pub link_from: Option<i64>,
    pub link_both_ways: Option<bool>,
}

/// Parameters for running graph maintenance.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MaintainParams {
    pub apply: Option<bool>,
    pub stale_superseded_days: Option<i64>,
    pub broken_link_mode: Option<String>,
    pub rewire_threshold: Option<f64>,
}

/// Parameters for seeding/bootstrapping memory.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SeedMemoryParams {
    pub project: Option<String>,
    pub tokens: Option<usize>,
}

#[cfg(any(feature = "direct", test))]
const METADATA_EDGE_TYPES: &[&str] = &[
    "belongs_to_project",
    "accessed",
    "modified",
    "observed_in",
    "created_in_session",
    "handled_by_tool",
];

#[cfg(any(feature = "direct", test))]
const NOISE_ENTITY_KINDS: &[&str] = &["ToolCall", "ReasoningLog", "TestRun"];

#[cfg(any(feature = "direct", test))]
const EDGE_PAGE_MULTIPLIER: usize = 4;

#[cfg(any(feature = "direct", test))]
const MIN_EDGE_LIMIT: usize = 32;

#[cfg(any(feature = "direct", test))]
const MAX_EDGE_LIMIT: usize = 200;

// ---------------------------------------------------------------------------
// Backend trait
// ---------------------------------------------------------------------------

#[async_trait]
pub trait Backend: Send + Sync + 'static {
    async fn store_discovery(&self, params: StoreDiscoveryParams) -> Result<Value>;
    async fn query_knowledge(&self, target: &str, project: Option<&str>) -> Result<Value>;
    async fn search(&self, query: &str, k: usize, project: Option<&str>) -> Result<Value>;
    async fn store_memory(&self, params: StoreMemoryParams) -> Result<Value>;
    async fn update_memory(&self, params: UpdateMemoryParams) -> Result<Value>;
    async fn add_memory(&self, params: AddMemoryParams) -> Result<Value>;
    async fn maintain(&self, params: MaintainParams) -> Result<Value>;
    async fn seed_memory(&self, params: SeedMemoryParams) -> Result<Value>;
    async fn query_memory(&self, params: QueryMemoryParams) -> Result<Value>;
    async fn list_sessions(&self, limit: i64) -> Result<Value>;
    async fn list_events(&self, limit: i64) -> Result<Value>;
    async fn navigate(
        &self,
        query: &str,
        k: usize,
        depth: u32,
        offset: usize,
        limit: usize,
    ) -> Result<Value>;
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

#[cfg(any(feature = "direct", test))]
fn edge_page_limit(limit: usize) -> usize {
    (limit.saturating_mul(EDGE_PAGE_MULTIPLIER)).clamp(MIN_EDGE_LIMIT, MAX_EDGE_LIMIT)
}

#[cfg(any(feature = "direct", test))]
fn serialize_paginated_view(
    entry: &atheneum::GraphEntity,
    depth: u32,
    entities: Vec<atheneum::GraphEntity>,
    edges: Vec<atheneum::GraphEdge>,
    offset: usize,
    limit: usize,
) -> Value {
    let total_input_entities = entities.len();
    let total_signal: Vec<_> = entities
        .into_iter()
        .filter(|e| !NOISE_ENTITY_KINDS.contains(&e.kind.as_str()))
        .collect();
    let total_signal_count = total_signal.len();
    let page_entities: Vec<_> = total_signal.into_iter().skip(offset).take(limit).collect();
    let returned = page_entities.len();
    let has_more = offset + returned < total_signal_count;
    let noise_filtered = total_input_entities.saturating_sub(total_signal_count);

    let mut visible_entity_ids: HashSet<i64> = page_entities.iter().map(|e| e.id).collect();
    visible_entity_ids.insert(entry.id);

    let edge_limit = edge_page_limit(limit);
    let mut seen_edges: HashSet<(i64, i64, String)> = HashSet::new();
    let mut total_signal_edges = 0usize;
    let mut serialized_edges = Vec::new();

    for edge in edges.into_iter().filter(|e| {
        !METADATA_EDGE_TYPES.contains(&e.edge_type.as_str())
            && visible_entity_ids.contains(&e.from_id)
            && visible_entity_ids.contains(&e.to_id)
    }) {
        let key = (edge.from_id, edge.to_id, edge.edge_type.clone());
        if !seen_edges.insert(key) {
            continue;
        }
        total_signal_edges += 1;
        if serialized_edges.len() < edge_limit {
            serialized_edges.push(json!({
                "type": edge.edge_type,
                "from_id": edge.from_id,
                "to_id": edge.to_id,
            }));
        }
    }

    json!({
        "entry": {
            "kind": entry.kind,
            "name": entry.name,
        },
        "depth": depth,
        "entities": page_entities.into_iter().map(|e| {
            json!({
                "kind": e.kind,
                "name": e.name,
            })
        }).collect::<Vec<_>>(),
        "edges": serialized_edges,
        "noise_filtered": noise_filtered,
        "total_signal_entities": total_signal_count,
        "offset": offset,
        "limit": limit,
        "has_more": has_more,
        "edge_limit": edge_limit,
        "total_signal_edges": total_signal_edges,
        "edges_has_more": total_signal_edges > edge_limit,
    })
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

        async fn update_memory(&self, _params: UpdateMemoryParams) -> Result<Value> {
            Err(anyhow::anyhow!(
                "update_memory requires direct backend (memory has no HTTP endpoint in envoy). \
                 Run with --features direct or set ATHENEUM_DIRECT=1"
            ))
        }

        async fn add_memory(&self, _params: AddMemoryParams) -> Result<Value> {
            Err(anyhow::anyhow!(
                "add_memory requires direct backend (memory has no HTTP endpoint in envoy). \
                 Run with --features direct or set ATHENEUM_DIRECT=1"
            ))
        }
        async fn maintain(&self, _params: MaintainParams) -> Result<Value> {
            Err(anyhow::anyhow!(
                "maintain requires direct backend. \
                 Run with --features direct or set ATHENEUM_DIRECT=1"
            ))
        }
        async fn seed_memory(&self, _params: SeedMemoryParams) -> Result<Value> {
            Ok(json!({
                "instructions": "HttpBackend connected. Run query/navigate to discover knowledge.",
                "token_estimate": 10
            }))
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

        async fn navigate(
            &self,
            query: &str,
            k: usize,
            depth: u32,
            offset: usize,
            limit: usize,
        ) -> Result<Value> {
            let path = format!(
                "/atheneum/graph/navigate?q={}&k={}&depth={depth}&offset={offset}&limit={limit}",
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

        async fn update_memory(&self, params: UpdateMemoryParams) -> Result<Value> {
            let patch = atheneum::MemoryPatch {
                content: params.content,
                importance: params.importance,
                tags: params.tags,
                replace_tags: params.replace_tags.unwrap_or(false),
            };
            let graph = self.graph.lock().await;
            tokio::task::block_in_place(|| {
                if patch.is_empty() {
                    return Err(anyhow::anyhow!(
                        "update_memory requires at least one of: content, importance, tags"
                    ));
                }
                let id = graph.update_memory(params.id, &patch)?;
                let entity = graph.get_entity(id)?;
                Ok(json!({
                    "memory_id": id,
                    "key": entity.name,
                    "scope": entity.data.get("scope").and_then(|v| v.as_str()),
                    "content": entity.data.get("content").and_then(|v| v.as_str()),
                    "confidence": entity.data.get("confidence").and_then(|v| v.as_f64()),
                    "updated_at": entity.data.get("updated_at").and_then(|v| v.as_str()),
                    "content_hash": entity.data.get("content_hash").and_then(|v| v.as_str()),
                    "tags": entity.data.get("tags"),
                }))
            })
        }

        async fn add_memory(&self, params: AddMemoryParams) -> Result<Value> {
            let graph = self.graph.lock().await;
            tokio::task::block_in_place(|| {
                let result = graph.upsert_memory_by_concept(
                    &params.concept,
                    &params.body_patch,
                    params.link_from,
                    params.link_both_ways.unwrap_or(true),
                )?;
                let entity = graph.get_entity(result.memory_id)?;
                Ok(json!({
                    "memory_id": result.memory_id,
                    "action": result.action,
                    "key": entity.name,
                    "scope": entity.data.get("scope").and_then(|v| v.as_str()),
                    "content": entity.data.get("content").and_then(|v| v.as_str()),
                    "confidence": entity.data.get("confidence").and_then(|v| v.as_f64()),
                    "updated_at": entity.data.get("updated_at").and_then(|v| v.as_str()),
                    "tags": entity.data.get("tags"),
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
                    params.include_superseded.unwrap_or(false),
                )?;
                Ok(serde_json::to_value(results)?)
            })
        }
        async fn maintain(&self, params: MaintainParams) -> Result<Value> {
            let graph = self.graph.lock().await;
            tokio::task::block_in_place(|| {
                let stale_superseded_days = params.stale_superseded_days.unwrap_or(30);
                let rewire_threshold = params.rewire_threshold.unwrap_or(0.3);
                let broken_link_mode = match params.broken_link_mode.as_deref() {
                    Some("sever") => atheneum::BrokenLinkMode::Sever,
                    _ => atheneum::BrokenLinkMode::Stub,
                };
                let apply = params.apply.unwrap_or(false);
                let report = graph.maintain(&atheneum::MaintainConfig {
                    rewire_threshold,
                    broken_link_mode,
                    stale_superseded_days,
                }, apply)?;
                Ok(serde_json::to_value(report)?)
            })
        }
        async fn seed_memory(&self, params: SeedMemoryParams) -> Result<Value> {
            let graph = self.graph.lock().await;
            tokio::task::block_in_place(|| {
                let seed = graph.seed_memory(params.project.as_deref(), params.tokens.unwrap_or(800))?;
                Ok(serde_json::to_value(seed)?)
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

        async fn navigate(
            &self,
            query: &str,
            k: usize,
            depth: u32,
            offset: usize,
            limit: usize,
        ) -> Result<Value> {
            let graph = self.graph.lock().await;
            tokio::task::block_in_place(|| {
                let results = graph.navigate(query, k, depth, None, None, None)?;
                let views: Vec<Value> = results
                    .into_iter()
                    .map(|v| {
                        serialize_paginated_view(
                            &v.entry, v.depth, v.entities, v.edges, offset, limit,
                        )
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

#[cfg(test)]
mod tests {
    use super::*;
    use atheneum::{GraphEdge, GraphEntity};

    fn entity(id: i64, kind: &str, name: &str) -> GraphEntity {
        GraphEntity {
            id,
            kind: kind.to_string(),
            name: name.to_string(),
            file_path: None,
            data: serde_json::json!({}),
        }
    }

    fn edge(id: i64, from_id: i64, to_id: i64, edge_type: &str) -> GraphEdge {
        GraphEdge {
            id,
            from_id,
            to_id,
            edge_type: edge_type.to_string(),
            data: serde_json::json!({}),
        }
    }

    #[test]
    fn serialize_paginated_view_caps_and_dedups_visible_edges() {
        let entry = entity(1, "File", "entry");
        let entities = vec![
            entity(2, "File", "a"),
            entity(3, "ToolCall", "noise"),
            entity(4, "File", "b"),
        ];
        let edges = vec![
            edge(10, 1, 2, "wikilink"),
            edge(11, 1, 2, "wikilink"),
            edge(12, 1, 4, "wikilink"),
            edge(13, 1, 3, "verified_by"),
            edge(14, 1, 2, "handled_by_tool"),
        ];

        let view = serialize_paginated_view(&entry, 2, entities, edges, 0, 1);

        assert_eq!(view["entities"].as_array().unwrap().len(), 1);
        assert_eq!(view["total_signal_entities"], 2);
        assert_eq!(view["has_more"], true);

        let serialized_edges = view["edges"].as_array().unwrap();
        assert_eq!(serialized_edges.len(), 1);
        assert_eq!(serialized_edges[0]["type"], "wikilink");
        assert_eq!(serialized_edges[0]["to_id"], 2);
        assert_eq!(view["total_signal_edges"], 1);
        assert_eq!(view["edges_has_more"], false);
    }
}
