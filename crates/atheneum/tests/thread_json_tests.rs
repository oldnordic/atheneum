//! Regression test for `atheneum thread --json` capacity-overflow panic.
//!
//! Root cause (fixed in 0.13.1): `subgraph_to_json_bounded` in
//! `crates/atheneum/src/main.rs` pre-allocated its edge vector with
//! `Vec::with_capacity(edge_cap)` where the thread JSON path passes
//! `usize::MAX` as the cap (meaning "unbounded"), overflowing the allocator.

use atheneum::graph::{AtheneumGraph, EdgeType};
use serde_json::json;
use std::process::Command;

/// Build a fixture DB with a decision chain the `thread` query can match.
fn make_fixture_db() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("fixture.db");
    {
        let g = AtheneumGraph::open(&db_path).expect("open fixture db");
        let first = g
            .store_discovery(
                "claude1",
                "Decision",
                "threadjson root decision",
                json!({"summary": "chose threadjson approach"}),
            )
            .expect("store discovery");
        let second = g
            .store_discovery(
                "claude1",
                "Decision",
                "threadjson followup decision",
                json!({"summary": "threadjson followup"}),
            )
            .expect("store discovery");
        g.insert_edge(second, first, EdgeType::CausedBy, json!({}))
            .expect("insert caused_by edge");
    }
    (dir, db_path)
}

#[test]
fn thread_json_does_not_panic_and_emits_valid_json() {
    let (_dir, db_path) = make_fixture_db();
    let output = Command::new(env!("CARGO_BIN_EXE_atheneum"))
        .arg("thread")
        .arg(&db_path)
        .arg("threadjson")
        .arg("--tokens")
        .arg("500")
        .arg("--depth")
        .arg("3")
        .arg("--json")
        .output()
        .expect("run atheneum thread --json");

    assert!(
        output.status.success(),
        "thread --json exited with {:?}; stderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout utf8");
    let value: serde_json::Value =
        serde_json::from_str(&stdout).expect("thread --json output must be valid JSON");
    assert_eq!(value["query"], json!("threadjson"));
    let subgraphs = value["subgraphs"]
        .as_array()
        .expect("subgraphs must be an array");
    assert!(
        !subgraphs.is_empty(),
        "fixture must produce at least one entry point"
    );
}

#[test]
fn thread_markdown_path_still_works() {
    let (_dir, db_path) = make_fixture_db();
    let output = Command::new(env!("CARGO_BIN_EXE_atheneum"))
        .arg("thread")
        .arg(&db_path)
        .arg("threadjson")
        .arg("--tokens")
        .arg("500")
        .arg("--depth")
        .arg("3")
        .output()
        .expect("run atheneum thread");

    assert!(
        output.status.success(),
        "thread (markdown) exited with {:?}; stderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
}
