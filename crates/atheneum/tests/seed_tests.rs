use atheneum::graph::AtheneumGraph;
use serde_json::json;

#[test]
fn test_seed_memory_primitive() {
    let graph = AtheneumGraph::open_in_memory().expect("open");

    // 1. Seed some concept entities
    graph.upsert_concept("Compiler Optimization", &json!({
        "summary": "Transforms source code to execute more efficiently",
        "scope": "project",
        "project_id": "envoy"
    })).unwrap();

    graph.upsert_concept("Memory Allocator", &json!({
        "summary": "Manages physical heap memory segments",
        "scope": "global"
    })).unwrap();

    // Seed some memories
    graph.store_memory("key1", "Vim preference details", "user", 1.0, None, None).unwrap();
    // sleep to guarantee different timestamps for sorting
    std::thread::sleep(std::time::Duration::from_millis(50));
    graph.store_memory("key2", "Emacs preference details", "project", 0.9, Some("envoy"), None).unwrap();

    // Seed a noise entity (ToolCall)
    graph.with_raw_connection(|conn| {
        conn.execute(
            "INSERT INTO graph_entities (kind, name, file_path, data) VALUES ('ToolCall', 'grep_search', NULL, '{}')",
            [],
        )?;
        Ok(())
    }).unwrap();

    // 2. Fetch seed with high token budget and no project filter
    let seed = graph.seed_memory(None, 1000).unwrap();
    println!("High budget instructions:\n{}", seed.instructions);

    // Concept names must be present
    assert!(seed.instructions.contains("Compiler Optimization"));
    assert!(seed.instructions.contains("Memory Allocator"));
    // Memories must be present
    assert!(seed.instructions.contains("key1"));
    assert!(seed.instructions.contains("key2"));
    // Noise entities must be excluded
    assert!(!seed.instructions.contains("grep_search"));

    // 3. Fetch seed with project="envoy"
    let seed_proj = graph.seed_memory(Some("envoy"), 1000).unwrap();
    println!("Envoy project instructions:\n{}", seed_proj.instructions);
    assert!(seed_proj.instructions.contains("Compiler Optimization"));
    assert!(seed_proj.instructions.contains("key2"));
    // Global and other project entities should not be matched if strict project filter is on
    // Wait, "Memory Allocator" has no project_id (global). If strict project filter excludes global:
    assert!(!seed_proj.instructions.contains("Memory Allocator"));
    assert!(!seed_proj.instructions.contains("key1"));

    // 4. Fetch seed with very low token budget (e.g. 20 tokens)
    let seed_low = graph.seed_memory(None, 20).unwrap();
    println!("Low budget instructions:\n{}", seed_low.instructions);
    // Should be truncated
    assert!(seed_low.instructions.contains("truncated") || seed_low.token_estimate <= 20);
}
