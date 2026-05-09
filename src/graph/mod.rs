//! Graph operations for Atheneum
//!
//! Provides entity and edge storage with provenance tracking.

use anyhow::Result;
use chrono::Utc;
use rusqlite::params;
use serde_json::{json, Value};
use sqlitegraph::{GraphEdge, GraphEntity, SqliteGraph};
use thiserror::Error;

/// Entity types in the Atheneum graph
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityType {
    Agent,
    Task,
    Event,
    Decision,
    ToolCall,
    FileChange,
    Verification,
    Knowledge,
}

impl EntityType {
    pub fn as_str(&self) -> &'static str {
        match self {
            EntityType::Agent => "Agent",
            EntityType::Task => "Task",
            EntityType::Event => "Event",
            EntityType::Decision => "Decision",
            EntityType::ToolCall => "ToolCall",
            EntityType::FileChange => "FileChange",
            EntityType::Verification => "Verification",
            EntityType::Knowledge => "Knowledge",
        }
    }
}

/// Edge types (relations) in the Atheneum graph
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeType {
    PerformedBy,
    AssignedTo,
    Called,
    Modified,
    VerifiedBy,
    DependsOn,
    CausedBy,
    Supersedes,
    Created,
}

impl EdgeType {
    pub fn as_str(&self) -> &'static str {
        match self {
            EdgeType::PerformedBy => "performed_by",
            EdgeType::AssignedTo => "assigned_to",
            EdgeType::Called => "called",
            EdgeType::Modified => "modified",
            EdgeType::VerifiedBy => "verified_by",
            EdgeType::DependsOn => "depends_on",
            EdgeType::CausedBy => "caused_by",
            EdgeType::Supersedes => "supersedes",
            EdgeType::Created => "created",
        }
    }
}

/// Errors specific to Atheneum operations
#[derive(Error, Debug)]
pub enum AtheneumError {
    #[error("SQLite graph error: {0}")]
    GraphError(#[from] sqlitegraph::SqliteGraphError),

    #[error("Entity not found: {0}")]
    EntityNotFound(i64),

    #[error("Edge not found: {0}")]
    EdgeNotFound(i64),

    #[error("Invalid entity data: {0}")]
    InvalidData(String),
}

/// Main Atheneum graph interface
pub struct AtheneumGraph {
    inner: SqliteGraph,
}

impl AtheneumGraph {
    /// Create a new in-memory graph (for testing)
    pub fn open_in_memory() -> Result<Self> {
        let inner = SqliteGraph::open_in_memory()?;
        Ok(Self { inner })
    }

    /// Open or create a graph at the given path
    pub fn open(path: &std::path::Path) -> Result<Self> {
        let inner = SqliteGraph::open(path)?;
        Ok(Self { inner })
    }

    /// Check if the graph is healthy
    pub fn is_healthy(&self) -> bool {
        true // TODO: add actual health checks
    }

    /// Get an entity by ID
    pub fn get_entity(&self, id: i64) -> Result<GraphEntity> {
        self.inner
            .get_entity(id)
            .map_err(|_e| AtheneumError::EntityNotFound(id).into())
    }

    /// Get an edge by ID
    pub fn get_edge(&self, id: i64) -> Result<GraphEdge> {
        self.inner
            .get_edge(id)
            .map_err(|_e| AtheneumError::EdgeNotFound(id).into())
    }

    /// Query all edges originating from an entity
    pub fn outgoing_edges(&self, entity_id: i64) -> Result<Vec<GraphEdge>> {
        get_outgoing_edges(&self.inner, entity_id)
    }

    /// Query all edges pointing to an entity
    pub fn incoming_edges(&self, entity_id: i64) -> Result<Vec<GraphEdge>> {
        get_incoming_edges(&self.inner, entity_id)
    }

