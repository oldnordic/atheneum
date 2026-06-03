//! Stage 11d: SQL+graph separation tests for the Knowledge domain.

use atheneum::graph::AtheneumGraph;
use rusqlite::params;
use serde_json::json;

#[test]
fn test_discovery_row_with_proper_columns() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let entity_id = graph
        .store_discovery_in_project(
            "agent_a",
            "Symbol",
            "build_router",
            Some("envoy"),
            json!({"file": "src/http.rs", "line": 555}),
        )
        .expect("store");

    let (agent, dtype, target, project): (String, String, String, Option<String>) = graph
        .with_raw_connection(|conn| {
            conn.query_row(
                "SELECT agent_name, discovery_type, target, project_id
                 FROM discoveries WHERE target = ?1",
                params!["build_router"],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .map_err(Into::into)
        })
        .expect("discoveries row");
    assert_eq!(agent, "agent_a");
    assert_eq!(dtype, "Symbol");
    assert_eq!(target, "build_router");
    assert_eq!(project.as_deref(), Some("envoy"));

    let entity = graph.get_entity(entity_id).expect("entity");
    assert!(entity.data["sql_id"].as_i64().is_some());
}

#[test]
fn test_wiki_page_row_idempotent_on_path() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let id1 = graph
        .ingest_wiki_page(
            "wiki/x.md",
            "---\ntitle: V1\n---\nbody v1 with [[foo]]\n",
            Some("envoy"),
        )
        .expect("v1");
    let id2 = graph
        .ingest_wiki_page("wiki/x.md", "---\ntitle: V2\n---\nbody v2\n", Some("envoy"))
        .expect("v2");
    assert_eq!(
        id1, id2,
        "re-ingesting the same path must update in place, not create a duplicate"
    );

    // Single SQL row, with V2's content
    let (title, count): (Option<String>, i64) = graph
        .with_raw_connection(|conn| {
            let c: i64 = conn.query_row(
                "SELECT COUNT(*) FROM wiki_pages WHERE path = ?1",
                params!["wiki/x.md"],
                |r| r.get(0),
            )?;
            let t: Option<String> = conn.query_row(
                "SELECT title FROM wiki_pages WHERE path = ?1",
                params!["wiki/x.md"],
                |r| r.get(0),
            )?;
            Ok((t, c))
        })
        .expect("row");
    assert_eq!(count, 1, "wiki_pages must enforce one row per path");
    assert_eq!(title.as_deref(), Some("V2"));
}

#[test]
fn test_journal_sections_rows_per_section() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let ids = graph
        .ingest_journal(
            "journals/2026_05_19.md",
            "## 09:00 | planning\nDecided roadmap.\n## 11:30 | impl\nWrote code.\n",
            Some("envoy"),
        )
        .expect("ingest");
    assert_eq!(ids.len(), 2);

    let titles: Vec<String> = graph
        .with_raw_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT title FROM journal_sections WHERE path = ?1 ORDER BY section_index",
            )?;
            let rows = stmt
                .query_map(params!["journals/2026_05_19.md"], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .expect("rows");
    assert_eq!(titles, vec!["planning".to_string(), "impl".to_string()]);
}

#[test]
fn test_aggregate_discoveries_count_by_target_per_project() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    for project in ["envoy", "envoy", "magellan"] {
        graph
            .store_discovery_in_project("a", "Symbol", "Message", Some(project), json!({}))
            .expect("store");
    }

    let envoy_count: i64 = graph
        .with_raw_connection(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM discoveries
                 WHERE target = 'Message' AND project_id = 'envoy'",
                [],
                |r| r.get(0),
            )
            .map_err(Into::into)
        })
        .expect("count");
    assert_eq!(envoy_count, 2);
}

#[test]
fn test_semantic_search_still_works_with_sql_backed_discoveries() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .store_discovery(
            "agent",
            "Symbol",
            "build_router",
            json!({"summary": "constructs axum Router with routes"}),
        )
        .expect("store");
    graph
        .store_discovery(
            "agent",
            "Symbol",
            "parse_yaml",
            json!({"summary": "parses YAML frontmatter"}),
        )
        .expect("store");

    graph.build_search_index().expect("build");
    let results = graph
        .lexical_search("router axum routes", 5, None, None)
        .expect("search");

    assert!(!results.is_empty());
    assert_eq!(results[0].name, "agent: build_router");
}
