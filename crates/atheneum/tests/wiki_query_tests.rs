//! Tests for wiki query APIs and wikilink graph navigation.
//! TDD: write tests first, watch them fail, then implement.

use atheneum::graph::{AtheneumGraph, EdgeType};

#[test]
fn test_get_wiki_page_by_path() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .ingest_wiki_page(
            "wiki/test.md",
            "---\ntitle: Test\n---\nHello world with [[Other Page]]\n",
            Some("proj"),
        )
        .expect("ingest");

    let page = graph
        .get_wiki_page("wiki/test.md")
        .expect("query")
        .expect("page exists");
    assert_eq!(page.title.as_deref(), Some("Test"));
    assert_eq!(page.path, "wiki/test.md");
    assert!(page.body.contains("Hello world"));
}

#[test]
fn test_get_wiki_page_not_found() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let page = graph.get_wiki_page("wiki/nonexistent.md").expect("query");
    assert!(page.is_none());
}

#[test]
fn test_list_wiki_pages() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .ingest_wiki_page("wiki/a.md", "---\ntitle: A\n---\nbody a\n", Some("proj"))
        .expect("ingest");
    graph
        .ingest_wiki_page("wiki/b.md", "---\ntitle: B\n---\nbody b\n", Some("proj"))
        .expect("ingest");

    let pages = graph.list_wiki_pages(Some("proj")).expect("list");
    assert_eq!(pages.len(), 2);
    let titles: Vec<_> = pages.iter().map(|p| p.title.as_deref()).collect();
    assert!(titles.contains(&Some("A")));
    assert!(titles.contains(&Some("B")));
}

#[test]
fn test_preview_entity_candidates_returns_ranked_wiki_matches() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .ingest_wiki_page(
            "wiki/router.md",
            "---\ntitle: HTTP Router\n---\nRoutes requests through the handler graph.\n",
            Some("proj"),
        )
        .expect("ingest");

    let candidates = graph
        .preview_entity_candidates("HTTP Router", 3, Some("proj"), Some("WikiPage"), 0.1)
        .expect("preview");

    assert!(
        !candidates.is_empty(),
        "preview should return at least one candidate"
    );
    assert_eq!(candidates[0].kind, "WikiPage");
    assert_eq!(candidates[0].name, "wiki/router.md");
}

#[test]
fn test_find_pages_by_wikilink() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .ingest_wiki_page(
            "wiki/a.md",
            "---\ntitle: A\n---\nSee also [[Target Page]]\n",
            Some("proj"),
        )
        .expect("ingest");
    graph
        .ingest_wiki_page(
            "wiki/b.md",
            "---\ntitle: B\n---\nAlso see [[Target Page]]\n",
            Some("proj"),
        )
        .expect("ingest");

    let pages = graph
        .find_pages_by_wikilink("Target Page", Some("proj"))
        .expect("find");
    assert_eq!(pages.len(), 2);
}

#[test]
fn test_wikilink_graph_edges_created() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let page_id = graph
        .ingest_wiki_page(
            "wiki/source.md",
            "---\ntitle: Source\n---\nLinks to [[Dest1]] and [[Dest2]]\n",
            Some("proj"),
        )
        .expect("ingest");

    let outgoing = graph.outgoing_wikilinks(page_id).expect("outgoing");
    assert_eq!(outgoing.len(), 2);
    let target_names: Vec<_> = outgoing.iter().map(|e| e.name.clone()).collect();
    assert!(target_names.contains(&"Dest1".to_string()));
    assert!(target_names.contains(&"Dest2".to_string()));
}

#[test]
fn test_ingest_wiki_page_auto_links_high_confidence_title_match() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let target_id = graph
        .ingest_wiki_page(
            "wiki/router.md",
            "---\ntitle: HTTP Router\n---\nRouting notes.\n",
            Some("proj"),
        )
        .expect("target");

    let source_id = graph
        .ingest_wiki_page(
            "wiki/source.md",
            "---\ntitle: Source\n---\nLinks to [[HTTP Router]].\n",
            Some("proj"),
        )
        .expect("source");

    let outgoing = graph.outgoing_wikilinks(source_id).expect("outgoing");
    assert_eq!(outgoing.len(), 1);
    assert_eq!(
        outgoing[0].id, target_id,
        "should link to the existing page"
    );
    assert_eq!(outgoing[0].name, "wiki/router.md");
}

#[test]
fn test_legacy_related_to_wikilink_edges_still_traverse() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let source = graph
        .ingest_wiki_page(
            "wiki/source.md",
            "---\ntitle: Source\n---\nbody\n",
            Some("proj"),
        )
        .expect("source");
    let dest = graph
        .ingest_wiki_page(
            "wiki/dest.md",
            "---\ntitle: Dest\n---\nbody\n",
            Some("proj"),
        )
        .expect("dest");

    graph
        .insert_edge(
            source,
            dest,
            EdgeType::RelatedTo,
            serde_json::json!({"link_type": "wikilink", "target": "Dest"}),
        )
        .expect("legacy edge");

    let outgoing = graph.outgoing_wikilinks(source).expect("outgoing");
    assert!(
        outgoing.iter().any(|e| e.id == dest),
        "legacy related_to wikilink edge should remain traversable"
    );
}

