//! Stage 11a + 11b: SQL+graph separation tests.
//!
//! Foundation: schema versioning + migration runner.
//! Execution domain: agents / reasoning_logs / tool_calls SQL tables
//! backing the existing audit-trail methods.

use atheneum::graph::AtheneumGraph;
use rusqlite::params;
use serde_json::json;

#[test]
fn test_schema_version_recorded_after_open() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let max: i64 = graph
        .with_raw_connection(|conn| {
            conn.query_row(
                "SELECT COALESCE(MAX(version), 0) FROM atheneum_schema_version",
                [],
                |row| row.get(0),
            )
            .map_err(Into::into)
        })
        .expect("query version");
    assert!(
        max >= 1,
        "at least migration version 1 should be recorded on open (got {})",
        max
    );
}

#[test]
fn test_open_is_idempotent_no_duplicate_versions() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    // Closing+reopening an in-memory DB resets it, so we just probe the count
    // after open. Migrations must apply each version exactly once.
    let count: i64 = graph
        .with_raw_connection(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM atheneum_schema_version WHERE version = 1",
                [],
                |row| row.get(0),
            )
            .map_err(Into::into)
        })
        .expect("query");
    assert_eq!(count, 1, "version 1 must appear exactly once");
}

#[test]
fn test_agents_row_exists_after_ensure_agent_via_reasoning_log() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .insert_reasoning_log("claude1", "thinking", Some("envoy"))
        .expect("log");

    let (name, project): (String, Option<String>) = graph
        .with_raw_connection(|conn| {
            conn.query_row(
                "SELECT name, project_id FROM agents WHERE name = ?1",
                params!["claude1"],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .map_err(Into::into)
        })
        .expect("agents row");
    assert_eq!(name, "claude1");
    // ensure_agent doesn't carry a project_id (the project lives on the log);
    // it's fine to be NULL here.
    let _ = project;
}

#[test]
fn test_reasoning_logs_row_has_proper_columns() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .insert_reasoning_log("claude2", "deciding", Some("envoy"))
        .expect("log");

    let (agent_name, content, project): (String, String, Option<String>) = graph
        .with_raw_connection(|conn| {
            conn.query_row(
                "SELECT a.name, r.content, r.project_id
                 FROM reasoning_logs r JOIN agents a ON a.id = r.agent_id
                 WHERE r.content = ?1",
                params!["deciding"],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .map_err(Into::into)
        })
        .expect("join row");
    assert_eq!(agent_name, "claude2");
    assert_eq!(content, "deciding");
    assert_eq!(project.as_deref(), Some("envoy"));
}

#[test]
fn test_tool_calls_row_has_proper_columns() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let log_id = graph
        .insert_reasoning_log("agent_x", "plan", Some("envoy"))
        .expect("log");
    graph
        .insert_tool_call(
            log_id,
            "magellan_find",
            json!({"name": "Foo"}),
            Some("envoy"),
        )
        .expect("tool");

    let (tool_name, args, project): (String, String, Option<String>) = graph
        .with_raw_connection(|conn| {
            conn.query_row(
                "SELECT tool_name, args, project_id FROM tool_calls WHERE tool_name = ?1",
                params!["magellan_find"],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .map_err(Into::into)
        })
        .expect("tool_calls row");
    assert_eq!(tool_name, "magellan_find");
    let args_v: serde_json::Value = serde_json::from_str(&args).unwrap();
    assert_eq!(args_v["name"], json!("Foo"));
    assert_eq!(project.as_deref(), Some("envoy"));
}

#[test]
fn test_sql_id_pointer_in_graph_entity() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let entity_id = graph
        .insert_reasoning_log("p", "p", Some("p"))
        .expect("log");
    let entity = graph.get_entity(entity_id).expect("retrieve");

    let pointed_sql_id = entity
        .data
        .get("sql_id")
        .and_then(|v| v.as_i64())
        .expect("sql_id should be on the pointer node");

    let sql_row_id: i64 = graph
        .with_raw_connection(|conn| {
            conn.query_row(
                "SELECT id FROM reasoning_logs WHERE content = ?1",
                params!["p"],
                |row| row.get(0),
            )
            .map_err(Into::into)
        })
        .expect("sql row");
    assert_eq!(
        pointed_sql_id, sql_row_id,
        "graph entity's data.sql_id must equal the actual reasoning_logs.id"
    );
}

#[test]
fn test_aggregation_over_sql_table_works() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let log_a = graph
        .insert_reasoning_log("agent_a", "thought 1", Some("p1"))
        .expect("log a");
    let log_b = graph
        .insert_reasoning_log("agent_a", "thought 2", Some("p1"))
        .expect("log b");
    let _log_c = graph
        .insert_reasoning_log("agent_a", "thought 3", Some("p2"))
        .expect("log c");
    graph
        .insert_tool_call(log_a, "cargo_test", json!({}), Some("p1"))
        .expect("tc a");
    graph
        .insert_tool_call(log_b, "cargo_test", json!({}), Some("p1"))
        .expect("tc b");

    // Count tool_calls per project — a real dashboard query that would have
    // needed json_extract before this stage.
    let p1_count: i64 = graph
        .with_raw_connection(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM tool_calls WHERE project_id = ?1",
                params!["p1"],
                |row| row.get(0),
            )
            .map_err(Into::into)
        })
        .expect("count");
    assert_eq!(p1_count, 2);
}
