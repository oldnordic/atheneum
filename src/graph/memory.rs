//! Memory-domain graph methods.
//!
//! `Memory` entities hold stable facts (user preferences, project conventions).
//! Distinct from `Knowledge` (merged discoveries) and `WikiPage` (documents).

use anyhow::Result;
use chrono::Utc;
use rusqlite::params;
use serde_json::{json, Value};
use sqlitegraph::GraphEntity;

use super::{AtheneumGraph, EntityType};

impl AtheneumGraph {
    /// Store a memory entry.
    ///
    /// Scope: `"user"` | `"project"` | `"agent"`
    pub fn store_memory(
        &self,
        key: &str,
        content: &str,
        scope: &str,
        confidence: f64,
        project_id: Option<&str>,
    ) -> Result<i64> {
        let created_at = Utc::now().to_rfc3339();

        let sql_id = self.with_raw_connection(|conn| {
            conn.execute(
                "INSERT INTO memory_entries
                    (key, scope, content, confidence, project_id, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![key, scope, content, confidence, project_id, created_at],
            )?;
            Ok(conn.last_insert_rowid())
        })?;

        let mut data = json!({
            "sql_id": sql_id,
            "key": key,
            "scope": scope,
            "content": content,
            "confidence": confidence,
            "created_at": created_at,
        });
        if let (Some(pid), Some(obj)) = (project_id, data.as_object_mut()) {
            obj.insert("project_id".to_string(), Value::String(pid.to_string()));
        }
        let entity = GraphEntity {
            id: 0,
            kind: EntityType::Memory.as_str().to_string(),
            name: key.to_string(),
            file_path: None,
            data,
        };
        let memory_id = self
            .inner
            .insert_entity(&entity)
            .map_err(|e| anyhow::anyhow!("Failed to insert Memory: {}", e))?;

        let indexed = GraphEntity {
            id: memory_id,
            ..entity
        };
        if let Err(e) = self.add_entity_to_search_index(&indexed) {
            eprintln!("[atheneum] memory auto-index warning: {}", e);
        }

        Ok(memory_id)
    }

    /// Query memory by key and optional scope/project.
    pub fn query_memory(
        &self,
        key: &str,
        scope: Option<&str>,
        project_id: Option<&str>,
    ) -> Result<Vec<GraphEntity>> {
        super::with_graph_conn(&self.inner, |conn| {
            let mut out = Vec::new();
            match (scope, project_id) {
                (Some(s), Some(pid)) => {
                    let mut stmt = conn.prepare(
                        "SELECT id, kind, name, file_path, data FROM graph_entities
                         WHERE kind = ?1 AND name = ?2
                           AND json_extract(data, '$.scope') = ?3
                           AND json_extract(data, '$.project_id') = ?4",
                    )?;
                    let mut rows = stmt.query_map(
                        params![EntityType::Memory.as_str(), key, s, pid],
                        row_to_entity,
                    )?;
                    while let Some(row) = rows.next() {
                        out.push(row?);
                    }
                }
                (Some(s), None) => {
                    let mut stmt = conn.prepare(
                        "SELECT id, kind, name, file_path, data FROM graph_entities
                         WHERE kind = ?1 AND name = ?2
                           AND json_extract(data, '$.scope') = ?3",
                    )?;
                    let mut rows = stmt
                        .query_map(params![EntityType::Memory.as_str(), key, s], row_to_entity)?;
                    while let Some(row) = rows.next() {
                        out.push(row?);
                    }
                }
                (None, Some(pid)) => {
                    let mut stmt = conn.prepare(
                        "SELECT id, kind, name, file_path, data FROM graph_entities
                         WHERE kind = ?1 AND name = ?2
                           AND json_extract(data, '$.project_id') = ?3",
                    )?;
                    let mut rows = stmt.query_map(
                        params![EntityType::Memory.as_str(), key, pid],
                        row_to_entity,
                    )?;
                    while let Some(row) = rows.next() {
                        out.push(row?);
                    }
                }
                (None, None) => {
                    let mut stmt = conn.prepare(
                        "SELECT id, kind, name, file_path, data FROM graph_entities
                         WHERE kind = ?1 AND name = ?2",
                    )?;
                    let mut rows =
                        stmt.query_map(params![EntityType::Memory.as_str(), key], row_to_entity)?;
                    while let Some(row) = rows.next() {
                        out.push(row?);
                    }
                }
            }
            Ok(out)
        })
    }

    /// List all memory entries for a scope.
    pub fn list_memory(
        &self,
        scope: Option<&str>,
        project_id: Option<&str>,
    ) -> Result<Vec<GraphEntity>> {
        super::with_graph_conn(&self.inner, |conn| {
            let mut out = Vec::new();
            match (scope, project_id) {
                (Some(s), Some(pid)) => {
                    let mut stmt = conn.prepare(
                        "SELECT id, kind, name, file_path, data FROM graph_entities
                         WHERE kind = ?1
                           AND json_extract(data, '$.scope') = ?2
                           AND json_extract(data, '$.project_id') = ?3",
                    )?;
                    let mut rows = stmt
                        .query_map(params![EntityType::Memory.as_str(), s, pid], row_to_entity)?;
                    while let Some(row) = rows.next() {
                        out.push(row?);
                    }
                }
                (Some(s), None) => {
                    let mut stmt = conn.prepare(
                        "SELECT id, kind, name, file_path, data FROM graph_entities
                         WHERE kind = ?1
                           AND json_extract(data, '$.scope') = ?2",
                    )?;
                    let mut rows =
                        stmt.query_map(params![EntityType::Memory.as_str(), s], row_to_entity)?;
                    while let Some(row) = rows.next() {
                        out.push(row?);
                    }
                }
                (None, Some(pid)) => {
                    let mut stmt = conn.prepare(
                        "SELECT id, kind, name, file_path, data FROM graph_entities
                         WHERE kind = ?1
                           AND json_extract(data, '$.project_id') = ?2",
                    )?;
                    let mut rows =
                        stmt.query_map(params![EntityType::Memory.as_str(), pid], row_to_entity)?;
                    while let Some(row) = rows.next() {
                        out.push(row?);
                    }
                }
                (None, None) => {
                    let mut stmt = conn.prepare(
                        "SELECT id, kind, name, file_path, data FROM graph_entities
                         WHERE kind = ?1",
                    )?;
                    let mut rows =
                        stmt.query_map(params![EntityType::Memory.as_str()], row_to_entity)?;
                    while let Some(row) = rows.next() {
                        out.push(row?);
                    }
                }
            }
            Ok(out)
        })
    }
}

fn row_to_entity(r: &rusqlite::Row) -> rusqlite::Result<GraphEntity> {
    Ok(GraphEntity {
        id: r.get(0)?,
        kind: r.get(1)?,
        name: r.get(2)?,
        file_path: r.get(3)?,
        data: {
            let s: String = r.get(4)?;
            serde_json::from_str(&s).unwrap_or(Value::Null)
        },
    })
}
