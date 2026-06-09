//! Stage 11c: SQL+graph separation tests for the Planning domain.

use atheneum::graph::{AtheneumGraph, BlockerType, KanbanStatus};
use rusqlite::params;
use serde_json::json;

#[test]
fn test_task_row_exists_after_create_task() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let entity_id = graph
        .create_task("Wire HNSW", Some("Add /atheneum/search"), Some("envoy"))
        .expect("create");

    let (title, status, project): (String, String, Option<String>) = graph
        .with_raw_connection(|conn| {
            conn.query_row(
                "SELECT title, status, project_id FROM tasks WHERE title = ?1",
                params!["Wire HNSW"],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .map_err(Into::into)
        })
        .expect("tasks row");
    assert_eq!(title, "Wire HNSW");
    assert_eq!(status, "TODO");
    assert_eq!(project.as_deref(), Some("envoy"));

    // The graph_entity pointer should carry an sql_id linking to the row
    let entity = graph.get_entity(entity_id).expect("retrieve");
    let pointed_sql_id = entity
        .data
        .get("sql_id")
        .and_then(|v| v.as_i64())
        .expect("sql_id on the pointer");
    let sql_row_id: i64 = graph
        .with_raw_connection(|conn| {
            conn.query_row(
                "SELECT id FROM tasks WHERE title = ?1",
                params!["Wire HNSW"],
                |r| r.get(0),
            )
            .map_err(Into::into)
        })
        .expect("row id");
    assert_eq!(pointed_sql_id, sql_row_id);
}

#[test]
fn test_update_task_status_writes_to_sql() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let id = graph
        .create_task("Refactor router", None, Some("envoy"))
        .expect("create");
    graph
        .update_task_status(id, KanbanStatus::InProgress)
        .expect("update");

    let (status, ts_present): (String, bool) = graph
        .with_raw_connection(|conn| {
            conn.query_row(
                "SELECT status, status_updated_at IS NOT NULL FROM tasks WHERE title = 'Refactor router'",
                [],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?)),
            )
            .map_err(Into::into)
        })
        .expect("query");
    assert_eq!(status, "IN_PROGRESS");
    assert!(ts_present, "status_updated_at should be set");
}

#[test]
fn test_requirements_row_with_fk_to_tasks() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let task_id = graph
        .create_task("Add CI", None, Some("envoy"))
        .expect("task");
    graph
        .add_requirement(task_id, "cargo test passes", Some("cargo test --workspace"))
        .expect("req");

    let (stmt, status, method): (String, String, Option<String>) = graph
        .with_raw_connection(|conn| {
            conn.query_row(
                "SELECT r.statement, r.status, r.verification_method
                 FROM requirements r
                 JOIN tasks t ON t.id = r.task_id
                 WHERE t.title = 'Add CI'",
                [],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .map_err(Into::into)
        })
        .expect("join");
    assert_eq!(stmt, "cargo test passes");
    assert_eq!(status, "UNMET");
    assert_eq!(method.as_deref(), Some("cargo test --workspace"));
}

#[test]
fn test_blockers_row_with_fk_to_tasks() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let task_id = graph
        .create_task("Ship", None, Some("envoy"))
        .expect("task");
    let b_id = graph
        .add_blocker(task_id, "waiting on upstream", BlockerType::Dependency)
        .expect("blocker");
    graph.resolve_blocker(b_id).expect("resolve");

    let (desc, kind, resolved): (String, String, Option<String>) = graph
        .with_raw_connection(|conn| {
            conn.query_row(
                "SELECT b.description, b.blocker_type, b.resolved_at
                 FROM blockers b JOIN tasks t ON t.id = b.task_id
                 WHERE t.title = 'Ship'",
                [],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .map_err(Into::into)
        })
        .expect("join");
    assert_eq!(desc, "waiting on upstream");
    assert_eq!(kind, "DEPENDENCY");
    assert!(
        resolved.is_some(),
        "resolve_blocker should stamp resolved_at"
    );
}

#[test]
fn test_aggregate_task_count_by_status_via_sql() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let t1 = graph.create_task("T1", None, Some("envoy")).expect("t1");
    let _t2 = graph.create_task("T2", None, Some("envoy")).expect("t2");
    let t3 = graph.create_task("T3", None, Some("envoy")).expect("t3");
    graph
        .update_task_status(t1, KanbanStatus::InProgress)
        .expect("t1 inprogress");
    graph
        .update_task_status(t3, KanbanStatus::Done)
        .expect("t3 done");
    let _other = graph
        .create_task("Other", None, Some("magellan"))
        .expect("other");

    // Pure SQL aggregation, no json_extract
    let counts: Vec<(String, i64)> = graph
        .with_raw_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT status, COUNT(*) FROM tasks WHERE project_id = ?1 GROUP BY status ORDER BY status",
            )?;
            let rows = stmt
                .query_map(params!["envoy"], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .expect("aggregate");

    assert!(counts.contains(&("DONE".to_string(), 1)));
    assert!(counts.contains(&("IN_PROGRESS".to_string(), 1)));
    assert!(counts.contains(&("TODO".to_string(), 1)));
}

#[test]
fn test_apply_kanban_updates_from_journal_uses_sql() {
    // End-to-end: ingest a journal with a kanban transition and confirm
    // the SQL `tasks` row flipped status, not just the graph_entity.
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let task_id = graph
        .create_task("Wire HNSW", None, Some("envoy"))
        .expect("task");
    let ids = graph
        .ingest_journal(
            "journals/2026_05_19.md",
            "## 14:30 | progress\n\"Wire HNSW\" -> DONE\n",
            Some("envoy"),
        )
        .expect("ingest");
    let applied = graph
        .apply_kanban_updates_from_journal(ids[0])
        .expect("apply");
    assert_eq!(applied.len(), 1);
    assert_eq!(applied[0].new_status, KanbanStatus::Done);

    // Confirm via SQL
    let sql_status: String = graph
        .with_raw_connection(|conn| {
            conn.query_row(
                "SELECT status FROM tasks WHERE title = 'Wire HNSW'",
                [],
                |r| r.get(0),
            )
            .map_err(Into::into)
        })
        .expect("status row");
    assert_eq!(sql_status, "DONE");
    // Plus the entity pointer's mirror still reads DONE for old callers
    let entity = graph.get_entity(task_id).expect("entity");
    assert_eq!(entity.data["status"], json!("DONE"));
}