#[test]
fn test_incoming_wikilinks() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .ingest_wiki_page(
            "wiki/dest.md",
            "---\ntitle: Dest\n---\nI am the destination\n",
            Some("proj"),
        )
        .expect("ingest");
    let source_id = graph
        .ingest_wiki_page(
            "wiki/source.md",
            "---\ntitle: Source\n---\nSee [[Dest]]\n",
            Some("proj"),
        )
        .expect("ingest");

    let dest_entity = graph
        .find_wiki_page_entity_id("wiki/dest.md")
        .expect("lookup")
        .expect("dest exists");

    let incoming = graph.incoming_wikilinks(dest_entity).expect("incoming");
    assert_eq!(incoming.len(), 1);
    assert_eq!(incoming[0].id, source_id);
}

#[test]
fn test_list_wiki_pages_filters_by_project() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .ingest_wiki_page("wiki/x.md", "---\ntitle: X\n---\nbody\n", Some("proj-a"))
        .expect("ingest");
    graph
        .ingest_wiki_page("wiki/y.md", "---\ntitle: Y\n---\nbody\n", Some("proj-b"))
        .expect("ingest");

    let pages_a = graph.list_wiki_pages(Some("proj-a")).expect("list");
    assert_eq!(pages_a.len(), 1);
    assert_eq!(pages_a[0].title.as_deref(), Some("X"));

    let all_pages = graph.list_wiki_pages(None).expect("list all");
    assert_eq!(all_pages.len(), 2);
}

#[test]
fn test_search_wiki_pages_uses_fts5_and_returns_excerpts() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .ingest_wiki_page(
            "wiki/router.md",
            "---\ntitle: HTTP Router\n---\nRoutes requests through the handler graph.\n",
            Some("proj"),
        )
        .expect("ingest");

    let hits = graph
        .search_wiki_pages("handler graph", Some("proj"), 0, 10)
        .expect("search");
    assert_eq!(hits.len(), 1, "should find the router page");
    assert_eq!(hits[0].path, "wiki/router.md");
    assert_eq!(hits[0].title.as_deref(), Some("HTTP Router"));
    assert!(
        hits[0].excerpt.contains("handler graph"),
        "excerpt should contain matched text"
    );
    assert!(
        hits[0].excerpt.len() <= 260,
        "excerpt should be bounded around query token"
    );
}

#[test]
fn test_search_wiki_pages_is_paginated() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    for i in 0..5 {
        graph
            .ingest_wiki_page(
                &format!("wiki/p{}.md", i),
                &format!(
                    "---\ntitle: Page {}\n---\nThis page discusses search pagination.\n",
                    i
                ),
                Some("proj"),
            )
            .expect("ingest");
    }

    let page1 = graph
        .search_wiki_pages("search pagination", Some("proj"), 0, 2)
        .expect("page1");
    assert_eq!(page1.len(), 2);
    let page2 = graph
        .search_wiki_pages("search pagination", Some("proj"), 2, 2)
        .expect("page2");
    assert_eq!(page2.len(), 2);
    let page3 = graph
        .search_wiki_pages("search pagination", Some("proj"), 4, 2)
        .expect("page3");
    assert_eq!(page3.len(), 1);

    let paths: Vec<_> = page1
        .iter()
        .chain(page2.iter())
        .chain(page3.iter())
        .map(|h| h.path.clone())
        .collect();
    let mut sorted = paths.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        5,
        "paginated results should cover all 5 pages without overlap"
    );
}

