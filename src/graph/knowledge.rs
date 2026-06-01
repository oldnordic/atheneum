use anyhow::Result;
use chrono::Utc;
use rusqlite::params;
use serde_json::{json, Value};
use sqlitegraph::GraphEntity;

use super::{AtheneumGraph, EntityType};

impl AtheneumGraph {
    pub fn query_knowledge(&self, target: &str) -> Result<Value> {
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

    pub fn query_knowledge_in_project(
        &self,
        target: &str,
        project_id: Option<&str>,
    ) -> Result<Value> {
        let Some(pid) = project_id else {
            return self.query_knowledge(target);
        };

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
