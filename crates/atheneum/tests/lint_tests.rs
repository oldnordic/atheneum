use atheneum::graph::{AtheneumGraph, LintConfig, MaintainConfig, BrokenLinkMode};
use serde_json::json;

#[test]
fn test_lint_and_maintain_orphans() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph.seed_standard_ontology().expect("seed");

    // 1. Seed two similar concept entities: "Rust Compiler" and "Rust Compilation"
    graph.upsert_concept("Rust Compiler", &json!({})).unwrap();
    graph.upsert_concept("Rust Compilation", &json!({})).unwrap();

    // 2. Both are orphans initially
    let lint = graph.lint_graph(&LintConfig::default()).unwrap();
    assert_eq!(lint.orphans.len(), 2);

    // 3. Maintain (dry-run): should find they are similar and suggest rewiring, but not change anything
    let rep_dry = graph.maintain(&MaintainConfig::default(), false).unwrap();
    assert_eq!(rep_dry.orphans_rewired, 2);

    let lint_after_dry = graph.lint_graph(&LintConfig::default()).unwrap();
    assert_eq!(lint_after_dry.orphans.len(), 2);

    // 4. Maintain (apply=true): should rewire them
    let rep_apply = graph.maintain(&MaintainConfig::default(), true).unwrap();
    assert_eq!(rep_apply.orphans_rewired, 2);

    // 5. Check lint_graph again: they are no longer orphans because they are rewired bidirectionally
    let lint_after = graph.lint_graph(&LintConfig::default()).unwrap();
    assert_eq!(lint_after.orphans.len(), 0);
}

#[test]
fn test_lint_and_maintain_broken_links() {
    let temp_dir = tempfile::tempdir().unwrap();
    let wiki_path = temp_dir.path().join("test_wiki.md");
    let wiki_path_str = wiki_path.to_str().unwrap();

    let graph = AtheneumGraph::open_in_memory().expect("open");

    // Seed a wiki page with a broken wikilink
    let body = "Look at [[Rust Style]]";
    std::fs::write(&wiki_path, body).unwrap();
    let _page_id = graph.ingest_wiki_page(wiki_path_str, body, None).unwrap();

    // The target "Rust Style" does not exist (it is a stub in database)
    let lint = graph.lint_graph(&LintConfig::default()).unwrap();
    assert_eq!(lint.broken_links.len(), 1);
    assert_eq!(lint.broken_links[0].target, "Rust Style");

    // Maintain using Stub mode
    let maintain_cfg = MaintainConfig {
        broken_link_mode: BrokenLinkMode::Stub,
        ..Default::default()
    };
    let rep = graph.maintain(&maintain_cfg, true).unwrap();
    assert_eq!(rep.broken_links_resolved, 1);

    // The stub file has been written on disk and ingested. It is no longer a stub.
    let lint_after = graph.lint_graph(&LintConfig::default()).unwrap();
    assert_eq!(lint_after.broken_links.len(), 0);
}

#[test]
fn test_lint_and_maintain_contradiction() {
    let graph = AtheneumGraph::open_in_memory().expect("open");

    // Store two contradicting memories under same key, different scope
    let _id1 = graph.store_memory("key1", "User likes Vim", "user", 1.0, None, None).unwrap();
    // sleep for a brief moment to make sure timestamps differ slightly
    std::thread::sleep(std::time::Duration::from_millis(50));
    let id2 = graph.store_memory("key1", "User prefers Emacs", "project", 0.9, Some("projA"), None).unwrap();

    // Verify both are retrieved by default query before maintain
    let active = graph.query_memory("key1", None, None, false).unwrap();
    assert_eq!(active.len(), 2);

    // Maintain: contradiction should be detected and older one (id1) superseded
    let rep = graph.maintain(&MaintainConfig::default(), true).unwrap();
    assert_eq!(rep.contradictions_superseded, 1);

    // query_memory with include_superseded=false should only return id2
    let active_after = graph.query_memory("key1", None, None, false).unwrap();
    assert_eq!(active_after.len(), 1);
    assert_eq!(active_after[0].id, id2);

    // query_memory with include_superseded=true should return both
    let all = graph.query_memory("key1", None, None, true).unwrap();
    assert_eq!(all.len(), 2);
}
