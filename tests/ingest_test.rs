//! Tests for wiki article ingestion
//! Tests are written FIRST (TDD) and will fail until implementation is complete.

use std::path::PathBuf;

use atheneum::graph::{AtheneumGraph, EdgeType, EntityType};

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

    let result = graph.ingest_article("test-article.md", article_content);

    assert!(result.is_ok(), "Article ingestion should succeed");

    let article_id = result.unwrap();

    // Verify the article was stored
    let article = graph
        .get_entity(article_id)
        .expect("Failed to retrieve article");

    assert_eq!(article.kind, EntityType::Knowledge.as_str());
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
        .ingest_article("another.md", article_content)
        .expect("Failed to ingest");

    // Should create an event recording the ingestion
    // The system agent gets ID 1 (first agent inserted)
    let events = graph
        .events_performed_by(1) // System agent
        .expect("Failed to get events");

    assert!(!events.is_empty(), "Should have created an ingestion event");

    let ingest_event = &events[0];
    assert_eq!(ingest_event.name, "article-ingested");
}

#[test]
fn test_ingest_real_wiki_article() {
    let graph = AtheneumGraph::open_in_memory().expect("Failed to create graph");

    let article_path =
        PathBuf::from("/home/feanor/wiki/concepts/core-hypothesis-sparse-inference.md");

    // Skip test if file doesn't exist
    if !article_path.exists() {
        return;
    }

    let content = std::fs::read_to_string(&article_path).expect("Failed to read article");

    let result = graph.ingest_article(article_path.to_str().unwrap(), &content);

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
fn test_ingest_creates_event_to_knowledge_edge() {
    let graph = AtheneumGraph::open_in_memory().expect("Failed to create graph");

    let article_content = r#"---
title: Edge Test
---

# Content
"#;

    let article_id = graph
        .ingest_article("edge-test.md", article_content)
        .expect("Failed to ingest");

    // Find the ingestion event
    let events = graph
        .events_performed_by(1)
        .expect("Failed to get events for system agent");

    assert!(!events.is_empty(), "Should have an ingestion event");
    let event = &events[0];

    // Verify there's an edge from event to the article (Created edge)
    // This tests graph traversal: Event --Created--> Knowledge
    let outgoing_edges = graph
        .outgoing_edges(event.id)
        .expect("Failed to get outgoing edges");

    let created_edge = outgoing_edges
        .iter()
        .find(|e| e.edge_type == EdgeType::Created.as_str() && e.to_id == article_id)
        .expect("Should have Created edge from event to article");

    assert_eq!(created_edge.from_id, event.id);
    assert_eq!(created_edge.to_id, article_id);
    assert_eq!(created_edge.edge_type, "created");
}
