//! Tests for graph navigation — neighbors, subgraph, navigate, stats.
//!
//! These tests drive TDD for:
//!   - get_entity / get_edge (already exist)
//!   - get_neighbors (NEW)
//!   - get_subgraph (NEW)
//!   - navigate (semantic search → subgraph) (NEW)
//!   - graph_stats (NEW)
//!   - auto-index on discovery write (NEW)

use atheneum::graph::{AtheneumGraph, EdgeType};
use serde_json::json;

fn make_chain_graph() -> anyhow::Result<(AtheneumGraph, i64, i64, i64)> {
    let g = AtheneumGraph::open_in_memory()?;

    let agent = g.insert_agent("claude1", json!({}))?;
    let discovery = g.store_discovery(
        "claude1",
        "Symbol",
        "build_router",
        json!({"file":"src/http.rs","summary":"builds axum router"}),
    )?;
    let event = g.insert_event(
        "discovery-stored",
        json!({"agent":"claude1","discovery_id":discovery}),
    )?;

    // event --Created--> discovery
    g.insert_edge(event, discovery, EdgeType::Created, json!({}))?;
    // event --PerformedBy--> agent
    g.insert_edge(event, agent, EdgeType::PerformedBy, json!({}))?;

    Ok((g, agent, discovery, event))
}

// ============================================================================
// Basics (already exist — regression smoke)
// ============================================================================

#[test]
fn test_get_entity() {
    let (g, _agent, discovery, _event) = make_chain_graph().expect("setup");

    let e = g.get_entity(discovery).expect("get_entity");
    assert_eq!(e.kind, "Discovery");
    assert_eq!(e.name, "claude1: build_router");
}

#[test]
fn test_get_edge() {
    let g = AtheneumGraph::open_in_memory().expect("open");
    let a = g.insert_agent("a", json!({})).expect("insert");
    let b = g.insert_agent("b", json!({})).expect("insert");
    let eid = g
        .insert_edge(a, b, EdgeType::RelatedTo, json!({}))
        .expect("edge");

    let edge = g.get_edge(eid).expect("get_edge");
    assert_eq!(edge.from_id, a);
    assert_eq!(edge.to_id, b);
    assert_eq!(edge.edge_type, "related_to");
}

// ============================================================================
// Neighbors (new)
// ============================================================================

#[test]
fn test_get_neighbors_returns_outgoing_and_incoming() {
    let (g, _agent, discovery, _event) = make_chain_graph().expect("setup");

    let (outgoing, incoming) = g.get_neighbors(discovery).expect("get_neighbors");

    // discovery has 1 incoming edge from event (Created)
    assert_eq!(incoming.len(), 1, "discovery should have 1 incoming edge");
    assert_eq!(incoming[0].edge_type, "created");

    // discovery has no outgoing edges
    assert_eq!(outgoing.len(), 0, "discovery should have 0 outgoing edges");
}

#[test]
fn test_get_neighbors_for_event() {
    let (g, _agent, _discovery, event) = make_chain_graph().expect("setup");

    let (outgoing, incoming) = g.get_neighbors(event).expect("get_neighbors");

    // Event has 2 outgoing edges: Created → discovery, PerformedBy → agent
    assert_eq!(outgoing.len(), 2, "event should have 2 outgoing edges");
    let types: Vec<&str> = outgoing.iter().map(|e| e.edge_type.as_str()).collect();
    assert!(types.contains(&"created"));
    assert!(types.contains(&"performed_by"));

    // Event has no incoming edges
    assert_eq!(incoming.len(), 0);
}

// ============================================================================
// Subgraph (new)
// ============================================================================

#[test]
fn test_get_subgraph_depth_0() {
    let (g, _agent, discovery, _event) = make_chain_graph().expect("setup");

    let sg = g.get_subgraph(discovery, 0).expect("get_subgraph");
    assert_eq!(sg.entry.id, discovery);
    assert_eq!(sg.depth, 0);
    assert!(sg.edges.is_empty());
    assert!(sg.entities.is_empty() || sg.entities.len() == 1); // entry-only at depth 0
}

#[test]
fn test_get_subgraph_depth_1() {
    let (g, agent, discovery, event) = make_chain_graph().expect("setup");

    let sg = g.get_subgraph(discovery, 1).expect("get_subgraph");

    // depth 1 from discovery: should reach the event
    let entity_ids: Vec<i64> = sg.entities.iter().map(|e| e.id).collect();
    assert!(
        entity_ids.contains(&event),
        "depth 1 should include event (the event that created discovery)"
    );
    // depth 1 should NOT reach agent (agent is 2 hops from discovery)
    assert!(
        !entity_ids.contains(&agent),
        "depth 1 should NOT include agent"
    );

    // should have at least 1 edge
    assert!(!sg.edges.is_empty(), "subgraph should have edges");
    let edge_types: Vec<&str> = sg.edges.iter().map(|e| e.edge_type.as_str()).collect();
    assert!(edge_types.contains(&"created") || edge_types.contains(&"performed_by"));
}

