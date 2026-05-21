//! Tests for Envoy-Atheneum Bridge API
//! Tests are written FIRST (TDD) and will fail until implementation is complete.
//!
//! Phase 1: atheneum core APIs
//! - Discovery entity (agent findings: symbols, CFG, issues, patterns)
//! - Handoff entity (context transfer between agents)
//! - Knowledge Query (aggregate discoveries + handoffs + token savings)

use atheneum::graph::{AtheneumGraph, EntityType};
use serde_json::json;

// ============================================================================
// Discovery API Tests
// ============================================================================

#[test]
fn test_store_discovery_symbol() {
    let graph = AtheneumGraph::open_in_memory().expect("Failed to create graph");

    let discovery_id = graph
        .store_discovery(
            "claude1",
            "symbol",
            "http_handler",
            json!({
                "file_path": "src/http.rs",
                "line": 42,
                "kind": "function",
                "signature": "pub fn http_handler(req: Request) -> Response"
            }),
        )
        .expect("Failed to store discovery");

    assert!(discovery_id > 0, "Discovery ID should be positive");

    let discovery = graph
        .get_entity(discovery_id)
        .expect("Failed to retrieve discovery");

    assert_eq!(discovery.kind, EntityType::Discovery.as_str());
    assert_eq!(discovery.name, "claude1: http_handler");
}

#[test]
fn test_store_discovery_cfg() {
    let graph = AtheneumGraph::open_in_memory().expect("Failed to create graph");

    let discovery_id = graph
        .store_discovery(
            "claude2",
            "cfg",
            "process_request",
            json!({
                "file_path": "src/http.rs",
                "function": "process_request",
                "cyclomatic_complexity": 8,
                "has_loop": true,
                "branches": 4
            }),
        )
        .expect("Failed to store discovery");

    let discovery = graph
        .get_entity(discovery_id)
        .expect("Failed to retrieve discovery");

    let data = discovery.data.as_object().expect("Data should be object");
    assert_eq!(data["discovery_type"], "cfg");
    assert_eq!(data["target"], "process_request");
}

#[test]
fn test_query_discoveries_by_target() {
    let graph = AtheneumGraph::open_in_memory().expect("Failed to create graph");

    // Agent A discovers a symbol
    graph
        .store_discovery(
            "claude1",
            "symbol",
            "http_handler",
            json!({"file": "src/http.rs", "line": 42}),
        )
        .expect("Failed to store discovery A");

    // Agent B also discovers the same symbol (from a different angle)
    graph
        .store_discovery(
            "claude2",
            "cfg",
            "http_handler",
            json!({"complexity": 5, "file": "src/http.rs"}),
        )
        .expect("Failed to store discovery B");

    // Query for all discoveries about http_handler
    let discoveries = graph
        .query_discoveries("http_handler")
        .expect("Failed to query discoveries");

    assert_eq!(discoveries.len(), 2, "Should find 2 discoveries");

    // Verify both agents are represented
    let agents: Vec<_> = discoveries
        .iter()
        .filter_map(|d| d.data.get("agent"))
        .map(|a| a.as_str())
        .collect();

    assert!(agents.contains(&Some("claude1")), "Should include claude1");
    assert!(agents.contains(&Some("claude2")), "Should include claude2");
}

#[test]
fn test_query_discoveries_empty() {
    let graph = AtheneumGraph::open_in_memory().expect("Failed to create graph");

    let discoveries = graph
        .query_discoveries("nonexistent")
        .expect("Failed to query");

    assert!(
        discoveries.is_empty(),
        "Should return empty for unknown target"
    );
}

#[test]
fn test_store_discovery_creates_provenance() {
    let graph = AtheneumGraph::open_in_memory().expect("Failed to create graph");

    let discovery_id = graph
        .store_discovery(
            "hermes",
            "pattern",
            "clone-escape-hatch",
            json!({"description": "Excessive .clone() calls"}),
        )
        .expect("Failed to store discovery");

    // Check that the discovery has provenance metadata
    let discovery = graph
        .get_entity(discovery_id)
        .expect("Failed to retrieve discovery");

    let data = discovery.data.as_object().expect("Data should be object");
    assert!(data.contains_key("agent"), "Should have agent field");
    assert!(data.contains_key("timestamp"), "Should have timestamp");
    assert_eq!(data["agent"], "hermes");
}

