//! Tests for Memory entity domain.
//!
//! Covers store/query/list + scope/project filtering + HNSW auto-index.

use atheneum::graph::{AtheneumGraph, EntityType};
use rusqlite::{params, Connection};

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
        .store_memory("timezone", "UTC+0", "user", 1.0, None, None)
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
        .store_memory("api_key", "abc123", "user", 1.0, None, None)
        .unwrap();
    graph
        .store_memory("api_key", "xyz789", "project", 1.0, Some("projA"), None)
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
        .store_memory("convention", "use anyhow", "project", 1.0, Some("p1"), None)
        .unwrap();
    graph
        .store_memory(
            "convention",
            "use thiserror",
            "project",
            1.0,
            Some("p2"),
            None,
        )
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
        .store_memory("a", "content-a", "user", 1.0, None, None)
        .unwrap();
    graph
        .store_memory("b", "content-b", "agent", 1.0, None, None)
        .unwrap();

    let user_mems = graph.list_memory(Some("user"), None).unwrap();
    assert_eq!(user_mems.len(), 1);
    assert_eq!(user_mems[0].name, "a");
}

#[test]
fn test_list_memory_all() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .store_memory("x", "content-x", "user", 1.0, None, None)
        .unwrap();
    graph
        .store_memory("y", "content-y", "project", 1.0, Some("p"), None)
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
            None,
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

#[test]
fn test_store_memory_upsert_updates_content() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let id1 = graph
        .store_memory(
            "prefers_concise",
            "User prefers concise output",
            "user",
            1.0,
            None,
            None,
        )
        .expect("store_memory first");

    let id2 = graph
        .store_memory(
            "prefers_concise",
            "Updated: very concise",
            "user",
            0.95,
            None,
            None,
        )
        .expect("store_memory second");

    assert_eq!(id1, id2, "upsert should return same entity id");

    let items = graph.query_memory("prefers_concise", None, None).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0].data.get("content").and_then(|v| v.as_str()),
        Some("Updated: very concise")
    );
    assert_eq!(
        items[0].data.get("confidence").and_then(|v| v.as_f64()),
        Some(0.95)
    );
}

#[test]
fn test_store_memory_preserves_created_at_on_upsert() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let id = graph
        .store_memory("timezone", "UTC+0", "user", 1.0, None, None)
        .unwrap();

    let first = graph.get_entity(id).unwrap();
    let created_at = first
        .data
        .get("created_at")
        .and_then(|v| v.as_str())
        .expect("created_at on first store");

    std::thread::sleep(std::time::Duration::from_millis(50));

    graph
        .store_memory("timezone", "UTC+1", "user", 1.0, None, None)
        .unwrap();

    let second = graph.get_entity(id).unwrap();
    let created_at2 = second
        .data
        .get("created_at")
        .and_then(|v| v.as_str())
        .expect("created_at on upsert");
    let updated_at = second.data.get("updated_at").and_then(|v| v.as_str());

    assert_eq!(created_at, created_at2, "created_at should be preserved");
    assert!(updated_at.is_some(), "updated_at should be set on upsert");
}

#[test]
fn test_store_memory_upsert_with_project_scope() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .store_memory("convention", "use anyhow", "project", 1.0, Some("p1"), None)
        .unwrap();
    graph
        .store_memory(
            "convention",
            "use thiserror",
            "project",
            1.0,
            Some("p2"),
            None,
        )
        .unwrap();

    // Same key, different project — should be separate entities
    let p1 = graph
        .query_memory("convention", Some("project"), Some("p1"))
        .unwrap();
    let p2 = graph
        .query_memory("convention", Some("project"), Some("p2"))
        .unwrap();
    assert_eq!(p1.len(), 1);
    assert_eq!(p2.len(), 1);
    assert_ne!(p1[0].id, p2[0].id);

    // Upsert p1
    graph
        .store_memory("convention", "use eyre", "project", 1.0, Some("p1"), None)
        .unwrap();
    let p1_updated = graph
        .query_memory("convention", Some("project"), Some("p1"))
        .unwrap();
    assert_eq!(p1_updated.len(), 1);
    assert_eq!(
        p1_updated[0].data.get("content").and_then(|v| v.as_str()),
        Some("use eyre")
    );
}

#[test]
fn test_store_memory_recreates_missing_sql_row_for_existing_entity() {
    let db_file = tempfile::NamedTempFile::new().expect("tempfile");
    let db_path = db_file.path().to_path_buf();
    let graph = AtheneumGraph::open(&db_path).expect("open");

    let id = graph
        .store_memory("timezone", "UTC+0", "user", 1.0, None, None)
        .expect("store initial");

    let conn = Connection::open(&db_path).expect("open sqlite");
    conn.execute(
        "DELETE FROM memory_entries WHERE key = ?1",
        params!["timezone"],
    )
    .expect("delete memory row");

    graph
        .store_memory("timezone", "UTC+1", "user", 0.9, None, None)
        .expect("store update");

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memory_entries WHERE key = ?1 AND scope = ?2",
            params!["timezone", "user"],
            |row| row.get(0),
        )
        .expect("count memory rows");
    assert_eq!(count, 1, "store_memory should recreate missing SQL row");

    let entity = graph.get_entity(id).expect("entity");
    assert_eq!(
        entity.data.get("content").and_then(|v| v.as_str()),
        Some("UTC+1")
    );
}

#[test]
fn test_preview_memory_is_read_only_and_returns_existing_matches() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .store_memory("timezone", "UTC+0", "user", 1.0, None, None)
        .expect("seed memory");

    let before = graph
        .entities_by_kind(EntityType::Memory.as_str())
        .expect("memory count before")
        .len();

    let preview = graph
        .preview_memory("timezone", "UTC+1", "user", 0.9, None, None, 5, 0.1)
        .expect("preview memory");

    let after = graph
        .entities_by_kind(EntityType::Memory.as_str())
        .expect("memory count after")
        .len();

    assert_eq!(before, after, "preview must not write memory");
    assert_eq!(preview.proposed_key, "timezone");
    assert_eq!(preview.exact_matches.len(), 1);
    assert!(
        preview
            .candidate_matches
            .iter()
            .any(|candidate| candidate.name == "timezone"),
        "preview should surface the existing memory candidate"
    );
    assert_eq!(
        preview
            .proposed_data
            .get("content_hash")
            .and_then(|value| value.as_str()),
        Some(preview.content_hash.as_str())
    );
}

#[test]
fn test_preview_memory_includes_exact_match_even_when_fuzzy_score_is_low() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .store_memory("timezone", "UTC+0", "user", 1.0, None, None)
        .expect("seed memory");

    let preview = graph
        .preview_memory(
            "timezone",
            "completely unrelated text",
            "user",
            0.9,
            None,
            None,
            5,
            0.95,
        )
        .expect("preview memory");

    assert_eq!(preview.exact_matches.len(), 1);
    assert!(
        preview
            .candidate_matches
            .iter()
            .any(|candidate| candidate.name == "timezone"),
        "exact matches should be merged into candidate results"
    );
}
