//! Tests for dynamic ontology support in atheneum.
//!
//! Stage 2 of the atheneum-py port: replace the hardcoded EntityType /
//! EdgeType enums with a registry of classes and properties that can be
//! extended at runtime. The validation is permissive by default (KeplAI
//! "open mode"): an edge is allowed unless a property explicitly restricts
//! its domain/range to specific class names.

use atheneum::graph::AtheneumGraph;

#[test]
fn test_define_class_creates_ontology_entity() {
    let graph = AtheneumGraph::open_in_memory().expect("open");

    let id = graph
        .define_class("Hypothesis", Some("A proposed but unverified explanation"))
        .expect("define_class should succeed");
    assert!(id > 0, "class id should be positive");

    let classes = graph.list_classes().expect("list_classes");
    let hypothesis = classes
        .iter()
        .find(|c| c.name == "Hypothesis")
        .expect("Hypothesis should appear in list_classes");
    assert_eq!(
        hypothesis.description.as_deref(),
        Some("A proposed but unverified explanation"),
    );
}

#[test]
fn test_define_class_is_idempotent_by_name() {
    let graph = AtheneumGraph::open_in_memory().expect("open");

    let id1 = graph
        .define_class("Bug", Some("First description"))
        .expect("first define");
    let id2 = graph
        .define_class("Bug", Some("Updated description"))
        .expect("second define should update, not duplicate");

    assert_eq!(id1, id2, "re-defining same name should update in place");

    let classes = graph.list_classes().expect("list_classes");
    let count = classes.iter().filter(|c| c.name == "Bug").count();
    assert_eq!(count, 1, "must not create duplicate entries for same name");
    assert_eq!(
        classes
            .iter()
            .find(|c| c.name == "Bug")
            .unwrap()
            .description
            .as_deref(),
        Some("Updated description"),
    );
}

#[test]
fn test_define_property_with_domain_and_range() {
    let graph = AtheneumGraph::open_in_memory().expect("open");

    graph.define_class("Agent", None).expect("define Agent");
    graph.define_class("Task", None).expect("define Task");

    let id = graph
        .define_property(
            "assigned_to",
            "Agent",
            "Task",
            Some("An agent is assigned to a task"),
        )
        .expect("define_property should succeed");
    assert!(id > 0);

    let props = graph.list_properties().expect("list_properties");
    let prop = props
        .iter()
        .find(|p| p.name == "assigned_to")
        .expect("assigned_to should appear");
    assert_eq!(prop.domain_class, "Agent");
    assert_eq!(prop.range_class, "Task");
}

#[test]
fn test_validate_edge_open_mode_when_undefined() {
    let graph = AtheneumGraph::open_in_memory().expect("open");

    // No property registered → open mode → edge is permitted
    let allowed = graph
        .validate_edge("Agent", "Task", "spontaneously_invented_relation")
        .expect("validate_edge");
    assert!(
        allowed,
        "undefined edge types must be allowed (KeplAI open mode)"
    );
}

#[test]
fn test_validate_edge_enforces_domain_range_when_defined() {
    let graph = AtheneumGraph::open_in_memory().expect("open");

    graph
        .define_property("modifies", "Agent", "CodeSymbol", None)
        .expect("define modifies");

    // Correct domain + range → allowed
    let ok = graph
        .validate_edge("Agent", "CodeSymbol", "modifies")
        .expect("validate ok");
    assert!(ok);

    // Wrong domain → rejected
    let wrong_from = graph
        .validate_edge("Task", "CodeSymbol", "modifies")
        .expect("validate wrong domain");
    assert!(
        !wrong_from,
        "edge with wrong domain class should be rejected"
    );

    // Wrong range → rejected
    let wrong_to = graph
        .validate_edge("Agent", "Task", "modifies")
        .expect("validate wrong range");
    assert!(!wrong_to, "edge with wrong range class should be rejected");
}

#[test]
fn test_validate_edge_any_wildcard() {
    let graph = AtheneumGraph::open_in_memory().expect("open");

    // ANY in either slot means "no restriction on that side"
    graph
        .define_property("touches", "ANY", "CodeSymbol", None)
        .expect("define touches");

    let from_agent = graph
        .validate_edge("Agent", "CodeSymbol", "touches")
        .expect("validate");
    assert!(from_agent, "ANY domain should accept any from-kind");

    let from_task = graph
        .validate_edge("Task", "CodeSymbol", "touches")
        .expect("validate");
    assert!(from_task);

    let wrong_range = graph
        .validate_edge("Agent", "Agent", "touches")
        .expect("validate");
    assert!(!wrong_range, "range still constrained when set");
}

#[test]
fn test_seed_standard_ontology_populates_core_kinds() {
    let graph = AtheneumGraph::open_in_memory().expect("open");

    graph.seed_standard_ontology().expect("seed");

    let classes = graph.list_classes().expect("list");
    let names: Vec<&str> = classes.iter().map(|c| c.name.as_str()).collect();

    // The 10 existing EntityType variants should be seeded so existing data
    // doesn't suddenly look "undefined". Plus a few new ones ported from
    // atheneum-py's seed: Project (for workspace scoping), CodeSymbol,
    // WikiPage, JournalSection, ReasoningLog.
    for required in [
        "Agent",
        "Task",
        "Event",
        "Decision",
        "ToolCall",
        "FileChange",
        "Verification",
        "Knowledge",
        "Discovery",
        "Handoff",
        "Project",
        "CodeSymbol",
        "WikiPage",
        "JournalSection",
        "ReasoningLog",
    ] {
        assert!(
            names.contains(&required),
            "seed must include {} (got: {:?})",
            required,
            names
        );
    }
}
