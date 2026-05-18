//! Stage 7: planning-domain tests.
//!
//! Tasks, Requirements, Blockers as first-class graph entities, with a
//! reader that re-attaches the parent → children relationships and the
//! killer feature: Stage 4's journal kanban extraction now drives Task
//! state transitions.

use atheneum::graph::{AtheneumGraph, BlockerType, KanbanStatus, TaskDetail};
use serde_json::json;

// ===========================================================================
// Task CRUD
// ===========================================================================

#[test]
fn test_create_task_defaults_to_todo() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let id = graph
        .create_task(
            "Wire HNSW search",
            Some("Add /atheneum/search"),
            Some("envoy"),
        )
        .expect("create");

    let entity = graph.get_entity(id).expect("retrieve");
    assert_eq!(entity.kind, "Task");
    assert_eq!(entity.data["title"], json!("Wire HNSW search"));
    assert_eq!(entity.data["project_id"], json!("envoy"));
    assert_eq!(
        entity.data["status"],
        json!("TODO"),
        "new tasks start in TODO"
    );
}

#[test]
fn test_update_task_status_transitions() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let id = graph
        .create_task("Refactor router", None, Some("envoy"))
        .expect("create");

    graph
        .update_task_status(id, KanbanStatus::InProgress)
        .expect("status update");
    let after = graph.get_entity(id).expect("retrieve");
    assert_eq!(after.data["status"], json!("IN_PROGRESS"));
    // A timestamp on the last transition is useful for audit
    assert!(
        after.data["status_updated_at"].as_str().is_some(),
        "status change should stamp status_updated_at"
    );

    graph
        .update_task_status(id, KanbanStatus::Done)
        .expect("status update done");
    let done = graph.get_entity(id).expect("retrieve");
    assert_eq!(done.data["status"], json!("DONE"));
}

#[test]
fn test_find_task_by_title_scoped_by_project() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let envoy_id = graph
        .create_task("Refactor router", None, Some("envoy"))
        .expect("envoy");
    graph
        .create_task("Refactor router", None, Some("magellan"))
        .expect("magellan");

    // Same title in two projects — find_task_by_title must scope by project
    let found = graph
        .find_task_by_title("Refactor router", Some("envoy"))
        .expect("find")
        .expect("present");
    assert_eq!(
        found, envoy_id,
        "must return the envoy task, not magellan's"
    );

    let missing = graph
        .find_task_by_title("nonexistent", Some("envoy"))
        .expect("find");
    assert!(missing.is_none());
}

#[test]
fn test_list_tasks_by_status() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let _t1 = graph.create_task("T1", None, Some("envoy")).expect("t1"); // TODO
    let t2 = graph.create_task("T2", None, Some("envoy")).expect("t2");
    graph
        .update_task_status(t2, KanbanStatus::InProgress)
        .expect("t2 in progress");
    let t3 = graph.create_task("T3", None, Some("envoy")).expect("t3");
    graph
        .update_task_status(t3, KanbanStatus::Done)
        .expect("t3 done");
    let _t4_other_project = graph.create_task("T4", None, Some("magellan")).expect("t4");

    let in_progress = graph
        .list_tasks_by_status(KanbanStatus::InProgress, Some("envoy"))
        .expect("list");
    assert_eq!(in_progress.len(), 1);
    assert_eq!(in_progress[0].data["title"], json!("T2"));
}

// ===========================================================================
// Requirements & Blockers
// ===========================================================================

#[test]
fn test_add_requirement_links_to_task() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let task_id = graph
        .create_task("Add CI", None, Some("envoy"))
        .expect("task");

    let req_id = graph
        .add_requirement(task_id, "cargo test passes", Some("cargo test --workspace"))
        .expect("requirement");

    let req = graph.get_entity(req_id).expect("retrieve");
    assert_eq!(req.kind, "Requirement");
    assert_eq!(req.data["task_id"], json!(task_id));
    assert_eq!(req.data["status"], json!("UNMET"));
    assert_eq!(
        req.data["verification_method"],
        json!("cargo test --workspace")
    );

    graph.mark_requirement_met(req_id).expect("mark met");
    let updated = graph.get_entity(req_id).expect("retrieve");
    assert_eq!(updated.data["status"], json!("MET"));
}

