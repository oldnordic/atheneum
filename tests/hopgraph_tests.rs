//! TDD tests for Atheneum HopGraph Phases 1–4.
//!
//! Covers:
//!   - EdgeType::Explains and EdgeType::DerivedFrom roundtrip
//!   - EntityType::WikiPage variant
//!   - Wiki pages as first-class graph entities (ingest creates GraphEntity)
//!   - get_subgraph with edge-type filter
//!   - Explains edge from wiki page to symbol via magellan bridge
//!   - Token estimation and subgraph truncation
//!   - hopgraph_query() convenience API
//!   - TextEmbedder trait + HashEmbedder + integration
//!   - Discovery consolidation → Knowledge entities

use atheneum::graph::{AtheneumGraph, EdgeType, EntityType};
use serde_json::json;

fn create_fake_magellan_db(symbols: &[(&str, &str)]) -> tempfile::NamedTempFile {
    let db = tempfile::NamedTempFile::new().expect("tempfile");
    let conn = rusqlite::Connection::open(db.path()).expect("open temp db");
    conn.execute_batch(
        "CREATE TABLE graph_entities (
            id        INTEGER PRIMARY KEY AUTOINCREMENT,
            kind      TEXT NOT NULL,
            name      TEXT NOT NULL,
            file_path TEXT,
            data      TEXT NOT NULL
        );",
    )
    .expect("schema");
    for (name, fqn) in symbols {
        let data = serde_json::json!({
            "fqn": fqn,
            "kind": "function",
        });
        conn.execute(
            "INSERT INTO graph_entities (kind, name, file_path, data) VALUES ('Symbol', ?1, 'src/lib.rs', ?2)",
            rusqlite::params![name, serde_json::to_string(&data).unwrap()],
        )
        .expect("insert symbol");
    }
    drop(conn);
    db
}

#[test]
fn test_edge_type_explains_roundtrip() {
    let e = EdgeType::Explains;
    assert_eq!(e.as_str(), "explains");
    assert_eq!(EdgeType::from_label("explains"), Some(EdgeType::Explains));
}

#[test]
fn test_edge_type_derived_from_roundtrip() {
    let e = EdgeType::DerivedFrom;
    assert_eq!(e.as_str(), "derived_from");
    assert_eq!(
        EdgeType::from_label("derived_from"),
        Some(EdgeType::DerivedFrom)
    );
}

#[test]
fn test_edge_type_explains_serde_roundtrip() {
    let json = serde_json::to_string(&EdgeType::Explains).expect("serialize");
    assert_eq!(json, "\"explains\"");
    let back: EdgeType = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, EdgeType::Explains);
}

#[test]
fn test_edge_type_derived_from_serde_roundtrip() {
    let json = serde_json::to_string(&EdgeType::DerivedFrom).expect("serialize");
    assert_eq!(json, "\"derived_from\"");
    let back: EdgeType = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, EdgeType::DerivedFrom);
}

#[test]
fn test_edge_type_explains_in_all() {
    assert!(
        EdgeType::all().contains(&EdgeType::Explains),
        "Explains must appear in EdgeType::all()"
    );
}

#[test]
fn test_edge_type_derived_from_in_all() {
    assert!(
        EdgeType::all().contains(&EdgeType::DerivedFrom),
        "DerivedFrom must appear in EdgeType::all()"
    );
}

#[test]
fn test_entity_type_wiki_page() {
    assert_eq!(EntityType::WikiPage.as_str(), "WikiPage");
}

#[test]
fn test_ingest_wiki_page_creates_graph_entity() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let page_id = graph
        .ingest_wiki_page(
            "wiki/test.md",
            "---\ntitle: Test\n---\nHello world\n",
            Some("proj"),
        )
        .expect("ingest");

    let entity = graph.get_entity(page_id).expect("get_entity");
    assert_eq!(entity.kind, "WikiPage");
    assert_eq!(entity.name, "wiki/test.md");
    assert_eq!(entity.file_path, Some("wiki/test.md".to_string()));
}

