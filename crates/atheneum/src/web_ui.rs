#![cfg(feature = "web-ui")]

use crate::{AtheneumGraph, LintConfig};
use askama::Template;
use axum::{
    extract::State,
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use serde_json::json;
use std::sync::Arc;

#[derive(Template)]
#[template(
    source = r##"<!DOCTYPE html>
<html>
<head>
    <title>Atheneum Force-Directed Graph</title>
    <script src="https://d3js.org/d3.v7.min.js"></script>
    <style>
        body { margin: 0; font-family: 'Segoe UI', Tahoma, Geneva, Verdana, sans-serif; background-color: #121212; color: #e0e0e0; overflow: hidden; }
        #canvas-container { width: 100vw; height: 100vh; position: absolute; top: 0; left: 0; }
        .node { stroke: #121212; stroke-width: 2px; cursor: pointer; }
        .link { stroke: #555; stroke-opacity: 0.4; stroke-width: 1.5px; }
        #control-panel { position: absolute; top: 15px; left: 15px; z-index: 100; background: rgba(30, 30, 30, 0.9); padding: 15px; border-radius: 8px; box-shadow: 0 4px 12px rgba(0,0,0,0.5); width: 280px; }
        button { background: #3f51b5; border: none; color: white; padding: 8px 12px; border-radius: 4px; cursor: pointer; margin-top: 5px; width: 100%; font-weight: bold; }
        button:hover { background: #5c6bc0; }
        #tooltip { position: absolute; display: none; background: rgba(0,0,0,0.9); color: #fff; padding: 10px; border-radius: 4px; font-size: 12px; max-width: 350px; z-index: 200; pointer-events: none; border: 1px solid #333; }
        h3 { margin-top: 0; color: #3f51b5; }
    </style>
</head>
<body>
    <div id="control-panel">
        <h3>Atheneum Graph Panel</h3>
        <p>Hover over nodes to inspect properties.</p>
        <button onclick="triggerDream()">Reflective Dream</button>
        <button onclick="triggerLint()">Run Lint Graph</button>
        <div id="status" style="margin-top: 10px; font-size: 12px; color: #aaa;"></div>
    </div>
    <div id="tooltip"></div>
    <div id="canvas-container"><svg id="graph-svg" style="width:100%; height:100%;"></svg></div>
    
    <script>
        const svg = d3.select("#graph-svg"),
              width = window.innerWidth,
              height = window.innerHeight;
              
        const simulation = d3.forceSimulation()
            .force("link", d3.forceLink().id(d => d.id).distance(120))
            .force("charge", d3.forceManyBody().strength(-300))
            .force("center", d3.forceCenter(width / 2, height / 2));
            
        d3.json("/api/graph").then(data => {
            const link = svg.append("g").selectAll(".link")
                .data(data.links)
                .enter().append("line")
                .attr("class", "link");
                
            const node = svg.append("g").selectAll(".node")
                .data(data.nodes)
                .enter().append("circle")
                .attr("class", "node")
                .attr("r", d => d.kind === "Concept" ? 10 : 7)
                .attr("fill", d => {
                    if (d.kind === "Concept") return "#2196f3";
                    if (d.kind === "Memory") return "#ffeb3b";
                    if (d.kind === "WikiPage") return "#4caf50";
                    return "#9c27b0";
                })
                .call(d3.drag()
                    .on("start", dragstarted)
                    .on("drag", dragged)
                    .on("end", dragended))
                .on("mouseover", showTooltip)
                .on("mouseout", hideTooltip);
                
            simulation.nodes(data.nodes).on("tick", ticked);
            simulation.force("link").links(data.links);
            
            function ticked() {
                link.attr("x1", d => d.source.x)
                    .attr("y1", d => d.source.y)
                    .attr("x2", d => d.target.x)
                    .attr("y2", d => d.target.y);
                node.attr("cx", d => d.x).attr("cy", d => d.y);
            }
        });

        function dragstarted(event, d) {
            if (!event.active) simulation.alphaTarget(0.3).restart();
            d.fx = d.x; d.fy = d.y;
        }
        function dragged(event, d) { d.fx = event.x; d.fy = event.y; }
        function dragended(event, d) {
            if (!event.active) simulation.alphaTarget(0);
            d.fx = null; d.fy = null;
        }

        const tooltip = d3.select("#tooltip");
        function showTooltip(event, d) {
            tooltip.style("display", "block")
                   .html(`<strong>${d.name}</strong><br/>Kind: ${d.kind}<br/><pre>${JSON.stringify(d.data, null, 2)}</pre>`)
                   .style("left", (event.pageX + 10) + "px")
                   .style("top", (event.pageY + 10) + "px");
        }
        function hideTooltip() { tooltip.style("display", "none"); }

        function triggerDream() {
            document.getElementById("status").innerText = "Dreaming...";
            fetch("/api/dream", { method: "POST" })
                .then(r => r.json())
                .then(data => {
                    document.getElementById("status").innerText = `Dream run complete! Scanned ${data.pages_scanned} pages.`;
                });
        }
        function triggerLint() {
            document.getElementById("status").innerText = "Linting...";
            fetch("/api/lint")
                .then(r => r.json())
                .then(data => {
                    document.getElementById("status").innerText = `Lint done! Found ${data.orphans.length} orphans, ${data.expired.length} expired.`;
                });
        }
    </script>
</body>
</html>"##,
    ext = "html"
)]
struct DashboardTemplate;

async fn handle_dashboard() -> impl IntoResponse {
    Html(DashboardTemplate.render().unwrap())
}

async fn handle_get_graph(
    State(graph): State<Arc<tokio::sync::Mutex<AtheneumGraph>>>,
) -> impl IntoResponse {
    let graph = graph.lock().await;
    let entities = graph
        .with_raw_connection(|conn| {
            let mut stmt = conn.prepare("SELECT id, kind, name, data FROM graph_entities")?;
            let rows = stmt.query_map([], |row| {
                let data_str: String = row.get(3)?;
                let data: serde_json::Value = serde_json::from_str(&data_str).unwrap_or(json!({}));
                Ok(json!({
                    "id": row.get::<_, i64>(0)?,
                    "kind": row.get::<_, String>(1)?,
                    "name": row.get::<_, String>(2)?,
                    "data": data,
                }))
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok::<_, anyhow::Error>(out)
        })
        .unwrap_or_default();

    let edges = graph
        .with_raw_connection(|conn| {
            let mut stmt = conn.prepare("SELECT from_id, to_id, edge_type FROM graph_edges")?;
            let rows = stmt.query_map([], |row| {
                Ok(json!({
                    "source": row.get::<_, i64>(0)?,
                    "target": row.get::<_, i64>(1)?,
                    "type": row.get::<_, String>(2)?,
                }))
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok::<_, anyhow::Error>(out)
        })
        .unwrap_or_default();

    Json(json!({ "nodes": entities, "links": edges }))
}

async fn handle_get_lint(
    State(graph): State<Arc<tokio::sync::Mutex<AtheneumGraph>>>,
) -> impl IntoResponse {
    let graph = graph.lock().await;
    let config = LintConfig::default();
    let report = graph.lint_graph(&config).unwrap();
    Json(report)
}

async fn handle_post_dream(
    State(graph): State<Arc<tokio::sync::Mutex<AtheneumGraph>>>,
) -> impl IntoResponse {
    let graph = graph.lock().await;
    let report = graph
        .dream_pass(
            crate::DreamMode::AutoMerge,
            None,
            None,
            &crate::DreamConfig::default(),
        )
        .unwrap();
    Json(report)
}

pub async fn start_web_server(graph: AtheneumGraph, port: u16) -> anyhow::Result<()> {
    let shared_graph = Arc::new(tokio::sync::Mutex::new(graph));
    let app = Router::new()
        .route("/", get(handle_dashboard))
        .route("/api/graph", get(handle_get_graph))
        .route("/api/lint", get(handle_get_lint))
        .route("/api/dream", post(handle_post_dream))
        .with_state(shared_graph);

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", port)).await?;
    println!("Atheneum Dashboard running at http://127.0.0.1:{}", port);
    axum::serve(listener, app).await?;
    Ok(())
}
