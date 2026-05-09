use std::path::PathBuf;
use atheneum::AtheneumGraph;

fn main() -> anyhow::Result<()> {
    // Open the persistent database
    let db_path = PathBuf::from("/home/feanor/wiki/atheneum.db");
    
    if !db_path.exists() {
        eprintln!("Database not found at: {}", db_path.display());
        eprintln!("Run 'batch_ingest_wiki' first to create the database.");
        std::process::exit(1);
    }
    
    let graph = AtheneumGraph::open(&db_path)?;
    println!("Opened Atheneum graph: {}", db_path.display());
    println!();
    
    // === Query 1: Get all knowledge entities ===
    println!("=== Query 1: Knowledge Entities (first 10) ===");
    let knowledge = graph.entities_by_kind("Knowledge")?;
    for entity in knowledge.iter().take(10) {
        println!("  [{}] {}", entity.id, entity.name);
        if let Some(title) = entity.data.get("title").and_then(|t| t.as_str()) {
            println!("      Title: {}", title);
        }
    }
    if knowledge.len() > 10 {
        println!("  ... and {} more", knowledge.len() - 10);
    }
    
    // === Query 2: Events performed by system agent ===
    println!("\n=== Query 2: Events by System Agent (first 5) ===");
    let events = graph.events_performed_by(1)?;
    for (i, event) in events.iter().take(5).enumerate() {
        println!("  {}. {} (ID: {})", i + 1, event.name, event.id);
    }
    if events.len() > 5 {
        println!("  ... and {} more", events.len() - 5);
    }
    
    // === Query 3: Graph traversal - outgoing edges ===
    println!("\n=== Query 3: Graph Traversal (Outgoing Edges) ===");
    if let Some(first_event) = events.first() {
        let outgoing = graph.outgoing_edges(first_event.id)?;
        println!(
            "Event '{}' (ID: {}) has {} outgoing edges:",
            first_event.name, first_event.id, outgoing.len()
        );
        for edge in &outgoing {
            let target = graph.get_entity(edge.to_id)?;
            println!(
                "  --[{}]--> [{}: {}] (ID: {})",
                edge.edge_type, target.kind, target.name, edge.to_id
            );
        }
    }
    
    // === Query 4: Find articles with specific tag ===
    println!("\n=== Query 4: Articles with 'sparse' tag ===");
    let mut found = 0;
    for entity in knowledge.iter().take(100) {
        if let Some(tags) = entity.data.get("tags").and_then(|t| t.as_array()) {
            let has_sparse = tags.iter().any(|t| t.as_str() == Some("sparse"));
            if has_sparse {
                println!("  [{}] {}", entity.id, entity.name);
                found += 1;
                if found >= 5 {
                    break;
                }
            }
        }
    }
    
    // === Query 5: Count entities by type ===
    println!("\n=== Query 5: Entity Counts by Type ===");
    let entity_counts = graph.count_entities_by_kind()?;
    for (kind, count) in entity_counts {
        println!("  {}: {}", kind, count);
    }
    
    // === Query 6: Count edges by type ===
    println!("\n=== Query 6: Edge Counts by Type ===");
    let edge_counts = graph.count_edges_by_type()?;
    for (edge_type, count) in edge_counts {
        println!("  {}: {}", edge_type, count);
    }
    
    println!("\nDatabase location: {}", db_path.display());
    
    Ok(())
}
