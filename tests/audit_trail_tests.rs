//! Stage 6: audit-trail wiring tests.
//!
//! Verifies that `(Agent)-[PerformedBy]->(ReasoningLog)-[Called]->(ToolCall)-[Modified]->(Target)`
//! provenance chains actually land in the graph when written via the new
//! helpers, and that we can walk them back via the read API.
//!
//! Edge-type mapping (Rust EdgeType ↔ Python label):
//!   PerformedBy  ↔ THOUGHT
//!   Called       ↔ USED
//!   Modified     ↔ MODIFIED

use atheneum::graph::{ActionRecord, AtheneumGraph, EdgeType, EntityType, ToolCallRecord};
use serde_json::json;

#[test]
fn test_insert_reasoning_log_links_to_agent() {
    let graph = AtheneumGraph::open_in_memory().expect("open");

    let log_id = graph
        .insert_reasoning_log("claude1", "decided to port Stage 4 first", Some("envoy"))
        .expect("insert");
    assert!(log_id > 0);

    let log = graph.get_entity(log_id).expect("retrieve");
    assert_eq!(log.kind, "ReasoningLog");
    assert_eq!(log.data["content"], json!("decided to port Stage 4 first"));
    assert_eq!(log.data["agent"], json!("claude1"));
    assert_eq!(log.data["project_id"], json!("envoy"));

    // There should be an incoming PerformedBy edge from the agent
    let incoming = graph.incoming_edges(log_id).expect("incoming");
    assert!(
        incoming
            .iter()
            .any(|e| e.edge_type == EdgeType::PerformedBy.as_str()),
        "must have a PerformedBy edge from the agent (got: {:?})",
        incoming
            .iter()
            .map(|e| e.edge_type.clone())
            .collect::<Vec<_>>(),
    );
}

#[test]
fn test_insert_tool_call_links_to_reasoning_log() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let log_id = graph
        .insert_reasoning_log("claude1", "deciding", Some("envoy"))
        .expect("log");
    let tool_id = graph
        .insert_tool_call(
            log_id,
            "magellan_find",
            json!({"name": "Message"}),
            Some("envoy"),
        )
        .expect("tool");

    let tool = graph.get_entity(tool_id).expect("retrieve");
    assert_eq!(tool.kind, "ToolCall");
    assert_eq!(tool.data["tool_name"], json!("magellan_find"));
    assert_eq!(tool.data["args"]["name"], json!("Message"));
    assert_eq!(tool.data["project_id"], json!("envoy"));

    // Should be a Called edge from log → tool
    let outgoing = graph.outgoing_edges(log_id).expect("outgoing");
    assert!(
        outgoing
            .iter()
            .any(|e| e.to_id == tool_id && e.edge_type == EdgeType::Called.as_str()),
        "must have a Called edge from ReasoningLog to ToolCall"
    );
}

#[test]
fn test_record_tool_modifies_creates_modified_edge() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let log_id = graph
        .insert_reasoning_log("claude1", "edit decision", None)
        .expect("log");
    let tool_id = graph
        .insert_tool_call(log_id, "edit_file", json!({"path": "src/lib.rs"}), None)
        .expect("tool");

    // Use a discovery entity as the target — it could just as well be a
    // FileChange or CodeSymbol, what matters is that *some* entity exists.
    let target_id = graph
        .store_discovery(
            "claude1",
            "Symbol",
            "lib_root",
            json!({"file": "src/lib.rs"}),
        )
        .expect("target entity");

    let edge_id = graph
        .record_tool_modifies(tool_id, target_id)
        .expect("modifies edge");
    assert!(edge_id > 0);

    let outgoing = graph.outgoing_edges(tool_id).expect("outgoing");
    assert!(
        outgoing
            .iter()
            .any(|e| { e.to_id == target_id && e.edge_type == EdgeType::Modified.as_str() }),
        "Modified edge from ToolCall to target must exist"
    );
}

