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

/// Parameters for the unified `navigate` tool.
#[derive(Debug, Clone)]
pub struct NavigateParams {
    pub query: String,
    pub k: usize,
    pub depth: Option<u32>,
    pub offset: usize,
    pub limit: usize,
    pub trace: Option<bool>,
    pub kind: SearchKind,
}

/// Parameters for the unified `code_query` tool.
#[derive(Debug, Clone)]
pub struct CodeQueryParams {
    pub project: String,
    pub tool: String,
    pub subcommand: String,
    pub args: Vec<String>,
}

/// Parameters for the `event` tool (envoy multi-agent coordination passthrough).
#[derive(Debug, Clone)]
pub struct EventParams {
    pub verb: String,
    pub payload: Value,
}

/// Parameters for the `refresh` tool — triggers `magellan refresh` for a
/// resolved project. llmgrep/mirage need no separate refresh call since
/// they read magellan's own db.
#[derive(Debug, Clone)]
pub struct RefreshParams {
    pub project: String,
    pub refresh_code: bool,
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
    async fn navigate(&self, params: NavigateParams) -> Result<Value>;
    async fn code_query(&self, params: CodeQueryParams) -> Result<Value>;
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
    async fn event(&self, params: EventParams) -> Result<Value>;
    async fn refresh(&self, params: RefreshParams) -> Result<Value>;
}

