//! Tests for Atheneum graph operations
//! Tests are written FIRST (TDD) and will fail until implementation is complete.

use atheneum::graph::{AtheneumGraph, EdgeType};
use chrono::Utc;
use serde_json::json;

#[test]
fn test_create_graph_in_memory() {
    let graph = AtheneumGraph::open_in_memory().expect("Failed to create in-memory graph");
    assert!(graph.is_healthy(), "Graph should be healthy after creation");
}

#[test]
fn test_create_graph_persistent() {
    let dir = tempfile::tempdir().expect("Failed to create temp dir");
    let db_path = dir.path().join("test.db");
    let graph = AtheneumGraph::open(&db_path).expect("Failed to create persistent graph");
    assert!(graph.is_healthy(), "Graph should be healthy after creation");
}

#[test]
fn test_insert_agent_entity() {
    let graph = AtheneumGraph::open_in_memory().expect("Failed to create graph");

    let agent_id = graph
        .insert_agent("hermes", json!({"type": "coordinator", "status": "active"}))
        .expect("Failed to insert agent");

    assert!(agent_id > 0, "Agent ID should be positive");

    let agent = graph
        .get_entity(agent_id)
        .expect("Failed to retrieve agent");
    assert_eq!(agent.kind, "Agent", "Entity kind should be Agent");
    assert_eq!(agent.name, "hermes", "Agent name should match");
}

#[test]
fn test_insert_task_entity() {
    let graph = AtheneumGraph::open_in_memory().expect("Failed to create graph");

    let task_id = graph
        .insert_task(
            "implement-feature-x",
            json!({"status": "pending", "priority": "high"}),
        )
        .expect("Failed to insert task");

    assert!(task_id > 0, "Task ID should be positive");

    let task = graph.get_entity(task_id).expect("Failed to retrieve task");
    assert_eq!(task.kind, "Task", "Entity kind should be Task");
    assert_eq!(task.name, "implement-feature-x", "Task name should match");
}

#[test]
fn test_insert_event_entity() {
    let graph = AtheneumGraph::open_in_memory().expect("Failed to create graph");

    let event_id = graph
        .insert_event(
            "message-sent",
            json!({"from": "claude1", "to": "hermes", "subject": "test"}),
        )
        .expect("Failed to insert event");

    assert!(event_id > 0, "Event ID should be positive");

    let event = graph
        .get_entity(event_id)
        .expect("Failed to retrieve event");
    assert_eq!(event.kind, "Event", "Entity kind should be Event");
    assert_eq!(event.name, "message-sent", "Event name should match");

    // Check timestamp was added
    let data = event.data.as_object().expect("Data should be object");
    assert!(
        data.contains_key("timestamp"),
        "Event should have timestamp"
    );
}

#[test]
fn test_insert_edge_performed_by() {
    let graph = AtheneumGraph::open_in_memory().expect("Failed to create graph");

    let agent_id = graph
        .insert_agent("claude1", json!({}))
        .expect("Failed to insert agent");

    let event_id = graph
        .insert_event("code-changed", json!({"file": "src/lib.rs"}))
        .expect("Failed to insert event");

    let edge_id = graph
        .insert_edge(
            event_id,
            agent_id,
            EdgeType::PerformedBy,
            json!({"provenance": {"actor": "system", "confidence": 1.0}}),
        )
        .expect("Failed to insert edge");

    assert!(edge_id > 0, "Edge ID should be positive");

    let edge = graph.get_edge(edge_id).expect("Failed to retrieve edge");
    assert_eq!(edge.from_id, event_id, "Edge from_id should match event");
    assert_eq!(edge.to_id, agent_id, "Edge to_id should match agent");
}

#[test]
fn test_insert_edge_assigned_to() {
    let graph = AtheneumGraph::open_in_memory().expect("Failed to create graph");

    let agent_id = graph
        .insert_agent("claude2", json!({}))
        .expect("Failed to insert agent");

    let task_id = graph
        .insert_task("fix-bug-123", json!({"status": "pending"}))
        .expect("Failed to insert task");

    let edge_id = graph
        .insert_edge(
            task_id,
            agent_id,
            EdgeType::AssignedTo,
            json!({"assigned_at": Utc::now().to_rfc3339()}),
        )
        .expect("Failed to insert edge");

    assert!(edge_id > 0, "Edge ID should be positive");
}

