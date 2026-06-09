use atheneum::graph::{AtheneumGraph, SessionParams};
use serde_json::json;

#[test]
fn query_memory_uses_cache_and_invalidates_on_store() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .store_memory("timezone", "UTC+0", "user", 1.0, None, None)
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
        .store_memory("timezone", "UTC+1", "user", 0.9, None, None)
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
}

#[test]
fn lexical_search_uses_cache_and_invalidates_on_write() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .store_discovery(
            "agent",
            "Symbol",
            "cache_target",
            json!({"summary": "test discovery for cache invalidation"}),
        )
        .expect("store discovery");
    graph.build_search_index().expect("build index");

    let first = graph
        .lexical_search("cache invalidation test", 5, None, None)
        .expect("first search");
    assert!(!first.is_empty(), "search should return results");

    let stats_after_first = graph.runtime_stats();
    assert_eq!(stats_after_first.search_queries, 1);
    assert_eq!(stats_after_first.cache_hits, 0);
    assert_eq!(stats_after_first.cache_misses, 1);

    let second = graph
        .lexical_search("cache invalidation test", 5, None, None)
        .expect("second search");
    assert_eq!(second.len(), first.len());

    let stats_after_second = graph.runtime_stats();
    assert_eq!(stats_after_second.search_queries, 2);
    assert_eq!(stats_after_second.cache_hits, 1);
    assert_eq!(stats_after_second.cache_misses, 1);

    graph
        .store_discovery(
            "agent",
            "Symbol",
            "cache_target_two",
            json!({"summary": "another test discovery for cache invalidation"}),
        )
        .expect("store second discovery");

    let refreshed = graph
        .lexical_search("cache invalidation test", 5, None, None)
        .expect("search after write");
    assert!(
        refreshed.len() >= first.len(),
        "invalidated search should see new discovery"
    );

    let stats_after_refresh = graph.runtime_stats();
    assert_eq!(stats_after_refresh.cache_hits, 1);
    assert_eq!(stats_after_refresh.cache_misses, 2);
}

#[test]
fn navigate_uses_cache_and_invalidates_on_edge_insert() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .store_discovery(
            "agent",
            "Symbol",
            "nav_target",
            json!({"summary": "navigation cache target discovery"}),
        )
        .expect("store discovery");
    graph.build_search_index().expect("build index");

    let first = graph
        .navigate("navigation cache target", 5, 2, None, None)
        .expect("first navigate");
    assert!(!first.is_empty(), "navigate should return results");

    let stats_after_first = graph.runtime_stats();
    assert_eq!(stats_after_first.navigation_queries, 1);
    assert_eq!(stats_after_first.cache_hits, 0);

    let second = graph
        .navigate("navigation cache target", 5, 2, None, None)
        .expect("second navigate");
    assert_eq!(second.len(), first.len());

    let stats_after_second = graph.runtime_stats();
    assert_eq!(stats_after_second.navigation_queries, 2);
    assert_eq!(stats_after_second.cache_hits, 1);

    // Insert an edge to invalidate the navigation cache.
    let discovery = graph
        .query_discoveries("nav_target")
        .expect("query discovery")
        .pop()
        .expect("discovery exists");
    let memory_id = graph
        .store_memory("nav_mem", "related", "user", 1.0, None, None)
        .expect("store memory");
    graph
        .insert_edge(
            discovery.id,
            memory_id,
            atheneum::graph::EdgeType::RelatedTo,
            json!({}),
        )
        .expect("insert edge");

    let refreshed = graph
        .navigate("navigation cache target", 5, 2, None, None)
        .expect("navigate after edge insert");
    assert!(
        refreshed
            .iter()
            .any(|v| v.entities.iter().any(|e| e.id == memory_id)),
        "invalidated navigate should traverse the new edge"
    );

    let stats_after_refresh = graph.runtime_stats();
    assert_eq!(stats_after_refresh.navigation_queries, 3);
    // Cache hit count remains 1 because the third navigate was invalidated.
    assert_eq!(stats_after_refresh.cache_hits, 1);
}