#[test]
fn test_add_blocker_creates_blocker_entity() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let task_id = graph
        .create_task("Ship feature", None, Some("envoy"))
        .expect("task");

    let b_id = graph
        .add_blocker(
            task_id,
            "Waiting on upstream PR #123",
            BlockerType::Dependency,
        )
        .expect("blocker");

    let b = graph.get_entity(b_id).expect("retrieve");
    assert_eq!(b.kind, "Blocker");
    assert_eq!(b.data["task_id"], json!(task_id));
    assert_eq!(b.data["blocker_type"], json!("DEPENDENCY"));
    assert!(
        b.data["resolved_at"].is_null(),
        "new blocker shouldn't be resolved"
    );

    graph.resolve_blocker(b_id).expect("resolve");
    let resolved = graph.get_entity(b_id).expect("retrieve");
    assert!(
        resolved.data["resolved_at"].as_str().is_some(),
        "resolve_blocker stamps resolved_at"
    );
}

#[test]
fn test_get_task_with_details_returns_full_record() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let task_id = graph
        .create_task("Big task", Some("desc"), Some("envoy"))
        .expect("task");
    graph
        .add_requirement(task_id, "tests pass", None)
        .expect("req");
    graph
        .add_requirement(task_id, "ci green", None)
        .expect("req2");
    graph
        .add_blocker(task_id, "no merge slot", BlockerType::InfoGap)
        .expect("blocker");

    let detail: TaskDetail = graph.get_task_with_details(task_id).expect("detail");
    assert_eq!(detail.task.id, task_id);
    assert_eq!(detail.requirements.len(), 2);
    assert_eq!(detail.blockers.len(), 1);
    assert_eq!(detail.blockers[0].data["blocker_type"], json!("INFO_GAP"));
    assert!(detail
        .requirements
        .iter()
        .any(|r| r.data["statement"] == json!("tests pass")));
}

// ===========================================================================
// Journal → Task auto-application (the killer feature)
// ===========================================================================

#[test]
fn test_apply_kanban_updates_from_journal_changes_task_status() {
    let graph = AtheneumGraph::open_in_memory().expect("open");

    let task_id = graph
        .create_task("Wire HNSW search", None, Some("envoy"))
        .expect("task");
    // Ingest a journal section that names this task in a kanban transition.
    let journal_ids = graph
        .ingest_journal(
            "journals/2026_05_18.md",
            "## 14:30 | progress\n\"Wire HNSW search\" -> DONE ✅\n",
            Some("envoy"),
        )
        .expect("ingest journal");
    assert_eq!(journal_ids.len(), 1);

    let applied = graph
        .apply_kanban_updates_from_journal(journal_ids[0])
        .expect("apply");
    assert_eq!(applied.len(), 1);
    assert_eq!(applied[0].task_id, task_id);
    assert_eq!(applied[0].new_status, KanbanStatus::Done);
    assert!(
        applied[0].previous_status == KanbanStatus::Todo,
        "task started at TODO"
    );

    let after = graph.get_entity(task_id).expect("retrieve");
    assert_eq!(after.data["status"], json!("DONE"));
}

#[test]
fn test_apply_kanban_updates_skips_unknown_tasks() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    // Journal references a task title that doesn't exist in any project.
    let journal_ids = graph
        .ingest_journal(
            "journals/ghost.md",
            "## 09:00 | misc\n\"Phantom task\" -> DONE\n",
            Some("envoy"),
        )
        .expect("ingest");
    let applied = graph
        .apply_kanban_updates_from_journal(journal_ids[0])
        .expect("apply");
    assert!(
        applied.is_empty(),
        "no matching task → nothing to apply (got {:?})",
        applied
    );
}