#[test]
fn test_ingest_wiki_page_stores_project_id_in_data() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let page_id = graph
        .ingest_wiki_page(
            "wiki/test.md",
            "---\ntitle: Test\n---\nHello world\n",
            Some("my-project"),
        )
        .expect("ingest");

    let entity = graph.get_entity(page_id).expect("get_entity");
    assert_eq!(
        entity.data.get("project_id").and_then(|v| v.as_str()),
        Some("my-project")
    );
}

#[test]
fn test_ingest_wiki_page_creates_wikilink_edges() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let page_id = graph
        .ingest_wiki_page(
            "wiki/source.md",
            "---\ntitle: Source\n---\nSee [[Dest]] and [[Other]]\n",
            Some("proj"),
        )
        .expect("ingest");

    let (outgoing, _) = graph.get_neighbors(page_id).expect("neighbors");
    let wikilink_edges: Vec<_> = outgoing
        .iter()
        .filter(|e| e.edge_type == EdgeType::Wikilink.as_str())
        .collect();
    assert_eq!(wikilink_edges.len(), 2, "should have 2 wikilink edges");
}

#[test]
fn test_ingest_wiki_page_idempotent() {
    let graph = AtheneumGraph::open_in_memory().expect("open");

    let id1 = graph
        .ingest_wiki_page("wiki/test.md", "---\ntitle: V1\n---\nv1\n", Some("proj"))
        .expect("ingest1");
    let id2 = graph
        .ingest_wiki_page("wiki/test.md", "---\ntitle: V2\n---\nv2\n", Some("proj"))
        .expect("ingest2");

    assert_eq!(
        id1, id2,
        "re-ingesting same path should return same entity id"
    );
}

#[test]
fn test_get_subgraph_filtered_by_edge_type() {
    let graph = AtheneumGraph::open_in_memory().expect("open");

    let page_id = graph
        .ingest_wiki_page(
            "wiki/source.md",
            "---\ntitle: Source\n---\nLinks to [[Dest]]\n",
            Some("proj"),
        )
        .expect("ingest");

    let agent = graph.insert_agent("test-agent", json!({})).expect("agent");
    graph
        .insert_edge(
            page_id,
            agent,
            EdgeType::Mentions,
            json!({"context": "test"}),
        )
        .expect("mentions edge");

    let sg = graph
        .get_subgraph_filtered(page_id, 2, &[EdgeType::Wikilink])
        .expect("subgraph filtered");

    assert!(
        sg.edges
            .iter()
            .any(|e| e.edge_type == EdgeType::Wikilink.as_str()),
        "should contain wikilink edge"
    );
    assert!(
        !sg.edges
            .iter()
            .any(|e| e.edge_type == EdgeType::Mentions.as_str()),
        "should NOT contain mentions edge"
    );
}

#[test]
fn test_get_subgraph_filtered_empty_types_returns_all() {
    let graph = AtheneumGraph::open_in_memory().expect("open");

    let page_id = graph
        .ingest_wiki_page(
            "wiki/source.md",
            "---\ntitle: Source\n---\nLinks to [[Dest]]\n",
            Some("proj"),
        )
        .expect("ingest");

    let sg = graph
        .get_subgraph_filtered(page_id, 2, &[])
        .expect("subgraph filtered");

    assert!(!sg.edges.is_empty(), "empty filter should return all edges");
}

#[test]
fn test_get_subgraph_filtered_multiple_types() {
    let graph = AtheneumGraph::open_in_memory().expect("open");

    let page_id = graph
        .ingest_wiki_page(
            "wiki/source.md",
            "---\ntitle: Source\n---\nLinks to [[Dest]]\n",
            Some("proj"),
        )
        .expect("ingest");

    let agent = graph.insert_agent("test-agent", json!({})).expect("agent");
    graph
        .insert_edge(page_id, agent, EdgeType::Explains, json!({}))
        .expect("explains edge");

    let sg = graph
        .get_subgraph_filtered(page_id, 2, &[EdgeType::Wikilink, EdgeType::Explains])
        .expect("subgraph filtered");

    assert!(
        sg.edges
            .iter()
            .any(|e| e.edge_type == EdgeType::Wikilink.as_str()),
        "should contain wikilink edge"
    );
    assert!(
        sg.edges
            .iter()
            .any(|e| e.edge_type == EdgeType::Explains.as_str()),
        "should contain explains edge"
    );
    assert_eq!(sg.edges.len(), 2, "1 wikilink + 1 explains = 2");
}

