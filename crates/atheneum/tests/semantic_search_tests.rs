//! Tests for HNSW semantic search over atheneum discoveries.
//!
//! Stage 3 of the atheneum-py port. Adds a small wrapper around
//! sqlitegraph's native HNSW so agents can ask "what do we know about X"
//! and get fuzzy matches against stored discoveries, not just exact target
//! name lookups.

use atheneum::graph::AtheneumGraph;
use rusqlite::Connection;
use serde_json::json;

fn assert_top_k_contains(
    results: &[atheneum::graph::SearchResult],
    top_k: usize,
    expected_name: &str,
) {
    assert!(
        results
            .iter()
            .take(top_k)
            .any(|result| result.name == expected_name),
        "expected '{}' in top {} results, got {:?}",
        expected_name,
        top_k,
        results
            .iter()
            .take(top_k)
            .map(|r| format!("{}:{}", r.kind, r.name))
            .collect::<Vec<_>>()
    );
}

fn build_mixed_navigation_fixture() -> AtheneumGraph {
    let graph = AtheneumGraph::open_in_memory().expect("open");

    graph
        .ingest_wiki_page(
            "wiki/rocmforge-capabilities.md",
            "---\n\
             title: ROCmForge Capabilities\n\
             source: rocmforge v1.0\n\
             tags: [rocmforge, capabilities]\n\
             ---\n\
             AMD GPU inference capabilities, decode graph, TurboQuant, and path routing.\n",
            Some("proj"),
        )
        .expect("rocmforge capabilities");
    graph
        .ingest_wiki_page(
            "wiki/rocmforge-architecture.md",
            "---\n\
             title: ROCmForge Architecture\n\
             source: rocmforge v1.0\n\
             tags: [rocmforge, architecture]\n\
             ---\n\
             Architecture overview for ROCmForge GPU inference and CPU fallback.\n",
            Some("proj"),
        )
        .expect("rocmforge architecture");
    graph
        .ingest_wiki_page(
            "wiki/atheneum-architecture.md",
            "---\n\
             title: Atheneum Architecture\n\
             source: atheneum v0.10.0\n\
             tags: [atheneum, architecture]\n\
             ---\n\
             Dual-layer graph and SQL architecture for agent coordination.\n",
            Some("proj"),
        )
        .expect("atheneum architecture");
    graph
        .ingest_wiki_page(
            "wiki/autoresearch-autonomous-agent-loop.md",
            "---\n\
             title: Autoresearch - Autonomous Agent Research Loop\n\
             tags: [karpathy, constraints]\n\
             ---\n\
             Constraint design: single file, fixed budget, single metric, git lineage.\n",
            Some("proj"),
        )
        .expect("autoresearch");
    graph
        .ingest_wiki_page(
            "wiki/dream-synthesis-2026-06-27.md",
            "---\n\
             title: Dream Synthesis — 2026-06-27\n\
             tags: [synthesis, thesis]\n\
             ---\n\
             Sovereign LLM platform thesis linking rocmforge, graphtransformer, and odincode.\n\
             Constraint design is a cross-project pattern.\n",
            Some("proj"),
        )
        .expect("dream synthesis");

    // Lower-signal or support-style entities that should not dominate.
    graph
        .ingest_wiki_page(
            "wiki/rocmforge-CHANGELOG.md",
            "---\n\
             title: ROCmForge Changelog\n\
             ---\n\
             Capabilities and architecture changes across releases.\n",
            Some("proj"),
        )
        .expect("rocmforge changelog");
    graph
        .ingest_wiki_page(
            "wiki/001-Untitled.md",
            "Random notes about Articles and Medium Reading List.\n",
            Some("proj"),
        )
        .expect("untitled page");
    graph
        .insert_reasoning_log(
            "agent",
            "odincode plus rocmforge plus graphtransformer forms a sovereign llm platform",
            Some("proj"),
        )
        .expect("reasoning log");

    graph
}