#[test]
fn test_record_agent_action_writes_full_chain() {
    let graph = AtheneumGraph::open_in_memory().expect("open");

    let target_a = graph
        .store_discovery("claude1", "Symbol", "fn_a", json!({}))
        .expect("target a");
    let target_b = graph
        .store_discovery("claude1", "Symbol", "fn_b", json!({}))
        .expect("target b");

    let trace = graph
        .record_agent_action(
            "claude1",
            "rename fn_a → fn_b and update callers",
            vec![
                ToolCallRecord {
                    tool_name: "splice_rename".to_string(),
                    args: json!({"from": "fn_a", "to": "fn_b"}),
                    modified_targets: vec![target_a, target_b],
                },
                ToolCallRecord {
                    tool_name: "cargo_test".to_string(),
                    args: json!({}),
                    modified_targets: vec![],
                },
            ],
            Some("envoy"),
        )
        .expect("record");

    assert!(trace.reasoning_log_id > 0);
    assert_eq!(trace.tool_call_ids.len(), 2);
    assert_eq!(
        trace.modified_edge_ids.len(),
        2,
        "two modified targets → two Modified edges (got {})",
        trace.modified_edge_ids.len()
    );

    // The ReasoningLog must be reachable from the agent via PerformedBy
    let agent_id = graph
        .entities_by_kind(EntityType::Agent.as_str())
        .expect("agents")
        .into_iter()
        .find(|a| a.name == "claude1")
        .expect("claude1 should exist")
        .id;
    let agent_out = graph.outgoing_edges(agent_id).expect("agent edges");
    assert!(
        agent_out
            .iter()
            .any(|e| e.to_id == trace.reasoning_log_id
                && e.edge_type == EdgeType::PerformedBy.as_str()),
        "agent must have PerformedBy edge to the reasoning log"
    );
}

#[test]
fn test_get_action_trace_walks_chain_back() {
    let graph = AtheneumGraph::open_in_memory().expect("open");

    let target = graph
        .store_discovery("a1", "Symbol", "thing", json!({}))
        .expect("target");

    graph
        .record_agent_action(
            "a1",
            "first action",
            vec![ToolCallRecord {
                tool_name: "edit".to_string(),
                args: json!({}),
                modified_targets: vec![target],
            }],
            Some("envoy"),
        )
        .expect("record");

    graph
        .record_agent_action(
            "a1",
            "second action",
            vec![ToolCallRecord {
                tool_name: "cargo_test".to_string(),
                args: json!({}),
                modified_targets: vec![],
            }],
            Some("envoy"),
        )
        .expect("record");

    let records: Vec<ActionRecord> = graph.get_action_trace("a1", Some("envoy")).expect("trace");
    assert_eq!(records.len(), 2, "two actions recorded");

    // The first action should have one tool call with one modified target
    let first = records
        .iter()
        .find(|r| r.reasoning_log.data["content"] == json!("first action"))
        .expect("first action");
    assert_eq!(first.tool_calls.len(), 1);
    assert_eq!(
        first.tool_calls[0].tool_call.data["tool_name"],
        json!("edit")
    );
    assert_eq!(first.tool_calls[0].modified.len(), 1);
    assert_eq!(first.tool_calls[0].modified[0].id, target);
}

#[test]
fn test_action_trace_filtered_by_project() {
    let graph = AtheneumGraph::open_in_memory().expect("open");

    graph
        .record_agent_action("shared_agent", "envoy work", vec![], Some("envoy"))
        .expect("envoy action");
    graph
        .record_agent_action("shared_agent", "magellan work", vec![], Some("magellan"))
        .expect("magellan action");

    let envoy_only = graph
        .get_action_trace("shared_agent", Some("envoy"))
        .expect("envoy");
    assert_eq!(envoy_only.len(), 1);
    assert_eq!(
        envoy_only[0].reasoning_log.data["content"],
        json!("envoy work"),
    );

    let mag_only = graph
        .get_action_trace("shared_agent", Some("magellan"))
        .expect("magellan");
    assert_eq!(mag_only.len(), 1);
    assert_eq!(
        mag_only[0].reasoning_log.data["content"],
        json!("magellan work"),
    );
}
