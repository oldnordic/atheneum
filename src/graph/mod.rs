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
            .query_map(
                params![EntityType::Handoff.as_str(), agent, pid],
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