#[test]
fn test_link_wiki_to_symbols_creates_explains_edges() {
    let magellan = create_fake_magellan_db(&[
        ("build_router", "atheneum::build_router"),
        ("AtheneumGraph", "atheneum::AtheneumGraph"),
    ]);

    let graph = AtheneumGraph::open_in_memory().expect("open");
    let page_id = graph
        .ingest_wiki_page(
            "wiki/http-handler.md",
            "---\ntitle: HTTP Handler\n---\nSee [[build_router]] for details.\n",
            Some("atheneum"),
        )
        .expect("ingest");

    let linked = graph
        .link_wiki_to_symbols(magellan.path(), "atheneum", Some("atheneum"))
        .expect("link");

    assert_eq!(linked, 1, "should link 1 wikilink to a symbol");

    let (outgoing, _) = graph.get_neighbors(page_id).expect("neighbors");
    let explains_edges: Vec<_> = outgoing
        .iter()
        .filter(|e| e.edge_type == EdgeType::Explains.as_str())
        .collect();
    assert_eq!(explains_edges.len(), 1, "should have 1 Explains edge");

    let target_id = explains_edges[0].to_id;
    let target = graph.get_entity(target_id).expect("target entity");
    assert_eq!(target.name, "atheneum: build_router");
}

#[test]
fn test_link_wiki_to_symbols_no_match_is_zero() {
    let magellan = create_fake_magellan_db(&[("unrelated_symbol", "foo::bar")]);

    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .ingest_wiki_page(
            "wiki/test.md",
            "---\ntitle: Test\n---\nSee [[build_router]] for details.\n",
            Some("atheneum"),
        )
        .expect("ingest");

    let linked = graph
        .link_wiki_to_symbols(magellan.path(), "atheneum", Some("atheneum"))
        .expect("link");

    assert_eq!(linked, 0, "no matching symbols should link 0");
}

#[test]
fn test_link_wiki_to_symbols_idempotent() {
    let magellan = create_fake_magellan_db(&[("build_router", "atheneum::build_router")]);

    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .ingest_wiki_page(
            "wiki/test.md",
            "---\ntitle: Test\n---\nSee [[build_router]]\n",
            Some("atheneum"),
        )
        .expect("ingest");

    let linked1 = graph
        .link_wiki_to_symbols(magellan.path(), "atheneum", Some("atheneum"))
        .expect("link1");
    let linked2 = graph
        .link_wiki_to_symbols(magellan.path(), "atheneum", Some("atheneum"))
        .expect("link2");

    assert_eq!(linked1, 1, "first run links 1 symbol");
    assert_eq!(linked2, 0, "second run should not duplicate edges");
}

#[test]
fn test_estimate_tokens_entity() {
    use atheneum::GraphEntity;

    let entity = GraphEntity {
        id: 1,
        kind: "WikiPage".to_string(),
        name: "wiki/test.md".to_string(),
        file_path: Some("wiki/test.md".to_string()),
        data: serde_json::json!({"title": "Test Page", "body": "Hello world"}),
    };
    let tokens = atheneum::graph::estimate_entity_tokens(&entity);
    assert!(tokens > 0, "token estimate should be positive");
    assert!(
        tokens < 500,
        "small entity should be under 500 tokens, got {}",
        tokens
    );
}

