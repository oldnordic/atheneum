use anyhow::Result;
use chrono::Utc;
use rusqlite::params;
use serde_json::{json, Value};
use sqlitegraph::GraphEntity;

use super::cache::CacheDomain;
use super::hashing::content_hash_excluding;
use super::{AtheneumGraph, DiscoveryPreview, EdgeType, EntityType, ProvenanceData};

impl AtheneumGraph {
    pub fn preview_discovery(
        &self,
        agent: &str,
        discovery_type: &str,
        target: &str,
        mut metadata: Value,
        k: usize,
        min_score: f32,
    ) -> Result<DiscoveryPreview> {
        if let Some(obj) = metadata.as_object_mut() {
            obj.insert("agent".to_string(), Value::String(agent.to_string()));
            obj.insert(
                "discovery_type".to_string(),
                Value::String(discovery_type.to_string()),
            );
            obj.insert("target".to_string(), Value::String(target.to_string()));
        }

        let content_hash =
            content_hash_excluding(&metadata, &["timestamp", "sql_id", "content_hash"])?;
        if let Some(obj) = metadata.as_object_mut() {
            obj.insert(
                "content_hash".to_string(),
                Value::String(content_hash.clone()),
            );
        }

        let project_id = metadata
            .get("project_id")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let exact_matches = self.query_discoveries_in_project(target, project_id.as_deref())?;
        let candidate_matches = self.preview_entity_candidates(
            target,
            k,
            project_id.as_deref(),
            Some(EntityType::Discovery.as_str()),
            min_score,
        )?;
        let candidate_matches =
            self.merge_exact_match_candidates(candidate_matches, &exact_matches, k);

        let disambiguation = self
            .resolve(
                target,
                0.3,
                project_id.as_deref(),
                Some(EntityType::Discovery.as_str()),
            )
            .ok();

        Ok(DiscoveryPreview {
            proposed_name: format!("{}: {}", agent, target),
            proposed_data: metadata,
            content_hash,
            exact_matches,
            candidate_matches,
            disambiguation,
        })
    }

