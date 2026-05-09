use atheneum::AtheneumGraph;
use std::fs;
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    // Persistent database - stores all articles to disk
    let db_path = PathBuf::from("/home/feanor/wiki/atheneum.db");
    let graph = AtheneumGraph::open(&db_path)?;

    println!("Opening Atheneum graph at: {}", db_path.display());

    let wiki_path = "/home/feanor/wiki";
    let mut ingested = 0;
    let mut errors = 0;

    // Walk the wiki directory
    let entries: Vec<_> = walkdir::WalkDir::new(wiki_path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|ext| ext == "md").unwrap_or(false))
        .collect();

    let total = entries.len();
    println!("Found {} markdown files in {}", total, wiki_path);

    for entry in entries {
        let path = entry.path();
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Failed to read {}: {}", path.display(), e);
                errors += 1;
                continue;
            }
        };

        // Get relative path from wiki root for storage
        let rel_path = path
            .strip_prefix(wiki_path)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        match graph.ingest_article(&rel_path, &content) {
            Ok(_id) => {
                ingested += 1;
                if ingested % 50 == 0 {
                    println!("Ingested {}/{} articles", ingested, total);
                }
            }
            Err(e) => {
                eprintln!("Failed to ingest {}: {}", rel_path, e);
                errors += 1;
            }
        }
    }

    println!("\n=== Summary ===");
    println!("Ingested: {}", ingested);
    println!("Errors: {}", errors);
    println!("Total: {}", total);
    println!("\nDatabase saved to: {}", db_path.display());
    println!("Run 'query_graph_persistent' to query the database.");

    Ok(())
}
