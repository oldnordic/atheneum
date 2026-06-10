use anyhow::Result;
use chrono::Utc;
use rusqlite::params;
use serde_json::{json, Value};
use sqlitegraph::GraphEntity;

use super::cache::{CacheDomain, QueryCacheKey, QueryCacheValue};
use super::{AtheneumGraph, EdgeType, EntityType};

impl AtheneumGraph {
    pub fn consolidate_discoveries(
        &self,
        target: &str,
        project_id: Option<&str>,
    ) -> Result<Option<i64>> {
        let discoveries = match project_id {
            Some(pid) => self.query_discoveries_in_project(target, Some(pid))?,
            None => self.query_discoveries(target)?,
        };

        if discoveries.is_empty() {
            return Ok(None);
        }

        let existing = self.find_entity_id_by_kind_and_name(
            EntityType::Knowledge.as_str(),
            &format!("Knowledge: {}", target),
        )?;

        if let Some(kid) = existing {
            return Ok(Some(kid));
        }

        let source_agents: Vec<String> = discoveries
            .iter()
            .filter_map(|d| {
                d.data
                    .get("agent")
                    .and_then(|a| a.as_str())
                    .map(String::from)
            })
            .collect();

        let discovery_types: Vec<String> = discoveries
            .iter()
            .filter_map(|d| {
                d.data
                    .get("discovery_type")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            })
            .collect();

        let files: Vec<Value> = discoveries
            .iter()
            .filter_map(|d| d.data.get("file").cloned())
            .collect();

        let mut data = json!({
            "target": target,
            "discovery_count": discoveries.len(),
            "source_agents": source_agents,
            "discovery_types": discovery_types,
            "consolidated_at": Utc::now().to_rfc3339(),
        });

        if let Some(obj) = data.as_object_mut() {
            if !files.is_empty() {
                obj.insert("source_files".to_string(), Value::Array(files));
            }
            if let Some(pid) = project_id {
                obj.insert("project_id".to_string(), Value::String(pid.to_string()));
            }
        }

        let entity = GraphEntity {
            id: 0,
            kind: EntityType::Knowledge.as_str().to_string(),
            name: format!("Knowledge: {}", target),
            file_path: None,
            data,
        };

        let knowledge_id = self
            .inner
            .insert_entity(&entity)
            .map_err(|e| anyhow::anyhow!("Failed to insert Knowledge: {}", e))?;

        for discovery in &discoveries {
            let _ = self.insert_edge(
                knowledge_id,
                discovery.id,
                EdgeType::DerivedFrom,
                json!({"consolidation": "auto"}),
            );
        }

        let indexed = GraphEntity {
            id: knowledge_id,
            ..entity
        };
        if let Err(e) = self.add_entity_to_search_index(&indexed) {
            eprintln!("[atheneum] knowledge auto-index warning: {}", e);
        }

        Ok(Some(knowledge_id))
    }

    pub fn consolidation_pass(&self, project_id: Option<&str>) -> Result<Vec<(String, i64)>> {
        self.runtime.record_consolidate_run();
        let targets: Vec<String> = self.with_raw_connection(|conn| {
            let sql = if project_id.is_some() {
                "SELECT DISTINCT json_extract(data, '$.target') FROM graph_entities
                 WHERE kind = 'Discovery' AND json_extract(data, '$.target') IS NOT NULL
                   AND json_extract(data, '$.project_id') = ?1"
            } else {
                "SELECT DISTINCT json_extract(data, '$.target') FROM graph_entities
                 WHERE kind = 'Discovery' AND json_extract(data, '$.target') IS NOT NULL"
            };

            let mut stmt = conn.prepare_cached(sql)?;
            let rows: Vec<String> = if let Some(pid) = project_id {
                stmt.query_map(rusqlite::params![pid], |row| row.get(0))?
                    .filter_map(|r| r.ok())
                    .collect()
            } else {
                stmt.query_map([], |row| row.get(0))?
                    .filter_map(|r| r.ok())
                    .collect()
            };
            Ok(rows)
        })?;

        let mut results = Vec::new();
        for target in &targets {
            if let Some(kid) = self.consolidate_discoveries(target, project_id)? {
                results.push((target.clone(), kid));
            }
        }
        Ok(results)
    }

