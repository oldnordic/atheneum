//! MCP tool definitions for Atheneum.
//!
//! Tools are registered manually via [`ToolRoute::new_dyn`] so we control the
//! input schemas directly as JSON Schema objects — no `schemars` dependency
//! is required.

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::model::{CallToolResult, Content, JsonObject, Tool};
use rmcp::ErrorData as McpError;
use serde_json::{json, Map, Value};

use crate::AtheneumMcpServer;

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub fn register_all(router: &mut ToolRouter<AtheneumMcpServer>) {
    router.add_route(store_discovery());
    router.add_route(query_knowledge());
    router.add_route(search());
    router.add_route(store_memory());
    router.add_route(update_memory());
    router.add_route(add_memory());
    router.add_route(query_memory());
    router.add_route(list_sessions());
    router.add_route(list_events());
    router.add_route(navigate());
    router.add_route(graph_stats());
    // Phase 3 additions
    router.add_route(search_memory());
    router.add_route(list_memory());
    router.add_route(memory_bootstrap());
    router.add_route(query_wiki());
    router.add_route(wiki_search());
    router.add_route(discoveries_recent());
    router.add_route(decision_search());
    router.add_route(thread());
    router.add_route(session_digest());
    router.add_route(get_entity());
    router.add_route(get_neighbors());
    router.add_route(dream());
    router.add_route(maintain());
    router.add_route(seed_memory());
    router.add_route(list_models());
    router.add_route(dream_semantic());
    router.add_route(pin_entity());
    router.add_route(unpin_entity());
}

// ---------------------------------------------------------------------------
// Helper: build CallToolResult from JSON value
// ---------------------------------------------------------------------------

fn json_result(value: Value) -> Result<CallToolResult, McpError> {
    let text = serde_json::to_string_pretty(&value)
        .map_err(|e| McpError::internal_error(format!("serialization failed: {e}"), None))?;
    Ok(CallToolResult::success(vec![Content::text(text)]))
}

fn extract_args(args: Option<JsonObject>) -> Value {
    Value::Object(args.unwrap_or_default())
}

// ---------------------------------------------------------------------------
// Tool: store_discovery
// ---------------------------------------------------------------------------

fn store_discovery() -> rmcp::handler::server::router::tool::ToolRoute<AtheneumMcpServer> {
    let schema = json!({
        "type": "object",
        "properties": {
            "target": { "type": "string", "description": "Entity or topic being observed" },
            "observation": { "type": "string", "description": "The observation text" },
            "confidence": { "type": "number", "minimum": 0.0, "maximum": 1.0, "description": "Confidence score (0.0–1.0)" },
            "tags": { "type": "array", "items": { "type": "string" }, "description": "Optional tags" },
            "project": { "type": "string", "description": "Optional project scope" }
        },
        "required": ["target", "observation", "confidence"]
    });
    let schema: Map<String, Value> = schema.as_object().unwrap().clone();

    rmcp::handler::server::router::tool::ToolRoute::new_dyn(
        Tool::new(
            "store_discovery",
            "Store a discovery into the knowledge graph.",
            schema,
        ),
        |ctx: rmcp::handler::server::tool::ToolCallContext<'_, AtheneumMcpServer>| {
            Box::pin(async move {
                let args = extract_args(ctx.arguments);
                let params = crate::backend::StoreDiscoveryParams {
                    target: args["target"].as_str().unwrap_or("").to_string(),
                    observation: args["observation"].as_str().unwrap_or("").to_string(),
                    confidence: args["confidence"].as_f64().unwrap_or(0.5),
                    tags: args["tags"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default(),
                    project: args["project"].as_str().map(String::from),
                };
                let result = ctx.service.backend.store_discovery(params).await;
                match result {
                    Ok(v) => json_result(v),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
                }
            })
        },
    )
}

// ---------------------------------------------------------------------------
// Tool: query_knowledge
// ---------------------------------------------------------------------------

fn query_knowledge() -> rmcp::handler::server::router::tool::ToolRoute<AtheneumMcpServer> {
    let schema = json!({
        "type": "object",
        "properties": {
            "target": { "type": "string", "description": "Entity name to query" },
            "project": { "type": "string", "description": "Optional project scope" }
        },
        "required": ["target"]
    });
    let schema: Map<String, Value> = schema.as_object().unwrap().clone();

    rmcp::handler::server::router::tool::ToolRoute::new_dyn(
        Tool::new(
            "query_knowledge",
            "Query knowledge about a target entity.",
            schema,
        ),
        |ctx: rmcp::handler::server::tool::ToolCallContext<'_, AtheneumMcpServer>| {
            Box::pin(async move {
                let args = extract_args(ctx.arguments);
                let target = args["target"].as_str().unwrap_or("").to_string();
                let project = args["project"].as_str().map(String::from);
                let result = ctx
                    .service
                    .backend
                    .query_knowledge(&target, project.as_deref())
                    .await;
                match result {
                    Ok(v) => json_result(v),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
                }
            })
        },
    )
}

// ---------------------------------------------------------------------------
// Tool: search
// ---------------------------------------------------------------------------

