use std::path::PathBuf;

use atheneum::AtheneumGraph;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 3 {
        eprintln!("Usage: batch_ingest_wiki <db-path> <wiki-dir> [project-id]");
        std::process::exit(1);
    }

    let db_path = PathBuf::from(&args[1]);
    let wiki_dir = PathBuf::from(&args[2]);
    let project_id = args.get(3).map(|s| s.as_str());

    if !wiki_dir.is_dir() {
        eprintln!("Not a directory: {}", wiki_dir.display());
        std::process::exit(1);
    }

    let graph = AtheneumGraph::open(&db_path)?;
    let ids = graph.sync_wiki_directory(&wiki_dir, project_id)?;

    println!(
        "Synced {} wiki pages from {}",
        ids.len(),
        wiki_dir.display()
    );
    for id in ids {
        println!("  -> graph entity id: {}", id);
    }

    Ok(())
}