#[test]
fn test_estimate_tokens_is_consistent() {
    use atheneum::GraphEntity;

    let e1 = GraphEntity {
        id: 1,
        kind: "A".to_string(),
        name: "x".to_string(),
        file_path: None,
        data: serde_json::json!({}),
    };
    let e2 = GraphEntity {
        id: 2,
        kind: "B".to_string(),
        name: "y".to_string(),
        file_path: None,
        data: serde_json::json!({"extra": "data here"}),
    };
    let t1 = atheneum::graph::estimate_entity_tokens(&e1);
    let t2 = atheneum::graph::estimate_entity_tokens(&e2);
    assert!(
        t2 > t1,
        "entity with more data should have higher token estimate ({} vs {})",
        t2,
        t1
    );
}

#[test]
fn test_truncate_subgraph_keeps_entry() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let page_id = graph
        .ingest_wiki_page(
            "wiki/entry.md",
            "---\ntitle: Entry\n---\nLinks to [[A]] and [[B]] and [[C]]\n",
            Some("proj"),
        )
        .expect("ingest");

    let sg = graph.get_subgraph(page_id, 2).expect("subgraph");
    let full_count = sg.entities.len();
    assert!(full_count > 1, "should have more than just the entry");

    let entry_tokens = atheneum::graph::estimate_entity_tokens(&sg.entry);
    let budget = entry_tokens + 1;

    let truncated = atheneum::graph::truncate_subgraph(sg, budget);
    assert!(
        truncated.entities.len() < full_count,
        "tight budget should drop some entities"
    );
    assert_eq!(truncated.entry.id, page_id, "entry must always be kept");
}

#[test]
fn test_truncate_subgraph_drops_orphan_edges() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let page_id = graph
        .ingest_wiki_page(
            "wiki/entry.md",
            "---\ntitle: Entry\n---\nLinks to [[A]] and [[B]]\n",
            Some("proj"),
        )
        .expect("ingest");

    let sg = graph.get_subgraph(page_id, 2).expect("subgraph");
    let truncated = atheneum::graph::truncate_subgraph(sg, 1);

    for edge in &truncated.edges {
        let from_exists = truncated.entities.iter().any(|e| e.id == edge.from_id);
        let to_exists = truncated.entities.iter().any(|e| e.id == edge.to_id);
        assert!(
            from_exists && to_exists,
            "orphan edge found: from={} to={}",
            edge.from_id,
            edge.to_id
        );
    }
}

#[test]
fn test_hopgraph_query_returns_subgraph() {
    let graph = AtheneumGraph::open_in_memory().expect("open");

    let page_id = graph
        .ingest_wiki_page(
            "wiki/router.md",
            "---\ntitle: Router\n---\nThe HTTP router builds endpoints\n",
            Some("atheneum"),
        )
        .expect("ingest");

    let agent = graph.insert_agent("test", json!({})).expect("agent");
    graph
        .insert_edge(page_id, agent, EdgeType::Mentions, json!({}))
        .expect("edge");

    let views = graph
        .hopgraph_query(
            "router",
            3,
            2,
            &[EdgeType::Wikilink, EdgeType::Explains],
            2000,
            Some("atheneum"),
        )
        .expect("hopgraph_query");

    assert!(
        !views.is_empty(),
        "should find at least one result for 'router'"
    );
}

#[test]
fn test_hopgraph_query_respects_token_budget() {
    let graph = AtheneumGraph::open_in_memory().expect("open");

    for i in 0..10 {
        let body = format!(
            "---\ntitle: Page {}\n---\nContent about topic {} with details\n",
            i, i
        );
        graph
            .ingest_wiki_page(&format!("wiki/page{}.md", i), &body, Some("proj"))
            .expect("ingest");
    }

    let views = graph
        .hopgraph_query("topic", 5, 2, &[], 50, Some("proj"))
        .expect("hopgraph_query");

    if !views.is_empty() {
        let total_tokens: usize = views
            .iter()
            .map(|v| {
                v.entities
                    .iter()
                    .map(atheneum::graph::estimate_entity_tokens)
                    .sum::<usize>()
            })
            .sum();
        assert!(
            total_tokens <= 150,
            "token budget of 50 should produce ~{} tokens, got {}",
            50,
            total_tokens
        );
    }
}