#[test]
fn test_hopgraph_edge_type_labels_are_stable() {
    let labels: Vec<&str> = EdgeType::all().iter().map(EdgeType::as_str).collect();

    for required in [
        "mentions",
        "wikilink",
        "implements",
        "depends_on",
        "calls",
        "accessed",
        "tested_by",
        "fixed_by",
        "regressed_by",
        "observed_in",
        "belongs_to_project",
        "similar_failure",
        "requires_skill",
        "handled_by_tool",
    ] {
        assert!(
            labels.contains(&required),
            "missing HopGraph relation label {required}; got {labels:?}"
        );
    }
}

#[test]
fn test_query_events_by_agent() {
    let graph = AtheneumGraph::open_in_memory().expect("Failed to create graph");

    let agent_id = graph
        .insert_agent("claude1", json!({}))
        .expect("Failed to insert agent");

    let event1_id = graph
        .insert_event("wrote-code", json!({"file": "a.rs"}))
        .expect("Failed to insert event1");

    let event2_id = graph
        .insert_event("ran-tests", json!({"file": "a.rs"}))
        .expect("Failed to insert event2");

    graph
        .insert_edge(event1_id, agent_id, EdgeType::PerformedBy, json!({}))
        .expect("Failed to insert edge1");

    graph
        .insert_edge(event2_id, agent_id, EdgeType::PerformedBy, json!({}))
        .expect("Failed to insert edge2");

    let events = graph
        .events_performed_by(agent_id)
        .expect("Failed to query events");

    assert_eq!(events.len(), 2, "Should find 2 events performed by agent");
}

#[test]
fn test_query_tasks_assigned_to_agent() {
    let graph = AtheneumGraph::open_in_memory().expect("Failed to create graph");

    let agent_id = graph
        .insert_agent("claude2", json!({}))
        .expect("Failed to insert agent");

    let task1_id = graph
        .insert_task("task-a", json!({}))
        .expect("Failed to insert task1");

    let task2_id = graph
        .insert_task("task-b", json!({}))
        .expect("Failed to insert task2");

    graph
        .insert_edge(task1_id, agent_id, EdgeType::AssignedTo, json!({}))
        .expect("Failed to insert edge1");

    graph
        .insert_edge(task2_id, agent_id, EdgeType::AssignedTo, json!({}))
        .expect("Failed to insert edge2");

    let tasks = graph
        .tasks_assigned_to(agent_id)
        .expect("Failed to query tasks");

    assert_eq!(tasks.len(), 2, "Should find 2 tasks assigned to agent");
}

#[test]
fn test_provenance_tracking() {
    let graph = AtheneumGraph::open_in_memory().expect("Failed to create graph");

    let event_id = graph
        .insert_event("decision-made", json!({"decision": "use-rust"}))
        .expect("Failed to insert event");

    let edge_id = graph
        .insert_edge(
            event_id,
            1, // dummy agent_id
            EdgeType::PerformedBy,
            json!({
                "provenance": {
                    "actor": "claude1",
                    "timestamp": Utc::now().to_rfc3339(),
                    "source": "manual",
                    "confidence": 0.9,
                    "method": "direct"
                }
            }),
        )
        .expect("Failed to insert edge");

    let edge = graph.get_edge(edge_id).expect("Failed to retrieve edge");
    let provenance = edge
        .data
        .get("provenance")
        .expect("Edge should have provenance");

    assert_eq!(
        provenance["actor"], "claude1",
        "Provenance actor should match"
    );
    assert_eq!(
        provenance["confidence"], 0.9,
        "Provenance confidence should match"
    );
}

#[test]
fn test_causal_chain() {
    let graph = AtheneumGraph::open_in_memory().expect("Failed to create graph");

    // Event A caused Event B, Event B caused Event C
    let event_a = graph
        .insert_event("a", json!({}))
        .expect("Failed to insert event A");

    let event_b = graph
        .insert_event("b", json!({}))
        .expect("Failed to insert event B");

    let event_c = graph
        .insert_event("c", json!({}))
        .expect("Failed to insert event C");

    graph
        .insert_edge(event_b, event_a, EdgeType::CausedBy, json!({}))
        .expect("Failed to insert A->B edge");

    graph
        .insert_edge(event_c, event_b, EdgeType::CausedBy, json!({}))
        .expect("Failed to insert B->C edge");

    let chain = graph
        .causal_chain(event_c)
        .expect("Failed to get causal chain");

    assert_eq!(chain.len(), 3, "Chain should have 3 events");
    assert_eq!(chain[0].name, "c", "First should be C");
    assert_eq!(chain[1].name, "b", "Second should be B");
    assert_eq!(chain[2].name, "a", "Third should be A");
}