    pub fn store_discovery(
        &self,
        agent: &str,
        discovery_type: &str,
        target: &str,
        mut metadata: Value,
    ) -> Result<i64> {
        let name = format!("{}: {}", agent, target);

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

        let content_hash =
            content_hash_excluding(&metadata, &["timestamp", "sql_id", "content_hash"])?;

        let agent_s = agent.to_string();
        let discovery_type_s = discovery_type.to_string();
        let target_s = target.to_string();
        let project_id_s = metadata
            .get("project_id")
            .and_then(|v| v.as_str())
            .map(String::from);
        let metadata_str = super::json_to_string(&metadata)?;
        let created_at = Utc::now().to_rfc3339();
        let sql_id = self.with_raw_connection(|conn| {
            conn.execute(
                "INSERT INTO discoveries
                    (agent_name, discovery_type, target, project_id, metadata, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    agent_s,
                    discovery_type_s,
                    target_s,
                    project_id_s,
                    metadata_str,
                    created_at
                ],
            )?;
            Ok(conn.last_insert_rowid())
        })?;

        if let Some(obj) = metadata.as_object_mut() {
            obj.insert("sql_id".to_string(), Value::Number(sql_id.into()));
            obj.insert("content_hash".to_string(), Value::String(content_hash));
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

        // Auto-index this discovery for semantic search.
        let mut indexed_entity = entity;
        indexed_entity.id = discovery_id;
        if let Err(e) = self.add_entity_to_search_index(&indexed_entity) {
            // Log but don't fail the store — indexing is a side-effect.
            eprintln!("[atheneum] auto-index warning: {}", e);
        }

        let event_id = self.insert_event(
            "discovery-stored",
            json!({
                "agent": agent,
                "target": target,
                "discovery_type": discovery_type,
                "discovery_id": discovery_id
            }),
        )?;

        let agent_id =
            match self.find_entity_id_by_kind_and_name(EntityType::Agent.as_str(), agent)? {
                Some(id) => id,
                None => self.insert_agent(agent, json!({}))?,
            };

        self.insert_edge(
            event_id,
            agent_id,
            EdgeType::PerformedBy,
            json!({"provenance": ProvenanceData::new("store_discovery").to_value()}),
        )?;

        self.runtime.record_knowledge_write();
        self.runtime.bump_generation(CacheDomain::Knowledge);
        Ok(discovery_id)
    }

    pub fn query_discoveries(&self, target: &str) -> Result<Vec<GraphEntity>> {
        super::with_graph_conn(&self.inner, |conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT id, kind, name, file_path, data FROM graph_entities
                 WHERE kind=?1 AND json_extract(data, '$.target') = ?2",
            )?;

            let rows = stmt.query_map(params![EntityType::Discovery.as_str(), target], |row| {
                Ok(GraphEntity {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    name: row.get(2)?,
                    file_path: row.get(3)?,
                    data: serde_json::from_str(row.get_ref(4)?.as_str()?)
                        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
                })
            })?;

            let mut discoveries = Vec::new();
            for row in rows {
                discoveries.push(row?);
            }
            Ok(discoveries)
        })
    }

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

    /// Return the N most recent discoveries for a project (no target required).
    /// Used by SubagentStart hook to push project context into initial LLM context.
    pub fn recent_project_context(&self, project: &str, limit: i64) -> Result<Vec<GraphEntity>> {
        super::with_graph_conn(&self.inner, |conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT id, kind, name, file_path, data FROM graph_entities
                 WHERE kind = ?1
                   AND (json_extract(data, '$.project_id') = ?2
                        OR json_extract(data, '$.project') = ?2)
                 ORDER BY id DESC LIMIT ?3",
            )?;
            let rows = stmt.query_map(
                params![EntityType::Discovery.as_str(), project, limit],
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
            )?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }

    pub fn query_discoveries_in_project(
        &self,
        target: &str,
        project_id: Option<&str>,
    ) -> Result<Vec<GraphEntity>> {
        let Some(pid) = project_id else {
            return self.query_discoveries(target);
        };

        super::with_graph_conn(&self.inner, |conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT id, kind, name, file_path, data FROM graph_entities
                 WHERE kind=?1
                   AND json_extract(data, '$.target') = ?2
                   AND json_extract(data, '$.project_id') = ?3",
            )?;

            let rows = stmt.query_map(
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
            )?;

            let mut discoveries = Vec::new();
            for row in rows {
                discoveries.push(row?);
            }
            Ok(discoveries)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_graph() -> AtheneumGraph {
        AtheneumGraph::open_in_memory().unwrap()
    }

    fn make_graph_with_discovery() -> AtheneumGraph {
        let graph = AtheneumGraph::open_in_memory().unwrap();
        graph
            .store_discovery_in_project(
                "test-agent",
                "bug_found",
                "http_handler",
                Some("test"),
                serde_json::json!({"detail": "connection pool leak"}),
            )
            .unwrap();
        graph
    }

    #[test]
    fn preview_discovery_includes_disambiguation() {
        let graph = make_graph_with_discovery();
        let preview = graph
            .preview_discovery(
                "test-agent",
                "bug_found",
                "http_handler",
                serde_json::json!({"detail": "connection pool leak"}),
                5,
                0.0,
            )
            .unwrap();
        // Disambiguation should be populated since we have a matching entity
        assert!(
            preview.disambiguation.is_some(),
            "preview should include disambiguation analysis"
        );
        let disamb = preview.disambiguation.unwrap();
        assert!(
            !disamb.candidates.is_empty(),
            "disambiguation should have candidates"
        );
    }

    #[test]
    fn preview_discovery_no_match_still_has_disambiguation() {
        let graph = make_graph();
        let preview = graph
            .preview_discovery(
                "test-agent",
                "bug_found",
                "http_handler",
                serde_json::json!({"detail": "something new"}),
                5,
                0.0,
            )
            .unwrap();
        // Even with no entities, resolve() returns an empty DisambiguationResult
        assert!(
            preview.disambiguation.is_some(),
            "preview should always include disambiguation (even if empty)"
        );
        let disamb = preview.disambiguation.unwrap();
        assert!(!disamb.is_resolved(), "empty graph should not resolve");
    }
}
