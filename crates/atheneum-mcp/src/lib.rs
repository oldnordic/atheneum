//! Atheneum MCP Server
//!
//! A Model Context Protocol server exposing Atheneum's agent memory
//! and knowledge graph as MCP tools.

pub mod backend;
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
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "atheneum-mcp",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "Atheneum MCP server: tools for agent memory, knowledge graph, \
                 search, and navigation.",
            )
            .with_protocol_version(rmcp::model::ProtocolVersion::V_2025_03_26)
    }

    fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<rmcp::model::ListToolsResult, McpError>>
           + rmcp::service::MaybeSendFuture
           + '_ {
        std::future::ready(Ok(rmcp::model::ListToolsResult {
            tools: self.router.list_all(),
            next_cursor: None,
            meta: None,
        }))
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
        async fn search(&self, _q: &str, _k: usize, _p: Option<&str>) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn store_memory(&self, _p: backend::StoreMemoryParams) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn query_memory(&self, _q: &str, _k: usize) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn list_sessions(&self, _l: i64) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn list_events(&self, _l: i64) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn navigate(&self, _q: &str, _k: usize, _d: u32) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn graph_stats(&self) -> anyhow::Result<Value> {
            Ok(Value::Null)
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
    fn all_nine_tools_registered() {
        let server = mock_server();
        let tools = server.router.list_all();
        let names: Vec<_> = tools.iter().map(|t| t.name.as_ref()).collect();
        assert!(names.contains(&"store_discovery"));
        assert!(names.contains(&"query_knowledge"));
        assert!(names.contains(&"search"));
        assert!(names.contains(&"store_memory"));
        assert!(names.contains(&"query_memory"));
        assert!(names.contains(&"list_sessions"));
        assert!(names.contains(&"list_events"));
        assert!(names.contains(&"navigate"));
        assert!(names.contains(&"graph_stats"));
        assert_eq!(tools.len(), 9);
    }

    #[test]
    fn get_tool_by_name() {
        let server = mock_server();
        assert!(server.get_tool("search").is_some());
        assert!(server.get_tool("nonexistent").is_none());
    }
}
