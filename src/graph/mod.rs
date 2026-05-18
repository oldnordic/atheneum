//! Graph operations for Atheneum
//!
//! Provides entity and edge storage with provenance tracking.

use anyhow::Result;
use chrono::Utc;
use regex::Regex;
use rusqlite::params;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlitegraph::hnsw::{DistanceMetric, HnswConfigBuilder};
use sqlitegraph::{GraphEdge, GraphEntity, SqliteGraph};
use std::hash::{Hash, Hasher};
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
    Discovery,
    Handoff,
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
            EntityType::Discovery => "Discovery",
            EntityType::Handoff => "Handoff",
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
    RelatedTo,
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
            EdgeType::RelatedTo => "related_to",
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

/// A class registered in the dynamic ontology.
///
/// Stored as a graph entity with `kind = "OntologyClass"`. Returned by
/// [`AtheneumGraph::list_classes`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OntologyClassInfo {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
}

/// A property (edge restriction) registered in the dynamic ontology.
///
/// `domain_class` and `range_class` may be specific class names or the
/// wildcard `"ANY"` to leave that side unconstrained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OntologyPropertyInfo {
    pub id: i64,
    pub name: String,
    pub domain_class: String,
    pub range_class: String,
    pub description: Option<String>,
}

/// Reserved graph entity kind for ontology class definitions.
pub const ONTOLOGY_CLASS_KIND: &str = "OntologyClass";

/// Reserved graph entity kind for ontology property definitions.
pub const ONTOLOGY_PROPERTY_KIND: &str = "OntologyProperty";

/// A single hit from [`AtheneumGraph::semantic_search`].
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// Graph entity id of the matching node.
    pub id: i64,
    /// `kind: name` style display label, copied from the entity.
    pub name: String,
    /// Entity kind (e.g. "Discovery"); useful for downstream filtering.
    pub kind: String,
    /// Distance score from HNSW (smaller = closer; metric is cosine).
    pub score: f32,
    /// Full entity `data` blob, in case the caller wants to read project_id,
    /// summary, etc. without a second round-trip.
    pub data: Value,
}

/// Reserved HNSW index name used by [`AtheneumGraph::build_search_index`].
const SEARCH_INDEX_NAME: &str = "discoveries";

/// Vector dimension for the built-in hash embedder. 128 keeps memory low
/// while still differentiating short snippets reasonably well.
const SEARCH_EMBED_DIM: usize = 128;

/// A single tool invocation to record on an agent action.
///
/// Used by [`AtheneumGraph::record_agent_action`]. `modified_targets` is
/// a list of existing graph-entity IDs (FileChange, CodeSymbol, Discovery,
/// etc.) that this tool call should be marked as having modified.
#[derive(Debug, Clone)]
pub struct ToolCallRecord {
    pub tool_name: String,
    pub args: Value,
    pub modified_targets: Vec<i64>,
}

/// IDs created by [`AtheneumGraph::record_agent_action`], returned so
/// callers can attach further edges or queries without re-querying.
#[derive(Debug, Clone)]
pub struct ActionTrace {
    pub agent_id: i64,
    pub reasoning_log_id: i64,
    pub tool_call_ids: Vec<i64>,
    pub modified_edge_ids: Vec<i64>,
}

/// One row of provenance returned by [`AtheneumGraph::get_action_trace`]:
/// a reasoning log plus the tool calls it spawned and what each modified.
#[derive(Debug, Clone)]
pub struct ActionRecord {
    pub reasoning_log: GraphEntity,
    pub tool_calls: Vec<ToolCallTrace>,
}

/// A single tool call and the entities it modified.
#[derive(Debug, Clone)]
pub struct ToolCallTrace {
    pub tool_call: GraphEntity,
    pub modified: Vec<GraphEntity>,
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

    // ========================================================================
    // Bridge API: Discovery
    // ========================================================================

    /// Store a discovery made by an agent
    ///
    /// Discoveries are findings about symbols, CFG, issues, or patterns.
    /// They create a Discovery entity and link it to the target via RelatedTo edge.
    pub fn store_discovery(
        &self,
        agent: &str,
        discovery_type: &str,
        target: &str,
        mut metadata: Value,
    ) -> Result<i64> {
        // Build discovery name: "agent: target"
        let name = format!("{}: {}", agent, target);

        // Merge provenance into metadata
        if let Some(obj) = metadata.as_object_mut() {
            obj.insert("agent".to_string(), Value::String(agent.to_string()));
            obj.insert(
                "discovery_type".to_string(),
                Value::String(discovery_type.to_string()),
            );
            obj.insert("target".to_string(), Value::String(target.to_string()));
            obj.insert(
                "timestamp".to_string(),
                Value::String(Utc::now().to_rfc3339()),
            );
        }

        let entity = GraphEntity {
            id: 0,
            kind: EntityType::Discovery.as_str().to_string(),
            name: name.clone(),
            file_path: None,
            data: metadata,
        };

        let discovery_id = self
            .inner
            .insert_entity(&entity)
            .map_err(|e| anyhow::anyhow!("Failed to insert discovery: {}", e))?;

        // Record the discovery event
        let event_id = self.insert_event(
            "discovery-stored",
            json!({
                "agent": agent,
                "target": target,
                "discovery_type": discovery_type,
                "discovery_id": discovery_id
            }),
        )?;

        // Ensure agent entity exists
        let agent_id = self
            .entities_by_kind(EntityType::Agent.as_str())?
            .into_iter()
            .find(|e| e.name == agent)
            .map(|e| e.id)
            .unwrap_or_else(|| {
                self.insert_agent(agent, json!({}))
                    .expect("Failed to create agent")
            });

        // Link event to agent
        self.insert_edge(
            event_id,
            agent_id,
            EdgeType::PerformedBy,
            json!({"provenance": {"actor": "atheneum", "method": "store_discovery"}}),
        )?;

        // Link discovery to target (via RelatedTo edge stored in discovery data)
        // We create a pseudo-entity for the target if it doesn't exist
        // This allows querying by target later

        Ok(discovery_id)
    }

