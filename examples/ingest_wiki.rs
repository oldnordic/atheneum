use std::fs;
use std::path::PathBuf;
use atheneum::AtheneumGraph;

fn main() -> anyhow::Result<()> {
    // Persistent storage - keeps articles between runs
    let db_path = PathBuf::from("/home/feanor/wiki/atheneum.db");
    let graph = AtheneumGraph::open(&db_path)?;

    let content =
        fs::read_to_string("/home/feanor/wiki/concepts/core-hypothesis-sparse-inference.md")?;

    let article_id =
        graph.ingest_article("core-hypothesis-sparse-inference.md", &content)?;

    println!("✅ Article ingested with ID: {}", article_id);

    let article = graph.get_entity(article_id)?;
    println!("\n📄 Article: {}", article.name);
    println!("   Type: {}", article.kind);

    let data = article.data.as_object().unwrap();
    println!("   Title: {}", data["title"]);
    println!("   Article Type: {}", data["type"]);
    println!("   Confidence: {}", data["confidence"]);
    println!("   Status: {}", data["status"]);

    if let Some(tags) = data.get("tags").and_then(|v| v.as_array()) {
        let tag_list: Vec<&str> = tags.iter().filter_map(|v| v.as_str()).collect();
        println!("   Tags: {}", tag_list.join(", "));
    }

    // Show the ingestion event
    let events = graph.events_performed_by(1)?;
    if let Some(event) = events.first() {
        println!("\n📝 Ingestion Event: {}", event.name);
        let event_data = event.data.as_object().unwrap();
        println!("   Article ID: {}", event_data["article_id"]);
        println!(
            "   Timestamp: {}",
            event_data["timestamp"].as_str().unwrap_or("N/A")
        );
    }

    // Show graph traversal
    println!("\n🔗 Graph traversal:");
    if let Some(event) = events.first() {
        let outgoing = graph.outgoing_edges(event.id)?;
        for edge in &outgoing {
            let target = graph.get_entity(edge.to_id)?;
            println!("   Event --[{}]--> {}", edge.edge_type, target.name);
        }
    }

    println!("\n💾 Database: {}", db_path.display());

    Ok(())
}
