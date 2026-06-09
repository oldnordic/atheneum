use atheneum::AtheneumGraph;
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 3 {
        eprintln!("Usage: ingest_wiki <db-path> <markdown-file>");
        std::process::exit(1);
    }

    let db_path = PathBuf::from(&args[1]);
    let md_file = &args[2];

    let graph = AtheneumGraph::open(&db_path)?;
    let content = std::fs::read_to_string(md_file)?;

    let article_name = std::path::Path::new(md_file)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| md_file.clone());
    let article_id = graph.ingest_wiki_page(&article_name, &content, None)?;

    println!("Article ingested with ID: {}", article_id);

    let article = graph.get_entity(article_id)?;
    println!("\nArticle: {}", article.name);
    println!("   Type: {}", article.kind);

    let data = article
        .data
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("article data is not an object"))?;
    println!("   Title: {}", data["title"]);
    println!("   Article Type: {}", data["type"]);
    println!("   Confidence: {}", data["confidence"]);
    println!("   Status: {}", data["status"]);

    if let Some(tags) = data.get("tags").and_then(|v| v.as_array()) {
        let tag_list: Vec<&str> = tags.iter().filter_map(|v| v.as_str()).collect();
        println!("   Tags: {}", tag_list.join(", "));
    }

    println!("\nDatabase: {}", db_path.display());

    Ok(())
}