fn search() -> rmcp::handler::server::router::tool::ToolRoute<AtheneumMcpServer> {
    let schema = json!({
        "type": "object",
        "properties": {
            "query": { "type": "string", "description": "Search query" },
            "k": { "type": "integer", "minimum": 1, "maximum": 100, "default": 10, "description": "Number of results to consider before pagination" },
            "project": { "type": "string", "description": "Optional project scope" },
            "kind": {
                "type": "string",
                "enum": ["knowledge", "code", "all"],
                "default": "knowledge",
                "description": "knowledge = atheneum only (default, unchanged behavior). code = magellan/llmgrep cross-project symbol search only. all = both, merged and provenance-tagged."
            },
            "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 20, "description": "Page size" },
            "cursor": { "type": "string", "description": "Opaque pagination cursor from a previous call's has_more response" }
        },
        "required": ["query"]
    });
    let schema: Map<String, Value> = schema.as_object().unwrap().clone();

    rmcp::handler::server::router::tool::ToolRoute::new_dyn(
        Tool::new("search", "Search the knowledge graph and/or cross-project code index. Impact/affected-style code results are a first-pass heuristic, not certainty.", schema),
        |ctx: rmcp::handler::server::tool::ToolCallContext<'_, AtheneumMcpServer>| {
            Box::pin(async move {
                let args = extract_args(ctx.arguments);
                let params = crate::backend::SearchParams {
                    query: args["query"].as_str().unwrap_or("").to_string(),
                    k: args["k"].as_u64().unwrap_or(10) as usize,
                    project: args["project"].as_str().map(String::from),
                    kind: crate::backend::SearchKind::from_str_default(args["kind"].as_str()),
                    limit: args["limit"].as_u64().map(|v| v as usize),
                    cursor: args["cursor"].as_str().map(String::from),
                };
                let result = ctx.service.backend.search(params).await;
                match result {
                    Ok(v) => json_result(v),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
                }
            })
        },
    )
}

// ---------------------------------------------------------------------------
// Tool: store_memory
// ---------------------------------------------------------------------------

fn store_memory() -> rmcp::handler::server::router::tool::ToolRoute<AtheneumMcpServer> {
    let schema = json!({
        "type": "object",
        "properties": {
            "key": { "type": "string", "description": "Exact lookup key. Optional; defaults to a stable key derived from content for backward compatibility." },
            "content": { "type": "string", "description": "Memory content" },
            "tags": { "type": "array", "items": { "type": "string" }, "description": "Optional tags" },
            "importance": { "type": "integer", "minimum": 1, "maximum": 10, "default": 5, "description": "Importance (1–10)" },
            "scope": { "type": "string", "description": "Optional memory scope, for example `agent`, `user`, or `project`." },
            "project": { "type": "string", "description": "Optional project id for project-scoped memory." }
        },
        "required": ["content"]
    });
    let schema: Map<String, Value> = schema.as_object().unwrap().clone();

    rmcp::handler::server::router::tool::ToolRoute::new_dyn(
        Tool::new("store_memory", "Store an episodic memory entry.", schema),
        |ctx: rmcp::handler::server::tool::ToolCallContext<'_, AtheneumMcpServer>| {
            Box::pin(async move {
                let args = extract_args(ctx.arguments);
                let params = crate::backend::StoreMemoryParams {
                    key: args["key"].as_str().map(ToString::to_string),
                    content: args["content"].as_str().unwrap_or("").to_string(),
                    tags: args["tags"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default(),
                    importance: args["importance"].as_i64().unwrap_or(5),
                    scope: args["scope"].as_str().map(ToString::to_string),
                    project: args["project"].as_str().map(ToString::to_string),
                };
                let result = ctx.service.backend.store_memory(params).await;
                match result {
                    Ok(v) => json_result(v),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
                }
            })
        },
    )
}

// ---------------------------------------------------------------------------
// Tool: update_memory
// ---------------------------------------------------------------------------

fn update_memory() -> rmcp::handler::server::router::tool::ToolRoute<AtheneumMcpServer> {
    let schema = json!({
        "type": "object",
        "properties": {
            "id": { "type": "integer", "description": "ID of the Memory entity to patch (required)." },
            "content": { "type": "string", "description": "New content body. If omitted, content is unchanged." },
            "importance": { "type": "integer", "minimum": 1, "maximum": 10, "description": "Re-maps to confidence (importance/10) on the same scale as store_memory. If omitted, confidence is unchanged." },
            "tags": { "type": "array", "items": { "type": "string" }, "description": "Tags to merge into the existing list (or replace when replace_tags=true)." },
            "replace_tags": { "type": "boolean", "default": false, "description": "If true, overwrite the tag list instead of merging." }
        },
        "required": ["id"]
    });
    let schema: Map<String, Value> = schema.as_object().unwrap().clone();

    rmcp::handler::server::router::tool::ToolRoute::new_dyn(
        Tool::new(
            "update_memory",
            "Patch an existing memory entry in place. Only the fields you provide are written; \
             tags merge by default (pass replace_tags=true to overwrite). Use this instead of \
             store_memory when correcting or enriching an existing memory, so you don't spawn a \
             duplicate row. At least one of content/importance/tags must be set.",
            schema,
        ),
        |ctx: rmcp::handler::server::tool::ToolCallContext<'_, AtheneumMcpServer>| {
            Box::pin(async move {
                let args = extract_args(ctx.arguments);
                let id = match args["id"].as_i64() {
                    Some(v) => v,
                    None => {
                        return Ok(CallToolResult::error(vec![Content::text(
                            "update_memory requires `id` (integer)",
                        )]))
                    }
                };
                let params = crate::backend::UpdateMemoryParams {
                    id,
                    content: args["content"].as_str().map(String::from),
                    importance: args["importance"].as_i64(),
                    tags: args["tags"].as_array().map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    }),
                    replace_tags: args["replace_tags"].as_bool(),
                };
                let result = ctx.service.backend.update_memory(params).await;
                match result {
                    Ok(v) => json_result(v),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
                }
            })
        },
    )
}