mod embedder_tests {
    use atheneum::graph::embed::{HashEmbedder, TextEmbedder};

    #[test]
    fn test_hash_embedder_dimension() {
        let embedder = HashEmbedder::new(128);
        assert_eq!(embedder.dimension(), 128);
    }

    #[test]
    fn test_hash_embedder_produces_vector() {
        let embedder = HashEmbedder::new(128);
        let vec = embedder.embed("hello world").expect("embed");
        assert_eq!(vec.len(), 128);
    }

    #[test]
    fn test_hash_embedder_normalized() {
        let embedder = HashEmbedder::new(128);
        let vec = embedder.embed("hello world").expect("embed");
        let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 0.01,
            "vector should be approximately unit length, norm = {}",
            norm
        );
    }

    #[test]
    fn test_hash_embedder_similar_texts_close() {
        let embedder = HashEmbedder::new(128);
        let v1 = embedder.embed("build router function").expect("embed");
        let v2 = embedder.embed("build router function").expect("embed");
        let v3 = embedder
            .embed("completely different topic xyz")
            .expect("embed");

        let sim_same = cosine_similarity(&v1, &v2);
        let sim_diff = cosine_similarity(&v1, &v3);

        assert!(
            sim_same > 0.99,
            "identical text should have similarity ~1.0, got {}",
            sim_same
        );
        assert!(
            sim_same > sim_diff,
            "identical text should be more similar than different text"
        );
    }

    #[test]
    fn test_graph_default_embedder_is_hash() {
        let graph = atheneum::graph::AtheneumGraph::open_in_memory().expect("open");
        assert_eq!(graph.embedder_dimension(), 128);
    }

    #[test]
    fn test_graph_search_with_hash_embedder() {
        let graph = atheneum::graph::AtheneumGraph::open_in_memory().expect("open");
        graph
            .ingest_wiki_page(
                "wiki/router.md",
                "---\ntitle: Router\n---\nBuilds HTTP router with axum\n",
                Some("proj"),
            )
            .expect("ingest");

        let results = graph
            .lexical_search("router", 5, Some("proj"))
            .expect("search");
        assert!(!results.is_empty(), "should find router page");
    }

    fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
        let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
        let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm_a == 0.0 || norm_b == 0.0 {
            0.0
        } else {
            dot / (norm_a * norm_b)
        }
    }
}

mod consolidation_tests {
    use atheneum::graph::{AtheneumGraph, EdgeType, EntityType};
    use serde_json::json;

    #[test]
    fn test_consolidate_discoveries_creates_knowledge_entity() {
        let graph = AtheneumGraph::open_in_memory().expect("open");

        graph
            .store_discovery("claude1", "Symbol", "build_router", json!({"file": "a.rs"}))
            .expect("d1");
        graph
            .store_discovery("claude2", "Symbol", "build_router", json!({"file": "b.rs"}))
            .expect("d2");

        let knowledge_id = graph
            .consolidate_discoveries("build_router", None)
            .expect("consolidate")
            .expect("should return knowledge id");

        let entity = graph.get_entity(knowledge_id).expect("get");
        assert_eq!(entity.kind, EntityType::Knowledge.as_str());
        assert!(
            entity.name.contains("build_router"),
            "knowledge name should contain target"
        );
    }

    #[test]
    fn test_consolidate_creates_derived_from_edges() {
        let graph = AtheneumGraph::open_in_memory().expect("open");

        let d1 = graph
            .store_discovery("claude1", "Symbol", "build_router", json!({"file": "a.rs"}))
            .expect("d1");
        let d2 = graph
            .store_discovery("claude2", "Symbol", "build_router", json!({"file": "b.rs"}))
            .expect("d2");

        let knowledge_id = graph
            .consolidate_discoveries("build_router", None)
            .expect("consolidate")
            .expect("should return knowledge id");

        let (outgoing, _) = graph.get_neighbors(knowledge_id).expect("neighbors");
        let derived_edges: Vec<_> = outgoing
            .iter()
            .filter(|e| e.edge_type == EdgeType::DerivedFrom.as_str())
            .collect();

        assert_eq!(
            derived_edges.len(),
            2,
            "should have DerivedFrom edges to 2 source discoveries"
        );

        let target_ids: Vec<_> = derived_edges.iter().map(|e| e.to_id).collect();
        assert!(target_ids.contains(&d1), "should link to discovery 1");
        assert!(target_ids.contains(&d2), "should link to discovery 2");
    }

