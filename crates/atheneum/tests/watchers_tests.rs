//! Stage 4: tests for the wiki + journal watchers port.
//!
//! Pure helpers (extract_wikilinks, parse_journal_sections,
//! extract_kanban_updates, content_hash) are tested first because
//! everything downstream depends on them. Then full ingestion paths
//! against a fresh in-memory graph. Finally a `sync_*_directory` smoke
//! test using `tempfile`.

use atheneum::graph::{
    content_hash, extract_kanban_updates, extract_wikilinks, parse_journal_sections, AtheneumGraph,
    KanbanStatus,
};
use serde_json::json;
use std::fs;

// ===========================================================================
// Pure helpers
// ===========================================================================

#[test]
fn test_extract_wikilinks_finds_all_brackets() {
    let body = "see [[grounded-coding]] and [[atheneum-py]] and [[grounded-coding]] again";
    let links = extract_wikilinks(body);
    // Duplicates preserved (caller can dedupe if needed) — useful evidence
    // for "how often is this page referenced"
    assert!(links.contains(&"grounded-coding".to_string()));
    assert!(links.contains(&"atheneum-py".to_string()));
    assert_eq!(links.len(), 3, "should preserve duplicates");
}

#[test]
fn test_extract_wikilinks_empty_when_none() {
    assert!(extract_wikilinks("no brackets at all").is_empty());
    assert!(extract_wikilinks("single [ bracket").is_empty());
    assert!(extract_wikilinks("[not a wikilink]").is_empty());
}

#[test]
fn test_content_hash_is_deterministic() {
    let a = content_hash("hello world");
    let b = content_hash("hello world");
    let c = content_hash("hello world!");
    assert_eq!(a, b, "same input must produce same hash");
    assert_ne!(a, c, "different input must produce different hash");
    assert_eq!(a.len(), 64, "sha256 hex digest is 64 chars");
}

// ===========================================================================
// Journal parsing
// ===========================================================================

#[test]
fn test_parse_journal_sections_segments_on_h2_with_time() {
    let journal = "## 09:15 | morning standup\n\
                   Discussed sprint goals\n\
                   ## 14:30 | refactor session\n\
                   Cleaned up the router\n";
    let sections = parse_journal_sections(journal);
    assert_eq!(sections.len(), 2);
    assert_eq!(sections[0].time.as_deref(), Some("09:15"));
    assert_eq!(sections[0].title, "morning standup");
    assert!(sections[0].body.contains("sprint goals"));
    assert_eq!(sections[1].time.as_deref(), Some("14:30"));
    assert_eq!(sections[1].title, "refactor session");
}

#[test]
fn test_parse_journal_sections_handles_h2_without_time() {
    let journal = "## general notes\n\
                   misc thoughts\n";
    let sections = parse_journal_sections(journal);
    assert_eq!(sections.len(), 1);
    assert!(
        sections[0].time.is_none(),
        "no time prefix → time should be None"
    );
    assert_eq!(sections[0].title, "general notes");
}

#[test]
fn test_extract_kanban_updates_recognizes_arrow_styles() {
    // Python regex accepts both ASCII -> and unicode → with optional emoji
    let body = r#"
"Refactor router" -> DONE ✅
'Wire HNSW search' → IN_PROGRESS
"Fix flaky test" -> BLOCKED 🛑
"#;
    let updates = extract_kanban_updates(body);
    assert_eq!(updates.len(), 3);
    assert_eq!(updates[0].task_title, "Refactor router");
    assert_eq!(updates[0].new_status, KanbanStatus::Done);
    assert_eq!(updates[1].task_title, "Wire HNSW search");
    assert_eq!(updates[1].new_status, KanbanStatus::InProgress);
    assert_eq!(updates[2].task_title, "Fix flaky test");
    assert_eq!(updates[2].new_status, KanbanStatus::Blocked);
}

#[test]
fn test_extract_kanban_updates_empty_when_none() {
    assert!(extract_kanban_updates("just prose, no transitions").is_empty());
}

// ===========================================================================
// ingest_wiki_page
// ===========================================================================

#[test]
fn test_ingest_wiki_page_creates_wikipage_entity() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let content = "---\n\
                   title: Grounded Coding\n\
                   tags: [discipline, agents]\n\
                   ---\n\
                   See also [[atheneum-py]].\n";

    let id = graph
        .ingest_wiki_page("wiki/grounded-coding.md", content, Some("envoy"))
        .expect("ingest");
    assert!(id > 0);

    let entity = graph.get_entity(id).expect("retrieve");
    assert_eq!(entity.kind, "WikiPage");
    assert_eq!(entity.data["title"], json!("Grounded Coding"));
    assert_eq!(entity.data["project_id"], json!("envoy"));
    assert_eq!(entity.data["wikilinks"], json!(["atheneum-py"]));
    // content_hash should be a 64-char hex string
    let h = entity.data["content_hash"]
        .as_str()
        .expect("content_hash str");
    assert_eq!(h.len(), 64);
}