    /// Query entities by kind (e.g., all Knowledge, all Events)
    pub fn entities_by_kind(&self, kind: &str) -> Result<Vec<GraphEntity>> {
        let conn = if let Some(direct) = self.inner.pool.direct_connection() {
            direct
        } else {
            &self
                .inner
                .pool
                .get()
                .map_err(|e| anyhow::anyhow!("Failed to get connection: {}", e))?
        };

        let mut stmt = conn
            .prepare_cached(
                "SELECT id, kind, name, file_path, data FROM graph_entities WHERE kind=?1",
            )
            .map_err(|e| anyhow::anyhow!("Failed to prepare statement: {}", e))?;

        let rows = stmt
            .query_map(params![kind], |row| {
                Ok(GraphEntity {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    name: row.get(2)?,
                    file_path: row.get(3)?,
                    data: serde_json::from_str(row.get_ref(4)?.as_str()?)
                        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
                })
            })
            .map_err(|e| anyhow::anyhow!("Failed to query: {}", e))?;

        let mut entities = Vec::new();
        for row in rows {
            entities.push(row.map_err(|e| anyhow::anyhow!("Failed to read row: {}", e))?);
        }
        Ok(entities)
    }

    /// Count entities by kind
    pub fn count_entities_by_kind(&self) -> Result<Vec<(String, i64)>> {
        let conn = if let Some(direct) = self.inner.pool.direct_connection() {
            direct
        } else {
            &self
                .inner
                .pool
                .get()
                .map_err(|e| anyhow::anyhow!("Failed to get connection: {}", e))?
        };

        let mut stmt = conn.prepare_cached(
            "SELECT kind, COUNT(*) as count FROM graph_entities GROUP BY kind ORDER BY count DESC"
        ).map_err(|e| anyhow::anyhow!("Failed to prepare statement: {}", e))?;

        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|e| anyhow::anyhow!("Failed to query: {}", e))?;

        let mut counts = Vec::new();
        for row in rows {
            counts.push(row.map_err(|e| anyhow::anyhow!("Failed to read row: {}", e))?);
        }
        Ok(counts)
    }

    /// Count edges by type
    pub fn count_edges_by_type(&self) -> Result<Vec<(String, i64)>> {
        let conn = if let Some(direct) = self.inner.pool.direct_connection() {
            direct
        } else {
            &self
                .inner
                .pool
                .get()
                .map_err(|e| anyhow::anyhow!("Failed to get connection: {}", e))?
        };

        let mut stmt = conn.prepare_cached(
            "SELECT edge_type, COUNT(*) as count FROM graph_edges GROUP BY edge_type ORDER BY count DESC"
        ).map_err(|e| anyhow::anyhow!("Failed to prepare statement: {}", e))?;

        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|e| anyhow::anyhow!("Failed to query: {}", e))?;