// ---------------------------------------------------------------------------
// Tool: add_memory
// ---------------------------------------------------------------------------

fn add_memory() -> rmcp::handler::server::router::tool::ToolRoute<AtheneumMcpServer> {
    let schema = json!({
        "type": "object",
        "properties": {
            "concept": { "type": "string", "description": "The name of the Concept entity to attach memory to (required)." },
            "body_patch": { "type": "string", "description": "Fact/memory details to enrich the concept memory with (required)." },
            "link_from": { "type": "integer", "description": "Optional ID of an existing entity to link to this memory." },
            "link_both_ways": { "type": "boolean", "default": true, "description": "If true, links both ways between the memory and link_from (requires link_from)." }
        },
        "required": ["concept", "body_patch"]
    });
    let schema: Map<String, Value> = schema.as_object().unwrap().clone();

    rmcp::handler::server::router::tool::ToolRoute::new_dyn(
        Tool::new(
            "add_memory",
            "Add a fact to a concept. If a memory is already attached to this concept, \
             the new fact is appended to it in place. If not, a new memory is created and attached. \
             Optionally links the memory to another existing entity (e.g. current tool call or task) either one-way or both-ways.",
            schema,
        ),
        |ctx: rmcp::handler::server::tool::ToolCallContext<'_, AtheneumMcpServer>| {
            Box::pin(async move {
                let args = extract_args(ctx.arguments);
                let concept = match args["concept"].as_str() {
                    Some(v) => v.to_string(),
                    None => {
                        return Ok(CallToolResult::error(vec![Content::text(
                            "add_memory requires `concept` (string)",
                        )]))
                    }
                };
                let body_patch = match args["body_patch"].as_str() {
                    Some(v) => v.to_string(),
                    None => {
                        return Ok(CallToolResult::error(vec![Content::text(
                            "add_memory requires `body_patch` (string)",
                        )]))
                    }
                };
                let params = crate::backend::AddMemoryParams {
                    concept,
                    body_patch,
                    link_from: args["link_from"].as_i64(),
                    link_both_ways: args["link_both_ways"].as_bool(),
                };
                let result = ctx.service.backend.add_memory(params).await;
                match result {
                    Ok(v) => json_result(v),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
                }
            })
        },
    )
}

// ---------------------------------------------------------------------------
// Tool: query_memory
// ---------------------------------------------------------------------------

fn query_memory() -> rmcp::handler::server::router::tool::ToolRoute<AtheneumMcpServer> {
    let schema = json!({
        "type": "object",
        "properties": {
            "key": { "type": "string", "description": "Exact memory key to retrieve." },
            "query": { "type": "string", "description": "Deprecated alias for `key` kept for backward compatibility." },
            "scope": { "type": "string", "description": "Optional scope filter for exact lookup." },
            "project": { "type": "string", "description": "Optional project filter for project-scoped memory." },
            "k": { "type": "integer", "minimum": 1, "maximum": 100, "default": 10, "description": "Number of results" },
            "include_superseded": { "type": "boolean", "description": "Optional flag to include superseded memory entries." }
        }
    });
    let schema: Map<String, Value> = schema.as_object().unwrap().clone();

    rmcp::handler::server::router::tool::ToolRoute::new_dyn(
        Tool::new(
            "query_memory",
            "Query episodic memory by exact key lookup.",
            schema,
        ),
        |ctx: rmcp::handler::server::tool::ToolCallContext<'_, AtheneumMcpServer>| {
            Box::pin(async move {
                let args = extract_args(ctx.arguments);
                let key = args["key"]
                    .as_str()
                    .or_else(|| args["query"].as_str())
                    .unwrap_or("")
                    .to_string();
                if key.is_empty() {
                    return Ok(CallToolResult::error(vec![Content::text(
                        "query_memory requires `key` (or deprecated `query`)",
                    )]));
                }
                let k = args["k"].as_u64().unwrap_or(10) as usize;
                let include_superseded = args["include_superseded"].as_bool();
                let params = crate::backend::QueryMemoryParams {
                    key,
                    scope: args["scope"].as_str().map(ToString::to_string),
                    project: args["project"].as_str().map(ToString::to_string),
                    k,
                    include_superseded,
                };
                let result = ctx.service.backend.query_memory(params).await;
                match result {
                    Ok(v) => json_result(v),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
                }
            })
        },
    )
}

// ---------------------------------------------------------------------------
// Tool: list_sessions
// ---------------------------------------------------------------------------

fn list_sessions() -> rmcp::handler::server::router::tool::ToolRoute<AtheneumMcpServer> {
    let schema = json!({
        "type": "object",
        "properties": {
            "limit": { "type": "integer", "minimum": 1, "maximum": 1000, "default": 100, "description": "Number of most recent sessions to return" }
        }
    });
    let schema: Map<String, Value> = schema.as_object().unwrap().clone();

    rmcp::handler::server::router::tool::ToolRoute::new_dyn(
        Tool::new("list_sessions", "List recorded agent sessions.", schema),
        |ctx: rmcp::handler::server::tool::ToolCallContext<'_, AtheneumMcpServer>| {
            Box::pin(async move {
                let args = extract_args(ctx.arguments);
                let limit = args["limit"].as_i64().unwrap_or(100);
                let result = ctx.service.backend.list_sessions(limit).await;
                match result {
                    Ok(v) => json_result(v),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
                }
            })
        },
    )
}

