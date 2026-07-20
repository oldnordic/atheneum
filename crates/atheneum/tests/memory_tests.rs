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
        .lexical_search("gpu_warning", 10, None, Some("Memory"), None)
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

// ===== update_memory tests (Story A1, spec FR-1) =====

#[test]
fn test_update_memory_replaces_content_and_recomputes_hash() {
    use atheneum::MemoryPatch;

    let graph = AtheneumGraph::open_in_memory().expect("open");
    let id = graph
        .store_memory("city", "Hyderabad", "user", 1.0, None, None)
        .expect("seed memory");

    let original = graph.get_entity(id).unwrap();
    let original_hash = original
        .data
        .get("content_hash")
        .and_then(|v| v.as_str())
        .map(String::from);

    let returned = graph
        .update_memory(id, &MemoryPatch { content: Some("Bangalore".into()), ..Default::default() })
        .expect("update_memory");
    assert_eq!(returned, id, "update_memory must return the same id");

    let updated = graph.get_entity(id).unwrap();
    assert_eq!(
        updated.data.get("content").and_then(|v| v.as_str()),
        Some("Bangalore"),
        "content must be replaced"
    );
    assert_eq!(updated.name, "city", "key is preserved");
    assert_eq!(
        updated.data.get("scope").and_then(|v| v.as_str()),
        Some("user"),
        "scope is preserved"
    );
    assert_eq!(
        updated.data.get("confidence").and_then(|v| v.as_f64()),
        Some(1.0),
        "confidence is preserved when not patched"
    );

    let new_hash = updated
        .data
        .get("content_hash")
        .and_then(|v| v.as_str())
        .map(String::from);
    assert_ne!(original_hash, new_hash, "content hash must change");
    assert_ne!(original_hash.as_deref(), Some("Bangalore"));
}

#[test]
fn test_update_memory_empty_patch_is_noop() {
    use atheneum::MemoryPatch;

    let graph = AtheneumGraph::open_in_memory().expect("open");
    let id = graph
        .store_memory("tz", "UTC+0", "user", 0.8, None, None)
        .expect("seed memory");
    let before = graph.get_entity(id).unwrap();

    let returned = graph
        .update_memory(id, &MemoryPatch::default())
        .expect("empty patch");
    assert_eq!(returned, id);

    let after = graph.get_entity(id).unwrap();
    assert_eq!(before.data, after.data, "empty patch must not touch data");
}

#[test]
fn test_update_memory_importance_maps_to_confidence() {
    use atheneum::MemoryPatch;

    let graph = AtheneumGraph::open_in_memory().expect("open");
    let id = graph
        .store_memory("note", "x", "agent", 1.0, None, None)
        .expect("seed memory");

    graph
        .update_memory(id, &MemoryPatch { importance: Some(5), ..Default::default() })
        .unwrap();

    let updated = graph.get_entity(id).unwrap();
    assert_eq!(
        updated.data.get("confidence").and_then(|v| v.as_f64()),
        Some(0.5),
        "importance=5 must map to confidence=0.5 (matches store_memory scale)"
    );
}

#[test]
fn test_update_memory_merges_tags_by_default() {
    use atheneum::MemoryPatch;

    let graph = AtheneumGraph::open_in_memory().expect("open");
    let id = graph
        .store_memory(
            "note",
            "x",
            "agent",
            1.0,
            None,
            Some(&["alpha".to_string(), "beta".to_string()]),
        )
        .expect("seed memory");

    graph
        .update_memory(
            id,
            &MemoryPatch {
                tags: Some(vec!["beta".into(), "gamma".into()]),
                ..Default::default()
            },
        )
        .unwrap();

    let updated = graph.get_entity(id).unwrap();
    let tags: Vec<String> = updated
        .data
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    assert_eq!(tags, vec!["alpha", "beta", "gamma"], "tags merge, dedup, preserve order");
}

#[test]
fn test_update_memory_replace_tags_overwrites() {
    use atheneum::MemoryPatch;

    let graph = AtheneumGraph::open_in_memory().expect("open");
    let id = graph
        .store_memory(
            "note",
            "x",
            "agent",
            1.0,
            None,
            Some(&["alpha".to_string(), "beta".to_string()]),
        )
        .expect("seed memory");

    graph
        .update_memory(
            id,
            &MemoryPatch {
                tags: Some(vec!["gamma".into(), "delta".into()]),
                replace_tags: true,
                ..Default::default()
            },
        )
        .unwrap();

    let updated = graph.get_entity(id).unwrap();
    let tags: Vec<String> = updated
        .data
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    assert_eq!(tags, vec!["gamma", "delta"], "replace_tags overwrites");
}

#[test]
fn test_update_memory_rejects_non_memory_entity() {
    use atheneum::MemoryPatch;

    let graph = AtheneumGraph::open_in_memory().expect("open");
    // An Agent entity, not a Memory.
    let id = graph.insert_agent("bot", serde_json::json!({})).expect("insert agent");

    let err = graph
        .update_memory(id, &MemoryPatch { content: Some("x".into()), ..Default::default() })
        .unwrap_err();
    assert!(
        err.to_string().contains("Entity not found") || err.to_string().contains(&id.to_string()),
        "non-Memory id must be rejected with EntityNotFound, got: {err}"
    );
}

#[test]
fn test_update_memory_rejects_missing_id() {
    use atheneum::MemoryPatch;

    let graph = AtheneumGraph::open_in_memory().expect("open");
    let err = graph
        .update_memory(999999, &MemoryPatch { content: Some("x".into()), ..Default::default() })
        .unwrap_err();
    assert!(
        err.to_string().contains("Entity not found") || err.to_string().contains("999999"),
        "missing id must be rejected with EntityNotFound, got: {err}"
    );
}

#[test]
fn test_update_memory_query_memory_reflects_new_content() {
    // Round-trip: store → update → query returns only the new content.
    use atheneum::MemoryPatch;

    let graph = AtheneumGraph::open_in_memory().expect("open");
    let id = graph
        .store_memory("city", "Hyderabad", "user", 1.0, None, None)
        .expect("seed memory");

    graph
        .update_memory(id, &MemoryPatch { content: Some("Bangalore".into()), ..Default::default() })
        .unwrap();

    let found = graph.query_memory("city", None, None).expect("query");
    assert_eq!(found.len(), 1, "no duplicate memory created");
    assert_eq!(
        found[0].data.get("content").and_then(|v| v.as_str()),
        Some("Bangalore"),
        "query must reflect patched content"
    );
}

#[test]
fn test_update_memory_mirrors_to_memory_entries_table() {
    // The dual-write path: graph_entities AND memory_entries must both reflect
    // the update, mirroring store_memory's contract.
    use atheneum::MemoryPatch;

    let graph = AtheneumGraph::open_in_memory().expect("open");
    let id = graph
        .store_memory("city", "Hyderabad", "user", 1.0, None, None)
        .expect("seed memory");

    graph
        .update_memory(id, &MemoryPatch { content: Some("Bangalore".into()), ..Default::default() })
        .unwrap();

    let row: (String, f64) = graph
        .with_raw_connection(|conn| {
            conn.query_row(
                "SELECT content, confidence FROM memory_entries WHERE key = ?1 AND scope = ?2",
                params!["city", "user"],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)),
            )
            .map_err(anyhow::Error::from)
        })
        .expect("memory_entries row must exist");
    assert_eq!(row.0, "Bangalore", "memory_entries.content must be updated");
    assert_eq!(row.1, 1.0, "confidence preserved in memory_entries");
}