        let mut counts = Vec::new();
        for row in rows {
            counts.push(row.map_err(|e| anyhow::anyhow!("Failed to read row: {}", e))?);
        }
        Ok(counts)
    }

    /// Insert an Agent entity
    pub fn insert_agent(&self, name: &str, data: Value) -> Result<i64> {
        let entity = GraphEntity {
            id: 0,
            kind: EntityType::Agent.as_str().to_string(),
            name: name.to_string(),
            file_path: None,
            data,
        };
        self.inner.insert_entity(&entity).map_err(Into::into)
    }

    /// Insert a Task entity
    pub fn insert_task(&self, name: &str, data: Value) -> Result<i64> {
        let entity = GraphEntity {
            id: 0,
            kind: EntityType::Task.as_str().to_string(),
            name: name.to_string(),
            file_path: None,
            data,
        };
        self.inner.insert_entity(&entity).map_err(Into::into)
    }

    /// Insert an Event entity (automatically adds timestamp)
    pub fn insert_event(&self, name: &str, mut data: Value) -> Result<i64> {
        // Auto-add timestamp if not present and data is an object
        if let Some(obj) = data.as_object_mut() {
            if !obj.contains_key("timestamp") {
                obj.insert(
                    "timestamp".to_string(),
                    Value::String(Utc::now().to_rfc3339()),
                );
            }
        }

        let entity = GraphEntity {
            id: 0,
            kind: EntityType::Event.as_str().to_string(),
            name: name.to_string(),
            file_path: None,
            data,
        };
        self.inner.insert_entity(&entity).map_err(Into::into)
    }

    /// Insert an edge between two entities
    pub fn insert_edge(
        &self,
        from_id: i64,
        to_id: i64,
        edge_type: EdgeType,
        data: Value,
    ) -> Result<i64> {
        let edge = GraphEdge {
            id: 0,
            from_id,
            to_id,
            edge_type: edge_type.as_str().to_string(),
            data,
        };
        self.inner.insert_edge(&edge).map_err(Into::into)
    }

    /// Query all events performed by an agent
    pub fn events_performed_by(&self, agent_id: i64) -> Result<Vec<GraphEntity>> {
        let mut events = Vec::new();

        // Get all edges pointing to this agent with "performed_by" type
        let edges = get_incoming_edges(&self.inner, agent_id)?
            .into_iter()
            .filter(|e| e.edge_type == EdgeType::PerformedBy.as_str());

        for edge in edges {
            if let Ok(entity) = self.get_entity(edge.from_id) {
                if entity.kind == EntityType::Event.as_str() {
                    events.push(entity);
                }
            }
        }

        Ok(events)
    }

    /// Query all tasks assigned to an agent
    pub fn tasks_assigned_to(&self, agent_id: i64) -> Result<Vec<GraphEntity>> {
        let mut tasks = Vec::new();

        // Get all incoming edges with "assigned_to" type
        let edges = get_incoming_edges(&self.inner, agent_id)?
            .into_iter()
            .filter(|e| e.edge_type == EdgeType::AssignedTo.as_str());

        for edge in edges {
            if let Ok(entity) = self.get_entity(edge.from_id) {
                if entity.kind == EntityType::Task.as_str() {
                    tasks.push(entity);
                }
            }
        }

        Ok(tasks)
    }

    /// Trace causal chain backwards from an event
    pub fn causal_chain(&self, event_id: i64) -> Result<Vec<GraphEntity>> {
        let mut chain = Vec::new();
        let mut current = Some(event_id);
        let mut visited = std::collections::HashSet::new();

        while let Some(id) = current {
            if !visited.insert(id) {
                break; // Cycle detected
            }

            if let Ok(entity) = self.get_entity(id) {
                chain.push(entity);

                // Find the "caused_by" edge
                if let Ok(edges) = get_outgoing_edges(&self.inner, id) {
                    current = edges
                        .into_iter()
                        .find(|e| e.edge_type == EdgeType::CausedBy.as_str())
                        .map(|e| e.to_id);
                } else {
                    current = None;
                }
            } else {
                current = None;
            }
        }

        Ok(chain)
    }

    /// Ingest a wiki article into the knowledge graph
    ///
    /// Parses YAML frontmatter and creates a Knowledge entity.
    /// Also creates an event recording the ingestion.
    pub fn ingest_article(&self, path: &str, content: &str) -> Result<i64> {
        // Parse YAML frontmatter (between --- markers)
        let (frontmatter, body) = parse_frontmatter(content)?;

        // Create the Knowledge entity
        let mut data = serde_json::json!({
            "path": path,
            "body": body,
        });

        // Merge frontmatter into data
        if let Some(obj) = data.as_object_mut() {
            if let Some(frontmatter_obj) = frontmatter.as_object() {
                for (key, value) in frontmatter_obj.iter() {
                    obj.insert(key.clone(), value.clone());
                }
            }
        }

        let entity = GraphEntity {
            id: 0,
            kind: EntityType::Knowledge.as_str().to_string(),
            name: path.to_string(),
            file_path: Some(path.to_string()),
            data,
        };

        let article_id = self
            .inner
            .insert_entity(&entity)
            .map_err(|e| anyhow::anyhow!("Failed to insert entity: {}", e))?;

        // Record the ingestion event
        let event_id = self.insert_event(
            "article-ingested",
            json!({
                "article_id": article_id,
                "path": path,
                "title": frontmatter.get("title").unwrap_or(&Value::String("Unknown".to_string())).clone()
            }),
        )?;

        // Link event to system agent (ID 0 represents "system")
        // First ensure system agent exists
        let _ = self.insert_agent("system", json!({"type": "system"}));

        self.insert_edge(
            event_id,
            1, // system agent (we'll use ID 1, created above)
            EdgeType::PerformedBy,
            json!({"provenance": {"actor": "atheneum", "method": "ingest"}}),
        )?;

        // Link event to the article (Event --Created--> Knowledge)
        self.insert_edge(
            event_id,
            article_id,
            EdgeType::Created,
            json!({"provenance": {"actor": "atheneum", "method": "ingest"}}),
        )?;

        Ok(article_id)
    }
}

