use atheneum::graph::AtheneumGraph;
use serde_json::json;

#[test]
fn test_query_trace_and_navigate_trace() {
    let graph = AtheneumGraph::open_in_memory().expect("open");

    // Seed a concept
    let concept_id = graph.upsert_concept("Grounded Theory", &json!({
        "summary": "Methodology for building theories from data",
        "scope": "global"
    })).unwrap();

    // Run navigate with trace enabled
    let (_views, trace_id) = graph.navigate_with_trace(
        "Grounded Theory",
        5,
        2,
        None,
        None,
        None,
        true,
    ).unwrap();

    // Verify trace_id is present
    assert!(trace_id.is_some());
    let tid = trace_id.unwrap();

    // Check that QueryTrace entity was created
    let trace_entity = graph.with_raw_connection(|conn| {
        let mut stmt = conn.prepare("SELECT kind, name, data FROM graph_entities WHERE id = ?1")?;
        let mut rows = stmt.query([tid])?;
        if let Some(row) = rows.next()? {
            let kind: String = row.get(0)?;
            let name: String = row.get(1)?;
            let data_str: String = row.get(2)?;
            let data: serde_json::Value = serde_json::from_str(&data_str).unwrap();
            Ok(Some((kind, name, data)))
        } else {
            Ok(None)
        }
    }).unwrap().unwrap();

    assert_eq!(trace_entity.0, "QueryTrace");
    assert!(trace_entity.1.to_lowercase().contains("trace: grounded theory"));
    let result_ids = trace_entity.2["result_ids"].as_array().unwrap();
    assert_eq!(result_ids.len(), 1);
    assert_eq!(result_ids[0].as_i64().unwrap(), concept_id);

    // Verify produced_by edges
    let edges = graph.with_raw_connection(|conn| {
        let mut stmt = conn.prepare("SELECT to_id, edge_type FROM graph_edges WHERE from_id = ?1")?;
        let rows = stmt.query_map([tid], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }).unwrap();

    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].0, concept_id);
    assert_eq!(edges[0].1, "produced_by");
}
