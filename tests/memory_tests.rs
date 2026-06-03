//! Tests for Memory entity domain.
//!
//! Covers store/query/list + scope/project filtering + HNSW auto-index.

use atheneum::graph::{AtheneumGraph, EntityType};

#[test]
fn test_store_memory_creates_entity() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let id = graph
        .store_memory(
            "prefers_concise",
            "User prefers concise output",
            "user",
            1.0,
            None,
        )
        .expect("store_memory");
    assert!(id > 0, "memory_id should be positive");

    let entity = graph.get_entity(id).expect("get_entity");
    assert_eq!(entity.kind, EntityType::Memory.as_str());
    assert_eq!(entity.name, "prefers_concise");
}

#[test]
fn test_query_memory_by_key() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .store_memory("timezone", "UTC+0", "user", 1.0, None)
        .unwrap();

    let found = graph
        .query_memory("timezone", None, None)
        .expect("query_memory");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "timezone");
}

#[test]
fn test_query_memory_filters_scope() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .store_memory("api_key", "abc123", "user", 1.0, None)
        .unwrap();
    graph
        .store_memory("api_key", "xyz789", "project", 1.0, Some("projA"))
        .unwrap();

    let user_only = graph.query_memory("api_key", Some("user"), None).unwrap();
    assert_eq!(user_only.len(), 1);
    assert_eq!(
        user_only[0].data.get("scope").and_then(|v| v.as_str()),
        Some("user")
    );
}

#[test]
fn test_query_memory_filters_project() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .store_memory("convention", "use anyhow", "project", 1.0, Some("p1"))
        .unwrap();
    graph
        .store_memory("convention", "use thiserror", "project", 1.0, Some("p2"))
        .unwrap();

    let p1_only = graph
        .query_memory("convention", Some("project"), Some("p1"))
        .unwrap();
    assert_eq!(p1_only.len(), 1);
    assert_eq!(
        p1_only[0].data.get("project_id").and_then(|v| v.as_str()),
        Some("p1")
    );
}

#[test]
fn test_list_memory_by_scope() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .store_memory("a", "content-a", "user", 1.0, None)
        .unwrap();
    graph
        .store_memory("b", "content-b", "agent", 1.0, None)
        .unwrap();

    let user_mems = graph.list_memory(Some("user"), None).unwrap();
    assert_eq!(user_mems.len(), 1);
    assert_eq!(user_mems[0].name, "a");
}

#[test]
fn test_list_memory_all() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .store_memory("x", "content-x", "user", 1.0, None)
        .unwrap();
    graph
        .store_memory("y", "content-y", "project", 1.0, Some("p"))
        .unwrap();

    let all = graph.list_memory(None, None).unwrap();
    assert_eq!(all.len(), 2);
}

#[test]
fn test_memory_entity_type_kind() {
    assert_eq!(EntityType::Memory.as_str(), "Memory");
}

#[test]
fn test_memory_searchable() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .store_memory(
            "gpu_warning",
            "Avoid unsafe GPU kernels",
            "project",
            1.0,
            Some("rocmforge"),
        )
        .unwrap();

    let results = graph
        .lexical_search("gpu_warning", 10, None, Some("Memory"))
        .unwrap();
    assert!(
        results.iter().any(|r| r.name == "gpu_warning"),
        "lexical search should find memory entity"
    );
}
