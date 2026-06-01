//! Tests for wiki article ingestion using the modern `ingest_wiki_page` API.
use atheneum::graph::{AtheneumGraph, EdgeType};

#[test]
fn test_ingest_simple_article() {
    let graph = AtheneumGraph::open_in_memory().expect("Failed to create graph");

    let article_content = r#"---
title: Test Article
created: 2026-05-09
type: concept
tags: [test, example]
---

# Test Article

This is a test article for ingestion.

## Section One

Some content here.
"#;

    let result = graph.ingest_wiki_page("test-article.md", article_content, None);

    assert!(result.is_ok(), "Article ingestion should succeed");

    let article_id = result.unwrap();

    // Verify the article was stored
    let article = graph
        .get_entity(article_id)
        .expect("Failed to retrieve article");

    assert_eq!(article.kind, "WikiPage");
    assert_eq!(article.name, "test-article.md");

    // Verify frontmatter was extracted
    let data = article.data.as_object().expect("Data should be object");
    assert_eq!(data["title"], "Test Article");
    assert_eq!(data["type"], "concept");
    assert!(data["tags"].is_array());
}

#[test]
fn test_ingest_article_creates_event() {
    let graph = AtheneumGraph::open_in_memory().expect("Failed to create graph");

    let article_content = r#"---
title: Another Test
---

# Content
"#;

    let _article_id = graph
        .ingest_wiki_page("another.md", article_content, None)
        .expect("Failed to ingest");

    // Verify the page is queryable via SQL table
    let page = graph
        .get_wiki_page("another.md")
        .expect("Failed to query wiki page")
        .expect("Wiki page should exist");
    assert_eq!(page.path, "another.md");
}

#[test]
fn test_ingest_real_wiki_article() {
    let graph = AtheneumGraph::open_in_memory().expect("Failed to create graph");

    let content = r#"---
title: "Core Hypothesis — Sparse Inference"
type: concept
confidence: high
status: working
---

# Core Hypothesis — Sparse Inference

Sparse inference is a reasoning strategy that..."#;

    let result = graph.ingest_wiki_page(
        "concepts/core-hypothesis-sparse-inference.md",
        content,
        None,
    );

    assert!(result.is_ok(), "Real article ingestion should succeed");

    let article_id = result.unwrap();
    let article = graph.get_entity(article_id).expect("Failed to retrieve");

    // Verify frontmatter extraction
    let data = article.data.as_object().expect("Data should be object");
    assert_eq!(data["title"], "Core Hypothesis — Sparse Inference");
    assert_eq!(data["type"], "concept");
    assert_eq!(data["confidence"], "high");
    assert_eq!(data["status"], "working");
}

#[test]
fn test_ingest_creates_wikilink_edge() {
    let graph = AtheneumGraph::open_in_memory().expect("Failed to create graph");

    let article_content = r#"---
title: Edge Test
---

# Content

Link to [[Another Page]] and [[Missing Page]].
"#;

    let article_id = graph
        .ingest_wiki_page("edge-test.md", article_content, None)
        .expect("Failed to ingest");

    // Verify outgoing wikilink edges
    let outgoing = graph
        .outgoing_wikilinks(article_id)
        .expect("Failed to get outgoing wikilinks");

    assert!(!outgoing.is_empty(), "Should have outgoing wikilink edges");

    // Verify at least one edge is a RelatedTo edge
    let wikilink_edges = graph
        .outgoing_edges(article_id)
        .expect("Failed to get outgoing edges")
        .into_iter()
        .filter(|e| e.edge_type == EdgeType::RelatedTo.as_str())
        .collect::<Vec<_>>();

    assert!(
        !wikilink_edges.is_empty(),
        "Should have RelatedTo edges from wikilinks"
    );

    // Missing target should have created a stub entity
    let stub_exists = outgoing.iter().any(|e| {
        e.data
            .get("stub")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    });
    assert!(
        stub_exists,
        "Should create stub for missing wikilink target"
    );
}