// ---------------------------------------------------------------------------
// Tool: list_events
// ---------------------------------------------------------------------------

fn list_events() -> rmcp::handler::server::router::tool::ToolRoute<AtheneumMcpServer> {
    let schema = json!({
        "type": "object",
        "properties": {
            "limit": { "type": "integer", "minimum": 1, "maximum": 1000, "default": 100, "description": "Max results" }
        }
    });
    let schema: Map<String, Value> = schema.as_object().unwrap().clone();

    rmcp::handler::server::router::tool::ToolRoute::new_dyn(
        Tool::new("list_events", "List recorded events.", schema),
        |ctx: rmcp::handler::server::tool::ToolCallContext<'_, AtheneumMcpServer>| {
            Box::pin(async move {
                let args = extract_args(ctx.arguments);
                let limit = args["limit"].as_i64().unwrap_or(100);
                let result = ctx.service.backend.list_events(limit).await;
                match result {
                    Ok(v) => json_result(v),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
                }
            })
        },
    )
}

// ---------------------------------------------------------------------------
// Tool: navigate
// ---------------------------------------------------------------------------

fn navigate() -> rmcp::handler::server::router::tool::ToolRoute<AtheneumMcpServer> {
    let schema = json!({
        "type": "object",
        "properties": {
            "query": { "type": "string", "description": "Natural language navigation query" },
            "k": { "type": "integer", "minimum": 1, "maximum": 50, "default": 10, "description": "Number of entry-point results" },
            "depth": { "type": "integer", "minimum": 1, "maximum": 5, "default": 2, "description": "BFS depth for subgraph traversal" },
            "offset": { "type": "integer", "minimum": 0, "default": 0, "description": "Entity offset for pagination (skip first N signal entities)" },
            "limit": { "type": "integer", "minimum": 1, "maximum": 500, "default": 50, "description": "Max signal entities to return per view (pagination)" },
            "trace": { "type": "boolean", "default": false, "description": "If true, record a QueryTrace entity for this query" },
            "kind": {
                "type": "string",
                "enum": ["knowledge", "code", "all"],
                "default": "knowledge",
                "description": "knowledge = atheneum graph only (default, unchanged behavior). code = magellan/llmgrep cross-project subgraph walk only. all = both, merged and provenance-tagged."
            }
        },
        "required": ["query"]
    });
    let schema: Map<String, Value> = schema.as_object().unwrap().clone();

    rmcp::handler::server::router::tool::ToolRoute::new_dyn(
        Tool::new(
            "navigate",
            "Navigate the knowledge graph using natural language. Results are paginated — use offset/limit to page through entities.",
            schema,
        ),
        |ctx: rmcp::handler::server::tool::ToolCallContext<'_, AtheneumMcpServer>| {
            Box::pin(async move {
                let args = extract_args(ctx.arguments);
                let params = crate::backend::NavigateParams {
                    query: args["query"].as_str().unwrap_or("").to_string(),
                    k: args["k"].as_u64().unwrap_or(10) as usize,
                    depth: args["depth"].as_u64().map(|v| v as u32),
                    offset: args["offset"].as_u64().unwrap_or(0) as usize,
                    limit: args["limit"].as_u64().unwrap_or(50) as usize,
                    trace: args["trace"].as_bool(),
                    kind: crate::backend::SearchKind::from_str_default(args["kind"].as_str()),
                };
                let result = ctx.service.backend.navigate(params).await;
                match result {
                    Ok(v) => json_result(v),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
                }
            })
        },
    )
}

// ---------------------------------------------------------------------------
// Tool: graph_stats
// ---------------------------------------------------------------------------

fn graph_stats() -> rmcp::handler::server::router::tool::ToolRoute<AtheneumMcpServer> {
    let schema = json!({
        "type": "object",
        "properties": {}
    });
    let schema: Map<String, Value> = schema.as_object().unwrap().clone();

    rmcp::handler::server::router::tool::ToolRoute::new_dyn(
        Tool::new(
            "graph_stats",
            "Return high-level knowledge graph statistics.",
            schema,
        ),
        |ctx: rmcp::handler::server::tool::ToolCallContext<'_, AtheneumMcpServer>| {
            Box::pin(async move {
                let result = ctx.service.backend.graph_stats().await;
                match result {
                    Ok(v) => json_result(v),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
                }
            })
        },
    )
}

// ---------------------------------------------------------------------------
// Phase 3: Additional tools
// ---------------------------------------------------------------------------

fn search_memory() -> rmcp::handler::server::router::tool::ToolRoute<AtheneumMcpServer> {
    let schema = json!({
        "type": "object",
        "properties": {
            "query": { "type": "string", "description": "Search query" },
            "k": { "type": "integer", "minimum": 1, "maximum": 100, "default": 10, "description": "Number of results" },
            "project": { "type": "string", "description": "Optional project scope" }
        },
        "required": ["query"]
    });
    let schema: Map<String, Value> = schema.as_object().unwrap().clone();
    rmcp::handler::server::router::tool::ToolRoute::new_dyn(
        Tool::new(
            "search_memory",
            "Lexical search over Memory-kind entities only.",
            schema,
        ),
        |ctx: rmcp::handler::server::tool::ToolCallContext<'_, AtheneumMcpServer>| {
            Box::pin(async move {
                let args = extract_args(ctx.arguments);
                let query = args["query"].as_str().unwrap_or("").to_string();
                let k = args["k"].as_u64().unwrap_or(10) as usize;
                let project = args["project"].as_str().map(String::from);
                match ctx
                    .service
                    .backend
                    .search_memory(&query, k, project.as_deref())
                    .await
                {
                    Ok(v) => json_result(v),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
                }
            })
        },
    )
}

