//! One real end-to-end pass through resolve -> dispatch -> envelope,
//! using one real magellan subprocess call and one real atheneum
//! in-process call. Not a substitute for Tasks 1-8's unit tests — proves
//! the wiring, not the logic.

use atheneum_mcp::backend::{Backend, SearchKind, SearchParams};

#[tokio::test(flavor = "multi_thread")]
async fn search_kind_all_returns_both_code_and_knowledge_hits_through_real_stack() {
    let tmp = tempfile::tempdir().unwrap();

    // Real magellan db: index a tiny fixture project.
    let fixture_project = tmp.path().join("fixture_project");
    std::fs::create_dir_all(&fixture_project).unwrap();
    std::fs::write(
        fixture_project.join("lib.rs"),
        "pub fn e2e_probe_symbol() -> u32 { 42 }",
    )
    .unwrap();
    let magellan_db = tmp.path().join("fixture.magellan.db");
    // `index` takes a single file (`--file`, root-relative) plus `--root` for
    // the project root — there is no whole-directory index subcommand in
    // this magellan version (that's `watch`, which runs continuously).
    let status = std::process::Command::new("magellan")
        .args([
            "index",
            "--db",
            magellan_db.to_str().unwrap(),
            "--root",
            fixture_project.to_str().unwrap(),
            "--file",
            "lib.rs",
        ])
        .status();
    if status.is_err() || !status.unwrap().success() {
        eprintln!("skipping e2e test: `magellan` binary not available on PATH");
        return;
    }

    // Real meta.db registering the fixture project.
    let meta_path = tmp.path().join("meta.db");
    let mut meta = atheneum::meta::MetaRouter::open_at(&meta_path).unwrap();
    meta.register_project(
        "e2e_fixture",
        fixture_project.to_str().unwrap(),
        magellan_db.to_str().unwrap(),
        None,
        Some("rust"),
    )
    .unwrap();
    let cross = atheneum::CrossRouter::from_meta(meta, 4);

    // Real atheneum graph, seeded with one memory whose content overlaps
    // the query term so both branches have something to find.
    let atheneum_db = tmp.path().join("fixture.atheneum.db");
    let graph = atheneum::AtheneumGraph::open(&atheneum_db).unwrap();
    // project must match the SearchParams.project below — lexical_search
    // filters by project_id when scoped, so an unscoped memory would be
    // invisible to a project-scoped search.
    graph
        .store_memory(
            "e2e-note",
            "note about e2e_probe_symbol behavior",
            "agent",
            0.8,
            Some("e2e_fixture"),
            None,
        )
        .unwrap();

    let backend = atheneum_mcp::backend::direct::DirectBackend::with_cross_router(
        std::sync::Arc::new(tokio::sync::Mutex::new(graph)),
        cross,
    );

    let result = backend
        .search(SearchParams {
            query: "e2e_probe_symbol".to_string(),
            k: 10,
            project: Some("e2e_fixture".to_string()),
            kind: SearchKind::All,
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();

    let items = result["items"].as_array().unwrap();
    assert!(
        items.iter().any(|i| i["provenance"] == "EXTRACTED"),
        "expected a code hit, got {items:?}"
    );
    assert!(
        items.iter().any(|i| i["provenance"] == "INFERRED"),
        "expected a knowledge hit, got {items:?}"
    );
    assert!(
        result["errors"].as_array().unwrap().is_empty(),
        "expected no errors on a clean real run, got {:?}",
        result["errors"]
    );
}
