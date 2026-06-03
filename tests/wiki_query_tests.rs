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