fn list_memory() -> rmcp::handler::server::router::tool::ToolRoute<AtheneumMcpServer> {
    let schema = json!({
        "type": "object",
        "properties": {
            "scope": { "type": "string", "description": "Filter by scope (e.g. memory, reference, user)" },
            "project": { "type": "string", "description": "Optional project scope" },
            "offset": { "type": "integer", "minimum": 0, "default": 0, "description": "Pagination offset" },
            "limit": { "type": "integer", "minimum": 1, "maximum": 200, "default": 20, "description": "Max results" }
        }
    });
    let schema: Map<String, Value> = schema.as_object().unwrap().clone();
    rmcp::handler::server::router::tool::ToolRoute::new_dyn(
        Tool::new("list_memory", "List stored memories (paginated).", schema),
        |ctx: rmcp::handler::server::tool::ToolCallContext<'_, AtheneumMcpServer>| {
            Box::pin(async move {
                let args = extract_args(ctx.arguments);
                let scope = args["scope"].as_str().map(String::from);
                let project = args["project"].as_str().map(String::from);
                let offset = args["offset"].as_u64().unwrap_or(0) as usize;
                let limit = args["limit"].as_u64().unwrap_or(20) as usize;
                match ctx
                    .service
                    .backend
                    .list_memory(scope.as_deref(), project.as_deref(), offset, limit)
                    .await
                {
                    Ok(v) => json_result(v),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
                }
            })
        },
    )
}

fn memory_bootstrap() -> rmcp::handler::server::router::tool::ToolRoute<AtheneumMcpServer> {
    let schema = json!({
        "type": "object",
        "properties": {
            "project": { "type": "string", "description": "Optional project scope" },
            "tokens": { "type": "integer", "minimum": 100, "maximum": 10000, "default": 1000, "description": "Token budget for the bootstrap packet" },
            "last_sessions": { "type": "integer", "minimum": 1, "maximum": 50, "default": 3, "description": "Number of recent sessions to include in digest" }
        }
    });
    let schema: Map<String, Value> = schema.as_object().unwrap().clone();
    rmcp::handler::server::router::tool::ToolRoute::new_dyn(
        Tool::new(
            "memory_bootstrap",
            "Compose a bounded bootstrap packet: relevant memories + session digest.",
            schema,
        ),
        |ctx: rmcp::handler::server::tool::ToolCallContext<'_, AtheneumMcpServer>| {
            Box::pin(async move {
                let args = extract_args(ctx.arguments);
                let project = args["project"].as_str().map(String::from);
                let tokens = args["tokens"].as_u64().unwrap_or(1000) as usize;
                let last_sessions = args["last_sessions"].as_u64().unwrap_or(3) as i64;
                match ctx
                    .service
                    .backend
                    .memory_bootstrap(project.as_deref(), tokens, last_sessions)
                    .await
                {
                    Ok(v) => json_result(v),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
                }
            })
        },
    )
}

fn query_wiki() -> rmcp::handler::server::router::tool::ToolRoute<AtheneumMcpServer> {
    let schema = json!({
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": "Wiki page path (full or partial)" }
        },
        "required": ["path"]
    });
    let schema: Map<String, Value> = schema.as_object().unwrap().clone();
    rmcp::handler::server::router::tool::ToolRoute::new_dyn(
        Tool::new(
            "query_wiki",
            "Fetch a wiki page by path (supports partial path matching).",
            schema,
        ),
        |ctx: rmcp::handler::server::tool::ToolCallContext<'_, AtheneumMcpServer>| {
            Box::pin(async move {
                let args = extract_args(ctx.arguments);
                let path = args["path"].as_str().unwrap_or("").to_string();
                match ctx.service.backend.query_wiki(&path).await {
                    Ok(v) => json_result(v),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
                }
            })
        },
    )
}

fn wiki_search() -> rmcp::handler::server::router::tool::ToolRoute<AtheneumMcpServer> {
    let schema = json!({
        "type": "object",
        "properties": {
            "query": { "type": "string", "description": "Full-text search query" },
            "project": { "type": "string", "description": "Optional project scope" },
            "limit": { "type": "integer", "minimum": 1, "maximum": 50, "default": 10, "description": "Max results" }
        },
        "required": ["query"]
    });
    let schema: Map<String, Value> = schema.as_object().unwrap().clone();
    rmcp::handler::server::router::tool::ToolRoute::new_dyn(
        Tool::new(
            "wiki_search",
            "Full-text search over wiki pages via FTS5.",
            schema,
        ),
        |ctx: rmcp::handler::server::tool::ToolCallContext<'_, AtheneumMcpServer>| {
            Box::pin(async move {
                let args = extract_args(ctx.arguments);
                let query = args["query"].as_str().unwrap_or("").to_string();
                let project = args["project"].as_str().map(String::from);
                let limit = args["limit"].as_u64().unwrap_or(10) as usize;
                match ctx
                    .service
                    .backend
                    .wiki_search(&query, project.as_deref(), limit)
                    .await
                {
                    Ok(v) => json_result(v),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
                }
            })
        },
    )
}