    /// Query all discoveries about a specific target
    pub fn query_discoveries(&self, target: &str) -> Result<Vec<GraphEntity>> {
        let conn = if let Some(direct) = self.inner.pool.direct_connection() {
            direct
        } else {
            &self
                .inner
                .pool
                .get()
                .map_err(|e| anyhow::anyhow!("Failed to get connection: {}", e))?
        };

        // Query discoveries where data->>'target' = target
        let mut stmt = conn
            .prepare_cached(
                "SELECT id, kind, name, file_path, data FROM graph_entities
                 WHERE kind=?1 AND json_extract(data, '$.target') = ?2",
            )
            .map_err(|e| anyhow::anyhow!("Failed to prepare statement: {}", e))?;

        let rows = stmt
            .query_map(params![EntityType::Discovery.as_str(), target], |row| {
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

        let mut discoveries = Vec::new();
        for row in rows {
            discoveries.push(row.map_err(|e| anyhow::anyhow!("Failed to read row: {}", e))?);
        }
        Ok(discoveries)
    }

    // ========================================================================
    // Bridge API: Handoff
    // ========================================================================

    /// Store a handoff manifest between agents
    pub fn store_handoff(&self, from_agent: &str, to_agent: &str, manifest: Value) -> Result<i64> {
        let name = format!("{} -> {}", from_agent, to_agent);

        let data = json!({
            "from_agent": from_agent,
            "to_agent": to_agent,
            "manifest": manifest,
            "created_at": Utc::now().to_rfc3339(),
            "claimed": false,
        });

        let entity = GraphEntity {
            id: 0,
            kind: EntityType::Handoff.as_str().to_string(),
            name,
            file_path: None,
            data,
        };

        let handoff_id = self
            .inner
            .insert_entity(&entity)
            .map_err(|e| anyhow::anyhow!("Failed to insert handoff: {}", e))?;

        // Record the handoff event
        let _event_id = self.insert_event(
            "handoff-created",
            json!({
                "from": from_agent,
                "to": to_agent,
                "handoff_id": handoff_id
            }),
        )?;

        // Ensure both agents exist
        let _ = self.insert_agent(from_agent, json!({}));
        let _ = self.insert_agent(to_agent, json!({}));

        Ok(handoff_id)
    }

    /// Get the most recent pending handoff for an agent
    ///
    /// Returns None if no pending handoffs exist
    pub fn get_pending_handoff(&self, agent: &str) -> Result<Option<GraphEntity>> {
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
                "SELECT id, kind, name, file_path, data FROM graph_entities
                 WHERE kind=?1 AND json_extract(data, '$.to_agent') = ?2
                 AND NOT json_extract(data, '$.claimed')
                 ORDER BY id DESC LIMIT 1",
            )
            .map_err(|e| anyhow::anyhow!("Failed to prepare statement: {}", e))?;

        let mut rows = stmt
            .query_map(params![EntityType::Handoff.as_str(), agent], |row| {
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

        match rows.next() {
            Some(Ok(entity)) => Ok(Some(entity)),
            Some(Err(e)) => Err(anyhow::anyhow!("Failed to read row: {}", e)),
            None => Ok(None),
        }
    }

    /// Mark a handoff as claimed
    pub fn mark_handoff_claimed(&self, handoff_id: i64) -> Result<()> {
        let conn = if let Some(direct) = self.inner.pool.direct_connection() {
            direct
        } else {
            &self
                .inner
                .pool
                .get()
                .map_err(|e| anyhow::anyhow!("Failed to get connection: {}", e))?
        };

        // Get the current entity data
        let entity = self.get_entity(handoff_id)?;

        // Update claimed flag
        let mut data = entity.data;
        if let Some(obj) = data.as_object_mut() {
            obj.insert("claimed".to_string(), Value::Bool(true));
            obj.insert(
                "claimed_at".to_string(),
                Value::String(Utc::now().to_rfc3339()),
            );
        }

        // Update the entity
        conn.execute(
            "UPDATE graph_entities SET data = ?1 WHERE id = ?2",
            params![serde_json::to_string(&data)?, handoff_id],
        )
        .map_err(|e| anyhow::anyhow!("Failed to update handoff: {}", e))?;

        Ok(())
    }

    // ========================================================================
    // Bridge API: Knowledge Query
    // ========================================================================

    /// Query all knowledge about a target
    ///
    /// Aggregates discoveries, handoffs, and calculates token savings.
    pub fn query_knowledge(&self, target: &str) -> Result<Value> {
        // Get all discoveries about this target
        let discoveries = self.query_discoveries(target).unwrap_or_default();

        // Get handoffs that mention this target (in task or files_analyzed)
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
                "SELECT id, kind, name, file_path, data FROM graph_entities
                 WHERE kind=?1 AND (
                     json_extract(data, '$.manifest.task') LIKE ?2 OR
                     EXISTS (
                         SELECT 1 FROM json_each(data, '$.manifest.files_analyzed')
                         WHERE json_each.value LIKE ?2
                     )
                 )",
            )
            .map_err(|e| anyhow::anyhow!("Failed to prepare statement: {}", e))?;

        let target_pattern = format!("%{}%", target);
        let rows = stmt
            .query_map(
                params![EntityType::Handoff.as_str(), target_pattern],
                |row| {
                    Ok(GraphEntity {
                        id: row.get(0)?,
                        kind: row.get(1)?,
                        name: row.get(2)?,
                        file_path: row.get(3)?,
                        data: serde_json::from_str(row.get_ref(4)?.as_str()?)
                            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
                    })
                },
            )
            .map_err(|e| anyhow::anyhow!("Failed to query: {}", e))?;

        let mut handoffs = Vec::new();
        for row in rows {
            handoffs.push(row.map_err(|e| anyhow::anyhow!("Failed to read row: {}", e))?);
        }

        // Calculate token savings
        let unique_agents: std::collections::HashSet<_> = discoveries
            .iter()
            .filter_map(|d| d.data.get("agent"))
            .filter_map(|a| a.as_str())
            .collect();

        let agent_count = unique_agents.len() as i64;
        let estimated_file_tokens = discoveries
            .iter()
            .filter_map(|d| d.data.get("token_count"))
            .filter_map(|t| t.as_i64())
            .next()
            .unwrap_or(15000); // Default assumption

        let without_sharing = agent_count * estimated_file_tokens;
        let with_sharing = estimated_file_tokens + (agent_count - 1).max(0) * 2500; // 2.5K summary per additional agent
        let saved = without_sharing.saturating_sub(with_sharing);
        let percentage_reduction = if without_sharing > 0 {
            (saved as f64 / without_sharing as f64) * 100.0
        } else {
            0.0
        };

        // Get total entity count
        let total_entities = conn
            .query_row("SELECT COUNT(*) FROM graph_entities", [], |row| row.get(0))
            .unwrap_or(0);

        Ok(json!({
            "target": target,
            "queried_at": Utc::now().to_rfc3339(),
            "total_entities": total_entities,
            "discovery_count": discoveries.len(),
            "discoveries": discoveries,
            "handoff_count": handoffs.len(),
            "handoffs": handoffs,
            "token_savings": {
                "unique_agents": agent_count,
                "estimated_file_tokens": estimated_file_tokens,
                "without_sharing": without_sharing,
                "with_sharing": with_sharing,
                "saved": saved,
                "percentage_reduction": percentage_reduction
            }
        }))
    }

    // ========================================================================
    // Project / Workspace scoping (ported from atheneum-py)
    //
    // project_id lets multiple projects (envoy, magellan, splice) share one
    // atheneum DB without name collisions. Stored inside the entity's `data`
    // JSON blob so no schema migration is needed; queried via json_extract.
    //
    // Pass project_id=None on read to get the legacy unfiltered behavior.
    // ========================================================================

    /// Like `store_discovery` but tags the entity with a project_id.
    pub fn store_discovery_in_project(
        &self,
        agent: &str,
        discovery_type: &str,
        target: &str,
        project_id: Option<&str>,
        mut metadata: Value,
    ) -> Result<i64> {
        if let (Some(pid), Some(obj)) = (project_id, metadata.as_object_mut()) {
            obj.insert("project_id".to_string(), Value::String(pid.to_string()));
        }
        self.store_discovery(agent, discovery_type, target, metadata)
    }

    /// Query discoveries for a target, optionally scoped to a project.
    pub fn query_discoveries_in_project(
        &self,
        target: &str,
        project_id: Option<&str>,
    ) -> Result<Vec<GraphEntity>> {
        let Some(pid) = project_id else {
            return self.query_discoveries(target);
        };

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
                "SELECT id, kind, name, file_path, data FROM graph_entities
                 WHERE kind=?1
                   AND json_extract(data, '$.target') = ?2
                   AND json_extract(data, '$.project_id') = ?3",
            )
            .map_err(|e| anyhow::anyhow!("Failed to prepare statement: {}", e))?;

        let rows = stmt
            .query_map(
                params![EntityType::Discovery.as_str(), target, pid],
                |row| {
                    Ok(GraphEntity {
                        id: row.get(0)?,
                        kind: row.get(1)?,
                        name: row.get(2)?,
                        file_path: row.get(3)?,
                        data: serde_json::from_str(row.get_ref(4)?.as_str()?)
                            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
                    })
                },
            )
            .map_err(|e| anyhow::anyhow!("Failed to query: {}", e))?;

        let mut discoveries = Vec::new();
        for row in rows {
            discoveries.push(row.map_err(|e| anyhow::anyhow!("Failed to read row: {}", e))?);
        }
        Ok(discoveries)
    }

    /// Like `store_handoff` but tags the handoff with a project_id.
    ///
    /// The project_id lives in the top-level handoff data, not nested under
    /// `manifest`, so query filters can reach it without parsing the manifest.
    pub fn store_handoff_in_project(
        &self,
        from_agent: &str,
        to_agent: &str,
        project_id: Option<&str>,
        manifest: Value,
    ) -> Result<i64> {
        let name = format!("{} -> {}", from_agent, to_agent);
        let mut data = json!({
            "from_agent": from_agent,
            "to_agent": to_agent,
            "manifest": manifest,
            "created_at": Utc::now().to_rfc3339(),
            "claimed": false,
        });
        if let (Some(pid), Some(obj)) = (project_id, data.as_object_mut()) {
            obj.insert("project_id".to_string(), Value::String(pid.to_string()));
        }

        let entity = GraphEntity {
            id: 0,
            kind: EntityType::Handoff.as_str().to_string(),
            name,
            file_path: None,
            data,
        };

        let handoff_id = self
            .inner
            .insert_entity(&entity)
            .map_err(|e| anyhow::anyhow!("Failed to insert handoff: {}", e))?;

        let _event_id = self.insert_event(
            "handoff-created",
            json!({
                "from": from_agent,
                "to": to_agent,
                "handoff_id": handoff_id,
                "project_id": project_id,
            }),
        )?;

        let _ = self.insert_agent(from_agent, json!({}));
        let _ = self.insert_agent(to_agent, json!({}));

        Ok(handoff_id)
    }

    /// Get the most recent pending handoff for an agent within a project.
    ///
    /// `project_id=None` falls back to the legacy unscoped lookup.
    pub fn get_pending_handoff_in_project(
        &self,
        agent: &str,
        project_id: Option<&str>,
    ) -> Result<Option<GraphEntity>> {
        let Some(pid) = project_id else {
            return self.get_pending_handoff(agent);
        };

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
                "SELECT id, kind, name, file_path, data FROM graph_entities
                 WHERE kind=?1
                   AND json_extract(data, '$.to_agent') = ?2
                   AND json_extract(data, '$.project_id') = ?3
                   AND NOT json_extract(data, '$.claimed')
                 ORDER BY id DESC LIMIT 1",
            )
            .map_err(|e| anyhow::anyhow!("Failed to prepare statement: {}", e))?;

        let mut rows = stmt
            .query_map(params![EntityType::Handoff.as_str(), agent, pid], |row| {
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

        match rows.next() {
            Some(Ok(entity)) => Ok(Some(entity)),
            Some(Err(e)) => Err(anyhow::anyhow!("Failed to read row: {}", e)),
            None => Ok(None),
        }
    }

    /// Aggregate knowledge for a target, scoped to a project when provided.
    pub fn query_knowledge_in_project(
        &self,
        target: &str,
        project_id: Option<&str>,
    ) -> Result<Value> {
        let Some(_pid) = project_id else {
            return self.query_knowledge(target);
        };

        let discoveries = self
            .query_discoveries_in_project(target, project_id)
            .unwrap_or_default();

        // Handoffs scoped to the same project that mention this target
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
                "SELECT id, kind, name, file_path, data FROM graph_entities
                 WHERE kind=?1
                   AND json_extract(data, '$.project_id') = ?2
                   AND (
                       json_extract(data, '$.manifest.task') LIKE ?3 OR
                       EXISTS (
                           SELECT 1 FROM json_each(data, '$.manifest.files_analyzed')
                           WHERE json_each.value LIKE ?3
                       )
                   )",
            )
            .map_err(|e| anyhow::anyhow!("Failed to prepare statement: {}", e))?;

        let target_pattern = format!("%{}%", target);
        let rows = stmt
            .query_map(
                params![
                    EntityType::Handoff.as_str(),
                    project_id.unwrap(),
                    target_pattern
                ],
                |row| {
                    Ok(GraphEntity {
                        id: row.get(0)?,
                        kind: row.get(1)?,
                        name: row.get(2)?,
                        file_path: row.get(3)?,
                        data: serde_json::from_str(row.get_ref(4)?.as_str()?)
                            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
                    })
                },
            )
            .map_err(|e| anyhow::anyhow!("Failed to query: {}", e))?;

        let mut handoffs = Vec::new();
        for row in rows {
            handoffs.push(row.map_err(|e| anyhow::anyhow!("Failed to read row: {}", e))?);
        }

        let unique_agents: std::collections::HashSet<_> = discoveries
            .iter()
            .filter_map(|d| d.data.get("agent"))
            .filter_map(|a| a.as_str())
            .collect();
        let agent_count = unique_agents.len() as i64;
        let estimated_file_tokens = discoveries
            .iter()
            .filter_map(|d| d.data.get("token_count"))
            .filter_map(|t| t.as_i64())
            .next()
            .unwrap_or(15000);
        let without_sharing = agent_count * estimated_file_tokens;
        let with_sharing = estimated_file_tokens + (agent_count - 1).max(0) * 2500;
        let saved = without_sharing.saturating_sub(with_sharing);
        let percentage_reduction = if without_sharing > 0 {
            (saved as f64 / without_sharing as f64) * 100.0
        } else {
            0.0
        };

        let total_entities = conn
            .query_row("SELECT COUNT(*) FROM graph_entities", [], |row| row.get(0))
            .unwrap_or(0);

        Ok(json!({
            "target": target,
            "project_id": project_id,
            "queried_at": Utc::now().to_rfc3339(),
            "total_entities": total_entities,
            "discovery_count": discoveries.len(),
            "discoveries": discoveries,
            "handoff_count": handoffs.len(),
            "handoffs": handoffs,
            "token_savings": {
                "unique_agents": agent_count,
                "estimated_file_tokens": estimated_file_tokens,
                "without_sharing": without_sharing,
                "with_sharing": with_sharing,
                "saved": saved,
                "percentage_reduction": percentage_reduction
            }
        }))
    }

    // ========================================================================
    // Dynamic Ontology (ported from atheneum-py OntologyService)
    //
    // Lets callers register new entity classes and edge properties at
    // runtime, instead of being constrained to the hardcoded EntityType /
    // EdgeType enums. Validation is permissive by default — undefined edge
    // types are allowed (KeplAI "open mode") — so existing data and
    // unregistered relations continue to work unchanged.
    // ========================================================================

    /// Register (or update) a class in the dynamic ontology.
    ///
    /// Idempotent by `name`: calling it twice with the same name updates
    /// the stored description rather than creating a duplicate entity.
    pub fn define_class(&self, name: &str, description: Option<&str>) -> Result<i64> {
        let existing = self.find_ontology_entity(ONTOLOGY_CLASS_KIND, name)?;

        let data = json!({
            "name": name,
            "description": description,
            "registered_at": Utc::now().to_rfc3339(),
        });

        if let Some(id) = existing {
            self.update_entity_data(id, &data)?;
            Ok(id)
        } else {
            let entity = GraphEntity {
                id: 0,
                kind: ONTOLOGY_CLASS_KIND.to_string(),
                name: name.to_string(),
                file_path: None,
                data,
            };
            self.inner
                .insert_entity(&entity)
                .map_err(|e| anyhow::anyhow!("Failed to insert OntologyClass: {}", e))
        }
    }

    /// Register (or update) a property (edge restriction) in the ontology.
    ///
    /// `domain_class` and `range_class` are class names that this edge may
    /// connect. Use the literal `"ANY"` to leave a side unconstrained.
    pub fn define_property(
        &self,
        name: &str,
        domain_class: &str,
        range_class: &str,
        description: Option<&str>,
    ) -> Result<i64> {
        let existing = self.find_ontology_entity(ONTOLOGY_PROPERTY_KIND, name)?;

        let data = json!({
            "name": name,
            "domain_class": domain_class,
            "range_class": range_class,
            "description": description,
            "registered_at": Utc::now().to_rfc3339(),
        });

        if let Some(id) = existing {
            self.update_entity_data(id, &data)?;
            Ok(id)
        } else {
            let entity = GraphEntity {
                id: 0,
                kind: ONTOLOGY_PROPERTY_KIND.to_string(),
                name: name.to_string(),
                file_path: None,
                data,
            };
            self.inner
                .insert_entity(&entity)
                .map_err(|e| anyhow::anyhow!("Failed to insert OntologyProperty: {}", e))
        }
    }

    /// List all registered ontology classes.
    pub fn list_classes(&self) -> Result<Vec<OntologyClassInfo>> {
        let entities = self.entities_by_kind(ONTOLOGY_CLASS_KIND)?;
        Ok(entities
            .into_iter()
            .map(|e| OntologyClassInfo {
                id: e.id,
                name: e.name.clone(),
                description: e
                    .data
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            })
            .collect())
    }

    /// List all registered ontology properties.
    pub fn list_properties(&self) -> Result<Vec<OntologyPropertyInfo>> {
        let entities = self.entities_by_kind(ONTOLOGY_PROPERTY_KIND)?;
        Ok(entities
            .into_iter()
            .map(|e| OntologyPropertyInfo {
                id: e.id,
                name: e.name.clone(),
                domain_class: e
                    .data
                    .get("domain_class")
                    .and_then(|v| v.as_str())
                    .unwrap_or("ANY")
                    .to_string(),
                range_class: e
                    .data
                    .get("range_class")
                    .and_then(|v| v.as_str())
                    .unwrap_or("ANY")
                    .to_string(),
                description: e
                    .data
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            })
            .collect())
    }

    /// Check whether `(from_kind)-[edge_type]->(to_kind)` is permitted by
    /// the ontology.
    ///
    /// Open-mode default: if no property is registered with this name, the
    /// edge is allowed. Once a property *is* registered, both its
    /// `domain_class` and `range_class` must match (the literal `"ANY"`
    /// matches anything).
    pub fn validate_edge(&self, from_kind: &str, to_kind: &str, edge_type: &str) -> Result<bool> {
        let props = self.list_properties()?;
        let Some(prop) = props.iter().find(|p| p.name == edge_type) else {
            return Ok(true); // open mode: undefined edges are allowed
        };
        let domain_ok = prop.domain_class == "ANY" || prop.domain_class == from_kind;
        let range_ok = prop.range_class == "ANY" || prop.range_class == to_kind;
        Ok(domain_ok && range_ok)
    }

    /// Seed the ontology with atheneum's standard set of classes.
    ///
    /// Idempotent — safe to call repeatedly on an existing DB. Combines
    /// the 10 historical [`EntityType`] variants with the additional kinds
    /// borrowed from the atheneum-py port (Project for workspace scoping,
    /// CodeSymbol, WikiPage, JournalSection, ReasoningLog).
    pub fn seed_standard_ontology(&self) -> Result<()> {
        const STANDARD: &[(&str, &str)] = &[
            ("Agent", "An autonomous participant"),
            ("Task", "A unit of work"),
            ("Event", "Something that happened, recorded for provenance"),
            ("Decision", "A choice made by an agent"),
            ("ToolCall", "An action taken by an agent"),
            ("FileChange", "A modification to a source file"),
            ("Verification", "A verification gate result"),
            ("Knowledge", "Persistent contextual information"),
            ("Discovery", "A dynamic insight found by an agent"),
            ("Handoff", "Context transfer between agents"),
            ("Project", "A workspace namespace"),
            ("CodeSymbol", "A source code entity"),
            ("WikiPage", "A static knowledge document"),
            ("JournalSection", "An entry in a Logseq journal"),
            ("ReasoningLog", "A stream of thought from an agent"),
        ];
        for (name, description) in STANDARD {
            self.define_class(name, Some(description))?;
        }
        Ok(())
    }

    // -- internal helpers for ontology storage -------------------------------

    fn find_ontology_entity(&self, kind: &str, name: &str) -> Result<Option<i64>> {
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
            .prepare_cached("SELECT id FROM graph_entities WHERE kind = ?1 AND name = ?2 LIMIT 1")
            .map_err(|e| anyhow::anyhow!("Failed to prepare statement: {}", e))?;
        let id_opt: Option<i64> = stmt.query_row(params![kind, name], |row| row.get(0)).ok();
        Ok(id_opt)
    }

    fn update_entity_data(&self, id: i64, data: &Value) -> Result<()> {
        let conn = if let Some(direct) = self.inner.pool.direct_connection() {
            direct
        } else {
            &self
                .inner
                .pool
                .get()
                .map_err(|e| anyhow::anyhow!("Failed to get connection: {}", e))?
        };
        conn.execute(
            "UPDATE graph_entities SET data = ?1 WHERE id = ?2",
            params![serde_json::to_string(data)?, id],
        )
        .map_err(|e| anyhow::anyhow!("Failed to update entity: {}", e))?;
        Ok(())
    }

    // ========================================================================
    // Semantic Search (ported from atheneum-py SearchService)
    //
    // Wraps sqlitegraph 3.0's native HNSW index with a deterministic
    // hash-based embedder so agents can ask "what do we know about X" and
    // get fuzzy matches against stored Discovery entities — not just exact
    // target-name lookups.
    //
    // The embedder is intentionally simple (bag-of-words → hashed buckets,
    // L2 normalized): no ML model dependencies, deterministic across
    // platforms, and good enough for short discovery summaries. Swap in a
    // real model later by feeding pre-computed vectors directly.
    // ========================================================================

    /// (Re-)build the semantic search index over all stored Discovery
    /// entities.
    ///
    /// Idempotent: if an index with the reserved name already exists it is
    /// dropped and rebuilt so callers don't have to think about stale state.
    /// Call this once after a batch of `store_discovery*` calls (or on
    /// startup) before invoking [`semantic_search`].
    pub fn build_search_index(&self) -> Result<()> {
        // Drop any existing index so this method stays idempotent.
        let _ = self.inner.delete_hnsw_index(SEARCH_INDEX_NAME);

        let config = HnswConfigBuilder::new()
            .dimension(SEARCH_EMBED_DIM)
            .distance_metric(DistanceMetric::Cosine)
            .build()
            .map_err(|e| anyhow::anyhow!("HNSW config build failed: {}", e))?;

        // Create the index (guard is dropped immediately; we'll get a
        // fresh mutable borrow per-insert below).
        {
            let _guard = self
                .inner
                .hnsw_index(SEARCH_INDEX_NAME, config)
                .map_err(|e| anyhow::anyhow!("hnsw_index create failed: {}", e))?;
        }

        let discoveries = self.entities_by_kind(EntityType::Discovery.as_str())?;
        for entity in discoveries {
            let text = embed_text_for_entity(&entity);
            let vector = hash_embed(&text, SEARCH_EMBED_DIM);
            let entity_id = entity.id;
            self.inner
                .get_hnsw_index_mut(SEARCH_INDEX_NAME, move |idx| {
                    idx.insert_vector(&vector, Some(json!({"entity_id": entity_id})))
                })
                .map_err(|e| anyhow::anyhow!("get_hnsw_index_mut failed: {}", e))?
                .map_err(|e| anyhow::anyhow!("insert_vector failed: {}", e))?;
        }

        Ok(())
    }

    /// Find up to `k` Discovery entities most similar to `query`.
    ///
    /// `project_id=Some(...)` post-filters results to that project (the
    /// underlying HNSW does not partition by project, so we ask for extra
    /// candidates and then filter). `project_id=None` returns matches from
    /// every project.
    pub fn semantic_search(
        &self,
        query: &str,
        k: usize,
        project_id: Option<&str>,
    ) -> Result<Vec<SearchResult>> {
        let query_vec = hash_embed(query, SEARCH_EMBED_DIM);
        // Over-fetch so the project filter still has room to return `k`.
        let fetch_k = if project_id.is_some() { k * 4 } else { k };

        let hits = self
            .inner
            .get_hnsw_index_ref(SEARCH_INDEX_NAME, |idx| idx.search(&query_vec, fetch_k))
            .map_err(|e| anyhow::anyhow!("search index lookup failed: {}", e))?
            .map_err(|e| anyhow::anyhow!("hnsw search failed: {}", e))?;

        let mut results = Vec::with_capacity(hits.len());
        for (vector_id, score) in hits {
            // Resolve vector_id → entity_id via the metadata we stored on insert.
            let metadata = self
                .inner
                .get_hnsw_index_ref(SEARCH_INDEX_NAME, |idx| {
                    idx.get_vector(vector_id).ok().flatten()
                })
                .map_err(|e| anyhow::anyhow!("get_vector failed: {}", e))?;
            let Some((_vec, meta)) = metadata else {
                continue;
            };
            let Some(entity_id) = meta.get("entity_id").and_then(|v| v.as_i64()) else {
                continue;
            };

            let entity = match self.get_entity(entity_id) {
                Ok(e) => e,
                Err(_) => continue, // entity was deleted since indexing — skip
            };

            if let Some(pid) = project_id {
                let entity_project = entity
                    .data
                    .get("project_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if entity_project != pid {
                    continue;
                }
            }

            results.push(SearchResult {
                id: entity.id,
                name: entity.name,
                kind: entity.kind,
                score,
                data: entity.data,
            });

            if results.len() >= k {
                break;
            }
        }
        Ok(results)
    }

    // ========================================================================
    // Audit Trail (ported from atheneum-py)
    //
    // Records the provenance chain
    //   (Agent)-[PerformedBy]->(ReasoningLog)-[Called]->(ToolCall)-[Modified]->(target)
    // so the graph stops being a bag of disconnected entities. Each write
    // step is a small helper; `record_agent_action` is the one-shot
    // convenience wrapper. `get_action_trace` walks the chain back.
    //
    // Edge-type mapping (the existing Rust enum carries the right semantics):
    //   PerformedBy ↔ atheneum-py THOUGHT
    //   Called      ↔ atheneum-py USED
    //   Modified    ↔ atheneum-py MODIFIED
    // ========================================================================

    /// Create a ReasoningLog entity for an agent's thought and link
    /// `(Agent)-[PerformedBy]->(ReasoningLog)`. The agent entity is
    /// created on demand if it doesn't exist.
    pub fn insert_reasoning_log(
        &self,
        agent: &str,
        content: &str,
        project_id: Option<&str>,
    ) -> Result<i64> {
        let agent_id = self.ensure_agent(agent)?;

        let mut data = json!({
            "agent": agent,
            "content": content,
            "timestamp": Utc::now().to_rfc3339(),
        });
        if let (Some(pid), Some(obj)) = (project_id, data.as_object_mut()) {
            obj.insert("project_id".to_string(), Value::String(pid.to_string()));
        }

        let entity = GraphEntity {
            id: 0,
            kind: "ReasoningLog".to_string(),
            name: format!(
                "{}: {}",
                agent,
                content.chars().take(48).collect::<String>()
            ),
            file_path: None,
            data,
        };
        let log_id = self
            .inner
            .insert_entity(&entity)
            .map_err(|e| anyhow::anyhow!("Failed to insert ReasoningLog: {}", e))?;

        self.insert_edge(
            agent_id,
            log_id,
            EdgeType::PerformedBy,
            json!({"provenance": {"actor": "atheneum", "method": "insert_reasoning_log"}}),
        )?;

        Ok(log_id)
    }

    /// Create a ToolCall entity and link `(ReasoningLog)-[Called]->(ToolCall)`.
    pub fn insert_tool_call(
        &self,
        reasoning_log_id: i64,
        tool_name: &str,
        args: Value,
        project_id: Option<&str>,
    ) -> Result<i64> {
        let mut data = json!({
            "tool_name": tool_name,
            "args": args,
            "timestamp": Utc::now().to_rfc3339(),
        });
        if let (Some(pid), Some(obj)) = (project_id, data.as_object_mut()) {
            obj.insert("project_id".to_string(), Value::String(pid.to_string()));
        }

        let entity = GraphEntity {
            id: 0,
            kind: "ToolCall".to_string(),
            name: tool_name.to_string(),
            file_path: None,
            data,
        };
        let tool_id = self
            .inner
            .insert_entity(&entity)
            .map_err(|e| anyhow::anyhow!("Failed to insert ToolCall: {}", e))?;

        self.insert_edge(
            reasoning_log_id,
            tool_id,
            EdgeType::Called,
            json!({"provenance": {"actor": "atheneum", "method": "insert_tool_call"}}),
        )?;

        Ok(tool_id)
    }

    /// Link `(ToolCall)-[Modified]->(target)` for an already-existing target
    /// entity. Returns the edge id so callers can decorate it later.
    pub fn record_tool_modifies(&self, tool_call_id: i64, target_id: i64) -> Result<i64> {
        self.insert_edge(
            tool_call_id,
            target_id,
            EdgeType::Modified,
            json!({"provenance": {"actor": "atheneum", "method": "record_tool_modifies"}}),
        )
    }

    /// One-shot recording of a full agent action.
    ///
    /// Equivalent to: `insert_reasoning_log`, then `insert_tool_call` per
    /// entry in `tool_calls`, then `record_tool_modifies` per modified
    /// target. Returns the IDs created so callers can do follow-up writes
    /// without re-querying.
    pub fn record_agent_action(
        &self,
        agent: &str,
        thought: &str,
        tool_calls: Vec<ToolCallRecord>,
        project_id: Option<&str>,
    ) -> Result<ActionTrace> {
        let log_id = self.insert_reasoning_log(agent, thought, project_id)?;
        let agent_id = self.ensure_agent(agent)?;

        let mut tool_call_ids = Vec::with_capacity(tool_calls.len());
        let mut modified_edge_ids = Vec::new();
        for tc in tool_calls {
            let tool_id = self.insert_tool_call(log_id, &tc.tool_name, tc.args, project_id)?;
            tool_call_ids.push(tool_id);
            for target in tc.modified_targets {
                let edge_id = self.record_tool_modifies(tool_id, target)?;
                modified_edge_ids.push(edge_id);
            }
        }

        Ok(ActionTrace {
            agent_id,
            reasoning_log_id: log_id,
            tool_call_ids,
            modified_edge_ids,
        })
    }

    /// Walk the provenance chain back: return every action the named agent
    /// performed, with each tool call and the entities it modified.
    /// `project_id=Some(p)` restricts to logs tagged with that project.
    pub fn get_action_trace(
        &self,
        agent: &str,
        project_id: Option<&str>,
    ) -> Result<Vec<ActionRecord>> {
        // Walk all ReasoningLogs for this agent (optionally project-scoped).
        let logs: Vec<GraphEntity> = self
            .entities_by_kind("ReasoningLog")?
            .into_iter()
            .filter(|log| log.data.get("agent").and_then(|v| v.as_str()) == Some(agent))
            .filter(|log| match project_id {
                None => true,
                Some(pid) => log.data.get("project_id").and_then(|v| v.as_str()) == Some(pid),
            })
            .collect();

        let mut records = Vec::with_capacity(logs.len());
        for log in logs {
            let log_id = log.id;
            // Tool calls reachable via Called edges out of this log
            let tool_call_entities: Vec<GraphEntity> = self
                .outgoing_edges(log_id)?
                .into_iter()
                .filter(|e| e.edge_type == EdgeType::Called.as_str())
                .filter_map(|e| self.get_entity(e.to_id).ok())
                .collect();

            let mut tool_calls = Vec::with_capacity(tool_call_entities.len());
            for tc in tool_call_entities {
                let modified: Vec<GraphEntity> = self
                    .outgoing_edges(tc.id)?
                    .into_iter()
                    .filter(|e| e.edge_type == EdgeType::Modified.as_str())
                    .filter_map(|e| self.get_entity(e.to_id).ok())
                    .collect();
                tool_calls.push(ToolCallTrace {
                    tool_call: tc,
                    modified,
                });
            }

            records.push(ActionRecord {
                reasoning_log: log,
                tool_calls,
            });
        }
        Ok(records)
    }

    /// Look up an Agent entity by name, creating it if missing. Used as a
    /// safety net so audit-trail helpers don't fail on first call for a
    /// new agent.
    fn ensure_agent(&self, name: &str) -> Result<i64> {
        if let Some(existing) = self
            .entities_by_kind(EntityType::Agent.as_str())?
            .into_iter()
            .find(|a| a.name == name)
        {
            return Ok(existing.id);
        }
        self.insert_agent(name, json!({}))
    }

    // ========================================================================
    // Wiki + Journal Ingestion (ported from atheneum-py watchers/)
    //
    // ingest_wiki_page: full markdown file → WikiPage entity with content
    //   hash + extracted wikilinks + project scoping. Idempotent by path.
    // ingest_journal:   Logseq-style daily journal → one JournalSection
    //   entity per H2 section, with extracted kanban transitions.
    // sync_*_directory: walk a dir, ingest every .md file.
    //
    // A live notify-based file watcher is intentionally out of scope here;
    // calling sync_wiki_directory on a debounce timer or fs event is a
    // thin downstream wrapper.
    // ========================================================================

    /// Ingest a Markdown wiki page.
    ///
    /// Parses optional YAML frontmatter, extracts `[[wikilinks]]`, computes
    /// a content hash for change detection, and stores everything as a
    /// `WikiPage` entity. Idempotent by `path` — re-ingesting the same path
    /// updates the existing entity in place rather than creating a duplicate.
    pub fn ingest_wiki_page(
        &self,
        path: &str,
        content: &str,
        project_id: Option<&str>,
    ) -> Result<i64> {
        let (frontmatter, body) = parse_frontmatter_lenient(content);
        let mut data = json!({
            "path": path,
            "body": body,
            "content_hash": content_hash(body),
            "wikilinks": extract_wikilinks(body),
        });
        if let Some(obj) = data.as_object_mut() {
            if let Some(fm_obj) = frontmatter.as_object() {
                for (k, v) in fm_obj {
                    obj.insert(k.clone(), v.clone());
                }
            }
            if let Some(pid) = project_id {
                obj.insert("project_id".to_string(), Value::String(pid.to_string()));
            }
        }

        let existing = self.find_ontology_entity("WikiPage", path)?;
        if let Some(id) = existing {
            self.update_entity_data(id, &data)?;
            Ok(id)
        } else {
            let entity = GraphEntity {
                id: 0,
                kind: "WikiPage".to_string(),
                name: path.to_string(),
                file_path: Some(path.to_string()),
                data,
            };
            self.inner
                .insert_entity(&entity)
                .map_err(|e| anyhow::anyhow!("Failed to insert WikiPage: {}", e))
        }
    }

    /// Ingest a Logseq-style journal file as a series of `JournalSection`
    /// entities — one per H2 header. Each section carries its kanban
    /// transitions and wikilinks.
    pub fn ingest_journal(
        &self,
        path: &str,
        content: &str,
        project_id: Option<&str>,
    ) -> Result<Vec<i64>> {
        let sections = parse_journal_sections(content);
        let mut ids = Vec::with_capacity(sections.len());
        for (idx, section) in sections.iter().enumerate() {
            let mut data = json!({
                "path": path,
                "section_index": idx,
                "time": section.time,
                "title": section.title,
                "body": section.body,
                "wikilinks": extract_wikilinks(&section.body),
                "kanban_updates": section
                    .kanban_updates
                    .iter()
                    .map(|u| {
                        json!({
                            "task_title": u.task_title,
                            "new_status": u.new_status.as_str(),
                        })
                    })
                    .collect::<Vec<_>>(),
            });
            if let (Some(pid), Some(obj)) = (project_id, data.as_object_mut()) {
                obj.insert("project_id".to_string(), Value::String(pid.to_string()));
            }

            let name = format!("{}#{}", path, idx);
            let entity = GraphEntity {
                id: 0,
                kind: "JournalSection".to_string(),
                name,
                file_path: Some(path.to_string()),
                data,
            };
            let id = self
                .inner
                .insert_entity(&entity)
                .map_err(|e| anyhow::anyhow!("Failed to insert JournalSection: {}", e))?;
            ids.push(id);
        }
        Ok(ids)
    }

    /// One-shot sync of every `.md` file in `dir` as a WikiPage. Non-recursive.
    pub fn sync_wiki_directory(
        &self,
        dir: &std::path::Path,
        project_id: Option<&str>,
    ) -> Result<Vec<i64>> {
        let mut ids = Vec::new();
        for entry in std::fs::read_dir(dir)
            .map_err(|e| anyhow::anyhow!("read_dir {} failed: {}", dir.display(), e))?
        {
            let entry = entry.map_err(|e| anyhow::anyhow!("dir entry: {}", e))?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            let content = std::fs::read_to_string(&path)
                .map_err(|e| anyhow::anyhow!("read {} failed: {}", path.display(), e))?;
            let id =
                self.ingest_wiki_page(path.to_str().unwrap_or_default(), &content, project_id)?;
            ids.push(id);
        }
        Ok(ids)
    }

    /// One-shot sync of every `.md` file in `dir` as a journal. Non-recursive.
    pub fn sync_journal_directory(
        &self,
        dir: &std::path::Path,
        project_id: Option<&str>,
    ) -> Result<Vec<i64>> {
        let mut all_ids = Vec::new();
        for entry in std::fs::read_dir(dir)
            .map_err(|e| anyhow::anyhow!("read_dir {} failed: {}", dir.display(), e))?
        {
            let entry = entry.map_err(|e| anyhow::anyhow!("dir entry: {}", e))?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            let content = std::fs::read_to_string(&path)
                .map_err(|e| anyhow::anyhow!("read {} failed: {}", path.display(), e))?;
            let ids =
                self.ingest_journal(path.to_str().unwrap_or_default(), &content, project_id)?;
            all_ids.extend(ids);
        }
        Ok(all_ids)
    }
}

/// Pull human-readable text out of a discovery entity for embedding.
///
/// We mix the symbol/target name, agent name, discovery type, file path,
/// and a free-form `summary` field so simple word-overlap embeddings work
/// against any of them.
fn embed_text_for_entity(entity: &GraphEntity) -> String {
    let mut parts = vec![entity.kind.clone(), entity.name.clone()];
    for key in [
        "target",
        "agent",
        "discovery_type",
        "file",
        "file_path",
        "summary",
        "signature",
        "kind",
    ] {
        if let Some(value) = entity.data.get(key).and_then(|v| v.as_str()) {
            parts.push(value.to_string());
        }
    }
    parts.join(" ")
}

/// Deterministic bag-of-words → fixed-dimension vector. Each word's hash
/// picks a bucket; the resulting vector is L2-normalized so cosine
/// distance is meaningful. Not as expressive as a real embedding model
/// but has no dependencies and runs anywhere.
fn hash_embed(text: &str, dim: usize) -> Vec<f32> {
    let mut vector = vec![0.0_f32; dim];
    for token in text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
    {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        token.to_ascii_lowercase().hash(&mut hasher);
        let bucket = (hasher.finish() as usize) % dim;
        vector[bucket] += 1.0;
    }
    // L2 normalize so cosine distance reflects direction, not magnitude.
    let norm: f32 = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in &mut vector {
            *v /= norm;
        }
    }
    vector
}

