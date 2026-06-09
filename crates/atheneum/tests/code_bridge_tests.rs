//! Stage 12: tests for the magellan → atheneum code bridge.
//!
//! The bridge reads symbol nodes from a magellan-format sqlitegraph DB
//! and stores them as Discoveries in atheneum. Idempotent by
//! `(agent_name, target, project_id)`: re-importing the same symbol
//! updates the existing discovery instead of duplicating it.

use atheneum::graph::AtheneumGraph;
use rusqlite::{params, Connection};
use serde_json::json;
use std::path::PathBuf;

/// Build a temporary sqlitegraph-shaped database with a few Symbol
/// entities resembling what magellan writes. We bypass magellan's
/// public API (it's a CLI, not a library) and write the rows directly
/// against the underlying SQLite schema.
fn build_fake_magellan_db(
    symbols: &[(&str, &str, i64)],
) -> (Connection, PathBuf, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("magellan.db");

    // Open a fresh on-disk connection and create the minimum schema the
    // bridge reads: graph_entities holding Symbol kinds.
    let conn = Connection::open(&path).expect("open");
    conn.execute_batch(
        "CREATE TABLE graph_entities (
             id INTEGER PRIMARY KEY,
             kind TEXT NOT NULL,
             name TEXT NOT NULL,
             file_path TEXT,
             data TEXT
         );",
    )
    .expect("schema");
    for (name, file, line) in symbols {
        conn.execute(
            "INSERT INTO graph_entities (kind, name, file_path, data) VALUES ('Symbol', ?1, ?2, ?3)",
            params![
                name,
                file,
                serde_json::to_string(&json!({
                    "fqn": format!("crate::{}", name),
                    "name": name,
                    "kind": "Function",
                    "start_line": line,
                    "end_line": line + 5
                }))
                .unwrap()
            ],
        )
        .expect("insert");
    }
    (conn, path, dir)
}

#[test]
fn test_import_symbol_from_magellan_creates_discovery() {
    let graph = AtheneumGraph::open_in_memory().expect("open atheneum");
    let (_conn, magellan_path, _td) = build_fake_magellan_db(&[
        ("build_router", "src/http.rs", 42),
        ("parse_yaml", "src/graph/mod.rs", 1523),
    ]);

    let entity_id = graph
        .import_symbol_from_magellan(&magellan_path, "build_router", "claude1", Some("envoy"))
        .expect("import")
        .expect("symbol should be found");

    let entity = graph.get_entity(entity_id).expect("entity");
    assert_eq!(entity.kind, "Discovery");
    assert_eq!(entity.data["discovery_type"], json!("Symbol"));
    assert_eq!(entity.data["target"], json!("build_router"));
    assert_eq!(entity.data["file"], json!("src/http.rs"));
    assert_eq!(entity.data["start_line"], json!(42));
    assert_eq!(entity.data["project_id"], json!("envoy"));
    // Should carry sql_id from Stage 11d
    assert!(entity.data["sql_id"].as_i64().is_some());
}

#[test]
fn test_import_symbol_not_found_returns_none() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let (_c, path, _td) = build_fake_magellan_db(&[("only_thing", "src/lib.rs", 1)]);

    let found = graph
        .import_symbol_from_magellan(&path, "doesnt_exist", "agent", None)
        .expect("import call");
    assert!(found.is_none(), "missing symbol should return None");
}

#[test]
fn test_import_symbol_is_idempotent_by_agent_target_project() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let (_c, path, _td) = build_fake_magellan_db(&[("foo", "src/x.rs", 1)]);

    let id1 = graph
        .import_symbol_from_magellan(&path, "foo", "claude1", Some("envoy"))
        .expect("first")
        .expect("found");
    let id2 = graph
        .import_symbol_from_magellan(&path, "foo", "claude1", Some("envoy"))
        .expect("second")
        .expect("found");
    assert_eq!(
        id1, id2,
        "re-importing the same (agent, target, project) must update in place"
    );

    // Exactly one discovery row for that combination
    let count: i64 = graph
        .with_raw_connection(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM discoveries
                 WHERE agent_name = 'claude1' AND target = 'foo' AND project_id = 'envoy'",
                [],
                |r| r.get(0),
            )
            .map_err(Into::into)
        })
        .expect("count");
    assert_eq!(count, 1);
}

#[test]
fn test_different_projects_get_separate_discoveries() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let (_c, path, _td) = build_fake_magellan_db(&[("shared", "src/lib.rs", 1)]);

    let envoy_id = graph
        .import_symbol_from_magellan(&path, "shared", "agent", Some("envoy"))
        .expect("envoy")
        .expect("found");
    let mag_id = graph
        .import_symbol_from_magellan(&path, "shared", "agent", Some("magellan"))
        .expect("magellan")
        .expect("found");
    assert_ne!(
        envoy_id, mag_id,
        "different project scoping should yield distinct discoveries"
    );
}

#[test]
fn test_import_all_symbols_bulk() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let (_c, path, _td) =
        build_fake_magellan_db(&[("a", "f.rs", 1), ("b", "f.rs", 10), ("c", "g.rs", 1)]);

    let count = graph
        .import_all_symbols_from_magellan(&path, "claude1", Some("envoy"), None)
        .expect("bulk");
    assert_eq!(count, 3);

    let envoy_total: i64 = graph
        .with_raw_connection(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM discoveries
                 WHERE agent_name = 'claude1' AND project_id = 'envoy' AND discovery_type = 'Symbol'",
                [],
                |r| r.get(0),
            )
            .map_err(Into::into)
        })
        .expect("count");
    assert_eq!(envoy_total, 3);
}