#[test]
fn test_semantic_search_returns_matching_discoveries() {
    let graph = AtheneumGraph::open_in_memory().expect("open");

    graph
        .store_discovery(
            "agent",
            "Symbol",
            "build_router",
            json!({"file": "src/http.rs", "summary": "constructs the axum Router with all routes"}),
        )
        .expect("store");

    graph
        .store_discovery(
            "agent",
            "Symbol",
            "parse_frontmatter",
            json!({"file": "src/graph/mod.rs", "summary": "parses YAML frontmatter from markdown"}),
        )
        .expect("store");

    graph.build_search_index().expect("build_search_index");

    // Query that semantically matches the router discovery
    let results = graph
        .lexical_search("router construction axum routes", 5, None, None, None)
        .expect("search");

    assert!(!results.is_empty(), "search should return some results");
    let first = &results[0];
    assert_eq!(
        first.name, "agent: build_router",
        "best match should be the router discovery (got: {})",
        first.name
    );
}

#[test]
fn test_semantic_search_respects_k_limit() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    for i in 0..7 {
        graph
            .store_discovery(
                "a",
                "Symbol",
                &format!("sym_{}", i),
                json!({"summary": format!("test discovery number {}", i)}),
            )
            .expect("store");
    }
    graph.build_search_index().expect("build_search_index");

    let results = graph
        .lexical_search("test discovery", 3, None, None, None)
        .expect("search");
    assert!(
        results.len() <= 3,
        "search must respect k limit (got {} results)",
        results.len()
    );
}

#[test]
fn test_semantic_search_filtered_by_project() {
    let graph = AtheneumGraph::open_in_memory().expect("open");

    graph
        .store_discovery_in_project(
            "a",
            "Symbol",
            "Message",
            Some("envoy"),
            json!({"summary": "envoy message struct"}),
        )
        .expect("store");
    graph
        .store_discovery_in_project(
            "a",
            "Symbol",
            "Message",
            Some("magellan"),
            json!({"summary": "magellan protocol message"}),
        )
        .expect("store");

    graph.build_search_index().expect("build_search_index");

    let envoy_only = graph
        .lexical_search("message", 10, Some("envoy"), None, None)
        .expect("search");
    assert!(
        envoy_only
            .iter()
            .all(|r| r.data.get("project_id").and_then(|v| v.as_str()) == Some("envoy")),
        "filter must only return envoy discoveries (got: {:?})",
        envoy_only
            .iter()
            .map(|r| r.data.get("project_id").cloned())
            .collect::<Vec<_>>()
    );
    assert!(
        !envoy_only.is_empty(),
        "envoy filter should still return results"
    );

    let mag_only = graph
        .lexical_search("message", 10, Some("magellan"), None, None)
        .expect("search");
    assert!(mag_only
        .iter()
        .all(|r| r.data.get("project_id").and_then(|v| v.as_str()) == Some("magellan")));
}

#[test]
fn test_semantic_search_falls_back_when_hnsw_index_is_inconsistent() {
    let db_file = tempfile::NamedTempFile::new().expect("tempfile");
    let db_path = db_file.path().to_path_buf();

    {
        let graph = AtheneumGraph::open(&db_path).expect("open");
        graph
            .store_discovery(
                "agent",
                "Symbol",
                "build_router",
                json!({"summary": "constructs the axum router"}),
            )
            .expect("store");
    }

    {
        let conn = Connection::open(&db_path).expect("open sqlite");
        conn.execute(
            "DELETE FROM hnsw_vectors
             WHERE index_id = (SELECT id FROM hnsw_indexes WHERE name = 'discoveries')
               AND id = (
                   SELECT MIN(id) FROM hnsw_vectors
                   WHERE index_id = (SELECT id FROM hnsw_indexes WHERE name = 'discoveries')
               )",
            [],
        )
        .expect("corrupt search index");
    }

    let graph = AtheneumGraph::open(&db_path).expect("reopen");
    let results = graph
        .lexical_search("build router", 5, None, None, None)
        .expect("search should fall back instead of failing");

    assert!(
        results.iter().any(|r| r.name == "agent: build_router"),
        "fallback search should still find the discovery"
    );
}

#[test]
fn test_lexical_search_respects_max_tokens() {
    let graph = AtheneumGraph::open_in_memory().expect("open");

    // Store several discoveries with verbose metadata
    for i in 0..5 {
        graph
            .store_discovery(
                "agent1",
                "symbol",
                &format!("target_{}", i),
                json!({"description": "a".repeat(1000)}),
            )
            .expect("store");
    }

    // Without budget: should find our 5 stored discoveries
    let all = graph
        .lexical_search("target", 10, None, None, None)
        .expect("search");
    assert!(
        all.len() >= 5,
        "should find at least the 5 stored discoveries, got {}",
        all.len()
    );

    // With tight budget: should return fewer results
    let truncated = graph
        .lexical_search("target", 10, None, None, Some(50))
        .expect("search");
    assert!(
        truncated.len() < all.len(),
        "max_tokens should truncate results: got {} vs {}",
        truncated.len(),
        all.len()
    );
}

