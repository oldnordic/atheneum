use anyhow::Result;
use chrono::Utc;
use rusqlite::params;
use serde_json::{json, Value};
use sqlitegraph::GraphEntity;

use super::{AtheneumGraph, EdgeType, EntityType};

impl AtheneumGraph {
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
            json!({"provenance": {"actor": "atheneum", "method": "store_discovery"}}),
        )?;

        Ok(discovery_id)
    }

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
}