/// Parse YAML frontmatter from markdown content
///
/// Returns (frontmatter as JSON, body content)
fn parse_frontmatter(content: &str) -> Result<(Value, &str)> {
    // Find the first ---
    let first_marker = content
        .find("---")
        .ok_or_else(|| anyhow::anyhow!("No frontmatter start marker"))?;

    // Find the second ---
    let second_marker = content[first_marker + 3..]
        .find("---")
        .ok_or_else(|| anyhow::anyhow!("No frontmatter end marker"))?
        + first_marker
        + 3;

    // Extract frontmatter YAML
    let frontmatter_yaml = &content[first_marker + 3..second_marker];

    // Parse YAML to JSON value
    let frontmatter: Value = serde_yaml::from_str(frontmatter_yaml)
        .map_err(|e| anyhow::anyhow!("Failed to parse YAML: {}", e))?;

    // Extract body (everything after the second ---)
    let body = &content[second_marker + 3..];

    Ok((frontmatter, body))
}

// Helper methods for querying edges by direction

/// Query all edges pointing to an entity
fn get_incoming_edges(graph: &SqliteGraph, entity_id: i64) -> Result<Vec<GraphEdge>> {
    // Try direct connection first (for in-memory databases)
    let conn = if let Some(direct) = graph.pool.direct_connection() {
        direct
    } else {
        // Fall back to pooled connection
        &graph
            .pool
            .get()
            .map_err(|e| anyhow::anyhow!("Failed to get connection: {}", e))?
    };

    let mut stmt = conn
        .prepare_cached(
            "SELECT id, from_id, to_id, edge_type, data FROM graph_edges WHERE to_id=?1 ORDER BY id",
        )
        .map_err(|e| anyhow::anyhow!("Failed to prepare statement: {}", e))?;

    let rows = stmt
        .query_map(params![entity_id], |row| {
            Ok(GraphEdge {
                id: row.get(0)?,
                from_id: row.get(1)?,
                to_id: row.get(2)?,
                edge_type: row.get(3)?,
                data: serde_json::from_str(row.get_ref(4)?.as_str()?)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
            })
        })
        .map_err(|e| anyhow::anyhow!("Failed to query edges: {}", e))?;

    let mut edges = Vec::new();
    for row in rows {
        edges.push(row.map_err(|e| anyhow::anyhow!("Failed to read row: {}", e))?);
    }
    Ok(edges)
}

/// Query all edges originating from an entity
fn get_outgoing_edges(graph: &SqliteGraph, entity_id: i64) -> Result<Vec<GraphEdge>> {
    // Try direct connection first (for in-memory databases)
    let conn = if let Some(direct) = graph.pool.direct_connection() {
        direct
    } else {
        // Fall back to pooled connection
        &graph
            .pool
            .get()
            .map_err(|e| anyhow::anyhow!("Failed to get connection: {}", e))?
    };

    let mut stmt = conn
        .prepare_cached(
            "SELECT id, from_id, to_id, edge_type, data FROM graph_edges WHERE from_id=?1 ORDER BY id",
        )
        .map_err(|e| anyhow::anyhow!("Failed to prepare statement: {}", e))?;

    let rows = stmt
        .query_map(params![entity_id], |row| {
            Ok(GraphEdge {
                id: row.get(0)?,
                from_id: row.get(1)?,
                to_id: row.get(2)?,
                edge_type: row.get(3)?,
                data: serde_json::from_str(row.get_ref(4)?.as_str()?)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
            })
        })
        .map_err(|e| anyhow::anyhow!("Failed to query edges: {}", e))?;

    let mut edges = Vec::new();
    for row in rows {
        edges.push(row.map_err(|e| anyhow::anyhow!("Failed to read row: {}", e))?);
    }
    Ok(edges)
}
