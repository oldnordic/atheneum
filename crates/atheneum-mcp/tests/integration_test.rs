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
    async fn search(&self, _params: backend::SearchParams) -> anyhow::Result<serde_json::Value> {
        Ok(json!({"results": [], "count": 0}))
    }
    async fn store_memory(
        &self,
        _p: backend::StoreMemoryParams,
    ) -> anyhow::Result<serde_json::Value> {
        Ok(json!({"memory_id": 7}))
    }
    async fn update_memory(
        &self,
        p: backend::UpdateMemoryParams,
    ) -> anyhow::Result<serde_json::Value> {
        // Mirror the contract: empty patch is an error surfaced by the direct
        // backend; here we just echo the id so the integration test can assert
        // the route + schema are wired.
        Ok(json!({"memory_id": p.id, "echoed": true}))
    }
    async fn add_memory(&self, p: backend::AddMemoryParams) -> anyhow::Result<serde_json::Value> {
        Ok(json!({"memory_id": 8, "concept": p.concept, "action": "CREATED"}))
    }
    async fn maintain(&self, p: backend::MaintainParams) -> anyhow::Result<serde_json::Value> {
        Ok(
            json!({"orphans_rewired": 0, "broken_links_resolved": 0, "contradictions_superseded": 0, "stale_superseded_pruned": 0, "expired_memories_pruned": 0, "apply": p.apply}),
        )
    }
    async fn query_memory(
        &self,
        _p: backend::QueryMemoryParams,
    ) -> anyhow::Result<serde_json::Value> {
        Ok(json!({"results": []}))
    }
    async fn list_sessions(&self, _l: i64) -> anyhow::Result<serde_json::Value> {
        Ok(json!({"sessions": []}))
    }
    async fn seed_memory(
        &self,
        _p: backend::SeedMemoryParams,
    ) -> anyhow::Result<serde_json::Value> {
        Ok(json!({
            "instructions": "Mock seed instructions",
            "token_estimate": 15
        }))
    }
    async fn list_events(&self, _l: i64) -> anyhow::Result<serde_json::Value> {
        Ok(json!({"events": []}))
    }
    async fn navigate(&self, _p: backend::NavigateParams) -> anyhow::Result<serde_json::Value> {
        Ok(json!({"subgraphs": [], "count": 0}))
    }
    async fn code_query(&self, _p: backend::CodeQueryParams) -> anyhow::Result<serde_json::Value> {
        Ok(serde_json::Value::Null)
    }
    async fn graph_stats(&self) -> anyhow::Result<serde_json::Value> {
        Ok(json!({"entity_count": 0, "edge_count": 0, "kinds": []}))
    }
    async fn search_memory(
        &self,
        _q: &str,
        _k: usize,
        _p: Option<&str>,
    ) -> anyhow::Result<serde_json::Value> {
        Ok(json!({"results": []}))
    }
    async fn list_memory(
        &self,
        _s: Option<&str>,
        _p: Option<&str>,
        _o: usize,
        _l: usize,
    ) -> anyhow::Result<serde_json::Value> {
        Ok(json!({"items": []}))
    }
    async fn memory_bootstrap(
        &self,
        _p: Option<&str>,
        _t: usize,
        _l: i64,
    ) -> anyhow::Result<serde_json::Value> {
        Ok(json!({"memories": []}))
    }
    async fn query_wiki(&self, _path: &str) -> anyhow::Result<serde_json::Value> {
        Ok(json!({"found": false}))
    }
    async fn wiki_search(
        &self,
        _q: &str,
        _p: Option<&str>,
        _l: usize,
    ) -> anyhow::Result<serde_json::Value> {
        Ok(json!({"results": []}))
    }
    async fn discoveries_recent(
        &self,
        _p: Option<&str>,
        _a: Option<&str>,
        _s: Option<&str>,
        _t: Option<&str>,
        _l: i64,
    ) -> anyhow::Result<serde_json::Value> {
        Ok(json!({"discoveries": []}))
    }
    async fn decision_search(
        &self,
        _q: &str,
        _p: Option<&str>,
        _l: i64,
    ) -> anyhow::Result<serde_json::Value> {
        Ok(json!({"decisions": []}))
    }
    async fn thread(
        &self,
        _q: &str,
        _k: usize,
        _d: u32,
        _p: Option<&str>,
        _t: usize,
    ) -> anyhow::Result<serde_json::Value> {
        Ok(json!([]))
    }
    async fn session_digest(&self, _p: Option<&str>, _l: i64) -> anyhow::Result<serde_json::Value> {
        Ok(json!({"digest": {}}))
    }
    async fn get_entity(&self, _id: i64) -> anyhow::Result<serde_json::Value> {
        Ok(json!({"entity": null}))
    }
    async fn get_neighbors(&self, _id: i64) -> anyhow::Result<serde_json::Value> {
        Ok(json!({"outgoing": [], "incoming": []}))
    }
    async fn dream(
        &self,
        _s: Option<&str>,
        _p: Option<&str>,
        _d: bool,
    ) -> anyhow::Result<serde_json::Value> {
        Ok(json!({"findings": []}))
    }
    async fn list_models(&self) -> anyhow::Result<serde_json::Value> {
        Ok(serde_json::json!([]))
    }
    async fn dream_semantic(
        &self,
        _params: backend::DreamSemanticParams,
    ) -> anyhow::Result<serde_json::Value> {
        Ok(serde_json::json!({ "merges_completed": 0, "details": [] }))
    }
    async fn pin_entity(&self, id: i64) -> anyhow::Result<serde_json::Value> {
        Ok(serde_json::json!({ "status": "success", "id": id, "pinned": true }))
    }
    async fn unpin_entity(&self, id: i64) -> anyhow::Result<serde_json::Value> {
        Ok(serde_json::json!({ "status": "success", "id": id, "pinned": false }))
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
    assert!(names.contains(&"wiki_search"));
    assert!(names.contains(&"decision_search"));
    assert!(names.contains(&"add_memory"));
    assert!(names.contains(&"maintain"));
    assert!(names.contains(&"seed_memory"));
    assert!(names.contains(&"code_query"));
    assert_eq!(tools.len(), 30);

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

// ---------------------------------------------------------------------------
// End-to-end test with real AtheneumGraph (requires --features direct)
// ---------------------------------------------------------------------------

#[cfg(feature = "direct")]
#[tokio::test(flavor = "multi_thread")]
async fn mcp_direct_backend_round_trip() {
    use atheneum::AtheneumGraph;
    use atheneum_mcp::backend::direct::DirectBackend;
    use std::sync::Arc;

    let (server_stream, client_stream) = tokio::io::duplex(4096);
    let (client_read, client_write) = tokio::io::split(client_stream);
    let mut client_reader = BufReader::new(client_read);
    let mut client_writer = client_write;

    let graph = Arc::new(tokio::sync::Mutex::new(
        AtheneumGraph::open_in_memory().unwrap(),
    ));
    let server = AtheneumMcpServer::new(Arc::new(DirectBackend::new(graph)));
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

    // 1. graph_stats on empty graph
    send_json(
        &mut client_writer,
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": { "name": "graph_stats", "arguments": {} }
        }),
    )
    .await;
    let stats_resp = recv_json(&mut client_reader).await;
    let stats_text = stats_resp["result"]["content"][0]["text"].as_str().unwrap();
    let stats: serde_json::Value = serde_json::from_str(stats_text).unwrap();
    assert_eq!(
        stats["entity_count"].as_i64(),
        Some(0),
        "graph should start empty"
    );

    // 2. store_memory
    send_json(
        &mut client_writer,
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "store_memory",
                "arguments": {
                    "content": "User prefers dark mode",
                    "tags": ["preference", "ui"],
                    "importance": 8
                }
            }
        }),
    )
    .await;
    let mem_resp = recv_json(&mut client_reader).await;
    let mem_text = mem_resp["result"]["content"][0]["text"].as_str().unwrap();
    let mem: serde_json::Value = serde_json::from_str(mem_text).unwrap();
    assert!(
        mem["memory_id"].as_i64().unwrap() > 0,
        "memory_id should be positive"
    );
    assert_eq!(
        mem["tags"].as_array().unwrap().len(),
        2,
        "tags should be preserved"
    );

    // 3. query_memory (exact key lookup)
    send_json(
        &mut client_writer,
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "query_memory",
                "arguments": { "key": "User prefers dark mode", "k": 5 }
            }
        }),
    )
    .await;
    let qmem_resp = recv_json(&mut client_reader).await;
    let qmem_text = qmem_resp["result"]["content"][0]["text"].as_str().unwrap();
    let qmem: serde_json::Value = serde_json::from_str(qmem_text).unwrap();
    let results = qmem.as_array().unwrap();
    assert!(
        !results.is_empty(),
        "query_memory should find stored memory by key"
    );
    let memory_id = results[0]["id"].as_i64().expect("memory entity has id");

    // 3b. update_memory — patch content + tags in place, assert no duplicate row
    send_json(
        &mut client_writer,
        json!({
            "jsonrpc": "2.0",
            "id": 99,
            "method": "tools/call",
            "params": {
                "name": "update_memory",
                "arguments": {
                    "id": memory_id,
                    "content": "User prefers light mode",
                    "tags": ["preference", "ui", "updated"],
                }
            }
        }),
    )
    .await;
    let upd_resp = recv_json(&mut client_reader).await;
    assert_eq!(upd_resp["id"], 99);
    let upd_text = upd_resp["result"]["content"][0]["text"].as_str().unwrap();
    let upd: serde_json::Value = serde_json::from_str(upd_text).unwrap();
    assert_eq!(
        upd["memory_id"].as_i64(),
        Some(memory_id),
        "update_memory must return the same id (no duplicate)"
    );
    assert_eq!(
        upd["content"].as_str(),
        Some("User prefers light mode"),
        "content must be patched"
    );
    assert_eq!(
        upd["tags"].as_array().map(|a| a.len()),
        Some(3),
        "tags must be merged"
    );

    // 3c. query_memory again — exactly one row, with the new content
    send_json(
        &mut client_writer,
        json!({
            "jsonrpc": "2.0",
            "id": 100,
            "method": "tools/call",
            "params": {
                "name": "query_memory",
                "arguments": { "key": "User prefers dark mode", "k": 5 }
            }
        }),
    )
    .await;
    let qmem2_resp = recv_json(&mut client_reader).await;
    let qmem2_text = qmem2_resp["result"]["content"][0]["text"].as_str().unwrap();
    let qmem2: serde_json::Value = serde_json::from_str(qmem2_text).unwrap();
    let results2 = qmem2.as_array().unwrap();
    assert_eq!(
        results2.len(),
        1,
        "no duplicate memory row after update_memory"
    );
    assert_eq!(
        results2[0]["data"]["content"].as_str(),
        Some("User prefers light mode"),
        "query must reflect patched content"
    );

    // 3d. add_memory — create concept + memory
    send_json(
        &mut client_writer,
        json!({
            "jsonrpc": "2.0",
            "id": 101,
            "method": "tools/call",
            "params": {
                "name": "add_memory",
                "arguments": {
                    "concept": "Editor Preference",
                    "body_patch": "Vim mode",
                    "link_from": memory_id,
                    "link_both_ways": true,
                }
            }
        }),
    )
    .await;
    let add_resp = recv_json(&mut client_reader).await;
    assert_eq!(add_resp["id"], 101);
    let add_text = add_resp["result"]["content"][0]["text"].as_str().unwrap();
    let add_val: serde_json::Value = serde_json::from_str(add_text).unwrap();
    assert_eq!(
        add_val["action"].as_str(),
        Some("CREATED"),
        "should create concept and memory"
    );
    let editor_memory_id = add_val["memory_id"].as_i64().unwrap();

    // 3e. add_memory — enrich existing concept memory
    send_json(
        &mut client_writer,
        json!({
            "jsonrpc": "2.0",
            "id": 102,
            "method": "tools/call",
            "params": {
                "name": "add_memory",
                "arguments": {
                    "concept": "Editor Preference",
                    "body_patch": "Use absolute line numbers",
                }
            }
        }),
    )
    .await;
    let enrich_resp = recv_json(&mut client_reader).await;
    assert_eq!(enrich_resp["id"], 102);
    let enrich_text = enrich_resp["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    let enrich_val: serde_json::Value = serde_json::from_str(enrich_text).unwrap();
    assert_eq!(
        enrich_val["action"].as_str(),
        Some("ENRICHED"),
        "should enrich existing concept memory"
    );
    assert_eq!(enrich_val["memory_id"].as_i64().unwrap(), editor_memory_id);
    assert_eq!(
        enrich_val["content"].as_str(),
        Some("Vim mode\nUse absolute line numbers"),
        "content must be enriched"
    );

    // 3f. maintain
    send_json(
        &mut client_writer,
        json!({
            "jsonrpc": "2.0",
            "id": 103,
            "method": "tools/call",
            "params": {
                "name": "maintain",
                "arguments": {
                    "apply": true,
                    "stale_superseded_days": 15,
                    "broken_link_mode": "sever",
                    "rewire_threshold": 0.5,
                }
            }
        }),
    )
    .await;
    let maintain_resp = recv_json(&mut client_reader).await;
    assert_eq!(maintain_resp["id"], 103);
    let maintain_text = maintain_resp["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    let maintain_val: serde_json::Value = serde_json::from_str(maintain_text).unwrap();
    assert_eq!(maintain_val["broken_links_resolved"].as_u64(), Some(0));

    // 3g. seed_memory
    send_json(
        &mut client_writer,
        json!({
            "jsonrpc": "2.0",
            "id": 104,
            "method": "tools/call",
            "params": {
                "name": "seed_memory",
                "arguments": {
                    "tokens": 400
                }
            }
        }),
    )
    .await;
    let seed_resp = recv_json(&mut client_reader).await;
    assert_eq!(seed_resp["id"], 104);
    let seed_text = seed_resp["result"]["content"][0]["text"].as_str().unwrap();
    let seed_val: serde_json::Value = serde_json::from_str(seed_text).unwrap();
    assert!(seed_val["instructions"].as_str().is_some());

    // 3h. navigate with trace
    send_json(
        &mut client_writer,
        json!({
            "jsonrpc": "2.0",
            "id": 105,
            "method": "tools/call",
            "params": {
                "name": "navigate",
                "arguments": {
                    "query": "Rust Style",
                    "trace": true
                }
            }
        }),
    )
    .await;
    let nav_resp = recv_json(&mut client_reader).await;
    assert_eq!(nav_resp["id"], 105);
    let nav_text = nav_resp["result"]["content"][0]["text"].as_str().unwrap();
    let nav_val: serde_json::Value = serde_json::from_str(nav_text).unwrap();
    assert!(nav_val["trace_id"].as_i64().is_some());

    // 4. store_discovery
    send_json(
        &mut client_writer,
        json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "store_discovery",
                "arguments": {
                    "target": "memory-leak",
                    "observation": "Connection pool not released",
                    "confidence": 0.95,
                    "tags": ["bug", "performance"]
                }
            }
        }),
    )
    .await;
    let disc_resp = recv_json(&mut client_reader).await;
    let disc_text = disc_resp["result"]["content"][0]["text"].as_str().unwrap();
    let disc: serde_json::Value = serde_json::from_str(disc_text).unwrap();
    assert!(
        disc["discovery_id"].as_i64().unwrap() > 0,
        "discovery_id should be positive"
    );

    // 5. query_knowledge
    send_json(
        &mut client_writer,
        json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {
                "name": "query_knowledge",
                "arguments": { "target": "memory-leak" }
            }
        }),
    )
    .await;
    let know_resp = recv_json(&mut client_reader).await;
    let know_text = know_resp["result"]["content"][0]["text"].as_str().unwrap();
    let know: serde_json::Value = serde_json::from_str(know_text).unwrap();
    let discovery_count = know["discovery_count"].as_i64().unwrap_or(0);
    assert!(discovery_count > 0, "query_knowledge should find discovery");

    // 6. graph_stats after mutations
    send_json(
        &mut client_writer,
        json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": { "name": "graph_stats", "arguments": {} }
        }),
    )
    .await;
    let stats2_resp = recv_json(&mut client_reader).await;
    let stats2_text = stats2_resp["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    let stats2: serde_json::Value = serde_json::from_str(stats2_text).unwrap();
    assert!(
        stats2["entity_count"].as_i64().unwrap() > 0,
        "graph should have entities after store_memory + store_discovery"
    );

    // Clean shutdown
    drop(client_writer);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), server_handle).await;
}