#[test]
fn hopgraph_query_uses_cache() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .store_discovery(
            "agent",
            "Symbol",
            "hop_target",
            json!({"summary": "hopgraph cache target discovery"}),
        )
        .expect("store discovery");
    graph.build_search_index().expect("build index");

    eprintln!("DEBUG before first hopgraph: {:?}", graph.runtime_stats());
    let first = graph
        .hopgraph_query("hopgraph cache target", 5, 2, &[], 10000, None)
        .expect("first hopgraph");
    assert!(!first.is_empty(), "hopgraph should return results");

    let stats_after_first = graph.runtime_stats();
    assert_eq!(stats_after_first.navigation_queries, 1);
    assert_eq!(stats_after_first.search_queries, 1);
    assert_eq!(stats_after_first.cache_hits, 0);
    assert_eq!(stats_after_first.cache_misses, 2);

    let second = graph
        .hopgraph_query("hopgraph cache target", 5, 2, &[], 10000, None)
        .expect("second hopgraph");
    assert_eq!(second.len(), first.len());

    let stats_after_second = graph.runtime_stats();
    assert_eq!(stats_after_second.navigation_queries, 2);
    assert_eq!(stats_after_second.search_queries, 1);
    assert_eq!(stats_after_second.cache_hits, 1);
    assert_eq!(stats_after_second.cache_misses, 2);
}

#[test]
fn hnsw_counters_track_hits_and_fallbacks() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    for i in 0..3 {
        graph
            .store_discovery(
                "agent",
                "Symbol",
                &format!("sym_{}", i),
                json!({"summary": format!("test discovery number {}", i)}),
            )
            .expect("store");
    }
    graph.build_search_index().expect("build index");

    // A broad query that matches HNSW fully with k=1 should be a hit.
    let _ = graph
        .lexical_search("test discovery", 1, None, None)
        .expect("search hit");
    let stats = graph.runtime_stats();
    assert_eq!(stats.search_queries, 1);
    assert!(stats.hnsw_hits >= 1, "HNSW should record a hit");

    // A query with k larger than the entity count forces fallback to token scan.
    let _ = graph
        .lexical_search("xyz_unlikely_fallback_query", 20, None, None)
        .expect("search fallback");
    let stats = graph.runtime_stats();
    assert_eq!(stats.search_queries, 2);
    assert!(
        stats.hnsw_fallback_scans >= 1,
        "fallback scan should be recorded"
    );
}

#[test]
fn memory_row_repair_counter_increments_on_missing_sql_row() {
    let db_file = tempfile::NamedTempFile::new().expect("tempfile");
    let db_path = db_file.path().to_path_buf();
    let graph = AtheneumGraph::open(&db_path).expect("open");

    let _id = graph
        .store_memory("timezone", "UTC+0", "user", 1.0, None, None)
        .expect("store initial");

    let conn = rusqlite::Connection::open(&db_path).expect("open sqlite");
    conn.execute(
        "DELETE FROM memory_entries WHERE key = ?1",
        rusqlite::params!["timezone"],
    )
    .expect("delete memory row");

    assert_eq!(
        graph.runtime_stats().memory_row_repairs,
        0,
        "no repair before update"
    );

    graph
        .store_memory("timezone", "UTC+1", "user", 0.9, None, None)
        .expect("store update triggers repair");

    assert_eq!(
        graph.runtime_stats().memory_row_repairs,
        1,
        "missing SQL row should count as one repair"
    );
}

#[test]
fn dream_and_consolidate_counters_increment() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .store_memory("a", "alpha", "user", 1.0, None, None)
        .expect("store a");
    graph
        .store_memory("b", "beta", "user", 1.0, None, None)
        .expect("store b");

    let _ = graph
        .dream_pass(
            atheneum::graph::DreamMode::DryRun,
            Some("user"),
            None,
            &atheneum::graph::DreamConfig::default(),
        )
        .expect("dream");
    assert_eq!(graph.runtime_stats().dream_runs, 1);

    let _ = graph
        .wiki_dream_pass(
            atheneum::graph::DreamMode::DryRun,
            None,
            &atheneum::graph::DreamConfig::default(),
        )
        .expect("wiki dream");
    assert_eq!(graph.runtime_stats().wiki_dream_runs, 1);

    // consolidation_pass over an empty discovery set still counts as a run.
    let _ = graph.consolidation_pass(None).expect("consolidation");
    assert_eq!(graph.runtime_stats().consolidate_runs, 1);
}
