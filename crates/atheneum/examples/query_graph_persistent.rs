use atheneum::AtheneumGraph;
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: query_graph_persistent <db-path>");
        std::process::exit(1);
    }

    let db_path = PathBuf::from(&args[1]);

    if !db_path.exists() {
        eprintln!("Database not found at: {}", db_path.display());
        eprintln!("Run 'batch_ingest_wiki' first to create the database.");
        std::process::exit(1);
    }

    let graph = AtheneumGraph::open(&db_path)?;
    println!("Opened Atheneum graph: {}", db_path.display());
    println!();

    println!("=== Knowledge Entities (first 10) ===");
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

    println!("\n=== Entity Counts by Type ===");
    let entity_counts = graph.count_entities_by_kind()?;
    for (kind, count) in entity_counts {
        println!("  {}: {}", kind, count);
    }

    println!("\n=== Edge Counts by Type ===");
    let edge_counts = graph.count_edges_by_type()?;
    for (edge_type, count) in edge_counts {
        println!("  {}: {}", edge_type, count);
    }

    println!("\nDatabase location: {}", db_path.display());

    Ok(())
}