/// Kanban transition states recognized in journal text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KanbanStatus {
    Todo,
    InProgress,
    Done,
    Blocked,
}

impl KanbanStatus {
    /// Canonical uppercase representation (also what gets stored in JSON).
    pub fn as_str(&self) -> &'static str {
        match self {
            KanbanStatus::Todo => "TODO",
            KanbanStatus::InProgress => "IN_PROGRESS",
            KanbanStatus::Done => "DONE",
            KanbanStatus::Blocked => "BLOCKED",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_uppercase().as_str() {
            "TODO" => Some(KanbanStatus::Todo),
            "IN_PROGRESS" | "IN-PROGRESS" | "INPROGRESS" => Some(KanbanStatus::InProgress),
            "DONE" => Some(KanbanStatus::Done),
            "BLOCKED" => Some(KanbanStatus::Blocked),
            _ => None,
        }
    }
}

/// A single kanban transition extracted from journal text:
/// `"Task title" -> STATUS` (or `→` for the unicode arrow).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KanbanUpdate {
    pub task_title: String,
    pub new_status: KanbanStatus,
}

/// One H2-delimited section of a Logseq-style daily journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalSection {
    /// Optional `HH:MM` prefix on the H2 header.
    pub time: Option<String>,
    /// Heading text after the optional time prefix.
    pub title: String,
    /// Everything between this H2 and the next.
    pub body: String,
    /// Kanban transitions found inside `body`.
    pub kanban_updates: Vec<KanbanUpdate>,
}

