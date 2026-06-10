//! Tests for project/workspace scoping in atheneum.
//!
//! Ported pattern from atheneum-py: each entity carries a `project_id` so
//! multiple projects (envoy, magellan, splice) can share one atheneum DB
//! without name collisions.
//!
//! Design choice: project_id is embedded in the entity's `data` JSON blob,
//! not a new column. sqlitegraph's flexible payload column makes this clean
//! and avoids a schema migration.

use atheneum::graph::AtheneumGraph;
use serde_json::json;

#[test]
fn test_discoveries_isolated_by_project() {
    let graph = AtheneumGraph::open_in_memory().expect("Failed to create graph");

    // Two projects, same target symbol name
    graph
        .store_discovery_in_project(
            "agent_a",
            "Symbol",
            "build_router",
            Some("envoy"),
            json!({"file": "src/http.rs", "line": 555}),
        )
        .expect("envoy discovery should store");

    graph
        .store_discovery_in_project(
            "agent_b",
            "Symbol",
            "build_router",
            Some("magellan"),
            json!({"file": "src/build/router.rs", "line": 42}),
        )
        .expect("magellan discovery should store");

    // Each project's query returns only its own discovery
    let envoy_results = graph
        .query_discoveries_in_project("build_router", Some("envoy"))
        .expect("envoy query should succeed");
    assert_eq!(envoy_results.len(), 1, "envoy should see 1 discovery");
    let envoy_data = &envoy_results[0].data;
    assert_eq!(envoy_data["file"], json!("src/http.rs"));

    let magellan_results = graph
        .query_discoveries_in_project("build_router", Some("magellan"))
        .expect("magellan query should succeed");
    assert_eq!(magellan_results.len(), 1, "magellan should see 1 discovery");
    let magellan_data = &magellan_results[0].data;
    assert_eq!(magellan_data["file"], json!("src/build/router.rs"));
}

#[test]
fn test_unscoped_query_returns_all_projects() {
    let graph = AtheneumGraph::open_in_memory().expect("Failed to create graph");

    graph
        .store_discovery_in_project(
            "agent_a",
            "Symbol",
            "shared_name",
            Some("envoy"),
            json!({"file": "envoy.rs"}),
        )
        .expect("envoy discovery should store");

    graph
        .store_discovery_in_project(
            "agent_b",
            "Symbol",
            "shared_name",
            Some("magellan"),
            json!({"file": "magellan.rs"}),
        )
        .expect("magellan discovery should store");

    // Querying without a project filter returns both (backward-compat)
    let all = graph
        .query_discoveries("shared_name")
        .expect("unscoped query should succeed");
    assert_eq!(
        all.len(),
        2,
        "unscoped query should return both discoveries"
    );
}

#[test]
fn test_discovery_project_id_persisted_in_data() {
    let graph = AtheneumGraph::open_in_memory().expect("Failed to create graph");

    let id = graph
        .store_discovery_in_project(
            "agent_x",
            "Pattern",
            "ratelimit",
            Some("envoy"),
            json!({"note": "uses tower-limit"}),
        )
        .expect("should store");

    let entity = graph.get_entity(id).expect("should retrieve");
    assert_eq!(
        entity.data["project_id"],
        json!("envoy"),
        "project_id must be persisted into the data blob"
    );
}

#[test]
fn test_handoff_isolated_by_project() {
    let graph = AtheneumGraph::open_in_memory().expect("Failed to create graph");

    graph
        .store_handoff_in_project(
            "sender",
            "receiver",
            Some("envoy"),
            json!({"status": "NEEDS_CONTEXT", "what_was_done": "envoy work"}),
        )
        .expect("envoy handoff should store");

    graph
        .store_handoff_in_project(
            "sender",
            "receiver",
            Some("magellan"),
            json!({"status": "NEEDS_CONTEXT", "what_was_done": "magellan work"}),
        )
        .expect("magellan handoff should store");

    // Pending handoff queries are project-scoped — receiver sees only the envoy one
    let envoy_pending = graph
        .get_pending_handoff_in_project("receiver", Some("envoy"))
        .expect("envoy pending query")
        .expect("envoy handoff present");
    assert_eq!(
        envoy_pending.data["manifest"]["what_was_done"],
        json!("envoy work")
    );

    let magellan_pending = graph
        .get_pending_handoff_in_project("receiver", Some("magellan"))
        .expect("magellan pending query")
        .expect("magellan handoff present");
    assert_eq!(
        magellan_pending.data["manifest"]["what_was_done"],
        json!("magellan work")
    );
}

#[test]
fn test_knowledge_scoped_by_project() {
    let graph = AtheneumGraph::open_in_memory().expect("Failed to create graph");

    graph
        .store_discovery_in_project(
            "a1",
            "Symbol",
            "Message",
            Some("envoy"),
            json!({"file": "src/message.rs"}),
        )
        .expect("envoy discovery should store");

    graph
        .store_discovery_in_project(
            "a2",
            "Symbol",
            "Message",
            Some("magellan"),
            json!({"file": "src/protocol.rs"}),
        )
        .expect("magellan discovery should store");

    let envoy_knowledge = graph
        .query_knowledge_in_project("Message", Some("envoy"), None)
        .expect("envoy knowledge query");
    assert_eq!(
        envoy_knowledge["discovery_count"],
        json!(1),
        "envoy knowledge should count only envoy discoveries"
    );

    let magellan_knowledge = graph
        .query_knowledge_in_project("Message", Some("magellan"), None)
        .expect("magellan knowledge query");
    assert_eq!(
        magellan_knowledge["discovery_count"],
        json!(1),
        "magellan knowledge should count only magellan discoveries"
    );
}