fn discoveries_recent() -> rmcp::handler::server::router::tool::ToolRoute<AtheneumMcpServer> {
    let schema = json!({
        "type": "object",
        "properties": {
            "project": { "type": "string", "description": "Optional project scope" },
            "agent": { "type": "string", "description": "Filter by agent name" },
            "session": { "type": "string", "description": "Filter by session ID" },
            "discovery_type": { "type": "string", "description": "Filter by type (Decision, Bug, Finding, Pattern, etc.)" },
            "limit": { "type": "integer", "minimum": 1, "maximum": 200, "default": 20, "description": "Max results" }
        }
    });
    let schema: Map<String, Value> = schema.as_object().unwrap().clone();
    rmcp::handler::server::router::tool::ToolRoute::new_dyn(
        Tool::new(
            "discoveries_recent",
            "List recent discoveries, optionally filtered.",
            schema,
        ),
        |ctx: rmcp::handler::server::tool::ToolCallContext<'_, AtheneumMcpServer>| {
            Box::pin(async move {
                let args = extract_args(ctx.arguments);
                let project = args["project"].as_str().map(String::from);
                let agent = args["agent"].as_str().map(String::from);
                let session = args["session"].as_str().map(String::from);
                let dtype = args["discovery_type"].as_str().map(String::from);
                let limit = args["limit"].as_u64().unwrap_or(20) as i64;
                match ctx
                    .service
                    .backend
                    .discoveries_recent(
                        project.as_deref(),
                        agent.as_deref(),
                        session.as_deref(),
                        dtype.as_deref(),
                        limit,
                    )
                    .await
                {
                    Ok(v) => json_result(v),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
                }
            })
        },
    )
}

fn decision_search() -> rmcp::handler::server::router::tool::ToolRoute<AtheneumMcpServer> {
    let schema = json!({
        "type": "object",
        "properties": {
            "query": { "type": "string", "description": "Search query (matches target/chosen/why text)" },
            "project": { "type": "string", "description": "Optional project scope" },
            "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 10, "description": "Max results" }
        },
        "required": ["query"]
    });
    let schema: Map<String, Value> = schema.as_object().unwrap().clone();
    rmcp::handler::server::router::tool::ToolRoute::new_dyn(
        Tool::new(
            "decision_search",
            "Search decisions by content (target/chosen/why substring match).",
            schema,
        ),
        |ctx: rmcp::handler::server::tool::ToolCallContext<'_, AtheneumMcpServer>| {
            Box::pin(async move {
                let args = extract_args(ctx.arguments);
                let query = args["query"].as_str().unwrap_or("").to_string();
                let project = args["project"].as_str().map(String::from);
                let limit = args["limit"].as_u64().unwrap_or(10) as i64;
                match ctx
                    .service
                    .backend
                    .decision_search(&query, project.as_deref(), limit)
                    .await
                {
                    Ok(v) => json_result(v),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
                }
            })
        },
    )
}

fn thread() -> rmcp::handler::server::router::tool::ToolRoute<AtheneumMcpServer> {
    let schema = json!({
        "type": "object",
        "properties": {
            "query": { "type": "string", "description": "Search query to find decision entry points" },
            "k": { "type": "integer", "minimum": 1, "maximum": 20, "default": 3, "description": "Number of entry points" },
            "depth": { "type": "integer", "minimum": 1, "maximum": 5, "default": 3, "description": "BFS depth along caused_by/led_to edges" },
            "project": { "type": "string", "description": "Optional project scope" },
            "tokens": { "type": "integer", "minimum": 200, "maximum": 5000, "default": 1500, "description": "Token budget per subgraph" }
        },
        "required": ["query"]
    });
    let schema: Map<String, Value> = schema.as_object().unwrap().clone();
    rmcp::handler::server::router::tool::ToolRoute::new_dyn(
        Tool::new(
            "thread",
            "Walk a decision chain (caused_by/led_to edges) from matching entry points.",
            schema,
        ),
        |ctx: rmcp::handler::server::tool::ToolCallContext<'_, AtheneumMcpServer>| {
            Box::pin(async move {
                let args = extract_args(ctx.arguments);
                let query = args["query"].as_str().unwrap_or("").to_string();
                let k = args["k"].as_u64().unwrap_or(3) as usize;
                let depth = args["depth"].as_u64().unwrap_or(3) as u32;
                let project = args["project"].as_str().map(String::from);
                let tokens = args["tokens"].as_u64().unwrap_or(1500) as usize;
                match ctx
                    .service
                    .backend
                    .thread(&query, k, depth, project.as_deref(), tokens)
                    .await
                {
                    Ok(v) => json_result(v),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
                }
            })
        },
    )
}

fn session_digest() -> rmcp::handler::server::router::tool::ToolRoute<AtheneumMcpServer> {
    let schema = json!({
        "type": "object",
        "properties": {
            "project": { "type": "string", "description": "Optional project scope" },
            "last_sessions": { "type": "integer", "minimum": 1, "maximum": 20, "default": 3, "description": "Number of recent sessions to digest" }
        }
    });
    let schema: Map<String, Value> = schema.as_object().unwrap().clone();
    rmcp::handler::server::router::tool::ToolRoute::new_dyn(
        Tool::new(
            "session_digest",
            "Compose a bounded session digest from recent sessions.",
            schema,
        ),
        |ctx: rmcp::handler::server::tool::ToolCallContext<'_, AtheneumMcpServer>| {
            Box::pin(async move {
                let args = extract_args(ctx.arguments);
                let project = args["project"].as_str().map(String::from);
                let last_sessions = args["last_sessions"].as_u64().unwrap_or(3) as i64;
                match ctx
                    .service
                    .backend
                    .session_digest(project.as_deref(), last_sessions)
                    .await
                {
                    Ok(v) => json_result(v),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
                }
            })
        },
    )
}