/// Shared implementation for the `event` tool — identical for
/// [`http::HttpBackend`] and [`direct::DirectBackend`] since it's a
/// separate envoy connection, independent of which atheneum backend mode
/// is active.
async fn event_impl(params: EventParams) -> Result<Value> {
    let mut envelope = crate::envelope::Envelope::new(1);
    let Some(verb) = crate::events::EnvoyVerb::from_str(&params.verb) else {
        envelope.errors.push(crate::envelope::EnvelopeError {
            backend: "event".to_string(),
            code: crate::envelope::ERR_PARSE_ERROR.to_string(),
            message: format!(
                "unknown event verb '{}', expected send|claim|heartbeat|create_dependency",
                params.verb
            ),
        });
        return Ok(envelope.to_value());
    };
    let base_url =
        std::env::var("ENVOY_URL").unwrap_or_else(|_| "http://localhost:9876".to_string());
    let client = crate::events::EnvoyClient::new(base_url);
    match client.call(verb, params.payload).await {
        Ok(v) => envelope.items.push(v),
        Err(e) => envelope.errors.push(crate::envelope::EnvelopeError {
            backend: "event".to_string(),
            code: if e.to_string().contains("timed out") {
                crate::envelope::ERR_TIMEOUT.to_string()
            } else {
                crate::envelope::ERR_BACKEND_UNAVAILABLE.to_string()
            },
            message: e.to_string(),
        }),
    }
    Ok(envelope.to_value())
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
            let mut path = format!(
                "/atheneum/search?q={}&k={}",
                encode(&params.query),
                params.k
            );
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

        async fn navigate(&self, params: NavigateParams) -> Result<Value> {
            // NOTE: HTTP backend does not yet support kind — forwards
            // query/k/depth/offset/limit only, same as before this task.
            // Tracked as a gap, not silently dropped.
            let depth = params.depth.unwrap_or(crate::envelope::DEFAULT_DEPTH);
            let path = format!(
                "/atheneum/graph/navigate?q={}&k={}&depth={depth}&offset={}&limit={}",
                encode(&params.query),
                params.k,
                params.offset,
                params.limit
            );
            self.get_json(&path).await
        }

        async fn code_query(&self, _params: CodeQueryParams) -> Result<Value> {
            Err(anyhow::anyhow!(
                "code_query not supported over HTTP backend"
            ))
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

        async fn event(&self, params: EventParams) -> Result<Value> {
            super::event_impl(params).await
        }

        async fn refresh(&self, _params: RefreshParams) -> Result<Value> {
            Err(anyhow::anyhow!("refresh not supported over HTTP backend"))
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
        // ponytail: test-only override so code_query's subprocess spawn can be
        // made to deterministically fail (bogus bin_dir) without depending on
        // whether magellan/llmgrep/mirage happen to be on the real PATH.
        code_bin_dir: Option<std::path::PathBuf>,
    }

    impl DirectBackend {
        pub fn new(graph: Arc<tokio::sync::Mutex<AtheneumGraph>>) -> Self {
            Self {
                graph,
                cross: None,
                code_bin_dir: None,
            }
        }

        pub fn with_cross_router(
            graph: Arc<tokio::sync::Mutex<AtheneumGraph>>,
            cross: atheneum::CrossRouter,
        ) -> Self {
            Self {
                graph,
                cross: Some(Arc::new(tokio::sync::Mutex::new(cross))),
                code_bin_dir: None,
            }
        }

        #[cfg(test)]
        pub fn with_code_bin_dir(mut self, dir: std::path::PathBuf) -> Self {
            self.code_bin_dir = Some(dir);
            self
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
            // Compatibility: today's callers pass no kind/limit/cursor and expect
            // the bare array lexical_search has always returned — not the new
            // envelope shape. Only opt into the envelope when the caller asks
            // for something the old shape can't express (code fan-out or
            // pagination). Errors propagate as Err here, exactly like before
            // this task, instead of being swallowed into envelope.errors.
            if params.kind == SearchKind::Knowledge
                && params.limit.is_none()
                && params.cursor.is_none()
            {
                let graph = self.graph.lock().await;
                return tokio::task::block_in_place(|| {
                    let results = graph.lexical_search(
                        &params.query,
                        params.k,
                        params.project.as_deref(),
                        None,
                        None,
                    )?;
                    Ok(serde_json::to_value(results)?)
                });
            }

            let limit = crate::envelope::clamp_limit(params.limit);
            let mut envelope = crate::envelope::Envelope::new(limit);

            if matches!(params.kind, SearchKind::Knowledge | SearchKind::All) {
                let graph = self.graph.lock().await;
                let knowledge_results = tokio::task::block_in_place(|| {
                    graph.lexical_search(
                        &params.query,
                        params.k,
                        params.project.as_deref(),
                        None,
                        None,
                    )
                });
                match knowledge_results {
                    Ok(results) => {
                        for r in results {
                            let mut v = serde_json::to_value(&r)?;
                            v["provenance"] =
                                serde_json::json!(crate::envelope::Provenance::Inferred);
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
                                // Finding 5: cross_search fans out across every
                                // attached project (it has no per-project
                                // scoping of its own). When the caller passed
                                // `project`, scope the *results* down to that
                                // project here — otherwise `code_stale` below
                                // (which IS scoped to `params.project`) would
                                // describe a single project while `items`
                                // silently contained hits from every project.
                                let results: Vec<_> = match params.project.as_deref() {
                                    Some(p) => {
                                        results.into_iter().filter(|r| r.project == p).collect()
                                    }
                                    None => results,
                                };
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
                        // Finding 3: push this for `All` too, not just `Code`
                        // — a `kind=all` caller must see the code backend's
                        // unavailability in `errors[]`, not a clean-looking
                        // response that's silently missing an entire source.
                        envelope.errors.push(crate::envelope::EnvelopeError {
                            backend: "code".to_string(),
                            code: crate::envelope::ERR_BACKEND_UNAVAILABLE.to_string(),
                            message: "code search unavailable: no CrossRouter configured"
                                .to_string(),
                        });
                    }
                }

                // code_stale: cheap magellan-side check, only meaningful when
                // a single project was actually resolved (cross_search above
                // fans out across every attached project, so there's no
                // single index to report on unless the caller scoped the
                // call with `project`). None means "not applicable to this
                // call", not "checked and clean".
                if let (Some(cross), Some(project_name)) = (&self.cross, params.project.as_deref())
                {
                    let magellan_db = {
                        let cross = cross.lock().await;
                        cross
                            .meta()
                            .get_project(project_name)
                            .ok()
                            .flatten()
                            .map(|p| p.magellan_db)
                    };
                    if let Some(magellan_db) = magellan_db {
                        let runner = match &self.code_bin_dir {
                            Some(dir) => {
                                crate::subprocess::CodeQueryRunner::with_bin_dir(dir.clone())
                            }
                            None => crate::subprocess::CodeQueryRunner::new(),
                        };
                        envelope.code_stale = runner.is_code_index_stale(&magellan_db).await.ok();
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
            let page: Vec<Value> = envelope
                .items
                .into_iter()
                .skip(offset)
                .take(limit)
                .collect();
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

        async fn navigate(&self, params: NavigateParams) -> Result<Value> {
            // Depth is always clamped for traversal safety, regardless of kind
            // (an unbounded BFS depth is a real cost even on the knowledge-only
            // path). Whether that clamp is *reported* to the caller differs by
            // shape below.
            let (depth, depth_clamped) = crate::envelope::clamp_depth(params.depth);

            if params.kind == SearchKind::Knowledge {
                // Compatibility: today's callers pass no kind (or kind=knowledge
                // explicitly) and expect the exact pre-existing
                // navigate_with_trace + serialize_paginated_view shape — a bare
                // array, or {subgraphs, trace_id} when trace=true. No envelope,
                // no provenance/source tags, no depth_clamped field: changing
                // this shape is exactly what's forbidden for the default path.
                let graph = self.graph.lock().await;
                return tokio::task::block_in_place(|| {
                    let (results, trace_id) = graph.navigate_with_trace(
                        &params.query,
                        params.k,
                        depth,
                        None,
                        None,
                        None,
                        params.trace.unwrap_or(false),
                    )?;
                    let views: Vec<Value> = results
                        .into_iter()
                        .map(|v| {
                            serialize_paginated_view(
                                &v.entry,
                                v.depth,
                                v.entities,
                                v.edges,
                                params.offset,
                                params.limit,
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
                });
            }

            // kind == Code | All: enveloped/fan-out shape.
            let mut envelope = crate::envelope::Envelope::new(params.limit.max(1));
            envelope.depth_clamped = depth_clamped;

            if matches!(params.kind, SearchKind::Knowledge | SearchKind::All) {
                let graph = self.graph.lock().await;
                let knowledge_result = tokio::task::block_in_place(|| {
                    graph.navigate_with_trace(
                        &params.query,
                        params.k,
                        depth,
                        None,
                        None,
                        None,
                        params.trace.unwrap_or(false),
                    )
                });
                match knowledge_result {
                    Ok((results, trace_id)) => {
                        for v in results {
                            let mut view = serialize_paginated_view(
                                &v.entry,
                                v.depth,
                                v.entities,
                                v.edges,
                                params.offset,
                                params.limit,
                            );
                            view["provenance"] = serde_json::json!(if v.depth <= 1 {
                                crate::envelope::Provenance::Inferred
                            } else {
                                crate::envelope::Provenance::Ambiguous
                            });
                            view["source"] = serde_json::json!("knowledge");
                            if let Some(tid) = &trace_id {
                                view["trace_id"] = serde_json::json!(tid);
                            }
                            envelope.items.push(view);
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
                        let code_result = tokio::task::block_in_place(|| {
                            cross.cross_navigate(&params.query, None, params.k, depth)
                        });
                        match code_result {
                            Ok(subgraphs) => {
                                for sg in subgraphs {
                                    envelope.items.push(json!({
                                        "project": sg.project,
                                        "entry_id": sg.entry_id,
                                        "entity_count": sg.entities.len(),
                                        "edge_count": sg.edges.len(),
                                        "entities": sg.entities.iter().map(|e| json!({
                                            "id": e.id, "kind": e.kind, "name": e.name, "file_path": e.file_path,
                                        })).collect::<Vec<_>>(),
                                        "provenance": if depth <= 1 { crate::envelope::Provenance::Extracted } else { crate::envelope::Provenance::Ambiguous },
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
                        // Finding 3: push this for `All` too, not just `Code`
                        // — a `kind=all` caller must see the code backend's
                        // unavailability in `errors[]`, not a clean-looking
                        // response that's silently missing an entire source.
                        envelope.errors.push(crate::envelope::EnvelopeError {
                            backend: "code".to_string(),
                            code: crate::envelope::ERR_BACKEND_UNAVAILABLE.to_string(),
                            message: "code navigate unavailable: no CrossRouter configured"
                                .to_string(),
                        });
                    }
                }

                // code_stale intentionally left None here: unlike `search`,
                // `NavigateParams` carries no `project` field to resolve a
                // single magellan db against — cross_navigate fans out across
                // every attached project (each subgraph in `envelope.items`
                // can belong to a different project), so there is no single
                // index whose staleness a bool could represent. Adding a
                // `project` scope to navigate is out of this task's scope.
            }

            Ok(envelope.to_value())
        }

        async fn code_query(&self, params: CodeQueryParams) -> Result<Value> {
            let mut envelope = crate::envelope::Envelope::new(1);
            let Some(cross) = &self.cross else {
                envelope.errors.push(crate::envelope::EnvelopeError {
                    backend: "code".to_string(),
                    code: crate::envelope::ERR_BACKEND_UNAVAILABLE.to_string(),
                    message: "code_query unavailable: no CrossRouter configured".to_string(),
                });
                return Ok(envelope.to_value());
            };

            let magellan_db = {
                let cross = cross.lock().await;
                match cross.meta().get_project(&params.project) {
                    Ok(Some(project)) => project.magellan_db,
                    Ok(None) => {
                        envelope.errors.push(crate::envelope::EnvelopeError {
                            backend: "code".to_string(),
                            code: crate::envelope::ERR_PROJECT_NOT_FOUND.to_string(),
                            message: format!("project '{}' not found in meta.db", params.project),
                        });
                        return Ok(envelope.to_value());
                    }
                    Err(e) => {
                        envelope.errors.push(crate::envelope::EnvelopeError {
                            backend: "code".to_string(),
                            code: crate::envelope::ERR_BACKEND_UNAVAILABLE.to_string(),
                            message: e.to_string(),
                        });
                        return Ok(envelope.to_value());
                    }
                }
            };

            let tool = match params.tool.as_str() {
                "magellan" => crate::subprocess::CodeTool::Magellan,
                "llmgrep" => crate::subprocess::CodeTool::Llmgrep,
                "mirage" => crate::subprocess::CodeTool::Mirage,
                other => {
                    envelope.errors.push(crate::envelope::EnvelopeError {
                        backend: "code".to_string(),
                        code: crate::envelope::ERR_PARSE_ERROR.to_string(),
                        message: format!(
                            "unknown tool '{other}', expected magellan|llmgrep|mirage"
                        ),
                    });
                    return Ok(envelope.to_value());
                }
            };

            if !tool
                .allowed_subcommands()
                .contains(&params.subcommand.as_str())
            {
                envelope.errors.push(crate::envelope::EnvelopeError {
                    backend: "code".to_string(),
                    code: crate::envelope::ERR_PARSE_ERROR.to_string(),
                    message: format!(
                        "subcommand '{}' is not permitted via code_query; code_query is \
                         read-only — use the dedicated refresh tool to mutate the index",
                        params.subcommand
                    ),
                });
                return Ok(envelope.to_value());
            }

            // Blocks the long-form `--db`/`--db=<path>` clap convention used
            // by every subcommand, plus magellan's `score` subcommand, which
            // has its own hand-rolled parser accepting a bare `-d` shorthand
            // for the same flag (magellan/src/cli/parsers/score.rs:23). That
            // parser only recognizes `-d <path>` (next-token form, via
            // parse_required_arg) — never `-d=<path>` — so no `-d=` guard is
            // needed. No other magellan/llmgrep/mirage subcommand has a
            // short `-d` alias (checked across magellan's parsers).
            if params
                .args
                .iter()
                .any(|a| a == "--db" || a.starts_with("--db=") || a == "-d")
            {
                envelope.errors.push(crate::envelope::EnvelopeError {
                    backend: "code".to_string(),
                    code: crate::envelope::ERR_PARSE_ERROR.to_string(),
                    message: "args must not include --db; project is resolved server-side"
                        .to_string(),
                });
                return Ok(envelope.to_value());
            }

            let mut args = vec![
                params.subcommand.clone(),
                "--db".to_string(),
                magellan_db.clone(),
            ];
            args.extend(params.args.clone());
            let runner = match &self.code_bin_dir {
                Some(dir) => crate::subprocess::CodeQueryRunner::with_bin_dir(dir.clone()),
                None => crate::subprocess::CodeQueryRunner::new(),
            };
            match runner.run(&magellan_db, tool, args).await {
                Ok(value) => {
                    let tagged = match value {
                        Value::Object(mut map) => {
                            map.insert(
                                "provenance".to_string(),
                                serde_json::json!(crate::envelope::Provenance::Extracted),
                            );
                            map.insert("source".to_string(), serde_json::json!("code"));
                            Value::Object(map)
                        }
                        value => json!({
                            "value": value,
                            "provenance": crate::envelope::Provenance::Extracted,
                            "source": "code",
                        }),
                    };
                    envelope.items.push(tagged);
                }
                Err(e) => {
                    let message = e.to_string();
                    envelope.errors.push(crate::envelope::EnvelopeError {
                        backend: "code".to_string(),
                        code: if message.contains("timed out") {
                            crate::envelope::ERR_TIMEOUT.to_string()
                        } else {
                            crate::envelope::ERR_BACKEND_UNAVAILABLE.to_string()
                        },
                        message,
                    });
                }
            }

            Ok(envelope.to_value())
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

        async fn event(&self, params: EventParams) -> Result<Value> {
            super::event_impl(params).await
        }

        async fn refresh(&self, params: RefreshParams) -> Result<Value> {
            let mut envelope = crate::envelope::Envelope::new(1);
            let Some(cross) = &self.cross else {
                envelope.errors.push(crate::envelope::EnvelopeError {
                    backend: "code".to_string(),
                    code: crate::envelope::ERR_BACKEND_UNAVAILABLE.to_string(),
                    message: "refresh unavailable: no CrossRouter configured".to_string(),
                });
                return Ok(envelope.to_value());
            };

            let magellan_db = {
                let cross = cross.lock().await;
                match cross.meta().get_project(&params.project) {
                    Ok(Some(project)) => project.magellan_db,
                    Ok(None) => {
                        envelope.errors.push(crate::envelope::EnvelopeError {
                            backend: "code".to_string(),
                            code: crate::envelope::ERR_PROJECT_NOT_FOUND.to_string(),
                            message: format!("project '{}' not found in meta.db", params.project),
                        });
                        return Ok(envelope.to_value());
                    }
                    Err(e) => {
                        envelope.errors.push(crate::envelope::EnvelopeError {
                            backend: "code".to_string(),
                            code: crate::envelope::ERR_BACKEND_UNAVAILABLE.to_string(),
                            message: e.to_string(),
                        });
                        return Ok(envelope.to_value());
                    }
                }
            };

            if !params.refresh_code {
                envelope.items.push(json!({
                    "project": params.project,
                    "refreshed": false,
                }));
                return Ok(envelope.to_value());
            }

            let runner = match &self.code_bin_dir {
                Some(dir) => crate::subprocess::CodeQueryRunner::with_bin_dir(dir.clone()),
                None => crate::subprocess::CodeQueryRunner::new(),
            };
            let args = vec![
                "refresh".to_string(),
                "--db".to_string(),
                magellan_db.clone(),
            ];
            match runner
                .run(&magellan_db, crate::subprocess::CodeTool::Magellan, args)
                .await
            {
                Ok(value) => {
                    let tagged = match value {
                        Value::Object(mut map) => {
                            map.insert(
                                "provenance".to_string(),
                                serde_json::json!(crate::envelope::Provenance::Extracted),
                            );
                            map.insert("source".to_string(), serde_json::json!("code"));
                            Value::Object(map)
                        }
                        value => json!({
                            "value": value,
                            "provenance": crate::envelope::Provenance::Extracted,
                            "source": "code",
                        }),
                    };
                    envelope.items.push(tagged);
                }
                Err(e) => {
                    let message = e.to_string();
                    envelope.errors.push(crate::envelope::EnvelopeError {
                        backend: "code".to_string(),
                        code: if message.contains("timed out") {
                            crate::envelope::ERR_TIMEOUT.to_string()
                        } else {
                            crate::envelope::ERR_BACKEND_UNAVAILABLE.to_string()
                        },
                        message,
                    });
                }
            }

            Ok(envelope.to_value())
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

    // ponytail: ENVOY_URL is process-global env state; two tests now mutate
    // it (`event_impl_unreachable_envoy_returns_backend_unavailable_error`
    // and `event_connection_failure_surfaces_in_errors_not_as_panic_or_exception`),
    // and cargo's default test harness runs tests concurrently across
    // threads. This lock serializes just those two so neither observes the
    // other's env mutation mid-call. tokio::sync::Mutex (not std) because
    // the guard must stay held across the `.await` on the envoy call.
    // Upgrade to a per-call injected base_url (no global env var at all) if
    // a third test ever needs ENVOY_URL.
    static ENVOY_URL_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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

    /// Same seeded-memory knowledge graph as `test_direct_backend_with_seeded_memory`,
    /// wired to a `CrossRouter` whose meta.db has had its own project
    /// registry table (`project_overlay` — the table `MetaRouter::open_at`'s
    /// non-magellan-attached `list_projects()` queries, per
    /// `MetaRouter::projects_source`) dropped out from under it after setup.
    ///
    /// This is the only way to make `CrossRouter::cross_search` itself
    /// return `Err`: a bad *individual* project (missing db file,
    /// incompatible schema) is caught internally in `cross_search`'s loop
    /// (`ensure_attached`/prepare/query failures all `tracing::warn!` +
    /// `continue`, never surfaced as `cross_search`'s own `Err`) — only a
    /// failure of the `self.meta.list_projects()` call that seeds that loop
    /// propagates out. Simply registering zero projects makes `list_projects`
    /// return `Ok(vec![])`, not `Err`, which doesn't exercise the
    /// `envelope.errors.push(...)` arm for the code backend at all.
    fn test_direct_backend_with_seeded_memory_and_broken_cross_router() -> direct::DirectBackend {
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
        let tmp_dir = tempfile::tempdir().unwrap().keep();
        let meta_path = tmp_dir.join("meta.db");
        let meta = atheneum::MetaRouter::open_at(&meta_path).unwrap();
        {
            let conn = rusqlite::Connection::open(&meta_path).unwrap();
            conn.execute("DROP TABLE project_overlay", []).unwrap();
        }
        let cross = atheneum::CrossRouter::from_meta(meta, 4);
        direct::DirectBackend::with_cross_router(Arc::new(tokio::sync::Mutex::new(graph)), cross)
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
        // Seed a knowledge-side hit that overlaps the code-side query term, so
        // kind=all tests genuinely exercise a merge of both provenance types.
        graph
            .store_memory(
                "shared_probe_symbol_note",
                "note about shared_probe_symbol",
                "agent",
                0.8,
                Some(project_name.as_str()),
                None,
            )
            .unwrap();
        let backend = direct::DirectBackend::with_cross_router(
            Arc::new(tokio::sync::Mutex::new(graph)),
            cross,
        );
        (backend, magellan_db, project_name)
    }

    /// Two registered magellan-shaped projects, each with an entity sharing
    /// the *same* name (`cross_project_probe_symbol`) so an unscoped
    /// `cross_search` genuinely returns hits from both. Used to prove
    /// `search(project=...)` actually scopes code results down to one
    /// project (Finding 5) instead of returning every project's hits.
    fn test_direct_backend_with_two_registered_code_projects(
    ) -> (direct::DirectBackend, String, String) {
        let tmp_dir = tempfile::tempdir().unwrap().keep();
        let meta_path = tmp_dir.join("meta.db");
        let magellan_db_a = tmp_dir.join("project_a.db");
        let magellan_db_b = tmp_dir.join("project_b.db");

        let make_db = |path: &std::path::Path| {
            let conn = rusqlite::Connection::open(path).unwrap();
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
                 (1, 'Symbol', 'cross_project_probe_symbol', 'src/lib.rs', '{}')",
                [],
            )
            .unwrap();
        };
        make_db(&magellan_db_a);
        make_db(&magellan_db_b);

        let project_a = "project_a".to_string();
        let project_b = "project_b".to_string();
        let mut meta = atheneum::MetaRouter::open_at(&meta_path).unwrap();
        meta.register_project(
            &project_a,
            "/project_a",
            magellan_db_a.to_str().unwrap(),
            None,
            Some("rust"),
        )
        .unwrap();
        meta.register_project(
            &project_b,
            "/project_b",
            magellan_db_b.to_str().unwrap(),
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
        (backend, project_a, project_b)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn search_kind_all_with_project_scopes_code_results_to_that_project() {
        let (backend, project_a, project_b) =
            test_direct_backend_with_two_registered_code_projects();
        let params = SearchParams {
            query: "cross_project_probe_symbol".to_string(),
            k: 10,
            project: Some(project_a.clone()),
            kind: SearchKind::All,
            limit: None,
            cursor: None,
        };
        let result = backend.search(params).await.unwrap();
        let items = result["items"].as_array().unwrap();
        assert!(
            !items.is_empty(),
            "expected at least the scoped project's hit, got {items:?}"
        );
        assert!(
            items.iter().all(|i| i["project"] != project_b),
            "search(project={project_a:?}) leaked a hit from unscoped project \
             {project_b:?}, got {items:?}"
        );
        assert!(
            items
                .iter()
                .any(|i| i["project"] == project_a && i["provenance"] == "EXTRACTED"),
            "expected the scoped project's own hit to survive filtering, got {items:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn search_kind_all_without_project_returns_hits_from_every_project() {
        // Companion to the scoped test above: no `project` means the
        // existing unscoped (searches-all-projects) behavior is preserved.
        let (backend, project_a, project_b) =
            test_direct_backend_with_two_registered_code_projects();
        let params = SearchParams {
            query: "cross_project_probe_symbol".to_string(),
            k: 10,
            project: None,
            kind: SearchKind::All,
            limit: None,
            cursor: None,
        };
        let result = backend.search(params).await.unwrap();
        let items = result["items"].as_array().unwrap();
        assert!(
            items.iter().any(|i| i["project"] == project_a),
            "expected a hit from project_a, got {items:?}"
        );
        assert!(
            items.iter().any(|i| i["project"] == project_b),
            "expected a hit from project_b, got {items:?}"
        );
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
        // Compatibility requirement: no kind/limit/cursor (or kind=knowledge
        // explicitly) must return the exact bare array lexical_search has
        // always produced — not the new envelope object.
        assert!(
            result.is_array(),
            "expected bare array (today's shape), got {result:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn search_with_explicit_limit_returns_enveloped_shape() {
        let backend = test_direct_backend_with_seeded_memory();
        let params = SearchParams {
            query: "seeded".to_string(),
            k: 10,
            project: None,
            kind: SearchKind::Knowledge,
            limit: Some(5),
            cursor: None,
        };
        let result = backend.search(params).await.unwrap();
        // Explicit limit opts into the new envelope shape even with kind=knowledge.
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
        assert!(
            items.iter().any(|i| i["provenance"] == "INFERRED"),
            "expected at least one INFERRED (knowledge) hit, got {items:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn search_kind_all_partial_failure_returns_working_backend_results() {
        // meta.db's own project registry table is dropped out from under the
        // CrossRouter -> `cross_search`'s `list_projects()` call genuinely
        // returns `Err`, exercising the code-branch `envelope.errors.push`
        // arm in `search` (not just an empty-but-successful code result) —
        // and the seeded knowledge branch must still return its item.
        let backend = test_direct_backend_with_seeded_memory_and_broken_cross_router();
        let params = SearchParams {
            query: "seeded".to_string(),
            k: 10,
            project: None,
            kind: SearchKind::All,
            limit: None,
            cursor: None,
        };
        let result = backend.search(params).await.unwrap();
        assert!(
            !result["items"].as_array().unwrap().is_empty(),
            "knowledge results must survive a code-side failure, got {result:?}"
        );
        let errors = result["errors"].as_array().unwrap();
        assert!(
            errors
                .iter()
                .any(|e| e["backend"] == "code"
                    && e["code"] == crate::envelope::ERR_BACKEND_UNAVAILABLE),
            "expected a genuine code-backend ERR_BACKEND_UNAVAILABLE from the broken \
             meta.db, got {errors:?}"
        );
    }

    // -----------------------------------------------------------------
    // navigate(kind=...) — direct backend dispatch
    // -----------------------------------------------------------------

    #[tokio::test(flavor = "multi_thread")]
    async fn navigate_without_kind_defaults_to_knowledge_only_unchanged_shape() {
        let backend = test_direct_backend_with_seeded_memory();
        let params = NavigateParams {
            query: "seeded".to_string(),
            k: 5,
            depth: None,
            offset: 0,
            limit: 20,
            trace: None,
            kind: SearchKind::Knowledge,
        };
        let result = backend.navigate(params).await.unwrap();
        // Compatibility requirement: no kind (or kind=knowledge explicitly), no
        // trace, must return the exact pre-existing bare array
        // navigate_with_trace + serialize_paginated_view has always produced —
        // not the new envelope object, and no depth_clamped field anywhere.
        assert!(
            result.is_array(),
            "expected bare array (today's shape), got {result:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn navigate_default_path_traversal_is_bounded_by_clamped_depth() {
        // Proves the *traversal itself* uses the clamped depth (3) on the
        // default kind=Knowledge path, not the raw requested depth (10) —
        // the compatibility-shape test above only proves the response shape
        // is unchanged, not that clamp_depth's result actually reaches
        // navigate_with_trace through the early-return branch.
        let graph = atheneum::AtheneumGraph::open_in_memory().unwrap();
        // Query token must be unique to the root entity — lexical_search
        // scores by fraction-of-tokens-matched, so a shared substring (e.g.
        // "chain") between root and hop names would make every hop its own
        // extra entry point and hide an unclamped traversal behind a false
        // positive. "zqx7rootanchor" appears nowhere else in this fixture.
        let root_id = graph
            .store_memory(
                "zqx7rootanchor",
                "zqx7rootanchor unique entry point for depth clamp chain test",
                "agent",
                0.8,
                None,
                None,
            )
            .unwrap();
        // Chain: root -[related_to]-> hop_1 -> hop_2 -> hop_3 -> hop_4.
        // hop_3 is 3 hops out (within the clamp), hop_4 is 4 hops out
        // (beyond it) and must not appear if the clamp is actually applied.
        let mut prev_id = root_id;
        for hop in 1..=4 {
            let id = graph
                .upsert_concept(&format!("unrelated_node_{hop}"), &serde_json::json!({}))
                .unwrap();
            graph
                .insert_edge(
                    prev_id,
                    id,
                    atheneum::EdgeType::RelatedTo,
                    serde_json::json!({}),
                )
                .unwrap();
            prev_id = id;
        }
        let backend = direct::DirectBackend::new(Arc::new(tokio::sync::Mutex::new(graph)));
        let params = NavigateParams {
            query: "zqx7rootanchor".to_string(),
            k: 5,
            depth: Some(10),
            offset: 0,
            limit: 50,
            trace: None,
            kind: SearchKind::Knowledge,
        };
        let result = backend.navigate(params).await.unwrap();
        let names: Vec<String> = result
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|view| view["entities"].as_array().unwrap().iter())
            .filter_map(|e| e["name"].as_str().map(str::to_string))
            .collect();
        assert!(
            names.contains(&"unrelated_node_3".to_string()),
            "expected hop_3 (3 hops, within clamp) to be reachable, got {names:?}"
        );
        assert!(
            !names.contains(&"unrelated_node_4".to_string()),
            "expected traversal to be capped at depth 3 (clamp_depth's result), \
             but hop_4 (4 hops from entry) was reached — the raw requested \
             depth (10) leaked into navigate_with_trace: {names:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn navigate_depth_beyond_max_is_clamped_and_flagged() {
        let backend = test_direct_backend_with_seeded_memory();
        let params = NavigateParams {
            query: "seeded".to_string(),
            k: 5,
            depth: Some(10),
            offset: 0,
            limit: 20,
            trace: None,
            kind: SearchKind::All,
        };
        let result = backend.navigate(params).await.unwrap();
        assert_eq!(result["depth_clamped"], true);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn navigate_kind_all_tags_code_hits_ambiguous_beyond_first_hop() {
        let (backend, _db_path, project_name) = test_direct_backend_with_registered_code_project();
        let params = NavigateParams {
            query: "shared_probe_symbol".to_string(),
            k: 5,
            depth: Some(2),
            offset: 0,
            limit: 20,
            trace: None,
            kind: SearchKind::All,
        };
        let _ = project_name; // cross_navigate in this router searches all registered projects
        let result = backend.navigate(params).await.unwrap();
        let items = result["items"].as_array().unwrap();
        assert!(
            items.iter().any(|i| i["source"] == "code"),
            "expected at least one code-sourced navigate item, got {items:?}"
        );
    }

    // -----------------------------------------------------------------
    // code_query — DirectBackend dispatch logic
    // -----------------------------------------------------------------

    #[tokio::test(flavor = "multi_thread")]
    async fn code_query_unknown_project_returns_project_not_found() {
        let (backend, _db_path, _project_name) = test_direct_backend_with_registered_code_project();
        let params = CodeQueryParams {
            project: "totally-unknown-project-xyz".to_string(),
            tool: "magellan".to_string(),
            subcommand: "status".to_string(),
            args: vec![],
        };
        let result = backend.code_query(params).await.unwrap();
        let errors = result["errors"].as_array().unwrap();
        assert!(
            errors
                .iter()
                .any(|e| e["code"] == crate::envelope::ERR_PROJECT_NOT_FOUND),
            "expected ERR_PROJECT_NOT_FOUND, got {errors:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn code_query_unknown_project_returns_project_not_found_not_panic() {
        let backend = test_direct_backend_with_registered_code_project().0;
        let params = CodeQueryParams {
            project: "definitely-not-a-registered-project".to_string(),
            tool: "magellan".to_string(),
            subcommand: "status".to_string(),
            args: vec![],
        };
        let result = backend.code_query(params).await.unwrap();
        assert_eq!(
            result["errors"][0]["code"],
            crate::envelope::ERR_PROJECT_NOT_FOUND
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn code_query_unknown_tool_returns_parse_error() {
        let (backend, _db_path, project_name) = test_direct_backend_with_registered_code_project();
        let params = CodeQueryParams {
            project: project_name,
            tool: "not-a-real-tool".to_string(),
            subcommand: "status".to_string(),
            args: vec![],
        };
        let result = backend.code_query(params).await.unwrap();
        let errors = result["errors"].as_array().unwrap();
        assert!(
            errors
                .iter()
                .any(|e| e["code"] == crate::envelope::ERR_PARSE_ERROR),
            "expected ERR_PARSE_ERROR, got {errors:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn code_query_mutating_subcommand_is_rejected_before_spawning() {
        // `index` writes to the magellan db — code_query is supposed to be
        // read-only (Finding 6). Using a bogus bin_dir too: if the allowlist
        // check were somehow skipped, the subprocess would still fail to
        // spawn, but we want to prove *this* is what caught it (PARSE_ERROR,
        // not BACKEND_UNAVAILABLE), and that no subprocess was even attempted.
        let (backend, _db_path, project_name) = test_direct_backend_with_registered_code_project();
        let backend = backend.with_code_bin_dir(std::path::PathBuf::from("/nonexistent-bin-dir"));
        let params = CodeQueryParams {
            project: project_name,
            tool: "magellan".to_string(),
            subcommand: "index".to_string(),
            args: vec![],
        };
        let result = backend.code_query(params).await.unwrap();
        let errors = result["errors"].as_array().unwrap();
        assert!(
            errors
                .iter()
                .any(|e| e["code"] == crate::envelope::ERR_PARSE_ERROR),
            "expected ERR_PARSE_ERROR for mutating subcommand 'index', got {errors:?}"
        );
        assert!(
            result["items"].as_array().unwrap().is_empty(),
            "rejected subcommand must not reach the subprocess and produce an item"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn code_query_args_db_override_is_rejected_before_spawning() {
        let (backend, _db_path, project_name) = test_direct_backend_with_registered_code_project();
        let params = CodeQueryParams {
            project: project_name,
            tool: "magellan".to_string(),
            subcommand: "status".to_string(),
            args: vec!["--db".to_string(), "/etc/passwd".to_string()],
        };
        let result = backend.code_query(params).await.unwrap();
        let errors = result["errors"].as_array().unwrap();
        assert!(
            errors
                .iter()
                .any(|e| e["code"] == crate::envelope::ERR_PARSE_ERROR),
            "expected ERR_PARSE_ERROR for --db override attempt, got {errors:?}"
        );
        // Rejected before reaching the subprocess — no items should have
        // been produced (a real spawn would either error differently or
        // succeed and push an item).
        assert!(result["items"].as_array().unwrap().is_empty());

        // Same rejection for the `--db=<path>` spelling.
        let (backend2, _db_path2, project_name2) =
            test_direct_backend_with_registered_code_project();
        let params2 = CodeQueryParams {
            project: project_name2,
            tool: "magellan".to_string(),
            subcommand: "status".to_string(),
            args: vec!["--db=/etc/passwd".to_string()],
        };
        let result2 = backend2.code_query(params2).await.unwrap();
        let errors2 = result2["errors"].as_array().unwrap();
        assert!(
            errors2
                .iter()
                .any(|e| e["code"] == crate::envelope::ERR_PARSE_ERROR),
            "expected ERR_PARSE_ERROR for --db=<path> override attempt, got {errors2:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn code_query_short_flag_db_override_on_score_subcommand_is_rejected() {
        // magellan's `score` subcommand has its own hand-rolled arg parser
        // (magellan/src/cli/parsers/score.rs) that accepts a bare `-d`
        // shorthand for `--db`, unlike every other subcommand. Confirms the
        // args-scan blocks this bypass too, not just the long-form flag.
        let (backend, _db_path, project_name) = test_direct_backend_with_registered_code_project();
        let params = CodeQueryParams {
            project: project_name,
            tool: "magellan".to_string(),
            subcommand: "score".to_string(),
            args: vec!["-d".to_string(), "/tmp/whatever".to_string()],
        };
        let result = backend.code_query(params).await.unwrap();
        let errors = result["errors"].as_array().unwrap();
        assert!(
            errors
                .iter()
                .any(|e| e["code"] == crate::envelope::ERR_PARSE_ERROR),
            "expected ERR_PARSE_ERROR for -d override attempt, got {errors:?}"
        );
        assert!(result["items"].as_array().unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn code_query_subprocess_spawn_failure_maps_to_backend_unavailable() {
        let (backend, _db_path, project_name) = test_direct_backend_with_registered_code_project();
        let backend = backend.with_code_bin_dir(std::path::PathBuf::from("/nonexistent-bin-dir"));
        let params = CodeQueryParams {
            project: project_name,
            tool: "magellan".to_string(),
            subcommand: "status".to_string(),
            args: vec![],
        };
        let result = backend.code_query(params).await.unwrap();
        let errors = result["errors"].as_array().unwrap();
        assert!(
            errors
                .iter()
                .any(|e| e["code"] == crate::envelope::ERR_BACKEND_UNAVAILABLE),
            "expected ERR_BACKEND_UNAVAILABLE (not a bare Err) for spawn failure, got {errors:?}"
        );
        // The caller-visible message must not leak raw subprocess
        // stderr/stdout (Finding 1) — a spawn failure has none to leak
        // anyway, but assert the message stays terse regardless.
        let message = errors[0]["message"].as_str().unwrap();
        assert!(
            message.contains("magellan"),
            "expected message to name the failing tool, got {message:?}"
        );
    }

    #[tokio::test]
    async fn event_impl_unknown_verb_returns_parse_error() {
        let result = event_impl(EventParams {
            verb: "not-a-real-verb".to_string(),
            payload: serde_json::json!({}),
        })
        .await
        .unwrap();
        let errors = result["errors"].as_array().unwrap();
        assert!(
            errors
                .iter()
                .any(|e| e["code"] == crate::envelope::ERR_PARSE_ERROR),
            "expected ERR_PARSE_ERROR for unknown verb, got {errors:?}"
        );
        assert!(result["items"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn event_impl_unreachable_envoy_returns_backend_unavailable_error() {
        // ponytail: ENVOY_URL is process-global; guarded by ENVOY_URL_LOCK
        // (see its doc comment) against the sibling test below, which also
        // mutates it. Port 9 is the "discard" service — same
        // unreachable-envoy stand-in events.rs's own test uses.
        let _guard = ENVOY_URL_LOCK.lock().await;
        // SAFETY: env mutation serialized by ENVOY_URL_LOCK above.
        unsafe {
            std::env::set_var("ENVOY_URL", "http://127.0.0.1:9");
        }
        let result = event_impl(EventParams {
            verb: "heartbeat".to_string(),
            payload: serde_json::json!({"agent_id": "test"}),
        })
        .await
        .unwrap();
        unsafe {
            std::env::remove_var("ENVOY_URL");
        }
        let errors = result["errors"].as_array().unwrap();
        assert!(
            errors
                .iter()
                .any(|e| e["code"] == crate::envelope::ERR_BACKEND_UNAVAILABLE
                    || e["code"] == crate::envelope::ERR_TIMEOUT),
            "expected BACKEND_UNAVAILABLE or TIMEOUT for unreachable envoy, got {errors:?}"
        );
        assert!(result["items"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn event_connection_failure_surfaces_in_errors_not_as_panic_or_exception() {
        // Distinct from `event_impl_unreachable_envoy_returns_backend_unavailable_error`:
        // that test calls the free function `event_impl` directly. This one
        // goes through the actual `Backend::event()` trait method on a real
        // `DirectBackend` (the path the MCP server dispatches through),
        // proving the trait-level wiring also turns a connection failure
        // into Ok(envelope-with-errors), not just event_impl in isolation.
        let backend = test_direct_backend_with_seeded_memory();
        let _guard = ENVOY_URL_LOCK.lock().await;
        // Pinned to a guaranteed-unreachable address (see sibling test) —
        // left at the default, envoy commonly *is* running locally in this
        // dev environment, which would make this assert nothing.
        // SAFETY: env mutation serialized by ENVOY_URL_LOCK above.
        unsafe {
            std::env::set_var("ENVOY_URL", "http://127.0.0.1:9");
        }
        let params = EventParams {
            verb: "heartbeat".to_string(),
            payload: serde_json::json!({"agent_id": "test"}),
        };
        let result = backend.event(params).await;
        unsafe {
            std::env::remove_var("ENVOY_URL");
        }
        let result =
            result.expect("event() must never return a raw exception, always Ok(envelope)");
        let errors = result["errors"].as_array().unwrap();
        assert!(
            errors
                .iter()
                .any(|e| e["code"] == crate::envelope::ERR_BACKEND_UNAVAILABLE
                    || e["code"] == crate::envelope::ERR_TIMEOUT),
            "expected BACKEND_UNAVAILABLE or TIMEOUT for unreachable envoy via backend.event(), \
             got {errors:?}"
        );
        assert!(result["items"].as_array().unwrap().is_empty());
    }

    // -----------------------------------------------------------------
    // refresh — DirectBackend dispatch logic
    // -----------------------------------------------------------------

    /// Writes a fake `magellan` executable into a fresh temp dir that
    /// ignores its args and prints `json_stdout` — mirrors
    /// `subprocess::tests::fake_magellan_bin_dir`, duplicated here since
    /// that helper lives in a different module's private test mod.
    fn fake_magellan_bin_dir(json_stdout: &str) -> tempfile::TempDir {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let script_path = dir.path().join("magellan");
        let mut file = std::fs::File::create(&script_path).unwrap();
        writeln!(file, "#!/bin/sh").unwrap();
        writeln!(file, "cat <<'EOF'\n{json_stdout}\nEOF").unwrap();
        drop(file);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        dir
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn refresh_unknown_project_returns_project_not_found() {
        let (backend, _db_path, _project_name) = test_direct_backend_with_registered_code_project();
        let params = RefreshParams {
            project: "totally-unknown-project-xyz".to_string(),
            refresh_code: true,
        };
        let result = backend.refresh(params).await.unwrap();
        let errors = result["errors"].as_array().unwrap();
        assert!(
            errors
                .iter()
                .any(|e| e["code"] == crate::envelope::ERR_PROJECT_NOT_FOUND),
            "expected ERR_PROJECT_NOT_FOUND, got {errors:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn refresh_code_false_skips_subprocess_entirely() {
        let (backend, _db_path, project_name) = test_direct_backend_with_registered_code_project();
        // Points at a nonexistent bin dir so any accidental spawn attempt
        // would fail loudly instead of silently succeeding.
        let backend = backend.with_code_bin_dir(std::path::PathBuf::from("/nonexistent-bin-dir"));
        let params = RefreshParams {
            project: project_name,
            refresh_code: false,
        };
        let result = backend.refresh(params).await.unwrap();
        assert!(result["errors"].as_array().unwrap().is_empty());
        assert_eq!(result["items"][0]["refreshed"], false);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn refresh_code_true_spawn_failure_maps_to_backend_unavailable() {
        let (backend, _db_path, project_name) = test_direct_backend_with_registered_code_project();
        let backend = backend.with_code_bin_dir(std::path::PathBuf::from("/nonexistent-bin-dir"));
        let params = RefreshParams {
            project: project_name,
            refresh_code: true,
        };
        let result = backend.refresh(params).await.unwrap();
        let errors = result["errors"].as_array().unwrap();
        assert!(
            errors
                .iter()
                .any(|e| e["code"] == crate::envelope::ERR_BACKEND_UNAVAILABLE),
            "expected ERR_BACKEND_UNAVAILABLE for spawn failure, got {errors:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn refresh_code_true_success_tags_result_extracted_code() {
        let (backend, _db_path, project_name) = test_direct_backend_with_registered_code_project();
        let bin_dir = fake_magellan_bin_dir(
            r#"{"updated":[],"deleted":[],"added":[],"unchanged":4,"dry_run":false}"#,
        );
        let backend = backend.with_code_bin_dir(bin_dir.path().to_path_buf());
        let params = RefreshParams {
            project: project_name,
            refresh_code: true,
        };
        let result = backend.refresh(params).await.unwrap();
        assert!(result["errors"].as_array().unwrap().is_empty());
        assert_eq!(result["items"][0]["provenance"], "EXTRACTED");
        assert_eq!(result["items"][0]["source"], "code");
    }

    // -----------------------------------------------------------------
    // code_stale — search wiring
    // -----------------------------------------------------------------

    #[tokio::test(flavor = "multi_thread")]
    async fn search_kind_code_with_project_scope_populates_code_stale() {
        let (backend, _db_path, project_name) = test_direct_backend_with_registered_code_project();
        let bin_dir = fake_magellan_bin_dir(
            r#"{"updated":[],"deleted":[],"added":[],"unchanged":4,"dry_run":true}"#,
        );
        let backend = backend.with_code_bin_dir(bin_dir.path().to_path_buf());
        let params = SearchParams {
            query: "shared_probe_symbol".to_string(),
            k: 10,
            project: Some(project_name),
            kind: SearchKind::Code,
            limit: None,
            cursor: None,
        };
        let result = backend.search(params).await.unwrap();
        assert_eq!(
            result["code_stale"], false,
            "expected code_stale to be checked and false for a clean fixture, got {result:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn search_kind_code_without_project_leaves_code_stale_none() {
        // No `project` scope means no single magellan db to check — spec's
        // "None means not applicable to this call".
        let (backend, _db_path, _project_name) = test_direct_backend_with_registered_code_project();
        let params = SearchParams {
            query: "shared_probe_symbol".to_string(),
            k: 10,
            project: None,
            kind: SearchKind::Code,
            limit: None,
            cursor: None,
        };
        let result = backend.search(params).await.unwrap();
        assert!(
            result["code_stale"].is_null(),
            "expected code_stale to stay None without a project scope, got {result:?}"
        );
    }
}
