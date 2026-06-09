use std::sync::Arc;

use atheneum_mcp::{backend, AtheneumMcpServer};
use rmcp::ServiceExt;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

struct MockBackend;

#[async_trait::async_trait]
impl backend::Backend for MockBackend {
    async fn store_discovery(
        &self,
        _p: backend::StoreDiscoveryParams,
    ) -> anyhow::Result<serde_json::Value> {
        Ok(json!({"discovery_id": 42}))
    }
    async fn query_knowledge(
        &self,
        _t: &str,
        _p: Option<&str>,
    ) -> anyhow::Result<serde_json::Value> {
        Ok(json!({"target": "test", "discoveries": []}))
    }
    async fn search(
        &self,
        _q: &str,
        _k: usize,
        _p: Option<&str>,
    ) -> anyhow::Result<serde_json::Value> {
        Ok(json!({"results": [], "count": 0}))
    }
    async fn store_memory(
        &self,
        _p: backend::StoreMemoryParams,
    ) -> anyhow::Result<serde_json::Value> {
        Ok(json!({"memory_id": 7}))
    }
    async fn query_memory(&self, _q: &str, _k: usize) -> anyhow::Result<serde_json::Value> {
        Ok(json!({"results": []}))
    }
    async fn list_sessions(&self, _l: i64) -> anyhow::Result<serde_json::Value> {
        Ok(json!({"sessions": []}))
    }
    async fn list_events(&self, _l: i64) -> anyhow::Result<serde_json::Value> {
        Ok(json!({"events": []}))
    }
    async fn navigate(&self, _q: &str, _k: usize, _d: u32) -> anyhow::Result<serde_json::Value> {
        Ok(json!({"entities": [], "plan": "test"}))
    }
    async fn graph_stats(&self) -> anyhow::Result<serde_json::Value> {
        Ok(json!({"entity_count": 0, "edge_count": 0, "kinds": []}))
    }
}

async fn send_json(w: &mut tokio::io::WriteHalf<tokio::io::DuplexStream>, msg: serde_json::Value) {
    let line = msg.to_string() + "\n";
    w.write_all(line.as_bytes()).await.unwrap();
    w.flush().await.unwrap();
}

async fn recv_json(
    r: &mut BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>,
) -> serde_json::Value {
    let mut line = String::new();
    r.read_line(&mut line).await.unwrap();
    serde_json::from_str(&line).unwrap()
}

#[tokio::test]
async fn mcp_server_initializes_and_lists_tools() {
    let (server_stream, client_stream) = tokio::io::duplex(4096);
    let (client_read, client_write) = tokio::io::split(client_stream);
    let mut client_reader = BufReader::new(client_read);
    let mut client_writer = client_write;

    let server = AtheneumMcpServer::new(Arc::new(MockBackend));
    let server_handle = tokio::spawn(async move {
        let running = server.serve(server_stream).await.unwrap();
        running.waiting().await.unwrap();
    });

    // Initialize
    send_json(
        &mut client_writer,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": { "name": "test-client", "version": "0.0.1" }
            }
        }),
    )
    .await;

    let init_response = recv_json(&mut client_reader).await;
    assert_eq!(init_response["id"], 1);
    assert!(init_response["result"]["serverInfo"]["name"]
        .as_str()
        .unwrap()
        .contains("atheneum-mcp"));

    // Initialized notification
    send_json(
        &mut client_writer,
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
    )
    .await;

    // List tools
    send_json(
        &mut client_writer,
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }),
    )
    .await;

    let list_response = recv_json(&mut client_reader).await;
    assert_eq!(list_response["id"], 2);
    let tools = list_response["result"]["tools"].as_array().unwrap();
    let names: Vec<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"graph_stats"));
    assert!(names.contains(&"search"));
    assert!(names.contains(&"navigate"));
    assert_eq!(tools.len(), 9);

    // Call graph_stats tool
    send_json(
        &mut client_writer,
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "graph_stats",
                "arguments": {}
            }
        }),
    )
    .await;

    let tool_response = recv_json(&mut client_reader).await;
    assert_eq!(tool_response["id"], 3);
    let content = tool_response["result"]["content"].as_array().unwrap();
    assert!(!content.is_empty());
    let text = content[0]["text"].as_str().unwrap();
    assert!(text.contains("entity_count"));

    // Clean shutdown
    drop(client_writer);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), server_handle).await;
}

#[tokio::test]
async fn mcp_tool_call_with_args() {
    let (server_stream, client_stream) = tokio::io::duplex(4096);
    let (client_read, client_write) = tokio::io::split(client_stream);
    let mut client_reader = BufReader::new(client_read);
    let mut client_writer = client_write;

    let server = AtheneumMcpServer::new(Arc::new(MockBackend));
    let server_handle = tokio::spawn(async move {
        let running = server.serve(server_stream).await.unwrap();
        running.waiting().await.unwrap();
    });

    // Initialize
    send_json(
        &mut client_writer,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": { "name": "test-client", "version": "0.0.1" }
            }
        }),
    )
    .await;
    let _ = recv_json(&mut client_reader).await;

    send_json(
        &mut client_writer,
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
    )
    .await;

    // Call search with args
    send_json(
        &mut client_writer,
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "search",
                "arguments": {
                    "query": "test query",
                    "k": 5
                }
            }
        }),
    )
    .await;

    let tool_response = recv_json(&mut client_reader).await;
    assert_eq!(tool_response["id"], 2);
    let content = tool_response["result"]["content"].as_array().unwrap();
    let text = content[0]["text"].as_str().unwrap();
    assert!(text.contains("results"));

    // Clean shutdown
    drop(client_writer);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), server_handle).await;
}
