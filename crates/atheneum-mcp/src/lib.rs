//! Atheneum MCP Server
//!
//! A Model Context Protocol server exposing Atheneum's agent memory
//! and knowledge graph as MCP tools.

pub mod backend;
pub mod envelope;
pub mod events;
pub mod subprocess;
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
    pub profile: tools::ToolProfile,
}

impl AtheneumMcpServer {
    pub fn try_new(
        backend: std::sync::Arc<dyn backend::Backend>,
    ) -> Result<Self, tools::ToolProfileError> {
        let profile = tools::ToolProfile::from_env()?;
        Ok(Self::new_with_profile(backend, profile))
    }

    pub fn new_with_profile(
        backend: std::sync::Arc<dyn backend::Backend>,
        profile: tools::ToolProfile,
    ) -> Self {
        let mut router = ToolRouter::new();
        tools::register_profile(&mut router, profile);
        Self {
            backend,
            router,
            profile,
        }
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

    #[expect(
        clippy::manual_async_fn,
        reason = "needs explicit + MaybeSendFuture bound that async fn sugar cannot express on stable"
    )]
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
                        let name_str = tool.name.as_ref();
                        if name_str == "navigate"
                            || name_str == "query_memory"
                            || name_str == "search"
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
        let tool_name = request.name.to_string();
        let is_core = self.profile == tools::ToolProfile::Core;
        let not_in_core = !tools::is_core_tool(&tool_name);
        let ctx = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        async move {
            if is_core && not_in_core {
                return Err(McpError::invalid_params(
                    format!("tool not available in core profile: {tool_name}"),
                    None,
                ));
            }
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
    use crate::tools::{self, ToolProfile};
    use rmcp::model::ProtocolVersion;
    use serde_json::Value;
    use std::sync::Arc;

    static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct EnvVarGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        orig: Option<std::ffi::OsString>,
    }

    impl EnvVarGuard {
        fn set(val: &str) -> Self {
            let lock = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            let orig = std::env::var_os(ToolProfile::ENV_VAR);
            std::env::set_var(ToolProfile::ENV_VAR, val);
            Self { _lock: lock, orig }
        }

        fn remove() -> Self {
            let lock = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            let orig = std::env::var_os(ToolProfile::ENV_VAR);
            std::env::remove_var(ToolProfile::ENV_VAR);
            Self { _lock: lock, orig }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(ref val) = self.orig {
                std::env::set_var(ToolProfile::ENV_VAR, val);
            } else {
                std::env::remove_var(ToolProfile::ENV_VAR);
            }
        }
    }

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
        async fn navigate(&self, _p: backend::NavigateParams) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn code_query(&self, _p: backend::CodeQueryParams) -> anyhow::Result<Value> {
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
        async fn query_wiki(
            &self,
            _path: &str,
            _offset: Option<usize>,
            _limit: Option<usize>,
        ) -> anyhow::Result<Value> {
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
        async fn event(&self, _p: backend::EventParams) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn refresh(&self, _p: backend::RefreshParams) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn pin_grounded_claim(
            &self,
            _p: backend::PinGroundedClaimParams,
        ) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
        async fn audit_claims(&self, _p: backend::AuditClaimsParams) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
    }

    fn mock_server() -> AtheneumMcpServer {
        AtheneumMcpServer::new_with_profile(Arc::new(MockBackend), ToolProfile::Full)
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
    fn all_tools_registered() {
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
        assert!(names.contains(&"code_query"));
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
        assert!(names.contains(&"event"));
        assert!(names.contains(&"refresh"));
        assert!(names.contains(&"pin_grounded_claim"));
        assert!(names.contains(&"audit_claims"));
        assert_eq!(tools.len(), 34);
    }

    #[test]
    fn get_tool_by_name() {
        let server = mock_server();
        assert!(server.get_tool("search").is_some());
        assert!(server.get_tool("nonexistent").is_none());
    }

    #[test]
    fn core_profile_lists_exactly_12_tools() {
        let server = AtheneumMcpServer::new_with_profile(Arc::new(MockBackend), ToolProfile::Core);
        let tools = server.router.list_all();
        let actual_names: std::collections::BTreeSet<String> =
            tools.iter().map(|t| t.name.to_string()).collect();
        let expected_names: std::collections::BTreeSet<String> =
            tools::CORE_TOOLS.iter().map(|s| s.to_string()).collect();
        assert_eq!(actual_names, expected_names);
        assert_eq!(actual_names.len(), 12);
    }

    #[test]
    fn full_profile_and_unset_lists_all_34_tools() {
        let server_full =
            AtheneumMcpServer::new_with_profile(Arc::new(MockBackend), ToolProfile::Full);
        assert_eq!(server_full.router.list_all().len(), 34);

        let _guard = EnvVarGuard::remove();
        let server_default = AtheneumMcpServer::try_new(Arc::new(MockBackend)).unwrap();
        assert_eq!(server_default.router.list_all().len(), 34);
    }

    #[test]
    fn tool_profile_env_parsing_and_rejection() {
        assert_eq!(ToolProfile::parse("core").unwrap(), ToolProfile::Core);
        assert_eq!(ToolProfile::parse("slim").unwrap(), ToolProfile::Core);
        assert_eq!(ToolProfile::parse("CORE").unwrap(), ToolProfile::Core);
        assert_eq!(ToolProfile::parse("SLIM").unwrap(), ToolProfile::Core);
        assert_eq!(ToolProfile::parse("full").unwrap(), ToolProfile::Full);
        assert_eq!(ToolProfile::parse("FULL").unwrap(), ToolProfile::Full);
        assert_eq!(ToolProfile::parse("  core  ").unwrap(), ToolProfile::Core);

        assert!(ToolProfile::parse("bogus").is_err());
        assert!(ToolProfile::parse("").is_err());
        assert!(ToolProfile::parse("all").is_err());
        assert_eq!(
            ToolProfile::parse("  bogus  ").unwrap_err().to_string(),
            "unknown ATHENEUM_TOOL_PROFILE value 'bogus': expected 'core' (alias: 'slim') or 'full'"
        );
    }

    #[tokio::test]
    async fn core_profile_negative_probe_wiki_search_call_fails() {
        use rmcp::ServiceExt;
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

        let server = AtheneumMcpServer::new_with_profile(Arc::new(MockBackend), ToolProfile::Core);
        let tools = server.router.list_all();
        let names: Vec<_> = tools.iter().map(|t| t.name.as_ref()).collect();

        // Absent from list
        assert!(!names.contains(&"wiki_search"));
        assert_eq!(server.get_tool("wiki_search"), None);

        // Calling it under core profile errors loudly with "tool not available in core profile"
        let (server_stream, client_stream) = tokio::io::duplex(4096);
        let (client_read, mut client_write) = tokio::io::split(client_stream);
        let mut client_reader = tokio::io::BufReader::new(client_read);

        let _server_handle = tokio::spawn(async move {
            let running = server.serve(server_stream).await.unwrap();
            running.waiting().await.unwrap();
        });

        // Initialize
        let init_msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": { "name": "test-client", "version": "0.0.1" }
            }
        });
        client_write
            .write_all(format!("{}\n", init_msg).as_bytes())
            .await
            .unwrap();
        client_write.flush().await.unwrap();
        let mut line = String::new();
        client_reader.read_line(&mut line).await.unwrap();

        // Initialized notification
        let notif = serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        client_write
            .write_all(format!("{}\n", notif).as_bytes())
            .await
            .unwrap();
        client_write.flush().await.unwrap();

        // Call non-core tool wiki_search
        let call_msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "wiki_search",
                "arguments": { "query": "test" }
            }
        });
        client_write
            .write_all(format!("{}\n", call_msg).as_bytes())
            .await
            .unwrap();
        client_write.flush().await.unwrap();

        line.clear();
        client_reader.read_line(&mut line).await.unwrap();
        let call_response: serde_json::Value = serde_json::from_str(&line).unwrap();

        assert_eq!(call_response["id"], 2);
        let err = call_response["error"]["message"].as_str().unwrap_or("");
        assert!(
            err.contains("tool not available in core profile"),
            "expected error containing 'tool not available in core profile', got: {}",
            err
        );
    }

    #[test]
    fn unknown_env_var_causes_startup_error_and_refuses_to_serve() {
        let _guard = EnvVarGuard::set("invalid_profile_xyz");
        let result = AtheneumMcpServer::try_new(Arc::new(MockBackend));
        drop(_guard);

        let err = match result {
            Err(e) => e,
            Ok(_) => panic!("expected Err on unknown profile"),
        };
        let err_msg = err.to_string();
        assert!(err_msg.contains("unknown ATHENEUM_TOOL_PROFILE value 'invalid_profile_xyz'"));
        assert!(err_msg.contains("expected 'core' (alias: 'slim') or 'full'"));
    }

    #[test]
    fn env_var_core_and_slim_activates_core_profile() {
        {
            let _guard = EnvVarGuard::set("core");
            let server_core = AtheneumMcpServer::try_new(Arc::new(MockBackend)).unwrap();
            assert_eq!(server_core.profile, ToolProfile::Core);
            assert_eq!(server_core.router.list_all().len(), 12);
        }

        {
            let _guard = EnvVarGuard::set("slim");
            let server_slim = AtheneumMcpServer::try_new(Arc::new(MockBackend)).unwrap();
            assert_eq!(server_slim.profile, ToolProfile::Core);
            assert_eq!(server_slim.router.list_all().len(), 12);
        }

        {
            let _guard = EnvVarGuard::set("full");
            let server_full = AtheneumMcpServer::try_new(Arc::new(MockBackend)).unwrap();
            assert_eq!(server_full.profile, ToolProfile::Full);
            assert_eq!(server_full.router.list_all().len(), 34);
        }

        {
            let _guard = EnvVarGuard::remove();
            let server_default = AtheneumMcpServer::try_new(Arc::new(MockBackend)).unwrap();
            assert_eq!(server_default.profile, ToolProfile::Full);
            assert_eq!(server_default.router.list_all().len(), 34);
        }
    }

    #[tokio::test]
    async fn core_profile_positive_and_negative_tool_calls() {
        use rmcp::ServiceExt;
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

        let server = AtheneumMcpServer::new_with_profile(Arc::new(MockBackend), ToolProfile::Core);

        let (server_stream, client_stream) = tokio::io::duplex(4096);
        let (client_read, mut client_write) = tokio::io::split(client_stream);
        let mut client_reader = tokio::io::BufReader::new(client_read);

        let _server_handle = tokio::spawn(async move {
            let running = server.serve(server_stream).await.unwrap();
            running.waiting().await.unwrap();
        });

        // Initialize
        let init_msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": { "name": "test-client", "version": "0.0.1" }
            }
        });
        client_write
            .write_all(format!("{}\n", init_msg).as_bytes())
            .await
            .unwrap();
        client_write.flush().await.unwrap();
        let mut line = String::new();
        client_reader.read_line(&mut line).await.unwrap();

        // Initialized notification
        let notif = serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        client_write
            .write_all(format!("{}\n", notif).as_bytes())
            .await
            .unwrap();
        client_write.flush().await.unwrap();

        // 1. Positive call: search (core tool) succeeds
        let call_search = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "search",
                "arguments": { "query": "hello" }
            }
        });
        client_write
            .write_all(format!("{}\n", call_search).as_bytes())
            .await
            .unwrap();
        client_write.flush().await.unwrap();
        line.clear();
        client_reader.read_line(&mut line).await.unwrap();
        let search_response: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(search_response["id"], 2);
        assert!(search_response["error"].is_null());
        assert!(search_response["result"].is_object());

        // 2. Negative call: code_query (non-core tool) fails loudly
        let call_code_query = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "code_query",
                "arguments": { "query": "foo" }
            }
        });
        client_write
            .write_all(format!("{}\n", call_code_query).as_bytes())
            .await
            .unwrap();
        client_write.flush().await.unwrap();
        line.clear();
        client_reader.read_line(&mut line).await.unwrap();
        let code_query_response: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(code_query_response["id"], 3);
        assert!(code_query_response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("tool not available in core profile"));
    }

    #[tokio::test]
    async fn core_profile_instruction_injection_covers_navigate_query_memory_search_only() {
        use rmcp::ServiceExt;
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

        let server = AtheneumMcpServer::new_with_profile(Arc::new(MockBackend), ToolProfile::Core);
        let (server_stream, client_stream) = tokio::io::duplex(4096);
        let (client_read, mut client_write) = tokio::io::split(client_stream);
        let mut client_reader = tokio::io::BufReader::new(client_read);

        let _server_handle = tokio::spawn(async move {
            let running = server.serve(server_stream).await.unwrap();
            running.waiting().await.unwrap();
        });

        // Initialize
        let init_msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": { "name": "test-client", "version": "0.0.1" }
            }
        });
        client_write
            .write_all(format!("{}\n", init_msg).as_bytes())
            .await
            .unwrap();
        client_write.flush().await.unwrap();
        let mut line = String::new();
        client_reader.read_line(&mut line).await.unwrap();

        // Initialized notification
        let notif = serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        client_write
            .write_all(format!("{}\n", notif).as_bytes())
            .await
            .unwrap();
        client_write.flush().await.unwrap();

        // Request tools/list
        let list_msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        });
        client_write
            .write_all(format!("{}\n", list_msg).as_bytes())
            .await
            .unwrap();
        client_write.flush().await.unwrap();
        line.clear();
        client_reader.read_line(&mut line).await.unwrap();
        let list_response: serde_json::Value = serde_json::from_str(&line).unwrap();

        assert_eq!(list_response["id"], 2);
        let tools = list_response["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 12);

        for tool in tools {
            let name = tool["name"].as_str().unwrap();
            let desc = tool["description"].as_str().unwrap_or("");
            if name == "navigate" || name == "query_memory" || name == "search" {
                assert!(
                    desc.contains("Current Knowledge Base Context:\nMock instructions"),
                    "tool {} should have injected instructions",
                    name
                );
            } else {
                assert!(
                    !desc.contains("Current Knowledge Base Context:"),
                    "tool {} should NOT have injected instructions",
                    name
                );
            }
        }
    }
}
