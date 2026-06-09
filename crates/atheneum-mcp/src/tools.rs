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
    router.add_route(query_memory());
    router.add_route(list_sessions());
    router.add_route(list_events());
    router.add_route(navigate());
    router.add_route(graph_stats());
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
            "k": { "type": "integer", "minimum": 1, "maximum": 100, "default": 10, "description": "Number of results" },
            "project": { "type": "string", "description": "Optional project scope" }
        },
        "required": ["query"]
    });
    let schema: Map<String, Value> = schema.as_object().unwrap().clone();

    rmcp::handler::server::router::tool::ToolRoute::new_dyn(
        Tool::new("search", "Lexical search over the knowledge graph.", schema),
        |ctx: rmcp::handler::server::tool::ToolCallContext<'_, AtheneumMcpServer>| {
            Box::pin(async move {
                let args = extract_args(ctx.arguments);
                let query = args["query"].as_str().unwrap_or("").to_string();
                let k = args["k"].as_u64().unwrap_or(10) as usize;
                let project = args["project"].as_str().map(String::from);
                let result = ctx
                    .service
                    .backend
                    .search(&query, k, project.as_deref())
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
// Tool: store_memory
// ---------------------------------------------------------------------------

fn store_memory() -> rmcp::handler::server::router::tool::ToolRoute<AtheneumMcpServer> {
    let schema = json!({
        "type": "object",
        "properties": {
            "content": { "type": "string", "description": "Memory content" },
            "tags": { "type": "array", "items": { "type": "string" }, "description": "Optional tags" },
            "importance": { "type": "integer", "minimum": 1, "maximum": 10, "default": 5, "description": "Importance (1–10)" }
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
// Tool: query_memory
// ---------------------------------------------------------------------------

fn query_memory() -> rmcp::handler::server::router::tool::ToolRoute<AtheneumMcpServer> {
    let schema = json!({
        "type": "object",
        "properties": {
            "query": { "type": "string", "description": "Memory query" },
            "k": { "type": "integer", "minimum": 1, "maximum": 100, "default": 10, "description": "Number of results" }
        },
        "required": ["query"]
    });
    let schema: Map<String, Value> = schema.as_object().unwrap().clone();

    rmcp::handler::server::router::tool::ToolRoute::new_dyn(
        Tool::new(
            "query_memory",
            "Query episodic memory by semantic similarity.",
            schema,
        ),
        |ctx: rmcp::handler::server::tool::ToolCallContext<'_, AtheneumMcpServer>| {
            Box::pin(async move {
                let args = extract_args(ctx.arguments);
                let query = args["query"].as_str().unwrap_or("").to_string();
                let k = args["k"].as_u64().unwrap_or(10) as usize;
                let result = ctx.service.backend.query_memory(&query, k).await;
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
            "k": { "type": "integer", "minimum": 1, "maximum": 50, "default": 10, "description": "Number of results" },
            "depth": { "type": "integer", "minimum": 1, "maximum": 5, "default": 2, "description": "BFS depth for subgraph traversal" }
        },
        "required": ["query"]
    });
    let schema: Map<String, Value> = schema.as_object().unwrap().clone();

    rmcp::handler::server::router::tool::ToolRoute::new_dyn(
        Tool::new(
            "navigate",
            "Navigate the knowledge graph using natural language.",
            schema,
        ),
        |ctx: rmcp::handler::server::tool::ToolCallContext<'_, AtheneumMcpServer>| {
            Box::pin(async move {
                let args = extract_args(ctx.arguments);
                let query = args["query"].as_str().unwrap_or("").to_string();
                let k = args["k"].as_u64().unwrap_or(10) as usize;
                let depth = args["depth"].as_u64().unwrap_or(2) as u32;
                let result = ctx.service.backend.navigate(&query, k, depth).await;
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