    pub fn query_knowledge(&self, target: &str, max_tokens: Option<usize>) -> Result<Value> {
        self.runtime.record_knowledge_query();
        let cache_key = QueryCacheKey::QueryKnowledge {
            target: target.to_string(),
            project_id: None,
            max_tokens,
        };
        if let Some(QueryCacheValue::Json(value)) =
            self.runtime.cache_get(&cache_key, CacheDomain::Knowledge)
        {
            return Ok(value);
        }

        let discoveries = self.query_discoveries(target).unwrap_or_default();

        let target_pattern = format!("%{}%", target);
        let (handoffs, total_entities) = super::with_graph_conn(&self.inner, |conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT id, kind, name, file_path, data FROM graph_entities
                 WHERE kind=?1 AND (
                     json_extract(data, '$.manifest.task') LIKE ?2 OR
                     EXISTS (
                         SELECT 1 FROM json_each(data, '$.manifest.files_analyzed')
                         WHERE json_each.value LIKE ?2
                     )
                 )",
            )?;

            let rows = stmt.query_map(
                params![EntityType::Handoff.as_str(), &target_pattern],
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

            let mut handoffs = Vec::new();
            for row in rows {
                handoffs.push(row?);
            }

            let total_entities = conn
                .query_row("SELECT COUNT(*) FROM graph_entities", [], |row| row.get(0))
                .unwrap_or(0);

            Ok((handoffs, total_entities))
        })?;

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

        let mut value = json!({
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
        });
        if let Some(max_tokens) = max_tokens {
            value = truncate_knowledge_value(value, max_tokens);
        }
        self.runtime.cache_store(
            cache_key,
            CacheDomain::Knowledge,
            QueryCacheValue::Json(value.clone()),
        );
        Ok(value)
    }

    pub fn query_knowledge_in_project(
        &self,
        target: &str,
        project_id: Option<&str>,
        max_tokens: Option<usize>,
    ) -> Result<Value> {
        let Some(pid) = project_id else {
            return self.query_knowledge(target, max_tokens);
        };

        self.runtime.record_knowledge_query();
        let cache_key = QueryCacheKey::QueryKnowledge {
            target: target.to_string(),
            project_id: Some(pid.to_string()),
            max_tokens,
        };
        if let Some(QueryCacheValue::Json(value)) =
            self.runtime.cache_get(&cache_key, CacheDomain::Knowledge)
        {
            return Ok(value);
        }

        let discoveries = self
            .query_discoveries_in_project(target, project_id)
            .unwrap_or_default();

        let target_pattern = format!("%{}%", target);
        let (handoffs, total_entities) = super::with_graph_conn(&self.inner, |conn| {
            let mut stmt = conn.prepare_cached(
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
            )?;

            let rows = stmt.query_map(
                params![EntityType::Handoff.as_str(), pid, &target_pattern],
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

            let mut handoffs = Vec::new();
            for row in rows {
                handoffs.push(row?);
            }

            let total_entities = conn
                .query_row("SELECT COUNT(*) FROM graph_entities", [], |row| row.get(0))
                .unwrap_or(0);

            Ok((handoffs, total_entities))
        })?;

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

        let mut value = json!({
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
        });
        if let Some(max_tokens) = max_tokens {
            value = truncate_knowledge_value(value, max_tokens);
        }
        self.runtime.cache_store(
            cache_key,
            CacheDomain::Knowledge,
            QueryCacheValue::Json(value.clone()),
        );
        Ok(value)
    }
}

/// Truncate a knowledge query response to fit within a token budget.
/// Removes entries from `discoveries` and `handoffs` arrays until the
/// estimated token count falls under the limit.
fn truncate_knowledge_value(mut value: Value, max_tokens: usize) -> Value {
    const CHARS_PER_TOKEN: usize = 4;
    let mut estimated = value.to_string().len() / CHARS_PER_TOKEN;
    if estimated <= max_tokens {
        return value;
    }
    value["truncated"] = json!(true);
    while estimated > max_tokens {
        let removed = if let Some(arr) = value.get_mut("discoveries").and_then(|v| v.as_array_mut())
        {
            if !arr.is_empty() {
                arr.pop();
                true
            } else {
                false
            }
        } else {
            false
        };
        if !removed {
            if let Some(arr) = value.get_mut("handoffs").and_then(|v| v.as_array_mut()) {
                if !arr.is_empty() {
                    arr.pop();
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        estimated = value.to_string().len() / CHARS_PER_TOKEN;
    }
    value
}