#[test]
fn test_get_subgraph_depth_2() {
    let (g, agent, discovery, _event) = make_chain_graph().expect("setup");

    let sg = g.get_subgraph(discovery, 2).expect("get_subgraph");

    let entity_ids: Vec<i64> = sg.entities.iter().map(|e| e.id).collect();
    assert!(
        entity_ids.contains(&agent),
        "depth 2 should reach agent (discovery→event→agent)"
    );
}

// ============================================================================
// Navigate — semantic search → subgraph (new)
// ============================================================================

#[test]
fn test_navigate_finds_entry_points_and_walks() {
    let (g, _agent, _discovery, _event) = make_chain_graph().expect("setup");

    // Navigate on "router" should find build_router → walk graph
    let views = g
        .navigate("router construction axum", 5, 2, None, None)
        .expect("navigate");

    assert!(
        !views.is_empty(),
        "navigate should return at least one subgraph view"
    );
    let sg = &views[0];
    // The entry should be a discovery about build_router
    assert!(
        sg.entry.name.contains("build_router"),
        "entry should be build_router: {}",
        sg.entry.name
    );
    // Depth 2 should have walked to the event and agent
    let names: Vec<&str> = sg.entities.iter().map(|e| e.name.as_str()).collect();
    assert!(
        names.iter().any(|n| n.contains("claude1: build_router")),
        "subgraph should contain discovery entity"
    );
}

#[test]
fn test_preview_navigate_query_repairs_plural_lowercase_kind() {
    let g = AtheneumGraph::open_in_memory().expect("open");

    let plan = g
        .preview_navigate_query("timezone", 5, 2, None, Some("memories"))
        .expect("preview_navigate_query");

    assert!(plan.executable, "repairable kind should stay executable");
    assert_eq!(plan.requested_kind.as_deref(), Some("memories"));
    assert_eq!(plan.resolved_kind.as_deref(), Some("Memory"));
    assert!(plan.kind_repaired, "kind should be marked repaired");
}

#[test]
fn test_navigate_repairs_lowercase_kind_filter() {
    let g = AtheneumGraph::open_in_memory().expect("open");
    g.store_memory("timezone", "UTC+0", "user", 1.0, None, None)
        .expect("store memory");

    let views = g
        .navigate("timezone", 5, 1, None, Some("memory"))
        .expect("navigate");

    assert!(!views.is_empty(), "navigate should repair lowercase kinds");
    assert_eq!(views[0].entry.kind, "Memory");
}

#[test]
fn test_preview_navigate_query_rejects_unknown_kind() {
    let g = AtheneumGraph::open_in_memory().expect("open");

    let plan = g
        .preview_navigate_query("timezone", 5, 2, None, Some("memz"))
        .expect("preview_navigate_query");

    assert!(!plan.executable, "unknown kind should be rejected");
    assert_eq!(plan.resolved_kind, None);
    assert!(
        !plan.errors.is_empty(),
        "rejected plans should explain why they are invalid"
    );
}

// ============================================================================
// Stats (new)
// ============================================================================

#[test]
fn test_graph_stats_returns_counts() {
    let g = AtheneumGraph::open_in_memory().expect("open");

    // Fresh DB: stats should exist but all counts are 0.
    let stats = g.graph_stats().expect("graph_stats");
    assert_eq!(stats.total_entities, 0, "fresh DB has 0 entities");
    assert_eq!(stats.entity_counts.len(), 0, "fresh DB has no kinds");

    g.insert_agent("new_agent", json!({"status":"active"}))
        .expect("insert");
    let stats_after = g.graph_stats().expect("graph_stats");

    let agent_count = stats_after
        .entity_counts
        .iter()
        .find(|(k, _)| k == "Agent")
        .map(|(_, c)| *c)
        .unwrap_or(0);
    assert_eq!(agent_count, 1, "Agent count should be 1 after insert");
    assert_eq!(stats_after.total_entities, 1, "total_entities should be 1");
}

// ============================================================================
// Auto-index on discovery write (new)
// ============================================================================

#[test]
fn test_discovery_auto_indexed() {
    let g = AtheneumGraph::open_in_memory().expect("open");

    g.store_discovery(
        "agent",
        "Symbol",
        "semantic_navigation",
        json!({"summary":"navigate with HNSW and graph traversals"}),
    )
    .expect("store_discovery");

    // After store_discovery we should be able to search WITHOUT manually calling build_search_index()
    let results = g
        .lexical_search("semantic navigation HNSW traversal", 5, None, None)
        .expect("semantic_search");

    assert!(
        !results.is_empty(),
        "auto-indexed discovery should be searchable immediately (got 0 results)"
    );
    assert_eq!(
        results[0].name, "agent: semantic_navigation",
        "best match should be the discovery we just stored"
    );
}