// ============================================================================
// Handoff API Tests
// ============================================================================

#[test]
fn test_store_handoff() {
    let graph = AtheneumGraph::open_in_memory().expect("Failed to create graph");

    let manifest = json!({
        "task": "implement auth",
        "files_analyzed": ["src/auth.rs", "src/user.rs"],
        "discoveries": 5,
        "token_budget_used": 45000,
        "token_budget_remaining": 45000,
        "next_steps": ["Add JWT validation", "Write tests"]
    });

    let handoff_id = graph
        .store_handoff("claude1", "claude2", manifest)
        .expect("Failed to store handoff");

    assert!(handoff_id > 0, "Handoff ID should be positive");

    let handoff = graph
        .get_entity(handoff_id)
        .expect("Failed to retrieve handoff");
    assert_eq!(handoff.kind, EntityType::Handoff.as_str());
    assert_eq!(handoff.name, "claude1 -> claude2");

    let data = handoff.data.as_object().expect("Data should be object");
    assert_eq!(data["from_agent"], "claude1");
    assert_eq!(data["to_agent"], "claude2");
    assert!(data.contains_key("manifest"));
}

#[test]
fn test_get_pending_handoff() {
    let graph = AtheneumGraph::open_in_memory().expect("Failed to create graph");

    // Create a handoff for claude2
    let manifest = json!({"task": "fix bug", "context": "analyzed"});
    graph
        .store_handoff("claude1", "claude2", manifest)
        .expect("Failed to store handoff");

    // claude2 checks for pending handoffs
    let pending = graph
        .get_pending_handoff("claude2")
        .expect("Failed to get pending handoff");

    assert!(pending.is_some(), "Should have a pending handoff");

    let handoff = pending.unwrap();
    assert_eq!(handoff.data["from_agent"], "claude1");
    assert_eq!(handoff.data["to_agent"], "claude2");
}

#[test]
fn test_get_pending_handoff_none() {
    let graph = AtheneumGraph::open_in_memory().expect("Failed to create graph");

    // Create a handoff for claude3, not claude2
    graph
        .store_handoff("claude1", "claude3", json!({}))
        .expect("Failed to store handoff");

    // claude2 checks for pending handoffs
    let pending = graph
        .get_pending_handoff("claude2")
        .expect("Failed to get pending handoff");

    assert!(
        pending.is_none(),
        "Should have no pending handoffs for claude2"
    );
}

#[test]
fn test_get_pending_handoff_returns_most_recent() {
    let graph = AtheneumGraph::open_in_memory().expect("Failed to create graph");

    // Create multiple handoffs for claude2
    graph
        .store_handoff("claude1", "claude2", json!({"seq": 1}))
        .expect("Failed to store handoff 1");

    graph
        .store_handoff("hermes", "claude2", json!({"seq": 2}))
        .expect("Failed to store handoff 2");

    let pending = graph
        .get_pending_handoff("claude2")
        .expect("Failed to get pending handoff");

    assert!(pending.is_some(), "Should have a pending handoff");

    // Should return the most recent (highest ID)
    assert_eq!(pending.unwrap().data["manifest"]["seq"], 2);
}

#[test]
fn test_handoff_marked_claimed() {
    let graph = AtheneumGraph::open_in_memory().expect("Failed to create graph");

    let handoff_id = graph
        .store_handoff("claude1", "claude2", json!({"task": "test"}))
        .expect("Failed to store handoff");

    // Mark as claimed
    graph
        .mark_handoff_claimed(handoff_id)
        .expect("Failed to mark handoff");

    // Should no longer appear in pending
    let pending = graph
        .get_pending_handoff("claude2")
        .expect("Failed to get pending handoff");

    assert!(pending.is_none(), "Claimed handoff should not be pending");
}

// ============================================================================
// Knowledge Query API Tests
// ============================================================================

