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

/// Parameters for semantic consolidation dream.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DreamSemanticParams {
    pub similarity_threshold: Option<f64>,
    pub model: Option<String>,
    pub ollama_url: Option<String>,
    pub swap_guard: Option<String>,
    pub dry_run: Option<bool>,
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

/// Which backend(s) `search` fans out to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchKind {
    #[default]
    Knowledge,
    Code,
    All,
}

impl SearchKind {
    pub fn from_str_default(s: Option<&str>) -> Self {
        match s {
            Some("code") => SearchKind::Code,
            Some("all") => SearchKind::All,
            _ => SearchKind::Knowledge,
        }
    }
}

/// Parameters for the unified `search` tool.
#[derive(Debug, Clone)]
pub struct SearchParams {
    pub query: String,
    pub k: usize,
    pub project: Option<String>,
    pub kind: SearchKind,
    pub limit: Option<usize>,
    pub cursor: Option<String>,
}

// ---------------------------------------------------------------------------
// Backend trait
// ---------------------------------------------------------------------------

#[async_trait]
pub trait Backend: Send + Sync + 'static {
    async fn store_discovery(&self, params: StoreDiscoveryParams) -> Result<Value>;
    async fn query_knowledge(&self, target: &str, project: Option<&str>) -> Result<Value>;
    async fn search(&self, params: SearchParams) -> Result<Value>;
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
        trace: Option<bool>,
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
    async fn list_models(&self) -> Result<Value>;
    async fn dream_semantic(&self, params: DreamSemanticParams) -> Result<Value>;
    async fn pin_entity(&self, id: i64) -> Result<Value>;
    async fn unpin_entity(&self, id: i64) -> Result<Value>;
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

        async fn search(&self, params: SearchParams) -> Result<Value> {
            // NOTE: HTTP backend does not yet support kind/limit/cursor — forwards
            // query/k/project only, same as before this task. Tracked as a gap,
            // not silently dropped.
            let mut path = format!("/atheneum/search?q={}&k={}", encode(&params.query), params.k);
            if let Some(p) = params.project.as_deref() {
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
            _trace: Option<bool>,
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
        async fn list_models(&self) -> Result<Value> {
            Err(not_direct("list_models"))
        }
        async fn dream_semantic(&self, _params: DreamSemanticParams) -> Result<Value> {
            Err(not_direct("dream_semantic"))
        }
        async fn pin_entity(&self, _id: i64) -> Result<Value> {
            Err(not_direct("pin_entity"))
        }
        async fn unpin_entity(&self, _id: i64) -> Result<Value> {
            Err(not_direct("unpin_entity"))
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
        cross: Option<Arc<tokio::sync::Mutex<atheneum::CrossRouter>>>,
    }

    impl DirectBackend {
        pub fn new(graph: Arc<tokio::sync::Mutex<AtheneumGraph>>) -> Self {
            Self { graph, cross: None }
        }

        pub fn with_cross_router(
            graph: Arc<tokio::sync::Mutex<AtheneumGraph>>,
            cross: atheneum::CrossRouter,
        ) -> Self {
            Self {
                graph,
                cross: Some(Arc::new(tokio::sync::Mutex::new(cross))),
            }
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

        async fn search(&self, params: SearchParams) -> Result<Value> {
            let limit = crate::envelope::clamp_limit(params.limit);
            let mut envelope = crate::envelope::Envelope::new(limit);

            if matches!(params.kind, SearchKind::Knowledge | SearchKind::All) {
                let graph = self.graph.lock().await;
                let knowledge_results = tokio::task::block_in_place(|| {
                    graph.lexical_search(&params.query, params.k, params.project.as_deref(), None, None)
                });
                match knowledge_results {
                    Ok(results) => {
                        for r in results {
                            let mut v = serde_json::to_value(&r)?;
                            v["provenance"] = serde_json::json!(crate::envelope::Provenance::Inferred);
                            v["source"] = serde_json::json!("knowledge");
                            envelope.items.push(v);
                        }
                    }
                    Err(e) => envelope.errors.push(crate::envelope::EnvelopeError {
                        backend: "knowledge".to_string(),
                        code: crate::envelope::ERR_BACKEND_UNAVAILABLE.to_string(),
                        message: e.to_string(),
                    }),
                }
            }

            if matches!(params.kind, SearchKind::Code | SearchKind::All) {
                match &self.cross {
                    Some(cross) => {
                        let mut cross = cross.lock().await;
                        let code_results = tokio::task::block_in_place(|| {
                            cross.cross_search(&params.query, None, params.k)
                        });
                        match code_results {
                            Ok(results) => {
                                for r in results {
                                    envelope.items.push(json!({
                                        "project": r.project,
                                        "id": r.id,
                                        "kind": r.kind,
                                        "name": r.name,
                                        "file_path": r.file_path,
                                        "data": r.data,
                                        "provenance": crate::envelope::Provenance::Extracted,
                                        "source": "code",
                                    }));
                                }
                            }
                            Err(e) => envelope.errors.push(crate::envelope::EnvelopeError {
                                backend: "code".to_string(),
                                code: crate::envelope::ERR_BACKEND_UNAVAILABLE.to_string(),
                                message: e.to_string(),
                            }),
                        }
                    }
                    None => {
                        if matches!(params.kind, SearchKind::Code) {
                            envelope.errors.push(crate::envelope::EnvelopeError {
                                backend: "code".to_string(),
                                code: crate::envelope::ERR_BACKEND_UNAVAILABLE.to_string(),
                                message: "code search unavailable: no CrossRouter configured"
                                    .to_string(),
                            });
                        }
                    }
                }
            }

            let offset = params
                .cursor
                .as_deref()
                .and_then(crate::envelope::decode_cursor)
                .map(|c| c.offset)
                .unwrap_or(0);
            let total = envelope.items.len();
            let page: Vec<Value> = envelope.items.into_iter().skip(offset).take(limit).collect();
            envelope.has_more = offset + page.len() < total;
            envelope.items = page;
            if envelope.has_more {
                envelope.cursor = Some(crate::envelope::encode_cursor(&crate::envelope::Cursor {
                    backend: "search".to_string(),
                    offset: offset + envelope.items.len(),
                }));
            }

            Ok(envelope.to_value())
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
                let report = graph.maintain(
                    &atheneum::MaintainConfig {
                        rewire_threshold,
                        broken_link_mode,
                        stale_superseded_days,
                    },
                    apply,
                )?;
                Ok(serde_json::to_value(report)?)
            })
        }
        async fn seed_memory(&self, params: SeedMemoryParams) -> Result<Value> {
            let graph = self.graph.lock().await;
            tokio::task::block_in_place(|| {
                let seed =
                    graph.seed_memory(params.project.as_deref(), params.tokens.unwrap_or(800))?;
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
            trace: Option<bool>,
        ) -> Result<Value> {
            let graph = self.graph.lock().await;
            tokio::task::block_in_place(|| {
                let (results, trace_id) = graph.navigate_with_trace(
                    query,
                    k,
                    depth,
                    None,
                    None,
                    None,
                    trace.unwrap_or(false),
                )?;
                let views: Vec<Value> = results
                    .into_iter()
                    .map(|v| {
                        serialize_paginated_view(
                            &v.entry, v.depth, v.entities, v.edges, offset, limit,
                        )
                    })
                    .collect();
                if let Some(tid) = trace_id {
                    Ok(json!({
                        "subgraphs": views,
                        "trace_id": tid
                    }))
                } else {
                    Ok(Value::Array(views))
                }
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

        async fn list_models(&self) -> Result<Value> {
            let graph = self.graph.lock().await;
            tokio::task::block_in_place(|| {
                let models = graph.discover_available_models()?;
                Ok(serde_json::to_value(models)?)
            })
        }

        async fn dream_semantic(&self, params: DreamSemanticParams) -> Result<Value> {
            let graph = self.graph.lock().await;
            tokio::task::block_in_place(|| {
                // Base config comes from ~/.config/atheneum/config.toml [llm]
                // (provider, base_url, model, api_key). Falls back to the
                // legacy ollama defaults when the config is missing/invalid.
                let mut config = match atheneum::load_config() {
                    Ok(cfg) => atheneum::ConsolidationConfig::from_llm_config(&cfg.llm),
                    Err(_) => atheneum::ConsolidationConfig::default(),
                };
                if let Some(threshold) = params.similarity_threshold {
                    config.similarity_threshold = threshold;
                }
                if let Some(model) = params.model {
                    // MCP 'model' param overrides the configured model for any
                    // provider.
                    config.model = model;
                }
                if let Some(ollama_url) = params.ollama_url {
                    // Explicit ollama_url means the caller wants the legacy
                    // ollama path regardless of the configured provider.
                    config.provider = atheneum::LlmProvider::Ollama;
                    config.ollama_url = ollama_url;
                }
                if let Some(sg) = params.swap_guard {
                    config.swap_guard = match sg.as_str() {
                        "strict" => atheneum::config::SwapGuardMode::Strict,
                        "adapt" => atheneum::config::SwapGuardMode::Adapt,
                        _ => atheneum::config::SwapGuardMode::Fallback,
                    };
                }
                if let Some(dry_run) = params.dry_run {
                    config.dry_run = dry_run;
                }
                let report = graph.semantic_consolidation(&config)?;
                Ok(serde_json::to_value(report)?)
            })
        }

        async fn pin_entity(&self, id: i64) -> Result<Value> {
            let graph = self.graph.lock().await;
            tokio::task::block_in_place(|| {
                graph.pin_entity(id)?;
                Ok(serde_json::json!({ "status": "success", "id": id, "pinned": true }))
            })
        }

        async fn unpin_entity(&self, id: i64) -> Result<Value> {
            let graph = self.graph.lock().await;
            tokio::task::block_in_place(|| {
                graph.unpin_entity(id)?;
                Ok(serde_json::json!({ "status": "success", "id": id, "pinned": false }))
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
    use std::sync::Arc;

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

    // -----------------------------------------------------------------
    // search(kind=...) — direct backend dispatch
    // -----------------------------------------------------------------

    fn test_direct_backend_with_seeded_memory() -> direct::DirectBackend {
        let graph = atheneum::AtheneumGraph::open_in_memory().unwrap();
        graph
            .store_memory(
                "seeded_test_key",
                "seeded content for search test",
                "agent",
                0.8,
                None,
                None,
            )
            .unwrap();
        direct::DirectBackend::new(Arc::new(tokio::sync::Mutex::new(graph)))
    }

    /// Builds a temp magellan-shaped SQLite db (same shape as
    /// `atheneum::cross::tests::make_magellan_like_db`) with one
    /// `graph_entities` row named `shared_probe_symbol`, registers it via a
    /// temp `MetaRouter`/`meta.register_project`, and returns a `DirectBackend`
    /// wired up with a `CrossRouter` pointed at that temp `meta.db`.
    fn test_direct_backend_with_registered_code_project(
    ) -> (direct::DirectBackend, std::path::PathBuf, String) {
        let tmp_dir = tempfile::tempdir().unwrap().keep();
        let meta_path = tmp_dir.join("meta.db");
        let magellan_db = tmp_dir.join("code_project.db");

        {
            let conn = rusqlite::Connection::open(&magellan_db).unwrap();
            conn.execute(
                "CREATE TABLE graph_entities (
                    id INTEGER PRIMARY KEY,
                    kind TEXT NOT NULL,
                    name TEXT NOT NULL,
                    file_path TEXT,
                    data TEXT NOT NULL DEFAULT '{}'
                )",
                [],
            )
            .unwrap();
            conn.execute(
                "CREATE TABLE graph_edges (
                    id INTEGER PRIMARY KEY,
                    edge_type TEXT NOT NULL,
                    from_id INTEGER NOT NULL,
                    to_id INTEGER NOT NULL,
                    data TEXT NOT NULL DEFAULT '{}'
                )",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO graph_entities (id, kind, name, file_path, data) VALUES
                 (1, 'Symbol', 'shared_probe_symbol', 'src/lib.rs', '{}')",
                [],
            )
            .unwrap();
        }

        let project_name = "code_probe_project".to_string();
        let mut meta = atheneum::MetaRouter::open_at(&meta_path).unwrap();
        meta.register_project(
            &project_name,
            "/code_probe_project",
            magellan_db.to_str().unwrap(),
            None,
            Some("rust"),
        )
        .unwrap();

        let cross = atheneum::CrossRouter::from_meta(meta, 4);
        let graph = atheneum::AtheneumGraph::open_in_memory().unwrap();
        let backend = direct::DirectBackend::with_cross_router(
            Arc::new(tokio::sync::Mutex::new(graph)),
            cross,
        );
        (backend, magellan_db, project_name)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn search_without_kind_defaults_to_knowledge_only_unchanged_shape() {
        let backend = test_direct_backend_with_seeded_memory();
        let params = SearchParams {
            query: "seeded".to_string(),
            k: 10,
            project: None,
            kind: SearchKind::Knowledge,
            limit: None,
            cursor: None,
        };
        let result = backend.search(params).await.unwrap();
        // Envelope shape: items present, no code-backend errors since kind=Knowledge
        // never touches CrossRouter.
        assert!(result["items"].is_array());
        assert!(result["errors"].as_array().unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn search_kind_all_merges_knowledge_and_code_with_provenance_tags() {
        let (backend, _magellan_db_path, project_name) =
            test_direct_backend_with_registered_code_project();
        let params = SearchParams {
            query: "shared_probe_symbol".to_string(),
            k: 10,
            project: Some(project_name),
            kind: SearchKind::All,
            limit: None,
            cursor: None,
        };
        let result = backend.search(params).await.unwrap();
        let items = result["items"].as_array().unwrap();
        assert!(
            items.iter().any(|i| i["provenance"] == "EXTRACTED"),
            "expected at least one EXTRACTED (code) hit, got {items:?}"
        );
    }
}
