use atheneum::AtheneumGraph;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 3 {
        eprintln!("Usage: query_graph <db-path> <wiki-dir>");
        eprintln!("  db-path:   Path to atheneum database (use :memory: for in-memory)");
        eprintln!("  wiki-dir:  Path to wiki directory with .md files");
        std::process::exit(1);
    }

    let db_arg = &args[1];
    let wiki_path = &args[2];

    let graph = if db_arg == ":memory:" {
        AtheneumGraph::open_in_memory()?
    } else {
        AtheneumGraph::open(std::path::Path::new(db_arg))?
    };

    println!("=== Batch Ingesting Wiki ===\n");

    let entries: Vec<_> = walkdir::WalkDir::new(wiki_path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|ext| ext == "md").unwrap_or(false))
        .collect();

    let total = entries.len();
    let mut ingested = 0;

    for entry in entries {
        let path = entry.path();
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let rel_path = path
            .strip_prefix(wiki_path)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        if graph.ingest_wiki_page(&rel_path, &content, None).is_ok() {
            ingested += 1;
        }
    }

    println!("Ingested {}/{} articles\n", ingested, total);

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

    Ok(())
}
