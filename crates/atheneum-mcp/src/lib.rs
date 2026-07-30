//! Atheneum MCP Server
//!
//! A Model Context Protocol server exposing Atheneum's agent memory
//! and knowledge graph as MCP tools.

pub mod backend;
pub mod envelope;
pub mod tools;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::ServerHandler;
use rmcp::model::{Implementation, ServerCapabilities, ServerInfo};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::ErrorData as McpError;

/// The MCP server instance.
pub struct AtheneumMcpServer {
    pub backend: std::sync::Arc<dyn backend::Backend>,
    pub router: ToolRouter<Self>,
}

impl AtheneumMcpServer {
    pub fn new(backend: std::sync::Arc<dyn backend::Backend>) -> Self {
        let mut router = ToolRouter::new();
        tools::register_all(&mut router);
        Self { backend, router }
    }
}

impl ServerHandler for AtheneumMcpServer {
    fn get_info(&self) -> ServerInfo {
        let instructions = tokio::runtime::Handle::try_current()
            .map(|handle| {
                if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::CurrentThread {
                    // block_in_place is not allowed on CurrentThread runtime
                    return "".to_string();
                }
                tokio::task::block_in_place(|| {
                    handle.block_on(async {
                        match self
                            .backend
                            .seed_memory(crate::backend::SeedMemoryParams {
                                project: None,
                                tokens: Some(400),
                            })
                            .await
                        {
                            Ok(v) => v["instructions"].as_str().unwrap_or("").to_string(),
                            Err(_) => "".to_string(),
                        }
                    })
                })
            })
            .unwrap_or_default();

        let final_instructions = if instructions.is_empty() {
            "Atheneum MCP server: tools for agent memory, knowledge graph, search, and navigation."
                .to_string()
        } else {
            format!(
                "Atheneum MCP server: tools for agent memory, knowledge graph, search, and navigation.\n\n\
                 Current Knowledge Base Context:\n{}",
                instructions
            )
        };

        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "atheneum-mcp",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(&final_instructions)
            .with_protocol_version(rmcp::model::ProtocolVersion::V_2025_03_26)
    }

