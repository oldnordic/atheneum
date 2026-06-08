use atheneum::graph::{AtheneumGraph, SessionParams};
use serde_json::json;

#[test]
fn query_memory_uses_cache_and_invalidates_on_store() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .store_memory("timezone", "UTC+0", "user", 1.0, None)
        .expect("store memory");

    let first = graph
        .query_memory("timezone", Some("user"), None)
        .expect("first query");
    assert_eq!(first.len(), 1);

    let stats_after_first = graph.runtime_stats();
    assert_eq!(stats_after_first.memory_queries, 1);
    assert_eq!(stats_after_first.cache_hits, 0);
    assert_eq!(stats_after_first.cache_misses, 1);

    let second = graph
        .query_memory("timezone", Some("user"), None)
        .expect("second query");
    assert_eq!(second.len(), 1);

    let stats_after_second = graph.runtime_stats();
    assert_eq!(stats_after_second.memory_queries, 2);
    assert_eq!(stats_after_second.cache_hits, 1);
    assert_eq!(stats_after_second.cache_misses, 1);

    graph
        .store_memory("timezone", "UTC+1", "user", 0.9, None)
        .expect("update memory");

    let refreshed = graph
        .query_memory("timezone", Some("user"), None)
        .expect("query after invalidation");
    assert_eq!(
        refreshed[0]
            .data
            .get("content")
            .and_then(|value| value.as_str()),
        Some("UTC+1")
    );

    let stats_after_refresh = graph.runtime_stats();
    assert_eq!(stats_after_refresh.memory_writes, 2);
    assert_eq!(stats_after_refresh.cache_hits, 1);
    assert_eq!(stats_after_refresh.cache_misses, 2);
}

#[test]
fn query_sessions_uses_cache_and_invalidates_on_session_write() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .record_session(SessionParams {
            session_id: "session-1".into(),
            agent_name: "codex".into(),
            project: "atheneum".into(),
            tool: "codex".into(),
            trigger: "cli".into(),
            model: None,
            git_branch: None,
            git_head: None,
            parent_session_id: None,
            relations: vec![],
        })
        .expect("record first session");

    let first = graph
        .query_sessions(Some("atheneum"), 10, None)
        .expect("first query");
    assert_eq!(first.len(), 1);

    let second = graph
        .query_sessions(Some("atheneum"), 10, None)
        .expect("second query");
    assert_eq!(second.len(), 1);

    let stats_after_second = graph.runtime_stats();
    assert_eq!(stats_after_second.session_queries, 2);
    assert_eq!(stats_after_second.cache_hits, 1);

    graph
        .record_session(SessionParams {
            session_id: "session-2".into(),
            agent_name: "codex".into(),
            project: "atheneum".into(),
            tool: "codex".into(),
            trigger: "subagent".into(),
            model: None,
            git_branch: None,
            git_head: None,
            parent_session_id: None,
            relations: vec![],
        })
        .expect("record second session");

    let refreshed = graph
        .query_sessions(Some("atheneum"), 10, None)
        .expect("query after second session");
    assert_eq!(refreshed.len(), 2);
    assert_eq!(refreshed[0].session_id, "session-2");

    let stats_after_refresh = graph.runtime_stats();
    assert_eq!(stats_after_refresh.session_writes, 2);
    assert_eq!(stats_after_refresh.cache_hits, 1);
    assert_eq!(stats_after_refresh.cache_misses, 2);
}

#[test]
fn knowledge_and_wiki_queries_cache_and_invalidate_on_writes() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .store_discovery_in_project(
            "codex",
            "Pattern",
            "query_cache",
            Some("atheneum"),
            json!({"summary": "cache repeated reads"}),
        )
        .expect("store discovery");
    graph
        .ingest_wiki_page(
            "wiki/cache.md",
            "---\ntitle: Cache\n---\nRemember [[query_cache]]\n",
            Some("atheneum"),
        )
        .expect("ingest wiki");

    let knowledge_first = graph
        .query_knowledge_in_project("query_cache", Some("atheneum"))
        .expect("knowledge first");
    assert_eq!(knowledge_first["discovery_count"], json!(1));

    let pages_first = graph.list_wiki_pages(Some("atheneum")).expect("list pages");
    assert_eq!(pages_first.len(), 1);

    let _knowledge_second = graph
        .query_knowledge_in_project("query_cache", Some("atheneum"))
        .expect("knowledge second");
    let _pages_second = graph
        .list_wiki_pages(Some("atheneum"))
        .expect("list pages second");

    let stats_after_hits = graph.runtime_stats();
    assert_eq!(stats_after_hits.knowledge_queries, 2);
    assert_eq!(stats_after_hits.wiki_queries, 2);
    assert_eq!(stats_after_hits.cache_hits, 2);

    graph
        .store_handoff_in_project(
            "codex",
            "hermes",
            Some("atheneum"),
            json!({"task": "query_cache", "files_analyzed": ["src/graph/cache.rs"]}),
        )
        .expect("store handoff");
    graph
        .ingest_wiki_page(
            "wiki/extra.md",
            "---\ntitle: Extra\n---\nSee [[query_cache]] again\n",
            Some("atheneum"),
        )
        .expect("ingest second wiki");

    let knowledge_refreshed = graph
        .query_knowledge_in_project("query_cache", Some("atheneum"))
        .expect("knowledge refreshed");
    assert_eq!(knowledge_refreshed["handoff_count"], json!(1));

    let pages_refreshed = graph
        .list_wiki_pages(Some("atheneum"))
        .expect("list pages refreshed");
    assert_eq!(pages_refreshed.len(), 2);

    let stats_after_refresh = graph.runtime_stats();
    assert_eq!(stats_after_refresh.knowledge_writes, 2);
    assert_eq!(stats_after_refresh.wiki_writes, 2);
    assert_eq!(stats_after_refresh.cache_hits, 2);
    assert_eq!(stats_after_refresh.cache_misses, 4);
}
