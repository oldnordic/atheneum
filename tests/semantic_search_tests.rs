//! Tests for HNSW semantic search over atheneum discoveries.
//!
//! Stage 3 of the atheneum-py port. Adds a small wrapper around
//! sqlitegraph's native HNSW so agents can ask "what do we know about X"
//! and get fuzzy matches against stored discoveries, not just exact target
//! name lookups.

use atheneum::graph::AtheneumGraph;
use rusqlite::Connection;
use serde_json::json;

#[test]
fn test_semantic_search_returns_matching_discoveries() {
    let graph = AtheneumGraph::open_in_memory().expect("open");

    graph
        .store_discovery(
            "agent",
            "Symbol",
            "build_router",
            json!({"file": "src/http.rs", "summary": "constructs the axum Router with all routes"}),
        )
        .expect("store");

    graph
        .store_discovery(
            "agent",
            "Symbol",
            "parse_frontmatter",
            json!({"file": "src/graph/mod.rs", "summary": "parses YAML frontmatter from markdown"}),
        )
        .expect("store");

    graph.build_search_index().expect("build_search_index");

    // Query that semantically matches the router discovery
    let results = graph
        .lexical_search("router construction axum routes", 5, None, None)
        .expect("search");

    assert!(!results.is_empty(), "search should return some results");
    let first = &results[0];
    assert_eq!(
        first.name, "agent: build_router",
        "best match should be the router discovery (got: {})",
        first.name
    );
}

#[test]
fn test_semantic_search_respects_k_limit() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    for i in 0..7 {
        graph
            .store_discovery(
                "a",
                "Symbol",
                &format!("sym_{}", i),
                json!({"summary": format!("test discovery number {}", i)}),
            )
            .expect("store");
    }
    graph.build_search_index().expect("build_search_index");

    let results = graph
        .lexical_search("test discovery", 3, None, None)
        .expect("search");
    assert!(
        results.len() <= 3,
        "search must respect k limit (got {} results)",
        results.len()
    );
}

#[test]
fn test_semantic_search_filtered_by_project() {
    let graph = AtheneumGraph::open_in_memory().expect("open");

    graph
        .store_discovery_in_project(
            "a",
            "Symbol",
            "Message",
            Some("envoy"),
            json!({"summary": "envoy message struct"}),
        )
        .expect("store");
    graph
        .store_discovery_in_project(
            "a",
            "Symbol",
            "Message",
            Some("magellan"),
            json!({"summary": "magellan protocol message"}),
        )
        .expect("store");

    graph.build_search_index().expect("build_search_index");

    let envoy_only = graph
        .lexical_search("message", 10, Some("envoy"), None)
        .expect("search");
    assert!(
        envoy_only
            .iter()
            .all(|r| r.data.get("project_id").and_then(|v| v.as_str()) == Some("envoy")),
        "filter must only return envoy discoveries (got: {:?})",
        envoy_only
            .iter()
            .map(|r| r.data.get("project_id").cloned())
            .collect::<Vec<_>>()
    );
    assert!(
        !envoy_only.is_empty(),
        "envoy filter should still return results"
    );

    let mag_only = graph
        .lexical_search("message", 10, Some("magellan"), None)
        .expect("search");
    assert!(mag_only
        .iter()
        .all(|r| r.data.get("project_id").and_then(|v| v.as_str()) == Some("magellan")));
}

#[test]
fn test_semantic_search_falls_back_when_hnsw_index_is_inconsistent() {
    let db_file = tempfile::NamedTempFile::new().expect("tempfile");
    let db_path = db_file.path().to_path_buf();

    {
        let graph = AtheneumGraph::open(&db_path).expect("open");
        graph
            .store_discovery(
                "agent",
                "Symbol",
                "build_router",
                json!({"summary": "constructs the axum router"}),
            )
            .expect("store");
    }

    {
        let conn = Connection::open(&db_path).expect("open sqlite");
        conn.execute(
            "DELETE FROM hnsw_vectors
             WHERE index_id = (SELECT id FROM hnsw_indexes WHERE name = 'discoveries')
               AND id = (
                   SELECT MIN(id) FROM hnsw_vectors
                   WHERE index_id = (SELECT id FROM hnsw_indexes WHERE name = 'discoveries')
               )",
            [],
        )
        .expect("corrupt search index");
    }

    let graph = AtheneumGraph::open(&db_path).expect("reopen");
    let results = graph
        .lexical_search("build router", 5, None, None)
        .expect("search should fall back instead of failing");

    assert!(
        results.iter().any(|r| r.name == "agent: build_router"),
        "fallback search should still find the discovery"
    );
}