    // reason: needs the explicit `+ MaybeSendFuture` bound `async fn` sugar
    // can't express on stable.
    #[allow(clippy::manual_async_fn)]
    fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<rmcp::model::ListToolsResult, McpError>>
           + rmcp::service::MaybeSendFuture
           + '_ {
        async move {
            let mut tools = self.router.list_all();
            if let Ok(seed) = self
                .backend
                .seed_memory(crate::backend::SeedMemoryParams {
                    project: None,
                    tokens: Some(400),
                })
                .await
            {
                if let Some(instructions) = seed["instructions"].as_str() {
                    for tool in &mut tools {
                        if tool.name == "navigate"
                            || tool.name == "query_memory"
                            || tool.name == "search"
                        {
                            let original =
                                tool.description.as_ref().map(|c| c.as_ref()).unwrap_or("");
                            let enriched = format!(
                                "{}\n\nCurrent Knowledge Base Context:\n{}",
                                original, instructions
                            );
                            tool.description = Some(std::borrow::Cow::Owned(enriched));
                        }
                    }
                }
            }
            Ok(rmcp::model::ListToolsResult {
                tools,
                next_cursor: None,
                meta: None,
            })
        }
    }

    fn call_tool(
        &self,
        request: rmcp::model::CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<rmcp::model::CallToolResult, McpError>>
           + rmcp::service::MaybeSendFuture
           + '_ {
        let ctx = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        async move {
            self.router
                .call(ctx)
                .await
                .map_err(|e| McpError::internal_error(format!("tool call failed: {e}"), None))
        }
    }

    fn get_tool(&self, name: &str) -> Option<rmcp::model::Tool> {
        self.router.get(name).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::ProtocolVersion;
    use serde_json::Value;
    use std::sync::Arc;

    struct MockBackend;

    #[async_trait::async_trait]
    impl backend::Backend for MockBackend {
        async fn store_discovery(
            &self,
            _p: backend::StoreDiscoveryParams,
        ) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn query_knowledge(&self, _t: &str, _p: Option<&str>) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn search(&self, _params: backend::SearchParams) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn store_memory(&self, _p: backend::StoreMemoryParams) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn query_memory(&self, _p: backend::QueryMemoryParams) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn update_memory(&self, _p: backend::UpdateMemoryParams) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn add_memory(&self, _p: backend::AddMemoryParams) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn maintain(&self, _p: backend::MaintainParams) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn seed_memory(&self, _p: backend::SeedMemoryParams) -> anyhow::Result<Value> {
            Ok(serde_json::json!({
                "instructions": "Mock instructions",
                "token_estimate": 10
            }))
        }
        async fn list_sessions(&self, _l: i64) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn list_events(&self, _l: i64) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn navigate(
            &self,
            _q: &str,
            _k: usize,
            _d: u32,
            _o: usize,
            _l: usize,
            _t: Option<bool>,
        ) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn graph_stats(&self) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn search_memory(
            &self,
            _q: &str,
            _k: usize,
            _p: Option<&str>,
        ) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn list_memory(
            &self,
            _s: Option<&str>,
            _p: Option<&str>,
            _o: usize,
            _l: usize,
        ) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn memory_bootstrap(
            &self,
            _p: Option<&str>,
            _t: usize,
            _l: i64,
        ) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn query_wiki(&self, _path: &str) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn wiki_search(
            &self,
            _q: &str,
            _p: Option<&str>,
            _l: usize,
        ) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn discoveries_recent(
            &self,
            _p: Option<&str>,
            _a: Option<&str>,
            _s: Option<&str>,
            _t: Option<&str>,
            _l: i64,
        ) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn decision_search(
            &self,
            _q: &str,
            _p: Option<&str>,
            _l: i64,
        ) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn thread(
            &self,
            _q: &str,
            _k: usize,
            _d: u32,
            _p: Option<&str>,
            _t: usize,
        ) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn session_digest(&self, _p: Option<&str>, _l: i64) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn get_entity(&self, _id: i64) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn get_neighbors(&self, _id: i64) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn dream(
            &self,
            _s: Option<&str>,
            _p: Option<&str>,
            _d: bool,
        ) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn list_models(&self) -> anyhow::Result<Value> {
            Ok(serde_json::json!([]))
        }
        async fn dream_semantic(
            &self,
            _params: backend::DreamSemanticParams,
        ) -> anyhow::Result<Value> {
            Ok(serde_json::json!({ "merges_completed": 0, "details": [] }))
        }
        async fn pin_entity(&self, id: i64) -> anyhow::Result<Value> {
            Ok(serde_json::json!({ "status": "success", "id": id, "pinned": true }))
        }
        async fn unpin_entity(&self, id: i64) -> anyhow::Result<Value> {
            Ok(serde_json::json!({ "status": "success", "id": id, "pinned": false }))
        }
    }

    fn mock_server() -> AtheneumMcpServer {
        AtheneumMcpServer::new(Arc::new(MockBackend))
    }

    #[test]
    fn server_info_is_correct() {
        let server = mock_server();
        let info = server.get_info();
        assert_eq!(info.protocol_version, ProtocolVersion::V_2025_03_26);
        assert_eq!(info.server_info.name, "atheneum-mcp");
        assert!(info.instructions.is_some());
    }

    #[test]
    fn all_twenty_tools_registered() {
        let server = mock_server();
        let tools = server.router.list_all();
        let names: Vec<_> = tools.iter().map(|t| t.name.as_ref()).collect();
        // Original 9
        assert!(names.contains(&"store_discovery"));
        assert!(names.contains(&"query_knowledge"));
        assert!(names.contains(&"search"));
        assert!(names.contains(&"store_memory"));
        assert!(names.contains(&"update_memory"));
        assert!(names.contains(&"add_memory"));
        assert!(names.contains(&"query_memory"));
        assert!(names.contains(&"list_sessions"));
        assert!(names.contains(&"list_events"));
        assert!(names.contains(&"navigate"));
        assert!(names.contains(&"graph_stats"));
        // Phase 3 additions
        assert!(names.contains(&"search_memory"));
        assert!(names.contains(&"list_memory"));
        assert!(names.contains(&"memory_bootstrap"));
        assert!(names.contains(&"query_wiki"));
        assert!(names.contains(&"wiki_search"));
        assert!(names.contains(&"discoveries_recent"));
        assert!(names.contains(&"decision_search"));
        assert!(names.contains(&"thread"));
        assert!(names.contains(&"session_digest"));
        assert!(names.contains(&"get_entity"));
        assert!(names.contains(&"get_neighbors"));
        assert!(names.contains(&"dream"));
        assert!(names.contains(&"maintain"));
        assert!(names.contains(&"seed_memory"));
        assert_eq!(tools.len(), 29);
    }

    #[test]
    fn get_tool_by_name() {
        let server = mock_server();
        assert!(server.get_tool("search").is_some());
        assert!(server.get_tool("nonexistent").is_none());
    }
}