fn get_entity() -> rmcp::handler::server::router::tool::ToolRoute<AtheneumMcpServer> {
    let schema = json!({
        "type": "object",
        "properties": {
            "id": { "type": "integer", "description": "Graph entity ID" }
        },
        "required": ["id"]
    });
    let schema: Map<String, Value> = schema.as_object().unwrap().clone();
    rmcp::handler::server::router::tool::ToolRoute::new_dyn(
        Tool::new("get_entity", "Fetch a single graph entity by ID.", schema),
        |ctx: rmcp::handler::server::tool::ToolCallContext<'_, AtheneumMcpServer>| {
            Box::pin(async move {
                let args = extract_args(ctx.arguments);
                let id = args["id"].as_i64().unwrap_or(0);
                match ctx.service.backend.get_entity(id).await {
                    Ok(v) => json_result(v),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
                }
            })
        },
    )
}

fn get_neighbors() -> rmcp::handler::server::router::tool::ToolRoute<AtheneumMcpServer> {
    let schema = json!({
        "type": "object",
        "properties": {
            "id": { "type": "integer", "description": "Graph entity ID" }
        },
        "required": ["id"]
    });
    let schema: Map<String, Value> = schema.as_object().unwrap().clone();
    rmcp::handler::server::router::tool::ToolRoute::new_dyn(
        Tool::new(
            "get_neighbors",
            "Get outgoing + incoming edges for an entity.",
            schema,
        ),
        |ctx: rmcp::handler::server::tool::ToolCallContext<'_, AtheneumMcpServer>| {
            Box::pin(async move {
                let args = extract_args(ctx.arguments);
                let id = args["id"].as_i64().unwrap_or(0);
                match ctx.service.backend.get_neighbors(id).await {
                    Ok(v) => json_result(v),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
                }
            })
        },
    )
}

fn dream() -> rmcp::handler::server::router::tool::ToolRoute<AtheneumMcpServer> {
    let schema = json!({
        "type": "object",
        "properties": {
            "scope": { "type": "string", "description": "Optional scope filter" },
            "project": { "type": "string", "description": "Optional project scope" },
            "dry_run": { "type": "boolean", "default": true, "description": "If true, report findings without mutating the graph" }
        }
    });
    let schema: Map<String, Value> = schema.as_object().unwrap().clone();
    rmcp::handler::server::router::tool::ToolRoute::new_dyn(
        Tool::new(
            "dream",
            "Run a reflective memory consolidation pass (dedup/stale/verbose detection).",
            schema,
        ),
        |ctx: rmcp::handler::server::tool::ToolCallContext<'_, AtheneumMcpServer>| {
            Box::pin(async move {
                let args = extract_args(ctx.arguments);
                let scope = args["scope"].as_str().map(String::from);
                let project = args["project"].as_str().map(String::from);
                let dry_run = args["dry_run"].as_bool().unwrap_or(true);
                match ctx
                    .service
                    .backend
                    .dream(scope.as_deref(), project.as_deref(), dry_run)
                    .await
                {
                    Ok(v) => json_result(v),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
                }
            })
        },
    )
}

fn maintain() -> rmcp::handler::server::router::tool::ToolRoute<AtheneumMcpServer> {
    let schema = json!({
        "type": "object",
        "properties": {
            "apply": { "type": "boolean", "default": false, "description": "If true, execute mutative repairs. If false, dry-run only." },
            "stale_superseded_days": { "type": "integer", "default": 30, "description": "Number of days before superseded memories are pruned" },
            "broken_link_mode": { "type": "string", "enum": ["stub", "sever"], "default": "stub", "description": "Broken link repair strategy" },
            "rewire_threshold": { "type": "number", "default": 0.3, "description": "Orphan concept similarity threshold (0.0 to 1.0)" }
        }
    });
    let schema: Map<String, Value> = schema.as_object().unwrap().clone();
    rmcp::handler::server::router::tool::ToolRoute::new_dyn(
        Tool::new(
            "maintain",
            "Perform database health checks and automatic repairs (orphan concept rewiring, broken link stubbing, memory contradiction superseding, stale row pruning).",
            schema,
        ),
        |ctx: rmcp::handler::server::tool::ToolCallContext<'_, AtheneumMcpServer>| {
            Box::pin(async move {
                let args = extract_args(ctx.arguments);
                let apply = args["apply"].as_bool();
                let stale_superseded_days = args["stale_superseded_days"].as_i64();
                let broken_link_mode = args["broken_link_mode"].as_str().map(String::from);
                let rewire_threshold = args["rewire_threshold"].as_f64();
                let params = crate::backend::MaintainParams {
                    apply,
                    stale_superseded_days,
                    broken_link_mode,
                    rewire_threshold,
                };
                match ctx.service.backend.maintain(params).await {
                    Ok(v) => json_result(v),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
                }
            })
        },
    )
}

