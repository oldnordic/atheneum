use anyhow::Result;
use chrono::Utc;
use rusqlite::params;
use serde_json::Value;
use sqlitegraph::{GraphEdge, GraphEntity, SqliteConfig, SqliteGraph};

use embed::HashEmbedder;

pub mod audit;
mod cache;
pub mod claude;
pub mod discovery;
pub mod dream;
pub mod embed;
pub mod evidence;
pub mod extraction;
pub mod handoff;
mod hashing;
pub mod knowledge;
pub mod magellan_bridge;
pub mod memory;
pub mod navigation;
pub mod ontology;
pub mod planning;
pub mod search;
pub mod types;
pub mod wiki;

use cache::GraphRuntime;
pub use cache::RuntimeStats;
pub use dream::{DreamConfig, DreamFinding, DreamMode, DreamPhase, DreamReport};
pub use navigation::{estimate_entity_tokens, truncate_subgraph};
pub use planning::{KanbanStatus, KanbanUpdate};
pub use types::ProvenanceData;
pub use types::{
    ActionRecord, ActionTrace, AppliedKanbanUpdate, AtheneumError, BlockerType,
    ClaudeTranscriptImportParams, ClaudeTranscriptImportSummary, CommitParams,
    DisambiguationResult, DiscoveryPreview, EdgeType, EndSessionParams, EntityType,
    FileAccessParams, FileWriteParams, FixChainParams, GraphStats, HandoffPreview, MemoryPreview,
    NavigateQueryPlan, Neighbors, OntologyClassInfo, OntologyPropertyInfo, PromptParams,
    QueryIntent, RecordEventParams, RelationEndpoint, RelationHint, RequirementStatus,
    ResolvedEntity, SearchResult, SessionParams, SessionProgressParams, SessionSummary,
    SubgraphView, TaskDetail, TestRunParams, ToolCallParams, ToolCallRecord, ToolCallTrace,
    ONTOLOGY_CLASS_KIND, ONTOLOGY_PROPERTY_KIND,
};
pub use wiki::{
    content_hash, extract_kanban_updates, extract_wikilinks, parse_journal_sections,
    JournalSection, WikiPage,
};

pub(super) fn json_to_string(v: &Value) -> Result<String> {
    serde_json::to_string(v).map_err(|e| anyhow::anyhow!("JSON serialization failed: {}", e))
}

pub struct AtheneumGraph {
    inner: SqliteGraph,
    embedder: Box<dyn embed::TextEmbedder>,
    runtime: GraphRuntime,
}

impl AtheneumGraph {
    pub fn open_in_memory() -> Result<Self> {
        let inner = SqliteGraph::open_in_memory()?;
        let g = Self {
            inner,
            embedder: Box::new(HashEmbedder::new(128)),
            runtime: GraphRuntime::default(),
        };
        g.run_startup_migrations()?;
        Ok(g)
    }

    pub fn open(path: &std::path::Path) -> Result<Self> {
        let cfg = SqliteConfig::new().with_pragma("busy_timeout", "5000");
        let inner = SqliteGraph::open_with_config(path, &cfg)?;
        let g = Self {
            inner,
            embedder: Box::new(HashEmbedder::new(128)),
            runtime: GraphRuntime::default(),
        };
        g.run_startup_migrations()?;
        Ok(g)
    }

    pub fn set_embedder(&mut self, embedder: Box<dyn embed::TextEmbedder>) {
        self.embedder = embedder;
    }

    pub fn embedder_dimension(&self) -> usize {
        self.embedder.dimension()
    }

    pub fn runtime_stats(&self) -> RuntimeStats {
        self.runtime.snapshot()
    }

    fn run_startup_migrations(&self) -> Result<()> {
        self.with_raw_connection(crate::db::run_migrations)
    }