#[test]
fn test_backfill_wiki_pages_repairs_stub_entities() {
    let graph = AtheneumGraph::open_in_memory().expect("open");

    // Simulate the old stub pattern: a real wiki_pages row but a stub graph entity.
    graph.with_raw_connection(|conn| {
        conn.execute(
            "INSERT INTO wiki_pages (path, title, body, content_hash, wikilinks, project_id, metadata, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                "wiki/stubbed.md",
                Some("Stubbed Article"),
                "This article links to [[HTTP Router]].",
                "abc123",
                "[]",
                Some("proj"),
                None::<String>,
                "2026-06-16T00:00:00Z",
                None::<String>,
            ],
        )?;
        let wiki_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO graph_entities (kind, name, file_path, data)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                "WikiPage",
                "wiki/stubbed.md",
                None::<String>,
                serde_json::to_string(&serde_json::json!({"stub": true, "project_id": "proj"})).unwrap(),
            ],
        )?;
        let entity_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO wiki_pages_fts (rowid, title, body, path) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![wiki_id, Some("Stubbed Article"), "This article links to [[HTTP Router]].", "wiki/stubbed.md"],
        )?;
        Ok(entity_id)
    }).expect("seed stub");

    let entity_id_before = graph
        .find_wiki_page_entity_id("wiki/stubbed.md")
        .expect("lookup")
        .expect("entity exists");

    let fixed = graph
        .backfill_wiki_pages_to_graph(Some("proj"))
        .expect("backfill");
    assert_eq!(fixed.len(), 1);
    assert_eq!(fixed[0].1, "wiki/stubbed.md");

    // Same path should map to the same graph entity id.
    let entity_id_after = graph
        .find_wiki_page_entity_id("wiki/stubbed.md")
        .expect("lookup")
        .expect("entity exists");
    assert_eq!(entity_id_before, entity_id_after);

    let entity = graph.get_entity(entity_id_after).expect("get entity");
    assert!(
        entity.data.get("body").is_some(),
        "stubbed entity should now contain a real body"
    );
    assert_eq!(
        entity.data.get("stub").and_then(|v| v.as_bool()),
        None,
        "stub flag should be removed"
    );

    // ingest_wiki_page creates stub targets for missing wikilinks, so the
    // backfilled page should now have an outgoing edge to the "HTTP Router" stub.
    let outgoing = graph.outgoing_wikilinks(entity_id_after).expect("outgoing");
    assert_eq!(
        outgoing.len(),
        1,
        "stub target should be created for unresolved wikilink"
    );
    assert_eq!(outgoing[0].name, "HTTP Router");
}

#[test]
fn test_backfill_wiki_pages_creates_missing_entities() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph.with_raw_connection(|conn| {
        conn.execute(
            "INSERT INTO wiki_pages (path, title, body, content_hash, wikilinks, project_id, metadata, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                "wiki/orphan.md",
                Some("Orphan Article"),
                "Body with no graph entity yet.",
                "def456",
                "[]",
                Some("proj"),
                None::<String>,
                "2026-06-16T00:00:00Z",
                None::<String>,
            ],
        )?;
        let wiki_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO wiki_pages_fts (rowid, title, body, path) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![wiki_id, Some("Orphan Article"), "Body with no graph entity yet.", "wiki/orphan.md"],
        )?;
        Ok(())
    }).expect("seed orphan");

    let before = graph
        .find_wiki_page_entity_id("wiki/orphan.md")
        .expect("lookup");
    assert!(
        before.is_none(),
        "orphan article should have no graph entity"
    );

    let fixed = graph
        .backfill_wiki_pages_to_graph(Some("proj"))
        .expect("backfill");
    assert_eq!(fixed.len(), 1);

    let after = graph
        .find_wiki_page_entity_id("wiki/orphan.md")
        .expect("lookup");
    assert!(
        after.is_some(),
        "orphan article should now have a graph entity"
    );
}

#[test]
fn test_search_wiki_pages_does_not_return_full_body() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let long_body = "word ".repeat(1000);
    graph
        .ingest_wiki_page(
            "wiki/long.md",
            &format!(
                "---\ntitle: Long\n---\n{} target_token {}\n",
                long_body, long_body
            ),
            Some("proj"),
        )
        .expect("ingest");

    let hits = graph
        .search_wiki_pages("target_token", Some("proj"), 0, 10)
        .expect("search");
    assert_eq!(hits.len(), 1);
    assert!(
        !hits[0].excerpt.contains(&long_body),
        "excerpt must not contain the full 1000-word block"
    );
    assert!(
        hits[0].excerpt.contains("target_token"),
        "excerpt should still include the matched token"
    );
}

#[test]
fn test_search_wiki_pages_by_path_fragment() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .ingest_wiki_page(
            "wiki/session-accountability.md",
            "---\ntitle: Accountability Notes\n---\nSome notes about project governance.\n",
            Some("proj"),
        )
        .expect("ingest");

    // The body does NOT contain "session", but the path does.
    let hits = graph
        .search_wiki_pages("session", Some("proj"), 0, 10)
        .expect("search");
    assert_eq!(hits.len(), 1, "should match path fragment");
    assert_eq!(hits[0].path, "wiki/session-accountability.md");
}

#[test]
fn test_search_wiki_pages_prefix_wildcard() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .ingest_wiki_page(
            "wiki/router.md",
            "---\ntitle: HTTP Router\n---\nRoutes requests through the handler graph.\n",
            Some("proj"),
        )
        .expect("ingest");

    // "rout" is a prefix of "Routes" (body) and "Router" (title).
    let hits = graph
        .search_wiki_pages("rout", Some("proj"), 0, 10)
        .expect("search");
    assert!(
        hits.iter().any(|h| h.path == "wiki/router.md"),
        "prefix wildcard should match 'rout' against 'Routes' and 'Router'"
    );
}

#[test]
fn test_search_wiki_pages_no_results_for_gibberish() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .ingest_wiki_page("wiki/a.md", "---\ntitle: A\n---\nbody\n", Some("proj"))
        .expect("ingest");

    let hits = graph
        .search_wiki_pages("xyz123nonsense", Some("proj"), 0, 10)
        .expect("search");
    assert!(hits.is_empty(), "gibberish query should return no results");
}