/// Extract `[[wikilink]]` targets from markdown body text.
///
/// Returns inner text of each `[[...]]` pair in document order. Duplicates
/// are preserved so callers can reason about reference frequency.
pub fn extract_wikilinks(content: &str) -> Vec<String> {
    // The pattern is small enough that compiling once per call is fine for
    // the volumes atheneum sees (KB-scale wiki bodies).
    let re = Regex::new(r"\[\[([^\[\]]+?)\]\]").expect("static regex");
    re.captures_iter(content)
        .map(|c| c[1].to_string())
        .collect()
}

/// Deterministic SHA-256 hex digest of `content`. Used for cheap change
/// detection on wiki pages — re-ingesting with the same hash means the
/// body didn't change, so downstream consumers can skip work.
pub fn content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Segment a Logseq-style journal on H2 headers. A header may be either
/// `## HH:MM | Topic` (the Logseq convention) or just `## Topic`.
///
/// Each returned section's `body` is everything between its header and the
/// next header (or end-of-file). `kanban_updates` is pre-extracted from
/// the body so callers don't need a second pass.
pub fn parse_journal_sections(content: &str) -> Vec<JournalSection> {
    // Locate every H2 header line and remember (start_of_body, time, title).
    let re = Regex::new(r"(?m)^##\s+(?:(\d{2}:\d{2})\s*\|\s*)?(.+?)\s*$").expect("static regex");
    let mut headers: Vec<(usize, Option<String>, String)> = Vec::new();
    for m in re.captures_iter(content) {
        let full = m.get(0).expect("full match");
        let body_start = full.end();
        let time = m.get(1).map(|t| t.as_str().to_string());
        let title = m.get(2).map(|t| t.as_str().to_string()).unwrap_or_default();
        headers.push((body_start, time, title));
    }

    let mut sections = Vec::with_capacity(headers.len());
    for (i, (body_start, time, title)) in headers.iter().enumerate() {
        let body_end = headers.get(i + 1).map(|h| {
            // Body ends just before the next header line — back up to the
            // newline that precedes the next `##` to keep the boundary
            // clean.
            let next_header_pos = h.0 - content[..h.0].rfind('\n').map(|n| h.0 - n).unwrap_or(0);
            next_header_pos.saturating_sub(0).max(*body_start)
        });
        // Default: body runs to the next header start (we'll over-include
        // a newline; trim() in the consumer cleans it up).
        let raw_end = headers
            .get(i + 1)
            .map(|h| {
                // Walk back to just after the previous newline so the next
                // `##` line isn't included in *this* section's body.
                content[..h.0]
                    .rfind("\n##")
                    .map(|p| p + 1) // include the newline at end of prev line
                    .unwrap_or(h.0)
            })
            .unwrap_or(content.len());
        let _ = body_end; // kept for clarity above; raw_end is the real cut

        let body = content[*body_start..raw_end].trim().to_string();
        let kanban_updates = extract_kanban_updates(&body);
        sections.push(JournalSection {
            time: time.clone(),
            title: title.clone(),
            body,
            kanban_updates,
        });
    }
    sections
}

/// Extract `"task" -> STATUS` (or `→`) transitions from journal text.
pub fn extract_kanban_updates(content: &str) -> Vec<KanbanUpdate> {
    let re = Regex::new(r#"["']([^"']+?)["']\s*(?:->|→)\s*(TODO|IN[_ -]?PROGRESS|DONE|BLOCKED)"#)
        .expect("static regex");
    re.captures_iter(content)
        .filter_map(|c| {
            let task = c.get(1)?.as_str().to_string();
            let status = KanbanStatus::parse(c.get(2)?.as_str())?;
            Some(KanbanUpdate {
                task_title: task,
                new_status: status,
            })
        })
        .collect()
}

/// Lenient frontmatter parser used by `ingest_wiki_page`. Returns
/// `(Value::Object({}), full_content)` when no frontmatter is found instead
/// of erroring, since wiki pages frequently skip the metadata block.
fn parse_frontmatter_lenient(content: &str) -> (Value, &str) {
    parse_frontmatter(content).unwrap_or((Value::Object(serde_json::Map::new()), content))
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
