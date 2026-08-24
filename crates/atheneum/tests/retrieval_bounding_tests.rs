use atheneum::AtheneumGraph;

#[test]
fn test_list_distinct_projects_aggregates_across_domains() {
    let graph = AtheneumGraph::open_in_memory().unwrap();
    graph
        .store_memory("pref1", "val1", "agent", 1.0, Some("project-alpha"), None)
        .unwrap();
    graph
        .create_task("Task 1", Some("Desc"), Some("project-beta"))
        .unwrap();
    graph
        .store_discovery(
            "agent-1",
            "Decision",
            "arch",
            serde_json::json!({"project_id": "project-gamma"}),
        )
        .unwrap();

    let projects = graph.list_distinct_projects().unwrap();
    assert_eq!(
        projects,
        vec!["project-alpha", "project-beta", "project-gamma"]
    );
}

#[test]
fn test_session_digest_hints_when_project_mismatch() {
    let graph = AtheneumGraph::open_in_memory().unwrap();
    graph
        .store_memory("pref1", "val1", "agent", 1.0, Some("grounded-kernel"), None)
        .unwrap();
    graph
        .create_task("Task 1", Some("Desc"), Some("memoria"))
        .unwrap();

    // Query for non-existent project "Projects"
    let report_json = graph.compose_digest_json(Some("Projects"), 3).unwrap();
    assert_eq!(report_json["unknown_project"], true);
    let known = report_json["known_projects"].as_array().unwrap();
    let known_names: Vec<&str> = known.iter().filter_map(|v| v.as_str()).collect();
    assert!(known_names.contains(&"grounded-kernel"));
    assert!(known_names.contains(&"memoria"));

    let report_text = graph.compose_digest(Some("Projects"), 3, 500).unwrap();
    assert!(report_text.contains("project 'Projects' matches no recorded project"));
    assert!(report_text.contains("grounded-kernel"));
    assert!(report_text.contains("memoria"));
}

#[test]
fn test_query_wiki_paginates_large_body() {
    let graph = AtheneumGraph::open_in_memory().unwrap();
    let large_body = "A".repeat(20000);
    graph
        .ingest_wiki_page("large-page.md", &large_body, None)
        .unwrap();

    let page = graph.get_wiki_page("large-page.md").unwrap().unwrap();
    assert_eq!(page.body.len(), 20000);

    let (slice, truncated, total) = atheneum::graph::paginate_body(&page.body, 0, 8192);
    assert_eq!(slice.len(), 8192);
    assert!(truncated);
    assert_eq!(total, 20000);

    let (slice2, truncated2, total2) = atheneum::graph::paginate_body(&page.body, 8192, 8192);
    assert_eq!(slice2.len(), 8192);
    assert!(truncated2);
    assert_eq!(total2, 20000);

    let (slice3, truncated3, total3) = atheneum::graph::paginate_body(&page.body, 16384, 8192);
    assert_eq!(slice3.len(), 20000 - 16384);
    assert!(!truncated3);
    assert_eq!(total3, 20000);
}