#[test]
fn test_query_knowledge_aggregates_discoveries() {
    let graph = AtheneumGraph::open_in_memory().expect("Failed to create graph");

    // Multiple agents discover the same symbol
    graph
        .store_discovery(
            "claude1",
            "symbol",
            "http_handler",
            json!({"file": "src/http.rs", "lines": 50}),
        )
        .expect("Failed to store discovery 1");

    graph
        .store_discovery("claude2", "cfg", "http_handler", json!({"complexity": 8}))
        .expect("Failed to store discovery 2");

    graph
        .store_discovery(
            "claude3",
            "issue",
            "http_handler",
            json!({"issue": "missing error handling"}),
        )
        .expect("Failed to store discovery 3");

    let knowledge = graph
        .query_knowledge("http_handler")
        .expect("Failed to query knowledge");

    assert_eq!(knowledge["target"], "http_handler");
    assert_eq!(knowledge["discovery_count"], 3);

    let discoveries = knowledge["discoveries"].as_array().unwrap();
    assert_eq!(discoveries.len(), 3);
}

#[test]
fn test_query_knowledge_includes_handoffs() {
    let graph = AtheneumGraph::open_in_memory().expect("Failed to create graph");

    // A handoff mentioning the target
    graph
        .store_handoff(
            "claude1",
            "claude2",
            json!({
                "task": "fix http_handler",
                "files_analyzed": ["src/http.rs"],
                "token_budget_used": 30000
            }),
        )
        .expect("Failed to store handoff");

    let knowledge = graph
        .query_knowledge("http_handler")
        .expect("Failed to query knowledge");

    // Handoff should be included because it mentions the target in the task
    let handoffs = knowledge["handoffs"].as_array().unwrap();
    assert_eq!(handoffs.len(), 1);
    assert_eq!(handoffs[0]["data"]["from_agent"], "claude1");
}

#[test]
fn test_query_knowledge_calculates_token_savings() {
    let graph = AtheneumGraph::open_in_memory().expect("Failed to create graph");

    // Simulate 3 agents each reading a 15K token file
    // Without knowledge sharing: 45K tokens
    // With knowledge sharing: 15K (first agent) + 2.5K (summary) * 2 = 20K tokens

    graph
        .store_discovery(
            "claude1",
            "symbol",
            "large_file.rs",
            json!({"file": "src/large_file.rs", "token_count": 15000}),
        )
        .expect("Failed to store discovery 1");

    graph
        .store_discovery(
            "claude2",
            "cfg",
            "large_file.rs",
            json!({"file": "src/large_file.rs", "token_count": 15000}),
        )
        .expect("Failed to store discovery 2");

    graph
        .store_discovery(
            "claude3",
            "issue",
            "large_file.rs",
            json!({"file": "src/large_file.rs", "token_count": 15000}),
        )
        .expect("Failed to store discovery 3");

    let knowledge = graph
        .query_knowledge("large_file.rs")
        .expect("Failed to query knowledge");

    let savings = knowledge["token_savings"].as_object().unwrap();
    assert!(savings["without_sharing"].as_i64().unwrap() > 0);
    assert!(savings["with_sharing"].as_i64().unwrap() > 0);
    assert!(savings["saved"].as_i64().unwrap() > 0);
    assert!(savings["percentage_reduction"].as_f64().unwrap() > 0.0);
}

#[test]
fn test_query_knowledge_empty_target() {
    let graph = AtheneumGraph::open_in_memory().expect("Failed to create graph");

    let knowledge = graph
        .query_knowledge("unknown")
        .expect("Failed to query knowledge");

    assert_eq!(knowledge["target"], "unknown");
    assert_eq!(knowledge["discovery_count"], 0);
    assert!(knowledge["discoveries"].as_array().unwrap().is_empty());
}

#[test]
fn test_query_knowledge_includes_metadata() {
    let graph = AtheneumGraph::open_in_memory().expect("Failed to create graph");

    graph
        .store_discovery(
            "claude1",
            "symbol",
            "test_target",
            json!({"file": "test.rs"}),
        )
        .expect("Failed to store discovery");

    let knowledge = graph
        .query_knowledge("test_target")
        .expect("Failed to query knowledge");

    let obj = knowledge.as_object().expect("Knowledge should be object");
    assert!(obj.contains_key("queried_at"));
    assert!(obj.contains_key("total_entities"));
}
