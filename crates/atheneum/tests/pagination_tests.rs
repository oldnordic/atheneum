//! Pagination tests for large read APIs.
//!
//! Verifies paged variants (query_events_page, query_sessions_page,
//! list_memory_page, list_wiki_pages_page) and that existing Vec-returning
//! methods remain backward compatible.

use atheneum::graph::{AtheneumGraph, RecordEventParams, SessionParams};
use serde_json::json;

#[test]
fn query_events_page_respects_offset_and_limit() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .record_session(SessionParams {
            session_id: "session-p1".into(),
            agent_name: "agent".into(),
            project: "proj".into(),
            tool: "tool".into(),
            trigger: "cli".into(),
            model: None,
            git_branch: None,
            git_head: None,
            parent_session_id: None,
            relations: vec![],
        })
        .expect("record session");

    for i in 0..5 {
        graph
            .record_event(RecordEventParams {
                event_type: "test_event".into(),
                entity_id: format!("ent-{}", i),
                session_id: "session-p1".into(),
                payload: json!({"seq": i}),
                relations: vec![],
            })
            .expect("record event");
    }

    let all = graph
        .query_events_page(Some("session-p1"), Some("test_event"), 0, 10)
        .expect("query all");
    assert_eq!(all.len(), 5);

    let page1 = graph
        .query_events_page(Some("session-p1"), Some("test_event"), 0, 2)
        .expect("page 1");
    assert_eq!(page1.len(), 2);

    let page2 = graph
        .query_events_page(Some("session-p1"), Some("test_event"), 2, 2)
        .expect("page 2");
    assert_eq!(page2.len(), 2);

    let page3 = graph
        .query_events_page(Some("session-p1"), Some("test_event"), 4, 2)
        .expect("page 3");
    assert_eq!(page3.len(), 1);

    // Pages are disjoint and concatenate to the full set in event_id DESC order.
    assert_eq!(page1[0]["payload"]["seq"], json!(4));
    assert_eq!(page1[1]["payload"]["seq"], json!(3));
    assert_eq!(page2[0]["payload"]["seq"], json!(2));
    assert_eq!(page2[1]["payload"]["seq"], json!(1));
    assert_eq!(page3[0]["payload"]["seq"], json!(0));

    // Backward-compatible Vec wrapper returns the same first page as before.
    let compat = graph
        .query_events(Some("session-p1"), Some("test_event"), 10)
        .expect("compat query");
    assert_eq!(compat, all);
}

#[test]
fn query_sessions_page_respects_offset_and_limit() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    for i in 0..4 {
        graph
            .record_session(SessionParams {
                session_id: format!("session-{}", i),
                agent_name: "agent".into(),
                project: "proj".into(),
                tool: "tool".into(),
                trigger: "cli".into(),
                model: None,
                git_branch: None,
                git_head: None,
                parent_session_id: None,
                relations: vec![],
            })
            .expect("record session");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    let all = graph
        .query_sessions_page(Some("proj"), None, 0, 10)
        .expect("query all");
    assert_eq!(all.len(), 4);

    let page1 = graph
        .query_sessions_page(Some("proj"), None, 0, 2)
        .expect("page 1");
    assert_eq!(page1.len(), 2);

    let page2 = graph
        .query_sessions_page(Some("proj"), None, 2, 2)
        .expect("page 2");
    assert_eq!(page2.len(), 2);

    assert_eq!(page1[0].session_id, "session-3");
    assert_eq!(page1[1].session_id, "session-2");
    assert_eq!(page2[0].session_id, "session-1");
    assert_eq!(page2[1].session_id, "session-0");

    let compat = graph
        .query_sessions(Some("proj"), 10, None)
        .expect("compat query");
    assert_eq!(compat, all);
}

#[test]
fn list_memory_page_pagination() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    for i in 0..5 {
        graph
            .store_memory(
                &format!("key-{}", i),
                &format!("content-{}", i),
                "user",
                1.0,
                None,
                None,
            )
            .expect("store memory");
    }

    let all = graph
        .list_memory_page(None, None, 0, usize::MAX)
        .expect("all");
    assert_eq!(all.len(), 5);

    let page1 = graph.list_memory_page(None, None, 0, 2).expect("page 1");
    assert_eq!(page1.len(), 2);

    let page2 = graph.list_memory_page(None, None, 2, 2).expect("page 2");
    assert_eq!(page2.len(), 2);

    let page3 = graph.list_memory_page(None, None, 4, 2).expect("page 3");
    assert_eq!(page3.len(), 1);

    let names: Vec<_> = all.iter().map(|e| e.name.as_str()).collect();
    let page_names: Vec<_> = page1
        .iter()
        .chain(page2.iter())
        .chain(page3.iter())
        .map(|e| e.name.as_str())
        .collect();
    assert_eq!(names, page_names);

    let compat = graph.list_memory(None, None).expect("compat list");
    assert_eq!(compat, all);
}

#[test]
fn list_wiki_pages_page_pagination() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    for i in 0..4 {
        graph
            .ingest_wiki_page(
                &format!("wiki/page-{}.md", i),
                &format!("---\ntitle: Page {}\n---\nbody\n", i),
                Some("proj"),
            )
            .expect("ingest");
    }

    let all = graph
        .list_wiki_pages_page(Some("proj"), 0, usize::MAX)
        .expect("all");
    assert_eq!(all.len(), 4);

    let page1 = graph
        .list_wiki_pages_page(Some("proj"), 0, 2)
        .expect("page 1");
    assert_eq!(page1.len(), 2);

    let page2 = graph
        .list_wiki_pages_page(Some("proj"), 2, 2)
        .expect("page 2");
    assert_eq!(page2.len(), 2);

    let paths: Vec<_> = all.iter().map(|p| p.path.as_str()).collect();
    let page_paths: Vec<_> = page1
        .iter()
        .chain(page2.iter())
        .map(|p| p.path.as_str())
        .collect();
    assert_eq!(paths, page_paths);

    let compat = graph.list_wiki_pages(Some("proj")).expect("compat list");
    assert_eq!(compat, all);
}
