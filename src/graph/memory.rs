//! Memory-domain graph methods.
//!
//! `Memory` entities hold stable facts (user preferences, project conventions).
//! Distinct from `Knowledge` (merged discoveries) and `WikiPage` (documents).

use anyhow::Result;
use chrono::Utc;
use rusqlite::params;
use serde_json::{json, Value};
use sqlitegraph::GraphEntity;

use super::cache::{CacheDomain, QueryCacheKey, QueryCacheValue};
use super::hashing::json_sha256_hex;
use super::{AtheneumGraph, EntityType, MemoryPreview};

impl AtheneumGraph {
    #[allow(
        clippy::too_many_arguments,
        reason = "Public preview API intentionally mirrors store_memory inputs plus ranking controls"
    )]
    pub fn preview_memory(
        &self,
        key: &str,
        content: &str,
        scope: &str,
        confidence: f64,
        project_id: Option<&str>,
        k: usize,
        min_score: f32,
    ) -> Result<MemoryPreview> {
        let mut proposed_data = json!({
            "key": key,
            "scope": scope,
            "content": content,
            "confidence": confidence,
        });
        if let (Some(pid), Some(obj)) = (project_id, proposed_data.as_object_mut()) {
            obj.insert("project_id".to_string(), Value::String(pid.to_string()));
        }

        let content_hash = memory_content_hash(&proposed_data)?;
        if let Some(obj) = proposed_data.as_object_mut() {
            obj.insert(
                "content_hash".to_string(),
                Value::String(content_hash.clone()),
            );
        }

        let exact_matches = self.query_memory(key, Some(scope), project_id)?;
        let candidate_matches = self.preview_entity_candidates(
            &format!("{key} {content}"),
            k,
            project_id,
            Some(EntityType::Memory.as_str()),
            min_score,
        )?;
        let candidate_matches =
            self.merge_exact_match_candidates(candidate_matches, &exact_matches, k);

        Ok(MemoryPreview {
            proposed_key: key.to_string(),
            proposed_data,
            content_hash,
            exact_matches,
            candidate_matches,
        })
    }

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
        let now = Utc::now().to_rfc3339();

        // Check for existing memory by composite key (key, scope, project_id).
        let existing_id = super::with_graph_conn(&self.inner, |conn| {
            let sql = if project_id.is_some() {
                "SELECT id FROM graph_entities
                 WHERE kind = ?1 AND name = ?2
                   AND json_extract(data, '$.scope') = ?3
                   AND json_extract(data, '$.project_id') = ?4"
            } else {
                "SELECT id FROM graph_entities
                 WHERE kind = ?1 AND name = ?2
                   AND json_extract(data, '$.scope') = ?3
                   AND json_extract(data, '$.project_id') IS NULL"
            };
            let mut stmt = conn.prepare(sql)?;
            let id: Option<i64> = if let Some(pid) = project_id {
                stmt.query_row(params![EntityType::Memory.as_str(), key, scope, pid], |r| {
                    r.get(0)
                })
                .ok()
            } else {
                stmt.query_row(params![EntityType::Memory.as_str(), key, scope], |r| {
                    r.get(0)
                })
                .ok()
            };
            Ok(id)
        })?;

        if let Some(memory_id) = existing_id {
            // Preserve original created_at and sql_id from existing entity.
            let entity = self.get_entity(memory_id)?;
            let created_at = entity
                .data
                .get("created_at")
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or_else(|| now.clone());
            let sql_id = entity
                .data
                .get("sql_id")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);

            let sql_id = self.with_raw_connection(|conn| {
                let updated = conn.execute(
                    "UPDATE memory_entries
                     SET content = ?1, confidence = ?2, updated_at = ?3
                     WHERE key = ?4 AND scope = ?5
                       AND COALESCE(project_id, '') = COALESCE(?6, '')",
                    params![content, confidence, &now, key, scope, project_id],
                )?;
                if updated > 0 {
                    return Ok(sql_id);
                }

                conn.execute(
                    "INSERT INTO memory_entries
                        (key, scope, content, confidence, project_id, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        key,
                        scope,
                        content,
                        confidence,
                        project_id,
                        &created_at,
                        &now
                    ],
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
                "updated_at": now,
            });
            if let (Some(pid), Some(obj)) = (project_id, data.as_object_mut()) {
                obj.insert("project_id".to_string(), Value::String(pid.to_string()));
            }

            self.update_entity_data(memory_id, &data)?;
            let indexed = GraphEntity {
                id: memory_id,
                kind: EntityType::Memory.as_str().to_string(),
                name: key.to_string(),
                file_path: None,
                data: data.clone(),
            };
            if let Err(e) = self.add_entity_to_search_index(&indexed) {
                eprintln!("[atheneum] memory auto-index warning: {}", e);
            }
            self.runtime.record_memory_write();
            self.runtime.bump_generation(CacheDomain::Memory);
            return Ok(memory_id);
        }

        // Insert new SQL row.
        let sql_id = self.with_raw_connection(|conn| {
            conn.execute(
                "INSERT INTO memory_entries
                    (key, scope, content, confidence, project_id, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                params![key, scope, content, confidence, project_id, &now],
            )?;
            Ok(conn.last_insert_rowid())
        })?;

        let mut data = json!({
            "sql_id": sql_id,
            "key": key,
            "scope": scope,
            "content": content,
            "confidence": confidence,
            "created_at": now,
            "updated_at": now,
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

        self.runtime.record_memory_write();
        self.runtime.bump_generation(CacheDomain::Memory);
        Ok(memory_id)
    }

    /// Query memory by key and optional scope/project.
    pub fn query_memory(
        &self,
        key: &str,
        scope: Option<&str>,
        project_id: Option<&str>,
    ) -> Result<Vec<GraphEntity>> {
        self.runtime.record_memory_query();
        let cache_key = QueryCacheKey::QueryMemory {
            key: key.to_string(),
            scope: scope.map(str::to_string),
            project_id: project_id.map(str::to_string),
        };
        if let Some(QueryCacheValue::Entities(entries)) =
            self.runtime.cache_get(&cache_key, CacheDomain::Memory)
        {
            return Ok(entries);
        }

        let out = super::with_graph_conn(&self.inner, |conn| {
            let mut out = Vec::new();
            match (scope, project_id) {
                (Some(s), Some(pid)) => {
                    let mut stmt = conn.prepare(
                        "SELECT id, kind, name, file_path, data FROM graph_entities
                         WHERE kind = ?1 AND name = ?2
                           AND json_extract(data, '$.scope') = ?3
                           AND json_extract(data, '$.project_id') = ?4",
                    )?;
                    let rows = stmt.query_map(
                        params![EntityType::Memory.as_str(), key, s, pid],
                        row_to_entity,
                    )?;
                    for row in rows {
                        out.push(row?);
                    }
                }
                (Some(s), None) => {
                    let mut stmt = conn.prepare(
                        "SELECT id, kind, name, file_path, data FROM graph_entities
                         WHERE kind = ?1 AND name = ?2
                           AND json_extract(data, '$.scope') = ?3",
                    )?;
                    let rows = stmt
                        .query_map(params![EntityType::Memory.as_str(), key, s], row_to_entity)?;
                    for row in rows {
                        out.push(row?);
                    }
                }
                (None, Some(pid)) => {
                    let mut stmt = conn.prepare(
                        "SELECT id, kind, name, file_path, data FROM graph_entities
                         WHERE kind = ?1 AND name = ?2
                           AND json_extract(data, '$.project_id') = ?3",
                    )?;
                    let rows = stmt.query_map(
                        params![EntityType::Memory.as_str(), key, pid],
                        row_to_entity,
                    )?;
                    for row in rows {
                        out.push(row?);
                    }
                }
                (None, None) => {
                    let mut stmt = conn.prepare(
                        "SELECT id, kind, name, file_path, data FROM graph_entities
                         WHERE kind = ?1 AND name = ?2",
                    )?;
                    let rows =
                        stmt.query_map(params![EntityType::Memory.as_str(), key], row_to_entity)?;
                    for row in rows {
                        out.push(row?);
                    }
                }
            }
            Ok(out)
        })?;
        self.runtime.cache_store(
            cache_key,
            CacheDomain::Memory,
            QueryCacheValue::Entities(out.clone()),
        );
        Ok(out)
    }

    /// List all memory entries for a scope.
    pub fn list_memory(
        &self,
        scope: Option<&str>,
        project_id: Option<&str>,
    ) -> Result<Vec<GraphEntity>> {
        self.runtime.record_memory_query();
        let cache_key = QueryCacheKey::ListMemory {
            scope: scope.map(str::to_string),
            project_id: project_id.map(str::to_string),
        };
        if let Some(QueryCacheValue::Entities(entries)) =
            self.runtime.cache_get(&cache_key, CacheDomain::Memory)
        {
            return Ok(entries);
        }

        let out = super::with_graph_conn(&self.inner, |conn| {
            let mut out = Vec::new();
            match (scope, project_id) {
                (Some(s), Some(pid)) => {
                    let mut stmt = conn.prepare(
                        "SELECT id, kind, name, file_path, data FROM graph_entities
                         WHERE kind = ?1
                           AND json_extract(data, '$.scope') = ?2
                           AND json_extract(data, '$.project_id') = ?3",
                    )?;
                    let rows = stmt
                        .query_map(params![EntityType::Memory.as_str(), s, pid], row_to_entity)?;
                    for row in rows {
                        out.push(row?);
                    }
                }
                (Some(s), None) => {
                    let mut stmt = conn.prepare(
                        "SELECT id, kind, name, file_path, data FROM graph_entities
                         WHERE kind = ?1
                           AND json_extract(data, '$.scope') = ?2",
                    )?;
                    let rows =
                        stmt.query_map(params![EntityType::Memory.as_str(), s], row_to_entity)?;
                    for row in rows {
                        out.push(row?);
                    }
                }
                (None, Some(pid)) => {
                    let mut stmt = conn.prepare(
                        "SELECT id, kind, name, file_path, data FROM graph_entities
                         WHERE kind = ?1
                           AND json_extract(data, '$.project_id') = ?2",
                    )?;
                    let rows =
                        stmt.query_map(params![EntityType::Memory.as_str(), pid], row_to_entity)?;
                    for row in rows {
                        out.push(row?);
                    }
                }
                (None, None) => {
                    let mut stmt = conn.prepare(
                        "SELECT id, kind, name, file_path, data FROM graph_entities
                         WHERE kind = ?1",
                    )?;
                    let rows =
                        stmt.query_map(params![EntityType::Memory.as_str()], row_to_entity)?;
                    for row in rows {
                        out.push(row?);
                    }
                }
            }
            Ok(out)
        })?;
        self.runtime.cache_store(
            cache_key,
            CacheDomain::Memory,
            QueryCacheValue::Entities(out.clone()),
        );
        Ok(out)
    }
}

fn memory_content_hash(data: &Value) -> Result<String> {
    let mut normalized = data.clone();
    if let Some(obj) = normalized.as_object_mut() {
        obj.remove("created_at");
        obj.remove("updated_at");
        obj.remove("sql_id");
        obj.remove("content_hash");
    }
    json_sha256_hex(&normalized)
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