#[test]
fn test_ingest_wiki_page_ignores_body_horizontal_rules_as_frontmatter() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let content =
        "# Article\n\nBody before rule.\n\n---\n\nnot: frontmatter\n\n---\n\nBody after rule.";

    let id = graph
        .ingest_wiki_page("wiki/no-frontmatter.md", content, Some("envoy"))
        .expect("ingest");
    let entity = graph.get_entity(id).expect("retrieve");
    let data = entity.data.as_object().expect("object data");

    assert!(!data.contains_key("not"));
    assert_eq!(data.get("body").and_then(|v| v.as_str()), Some(content));
}

#[test]
fn test_ingest_wiki_page_auto_indexed_for_navigation() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let content = "---\n\
                   title: Dynamic Workflows\n\
                   ---\n\
                   Claude Code dynamic workflows use subagents and graph navigation.\n";

    graph
        .ingest_wiki_page("wiki/dynamic-workflows.md", content, Some("forge"))
        .expect("ingest");

    let hits = graph
        .lexical_search("dynamic workflows subagents", 5, Some("forge"), None)
        .expect("search");

    assert!(
        hits.iter()
            .any(|hit| hit.kind == "WikiPage" && hit.name == "wiki/dynamic-workflows.md"),
        "ingested wiki page should be searchable immediately: {hits:?}"
    );
}

#[test]
fn test_ingest_wiki_page_idempotent_on_same_path() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let content_v1 = "---\ntitle: V1\n---\nbody v1\n";
    let content_v2 = "---\ntitle: V2\n---\nbody v2\n";

    let id1 = graph
        .ingest_wiki_page("notes/x.md", content_v1, None)
        .expect("first");
    let id2 = graph
        .ingest_wiki_page("notes/x.md", content_v2, None)
        .expect("second");

    assert_eq!(
        id1, id2,
        "re-ingesting the same path must update in place, not create a duplicate"
    );

    let entity = graph.get_entity(id1).expect("retrieve");
    assert_eq!(entity.data["title"], json!("V2"));
}

// ===========================================================================
// ingest_journal
// ===========================================================================

#[test]
fn test_ingest_journal_creates_one_entity_per_section() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let journal = "\
## 09:00 | planning
Decided on roadmap.
\"Stage 4\" -> IN_PROGRESS
## 11:30 | implementation
Wrote ingest_journal.
\"Stage 4\" -> DONE ✅
";

    let ids = graph
        .ingest_journal("journals/2026_05_18.md", journal, Some("forge"))
        .expect("ingest");
    assert_eq!(ids.len(), 2, "two sections → two JournalSection entities");

    let section1 = graph.get_entity(ids[0]).expect("retrieve");
    assert_eq!(section1.kind, "JournalSection");
    assert_eq!(section1.data["project_id"], json!("forge"));
    assert_eq!(section1.data["title"], json!("planning"));
    let kanban1 = section1.data["kanban_updates"]
        .as_array()
        .expect("kanban_updates array");
    assert_eq!(kanban1.len(), 1);
    assert_eq!(kanban1[0]["task_title"], json!("Stage 4"));
    assert_eq!(kanban1[0]["new_status"], json!("IN_PROGRESS"));
}

// ===========================================================================
// sync_wiki_directory
// ===========================================================================

#[test]
fn test_sync_wiki_directory_ingests_all_md_files() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let dir = tempfile::tempdir().expect("tempdir");

    fs::write(
        dir.path().join("alpha.md"),
        "---\ntitle: Alpha\n---\nlink to [[beta]]\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("beta.md"),
        "---\ntitle: Beta\n---\nno links here\n",
    )
    .unwrap();
    fs::write(dir.path().join("not_markdown.txt"), "skip me\n").unwrap();

    let ids = graph
        .sync_wiki_directory(dir.path(), Some("envoy"))
        .expect("sync");
    assert_eq!(
        ids.len(),
        2,
        "sync should ingest only .md files (got {} ids)",
        ids.len()
    );

    let pages = graph.entities_by_kind("WikiPage").expect("list");
    let real_pages: Vec<_> = pages
        .iter()
        .filter(|p| p.data.get("stub").and_then(|s| s.as_bool()) != Some(true))
        .collect();
    assert_eq!(
        real_pages.len(),
        2,
        "expected alpha and beta real WikiPages, got {} real pages",
        real_pages.len()
    );
    let titles: Vec<String> = pages
        .iter()
        .filter_map(|p| {
            p.data
                .get("title")
                .and_then(|t| t.as_str())
                .map(String::from)
        })
        .collect();
    assert!(titles.contains(&"Alpha".to_string()));
    assert!(titles.contains(&"Beta".to_string()));
}