    #[test]
    fn test_consolidate_merges_metadata() {
        let graph = AtheneumGraph::open_in_memory().expect("open");

        graph
            .store_discovery(
                "claude1",
                "Symbol",
                "build_router",
                json!({"file": "a.rs", "lines": 100}),
            )
            .expect("d1");
        graph
            .store_discovery(
                "claude2",
                "Symbol",
                "build_router",
                json!({"file": "b.rs", "lines": 200}),
            )
            .expect("d2");

        let knowledge_id = graph
            .consolidate_discoveries("build_router", None)
            .expect("consolidate")
            .expect("should return knowledge id");

        let entity = graph.get_entity(knowledge_id).expect("get");
        let agents = entity
            .data
            .get("source_agents")
            .and_then(|v| v.as_array())
            .expect("source_agents");
        assert_eq!(agents.len(), 2, "should list both agents");

        let discovery_count = entity
            .data
            .get("discovery_count")
            .and_then(|v| v.as_i64())
            .expect("discovery_count");
        assert_eq!(discovery_count, 2);
    }

    #[test]
    fn test_consolidate_idempotent() {
        let graph = AtheneumGraph::open_in_memory().expect("open");

        graph
            .store_discovery("claude1", "Symbol", "build_router", json!({}))
            .expect("d1");
        graph
            .store_discovery("claude2", "Symbol", "build_router", json!({}))
            .expect("d2");

        let k1 = graph
            .consolidate_discoveries("build_router", None)
            .expect("c1")
            .expect("id1");
        let k2 = graph
            .consolidate_discoveries("build_router", None)
            .expect("c2")
            .expect("id2");

        assert_eq!(
            k1, k2,
            "re-consolidating should return same knowledge entity"
        );
    }

    #[test]
    fn test_consolidate_single_discovery_still_creates_knowledge() {
        let graph = AtheneumGraph::open_in_memory().expect("open");

        graph
            .store_discovery(
                "claude1",
                "Symbol",
                "unique_target",
                json!({"note": "only one"}),
            )
            .expect("d1");

        let knowledge_id = graph
            .consolidate_discoveries("unique_target", None)
            .expect("consolidate")
            .expect("should return knowledge id");

        let entity = graph.get_entity(knowledge_id).expect("get");
        assert_eq!(entity.kind, EntityType::Knowledge.as_str());
    }

    #[test]
    fn test_consolidate_no_discoveries_returns_none() {
        let graph = AtheneumGraph::open_in_memory().expect("open");

        let result = graph
            .consolidate_discoveries("nonexistent_target", None)
            .expect("ok");
        assert!(result.is_none(), "no discoveries should return None");
    }

    #[test]
    fn test_consolidation_pass() {
        let graph = AtheneumGraph::open_in_memory().expect("open");

        graph
            .store_discovery("claude1", "Symbol", "target_a", json!({}))
            .expect("d1");
        graph
            .store_discovery("claude2", "Symbol", "target_a", json!({}))
            .expect("d2");
        graph
            .store_discovery("claude1", "Symbol", "target_b", json!({}))
            .expect("d3");

        let report = graph.consolidation_pass(None).expect("pass");
        assert!(report.len() >= 2, "should consolidate at least 2 targets");

        for (target, knowledge_id) in &report {
            let entity = graph.get_entity(*knowledge_id).expect("get");
            assert_eq!(entity.kind, EntityType::Knowledge.as_str());
            assert!(entity.data.get("target").and_then(|v| v.as_str()) == Some(target.as_str()));
        }
    }
}