fn seed_memory() -> rmcp::handler::server::router::tool::ToolRoute<AtheneumMcpServer> {
    let schema = json!({
        "type": "object",
        "properties": {
            "project": { "type": "string", "description": "Optional project scope" },
            "tokens": { "type": "integer", "default": 800, "description": "Token budget for the seed summary" }
        }
    });
    let schema: Map<String, Value> = schema.as_object().unwrap().clone();
    rmcp::handler::server::router::tool::ToolRoute::new_dyn(
        Tool::new(
            "seed_memory",
            "Generate a compact seed summary of standard instructions, active concepts, and recent memories to bootstrap the client model.",
            schema,
        ),
        |ctx: rmcp::handler::server::tool::ToolCallContext<'_, AtheneumMcpServer>| {
            Box::pin(async move {
                let args = extract_args(ctx.arguments);
                let project = args["project"].as_str().map(String::from);
                let tokens = args["tokens"].as_u64().map(|v| v as usize);
                let params = crate::backend::SeedMemoryParams {
                    project,
                    tokens,
                };
                match ctx.service.backend.seed_memory(params).await {
                    Ok(v) => json_result(v),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
                }
            })
        },
    )
}

fn list_models() -> rmcp::handler::server::router::tool::ToolRoute<AtheneumMcpServer> {
    let schema = json!({
        "type": "object",
        "properties": {}
    });
    let schema: Map<String, Value> = schema.as_object().unwrap().clone();
    rmcp::handler::server::router::tool::ToolRoute::new_dyn(
        Tool::new(
            "list_models",
            "List all loaded LLM models from Ollama or llama.cpp endpoint.",
            schema,
        ),
        |ctx: rmcp::handler::server::tool::ToolCallContext<'_, AtheneumMcpServer>| {
            Box::pin(async move {
                match ctx.service.backend.list_models().await {
                    Ok(v) => json_result(v),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
                }
            })
        },
    )
}

fn dream_semantic() -> rmcp::handler::server::router::tool::ToolRoute<AtheneumMcpServer> {
    let schema = json!({
        "type": "object",
        "properties": {
            "similarity_threshold": { "type": "number", "default": 0.4, "description": "Min Jaccard lexical similarity to trigger merge decision." },
            "model": { "type": "string", "default": "gemma4:e2b", "description": "Llama model to run LLM merge evaluation." },
            "ollama_url": { "type": "string", "default": "http://127.0.0.1:11434", "description": "Host URL of local Ollama server." },
            "swap_guard": { "type": "string", "default": "fallback", "description": "Swap guard mode (strict, adapt, fallback)." },
            "dry_run": { "type": "boolean", "default": false, "description": "If true, report candidate merges without executing them." }
        }
    });
    let schema: Map<String, Value> = schema.as_object().unwrap().clone();
    rmcp::handler::server::router::tool::ToolRoute::new_dyn(
        Tool::new(
            "dream_semantic",
            "Consolidate similar redundant concepts semantically using a local LLM or lexical trigram fallback.",
            schema,
        ),
        |ctx: rmcp::handler::server::tool::ToolCallContext<'_, AtheneumMcpServer>| {
            Box::pin(async move {
                let args = extract_args(ctx.arguments);
                let similarity_threshold = args["similarity_threshold"].as_f64();
                let model = args["model"].as_str().map(String::from);
                let ollama_url = args["ollama_url"].as_str().map(String::from);
                let swap_guard = args["swap_guard"].as_str().map(String::from);
                let dry_run = args["dry_run"].as_bool();
                let params = crate::backend::DreamSemanticParams {
                    similarity_threshold,
                    model,
                    ollama_url,
                    swap_guard,
                    dry_run,
                };
                match ctx.service.backend.dream_semantic(params).await {
                    Ok(v) => json_result(v),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
                }
            })
        },
    )
}

fn pin_entity() -> rmcp::handler::server::router::tool::ToolRoute<AtheneumMcpServer> {
    let schema = json!({
        "type": "object",
        "properties": {
            "id": { "type": "integer", "description": "Unique ID of the concept/memory to pin" }
        },
        "required": ["id"]
    });
    let schema: Map<String, Value> = schema.as_object().unwrap().clone();
    rmcp::handler::server::router::tool::ToolRoute::new_dyn(
        Tool::new(
            "pin_entity",
            "Pin a concept or memory to prevent eviction and prioritize it in seeding.",
            schema,
        ),
        |ctx: rmcp::handler::server::tool::ToolCallContext<'_, AtheneumMcpServer>| {
            Box::pin(async move {
                let args = extract_args(ctx.arguments);
                let id = args["id"].as_i64().unwrap_or(0);
                match ctx.service.backend.pin_entity(id).await {
                    Ok(v) => json_result(v),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
                }
            })
        },
    )
}

fn unpin_entity() -> rmcp::handler::server::router::tool::ToolRoute<AtheneumMcpServer> {
    let schema = json!({
        "type": "object",
        "properties": {
            "id": { "type": "integer", "description": "Unique ID of the concept/memory to unpin" }
        },
        "required": ["id"]
    });
    let schema: Map<String, Value> = schema.as_object().unwrap().clone();
    rmcp::handler::server::router::tool::ToolRoute::new_dyn(
        Tool::new(
            "unpin_entity",
            "Unpin a previously pinned concept or memory.",
            schema,
        ),
        |ctx: rmcp::handler::server::tool::ToolCallContext<'_, AtheneumMcpServer>| {
            Box::pin(async move {
                let args = extract_args(ctx.arguments);
                let id = args["id"].as_i64().unwrap_or(0);
                match ctx.service.backend.unpin_entity(id).await {
                    Ok(v) => json_result(v),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
                }
            })
        },
    )
}