    pub fn with_raw_connection<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&rusqlite::Connection) -> Result<R>,
    {
        if let Some(direct) = self.inner.pool.direct_connection() {
            f(direct)
        } else {
            let pooled = self
                .inner
                .pool
                .get()
                .map_err(|e| anyhow::anyhow!("Failed to get connection: {}", e))?;
            f(&pooled)
        }
    }

    pub fn is_healthy(&self) -> bool {
        self.with_raw_connection(|conn| {
            conn.execute_batch("SELECT 1").ok();
            Ok(())
        })
        .is_ok()
    }

    pub fn get_entity(&self, id: i64) -> Result<GraphEntity> {
        self.inner
            .get_entity(id)
            .map_err(|_e| AtheneumError::EntityNotFound(id).into())
    }

    pub fn get_edge(&self, id: i64) -> Result<GraphEdge> {
        self.inner
            .get_edge(id)
            .map_err(|_e| AtheneumError::EdgeNotFound(id).into())
    }

    pub fn outgoing_edges(&self, entity_id: i64) -> Result<Vec<GraphEdge>> {
        self.with_raw_connection(|conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT id, from_id, to_id, edge_type, data FROM graph_edges WHERE from_id=?1 ORDER BY id"
            )?;
            let rows = stmt.query_map(rusqlite::params![entity_id], |row| {
                Ok(GraphEdge {
                    id: row.get(0)?,
                    from_id: row.get(1)?,
                    to_id: row.get(2)?,
                    edge_type: row.get(3)?,
                    data: serde_json::from_str(row.get_ref(4)?.as_str()?)
                        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
                })
            })?;
            let mut edges = Vec::new();
            for row in rows {
                edges.push(row?);
            }
            Ok(edges)
        })
    }

    pub fn incoming_edges(&self, entity_id: i64) -> Result<Vec<GraphEdge>> {
        self.with_raw_connection(|conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT id, from_id, to_id, edge_type, data FROM graph_edges WHERE to_id=?1 ORDER BY id"
            )?;
            let rows = stmt.query_map(rusqlite::params![entity_id], |row| {
                Ok(GraphEdge {
                    id: row.get(0)?,
                    from_id: row.get(1)?,
                    to_id: row.get(2)?,
                    edge_type: row.get(3)?,
                    data: serde_json::from_str(row.get_ref(4)?.as_str()?)
                        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
                })
            })?;
            let mut edges = Vec::new();
            for row in rows {
                edges.push(row?);
            }
            Ok(edges)
        })
    }

    pub fn all_entities(&self) -> Result<Vec<GraphEntity>> {
        with_graph_conn(&self.inner, |conn| {
            let mut stmt =
                conn.prepare_cached("SELECT id, kind, name, file_path, data FROM graph_entities")?;

            let rows = stmt.query_map([], |row| {
                Ok(GraphEntity {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    name: row.get(2)?,
                    file_path: row.get(3)?,
                    data: serde_json::from_str(row.get_ref(4)?.as_str()?)
                        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
                })
            })?;

            let mut entities = Vec::new();
            for row in rows {
                entities.push(row?);
            }
            Ok(entities)
        })
    }

    pub fn entities_by_kind(&self, kind: &str) -> Result<Vec<GraphEntity>> {
        with_graph_conn(&self.inner, |conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT id, kind, name, file_path, data FROM graph_entities WHERE kind=?1",
            )?;

            let rows = stmt.query_map(params![kind], |row| {
                Ok(GraphEntity {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    name: row.get(2)?,
                    file_path: row.get(3)?,
                    data: serde_json::from_str(row.get_ref(4)?.as_str()?)
                        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
                })
            })?;

            let mut entities = Vec::new();
            for row in rows {
                entities.push(row?);
            }
            Ok(entities)
        })
    }

    pub fn count_entities_by_kind(&self) -> Result<Vec<(String, i64)>> {
        with_graph_conn(&self.inner, |conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT kind, COUNT(*) as count FROM graph_entities GROUP BY kind ORDER BY count DESC"
            )?;

            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?;

            let mut counts = Vec::new();
            for row in rows {
                counts.push(row?);
            }
            Ok(counts)
        })
    }

    pub fn count_edges_by_type(&self) -> Result<Vec<(String, i64)>> {
        with_graph_conn(&self.inner, |conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT edge_type, COUNT(*) as count FROM graph_edges GROUP BY edge_type ORDER BY count DESC"
            )?;

            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?;

            let mut counts = Vec::new();
            for row in rows {
                counts.push(row?);
            }
            Ok(counts)
        })
    }

    pub fn insert_agent(&self, name: &str, data: Value) -> Result<i64> {
        let metadata_str = json_to_string(&data)?;
        let sql_id = self.with_raw_connection(|conn| {
            let project_id = data.get("project_id").and_then(|v| v.as_str());
            conn.execute(
                "INSERT OR IGNORE INTO agents (name, project_id, metadata, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![name, project_id, metadata_str, Utc::now().to_rfc3339()],
            )?;
            let id: i64 = conn.query_row(
                "SELECT id FROM agents WHERE name = ?1",
                rusqlite::params![name],
                |row| row.get(0),
            )?;
            Ok(id)
        })?;

        let mut data = data;
        if let Some(obj) = data.as_object_mut() {
            obj.insert("sql_id".to_string(), Value::Number(sql_id.into()));
        }

        let entity = GraphEntity {
            id: 0,
            kind: EntityType::Agent.as_str().to_string(),
            name: name.to_string(),
            file_path: None,
            data,
        };
        self.inner.insert_entity(&entity).map_err(Into::into)
    }

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

    pub fn insert_event(&self, name: &str, mut data: Value) -> Result<i64> {
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

    /// Upsert a Concept entity by name. If an entity of kind "Concept" with
    /// the same name already exists, returns its ID. Otherwise creates a new one.
    pub fn upsert_concept(&self, name: &str, data: &Value) -> Result<i64> {
        let existing = with_graph_conn(&self.inner, |conn| {
            let mut stmt = conn
                .prepare("SELECT id FROM graph_entities WHERE kind = ?1 AND name = ?2 LIMIT 1")?;
            Ok(stmt.query_row(params![EntityType::Concept.as_str(), name], |r| r.get(0))?)
        });
        if let Ok(id) = existing {
            return Ok(id);
        }
        let entity = GraphEntity {
            id: 0,
            kind: EntityType::Concept.as_str().to_string(),
            name: name.to_string(),
            file_path: None,
            data: data.clone(),
        };
        self.inner.insert_entity(&entity).map_err(Into::into)
    }

    pub fn insert_edge(
        &self,
        from_id: i64,
        to_id: i64,
        edge_type: EdgeType,
        data: Value,
    ) -> Result<i64> {
        let edge_type_str = edge_type.as_str();

        // Validate domain/range against ontology
        let from_entity = self.get_entity(from_id)?;
        let to_entity = self.get_entity(to_id)?;

        let valid = self.validate_edge(&from_entity.kind, &to_entity.kind, edge_type_str)?;
        if !valid {
            // Find the property definition for the error message
            let prop = self
                .list_properties()?
                .into_iter()
                .find(|p| p.name == edge_type_str);

            if let Some(p) = prop {
                return Err(AtheneumError::EdgeValidation {
                    edge_type: edge_type_str.to_string(),
                    from_kind: from_entity.kind,
                    to_kind: to_entity.kind,
                    domain: p.domain_class,
                    range: p.range_class,
                }
                .into());
            }
        }

        let edge = GraphEdge {
            id: 0,
            from_id,
            to_id,
            edge_type: edge_type_str.to_string(),
            data,
        };
        self.inner.insert_edge(&edge).map_err(Into::into)
    }

    pub fn events_performed_by(&self, agent_id: i64) -> Result<Vec<GraphEntity>> {
        let mut events = Vec::new();

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

    pub fn tasks_assigned_to(&self, agent_id: i64) -> Result<Vec<GraphEntity>> {
        let mut tasks = Vec::new();

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

    pub fn causal_chain(&self, event_id: i64) -> Result<Vec<GraphEntity>> {
        let mut chain = Vec::new();
        let mut current = Some(event_id);
        let mut visited = std::collections::HashSet::new();

        while let Some(id) = current {
            if !visited.insert(id) {
                break;
            }

            if let Ok(entity) = self.get_entity(id) {
                chain.push(entity);

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

    pub(super) fn find_entity_id_by_data(
        &self,
        kind: &str,
        key: &str,
        value: &str,
    ) -> Result<Option<i64>> {
        self.with_raw_connection(|conn| {
            Ok(conn
                .query_row(
                    "SELECT id FROM graph_entities WHERE kind = ?1 AND json_extract(data, ?2) = ?3 LIMIT 1",
                    rusqlite::params![kind, format!("$.{}", key), value],
                    |r| r.get(0),
                )
                .ok())
        })
    }

    pub(super) fn find_entity_id_by_kind_and_name(
        &self,
        kind: &str,
        name: &str,
    ) -> Result<Option<i64>> {
        self.with_raw_connection(|conn| {
            Ok(conn
                .query_row(
                    "SELECT id FROM graph_entities WHERE kind = ?1 AND name = ?2 LIMIT 1",
                    rusqlite::params![kind, name],
                    |r| r.get(0),
                )
                .ok())
        })
    }

    fn update_entity_data(&self, id: i64, data: &Value) -> Result<()> {
        with_graph_conn(&self.inner, |conn| {
            conn.execute(
                "UPDATE graph_entities SET data = ?1 WHERE id = ?2",
                params![serde_json::to_string(data)?, id],
            )?;
            Ok(())
        })
    }
}

fn parse_frontmatter(content: &str) -> Result<(Value, &str)> {
    let rest = content
        .strip_prefix("---\r\n")
        .or_else(|| content.strip_prefix("---\n"))
        .ok_or_else(|| anyhow::anyhow!("No frontmatter start marker"))?;

    let mut offset = 0;
    let mut closing = None;
    for line in rest.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed == "---" {
            closing = Some((offset, offset + line.len()));
            break;
        }
        offset += line.len();
    }

    let (frontmatter_end, body_start) =
        closing.ok_or_else(|| anyhow::anyhow!("No frontmatter end marker"))?;
    let frontmatter_text = &rest[..frontmatter_end];

    let mut map = serde_json::Map::new();
    for line in frontmatter_text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let rest = rest.trim();
        let value = if rest.starts_with('[') && rest.ends_with(']') {
            let inner = &rest[1..rest.len() - 1];
            let items: Vec<Value> = inner
                .split(',')
                .map(|s| Value::String(s.trim().trim_matches('"').trim_matches('\'').to_string()))
                .collect();
            Value::Array(items)
        } else if rest == "true" {
            Value::Bool(true)
        } else if rest == "false" {
            Value::Bool(false)
        } else if let Ok(n) = rest.parse::<i64>() {
            Value::Number(n.into())
        } else if let Ok(n) = rest.parse::<f64>() {
            serde_json::Number::from_f64(n)
                .map(Value::Number)
                .unwrap_or_else(|| Value::String(rest.to_string()))
        } else {
            Value::String(rest.trim_matches('"').trim_matches('\'').to_string())
        };
        map.insert(key.to_string(), value);
    }

    let body = &rest[body_start..];

    Ok((Value::Object(map), body))
}

pub(super) fn with_graph_conn<F, R>(graph: &SqliteGraph, f: F) -> Result<R>
where
    F: FnOnce(&rusqlite::Connection) -> Result<R>,
{
    if let Some(direct) = graph.pool.direct_connection() {
        f(direct)
    } else {
        let pooled = graph
            .pool
            .get()
            .map_err(|e| anyhow::anyhow!("Failed to get connection: {}", e))?;
        f(&pooled)
    }
}

fn get_incoming_edges(graph: &SqliteGraph, entity_id: i64) -> Result<Vec<GraphEdge>> {
    with_graph_conn(graph, |conn| {
        let mut stmt = conn.prepare_cached(
            "SELECT id, from_id, to_id, edge_type, data FROM graph_edges WHERE to_id=?1 ORDER BY id",
        )?;
        let rows = stmt.query_map(params![entity_id], |row| {
            Ok(GraphEdge {
                id: row.get(0)?,
                from_id: row.get(1)?,
                to_id: row.get(2)?,
                edge_type: row.get(3)?,
                data: serde_json::from_str(row.get_ref(4)?.as_str()?)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
            })
        })?;
        let mut edges = Vec::new();
        for row in rows {
            edges.push(row?);
        }
        Ok(edges)
    })
}

fn get_outgoing_edges(graph: &SqliteGraph, entity_id: i64) -> Result<Vec<GraphEdge>> {
    with_graph_conn(graph, |conn| {
        let mut stmt = conn.prepare_cached(
            "SELECT id, from_id, to_id, edge_type, data FROM graph_edges WHERE from_id=?1 ORDER BY id",
        )?;
        let rows = stmt.query_map(params![entity_id], |row| {
            Ok(GraphEdge {
                id: row.get(0)?,
                from_id: row.get(1)?,
                to_id: row.get(2)?,
                edge_type: row.get(3)?,
                data: serde_json::from_str(row.get_ref(4)?.as_str()?)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
            })
        })?;
        let mut edges = Vec::new();
        for row in rows {
            edges.push(row?);
        }
        Ok(edges)
    })
}

#[cfg(test)]
mod tests {
    use crate::graph::{AtheneumGraph, EdgeType, EntityType};

    #[test]
    fn insert_edge_rejects_invalid_domain() {
        let graph = AtheneumGraph::open_in_memory().expect("in-memory graph");
        graph.seed_standard_ontology().expect("seed ontology");

        // AssignedTo requires domain=Task, range=Agent
        // Creating a Knowledge entity as 'from' should fail validation
        let agent_id = graph
            .insert_agent("test-agent", serde_json::json!({}))
            .expect("insert agent");

        // Insert a raw entity with kind Knowledge (not Task)
        let knowledge = sqlitegraph::GraphEntity {
            id: 0,
            kind: EntityType::Knowledge.as_str().to_string(),
            name: "some-knowledge".to_string(),
            file_path: None,
            data: serde_json::json!({}),
        };
        let knowledge_id = graph
            .inner
            .insert_entity(&knowledge)
            .expect("insert knowledge");

        let result = graph.insert_edge(
            knowledge_id,
            agent_id,
            EdgeType::AssignedTo,
            serde_json::json!({}),
        );

        assert!(
            result.is_err(),
            "insert_edge should reject AssignedTo from Knowledge to Agent -- domain must be Task"
        );
    }

    #[test]
    fn insert_edge_accepts_valid_domain_range() {
        let graph = AtheneumGraph::open_in_memory().expect("in-memory graph");
        graph.seed_standard_ontology().expect("seed ontology");

        let task_id = graph
            .insert_task("test-task", serde_json::json!({}))
            .expect("insert task");
        let agent_id = graph
            .insert_agent("test-agent", serde_json::json!({}))
            .expect("insert agent");

        let result = graph.insert_edge(
            task_id,
            agent_id,
            EdgeType::AssignedTo,
            serde_json::json!({}),
        );

        assert!(
            result.is_ok(),
            "insert_edge should accept AssignedTo from Task to Agent"
        );
    }

    #[test]
    fn insert_edge_accepts_any_domain() {
        let graph = AtheneumGraph::open_in_memory().expect("in-memory graph");
        graph.seed_standard_ontology().expect("seed ontology");

        // CausedBy has domain=ANY, range=ANY
        let knowledge_id = graph
            .store_memory("test-key", "test content", "memory", 1.0, None)
            .expect("insert memory");
        let agent_id = graph
            .insert_agent("test-agent", serde_json::json!({}))
            .expect("insert agent");

        let result = graph.insert_edge(
            knowledge_id,
            agent_id,
            EdgeType::CausedBy,
            serde_json::json!({}),
        );

        assert!(
            result.is_ok(),
            "insert_edge should accept CausedBy from any entity kind to any entity kind"
        );
    }
}