#[test]
fn test_lexical_search_prefers_wikipage_over_file_shadow() {
    let graph = AtheneumGraph::open_in_memory().expect("open");

    graph
        .ingest_wiki_page(
            "wiki/rocmforge-capabilities.md",
            "---\n\
             title: ROCmForge Capabilities\n\
             source: rocmforge v1.0\n\
             ---\n\
             GPU inference capabilities and dispatch details.\n",
            Some("proj"),
        )
        .expect("ingest wiki page");

    graph
        .insert_reasoning_log(
            "agent",
            "read wiki/rocmforge-capabilities.md before writing capability notes",
            Some("proj"),
        )
        .expect("reasoning log");

    let results = graph
        .lexical_search("rocmforge capabilities", 10, Some("proj"), None, None)
        .expect("search");

    assert!(!results.is_empty(), "should return results");
    assert_eq!(
        results[0].kind, "WikiPage",
        "canonical wiki page should outrank file/transcript style matches"
    );
    assert_eq!(
        results[0].name, "wiki/rocmforge-capabilities.md",
        "top hit should be the canonical capabilities page"
    );
}

#[test]
fn test_lexical_search_prefers_architecture_doc_over_changelog_like_page() {
    let graph = AtheneumGraph::open_in_memory().expect("open");

    graph
        .ingest_wiki_page(
            "wiki/rocmforge-architecture.md",
            "---\n\
             title: ROCmForge Architecture\n\
             source: rocmforge v1.0\n\
             ---\n\
             Architecture overview for ROCmForge GPU inference.\n",
            Some("proj"),
        )
        .expect("ingest architecture page");

    graph
        .ingest_wiki_page(
            "wiki/rocmforge-CHANGELOG.md",
            "---\n\
             title: ROCmForge Changelog\n\
             ---\n\
             Architecture work and capability changes across releases.\n",
            Some("proj"),
        )
        .expect("ingest changelog");

    let results = graph
        .lexical_search("rocmforge architecture", 10, Some("proj"), None, None)
        .expect("search");

    assert!(
        results.len() >= 2,
        "expected both architecture and changelog pages to match"
    );
    assert_eq!(results[0].kind, "WikiPage");
    assert_eq!(
        results[0].name, "wiki/rocmforge-architecture.md",
        "architecture page should outrank changelog for architecture query"
    );
}

#[test]
fn test_navigation_benchmark_queries_prefer_authoritative_pages() {
    let graph = build_mixed_navigation_fixture();

    let benchmark_cases = [
        (
            "rocmforge capabilities",
            "wiki/rocmforge-capabilities.md",
            1usize,
        ),
        (
            "atheneum architecture",
            "wiki/atheneum-architecture.md",
            1usize,
        ),
        (
            "constraint design",
            "wiki/autoresearch-autonomous-agent-loop.md",
            3usize,
        ),
        (
            "sovereign llm platform",
            "wiki/dream-synthesis-2026-06-27.md",
            3usize,
        ),
    ];

    for (query, expected_name, top_k) in benchmark_cases {
        let results = graph
            .lexical_search(query, 10, Some("proj"), None, None)
            .expect("search");
        assert!(
            !results.is_empty(),
            "query '{}' should return results",
            query
        );
        assert_top_k_contains(&results, top_k, expected_name);
    }
}

#[test]
fn test_navigation_benchmark_penalizes_low_signal_results_for_architecture_queries() {
    let graph = build_mixed_navigation_fixture();

    let results = graph
        .lexical_search("rocmforge architecture", 5, Some("proj"), None, None)
        .expect("search");

    assert!(
        results.len() >= 3,
        "expected at least 3 results for architecture query"
    );
    assert_eq!(results[0].name, "wiki/rocmforge-architecture.md");
    assert!(
        results
            .iter()
            .take(3)
            .all(|result| result.name != "wiki/001-Untitled.md"),
        "untitled low-signal page should not appear in top 3 for architecture query: {:?}",
        results
            .iter()
            .take(3)
            .map(|r| format!("{}:{}", r.kind, r.name))
            .collect::<Vec<_>>()
    );
}
