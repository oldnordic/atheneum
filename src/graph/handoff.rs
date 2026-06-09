use anyhow::Result;
use chrono::Utc;
use rusqlite::params;
use serde_json::{json, Value};
use sqlitegraph::GraphEntity;

use super::cache::CacheDomain;
use super::hashing::content_hash_excluding;
use super::{AtheneumGraph, EntityType, HandoffPreview};

impl AtheneumGraph {
    pub fn preview_handoff(
        &self,
        from_agent: &str,
        to_agent: &str,
        project_id: Option<&str>,
        manifest: Value,
        k: usize,
        min_score: f32,
    ) -> Result<HandoffPreview> {
        let proposed_name = format!("{} -> {}", from_agent, to_agent);
        let mut proposed_data = json!({
            "from_agent": from_agent,
            "to_agent": to_agent,
            "manifest": manifest,
            "claimed": false,
        });
        if let (Some(pid), Some(obj)) = (project_id, proposed_data.as_object_mut()) {
            obj.insert("project_id".to_string(), Value::String(pid.to_string()));
        }

        let content_hash = content_hash_excluding(
            &proposed_data,
            &["created_at", "claimed", "claimed_at", "content_hash"],
        )?;
        if let Some(obj) = proposed_data.as_object_mut() {
            obj.insert(
                "content_hash".to_string(),
                Value::String(content_hash.clone()),
            );
        }

        let exact_matches = self.query_handoffs_between(from_agent, to_agent, project_id)?;
        let candidate_matches = self.preview_entity_candidates(
            &format!("{from_agent} {to_agent} {manifest}"),
            k,
            project_id,
            Some(EntityType::Handoff.as_str()),
            min_score,
        )?;
        let candidate_matches =
            self.merge_exact_match_candidates(candidate_matches, &exact_matches, k);

        Ok(HandoffPreview {
            proposed_name,
            proposed_data,
            content_hash,
            exact_matches,
            candidate_matches,
        })
    }

    pub fn store_handoff(&self, from_agent: &str, to_agent: &str, manifest: Value) -> Result<i64> {
        let name = format!("{} -> {}", from_agent, to_agent);

        let mut data = json!({
            "from_agent": from_agent,
            "to_agent": to_agent,
            "manifest": manifest,
            "created_at": Utc::now().to_rfc3339(),
            "claimed": false,
        });
        let content_hash = content_hash_excluding(
            &data,
            &["created_at", "claimed", "claimed_at", "content_hash"],
        )?;
        if let Some(obj) = data.as_object_mut() {
            obj.insert("content_hash".to_string(), Value::String(content_hash));
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
                "handoff_id": handoff_id
            }),
        )?;

        let _ = self.insert_agent(from_agent, json!({}));
        let _ = self.insert_agent(to_agent, json!({}));

        self.runtime.record_knowledge_write();
        self.runtime.bump_generation(CacheDomain::Knowledge);
        Ok(handoff_id)
    }

    pub fn get_pending_handoff(&self, agent: &str) -> Result<Option<GraphEntity>> {
        super::with_graph_conn(&self.inner, |conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT id, kind, name, file_path, data FROM graph_entities
                 WHERE kind=?1 AND json_extract(data, '$.to_agent') = ?2
                 AND NOT json_extract(data, '$.claimed')
                 ORDER BY id DESC LIMIT 1",
            )?;

            let mut rows = stmt.query_map(params![EntityType::Handoff.as_str(), agent], |row| {
                Ok(GraphEntity {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    name: row.get(2)?,
                    file_path: row.get(3)?,
                    data: serde_json::from_str(row.get_ref(4)?.as_str()?)
                        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
                })
            })?;

            match rows.next() {
                Some(Ok(entity)) => Ok(Some(entity)),
                Some(Err(e)) => Err(anyhow::anyhow!("Failed to read row: {}", e)),
                None => Ok(None),
            }
        })
    }

    pub fn mark_handoff_claimed(&self, handoff_id: i64) -> Result<()> {
        let entity = self.get_entity(handoff_id)?;

        let mut data = entity.data;
        if let Some(obj) = data.as_object_mut() {
            obj.insert("claimed".to_string(), Value::Bool(true));
            obj.insert(
                "claimed_at".to_string(),
                Value::String(Utc::now().to_rfc3339()),
            );
        }

        super::with_graph_conn(&self.inner, |conn| {
            conn.execute(
                "UPDATE graph_entities SET data = ?1 WHERE id = ?2",
                params![serde_json::to_string(&data)?, handoff_id],
            )?;
            Ok(())
        })?;
        self.runtime.record_knowledge_write();
        self.runtime.bump_generation(CacheDomain::Knowledge);
        Ok(())
    }

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
        let content_hash = content_hash_excluding(
            &data,
            &["created_at", "claimed", "claimed_at", "content_hash"],
        )?;
        if let Some(obj) = data.as_object_mut() {
            obj.insert("content_hash".to_string(), Value::String(content_hash));
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

        self.runtime.record_knowledge_write();
        self.runtime.bump_generation(CacheDomain::Knowledge);
        Ok(handoff_id)
    }

    pub fn get_pending_handoff_in_project(
        &self,
        agent: &str,
        project_id: Option<&str>,
    ) -> Result<Option<GraphEntity>> {
        let Some(pid) = project_id else {
            return self.get_pending_handoff(agent);
        };

        super::with_graph_conn(&self.inner, |conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT id, kind, name, file_path, data FROM graph_entities
                 WHERE kind=?1
                   AND json_extract(data, '$.to_agent') = ?2
                   AND json_extract(data, '$.project_id') = ?3
                   AND NOT json_extract(data, '$.claimed')
                 ORDER BY id DESC LIMIT 1",
            )?;

            let mut rows =
                stmt.query_map(params![EntityType::Handoff.as_str(), agent, pid], |row| {
                    Ok(GraphEntity {
                        id: row.get(0)?,
                        kind: row.get(1)?,
                        name: row.get(2)?,
                        file_path: row.get(3)?,
                        data: serde_json::from_str(row.get_ref(4)?.as_str()?)
                            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
                    })
                })?;

            match rows.next() {
                Some(Ok(entity)) => Ok(Some(entity)),
                Some(Err(e)) => Err(anyhow::anyhow!("Failed to read row: {}", e)),
                None => Ok(None),
            }
        })
    }

    fn query_handoffs_between(
        &self,
        from_agent: &str,
        to_agent: &str,
        project_id: Option<&str>,
    ) -> Result<Vec<GraphEntity>> {
        super::with_graph_conn(&self.inner, |conn| {
            let mut out = Vec::new();
            match project_id {
                Some(pid) => {
                    let mut stmt = conn.prepare_cached(
                        "SELECT id, kind, name, file_path, data FROM graph_entities
                         WHERE kind=?1
                           AND json_extract(data, '$.from_agent') = ?2
                           AND json_extract(data, '$.to_agent') = ?3
                           AND json_extract(data, '$.project_id') = ?4",
                    )?;
                    let rows = stmt.query_map(
                        params![EntityType::Handoff.as_str(), from_agent, to_agent, pid],
                        |row| {
                            Ok(GraphEntity {
                                id: row.get(0)?,
                                kind: row.get(1)?,
                                name: row.get(2)?,
                                file_path: row.get(3)?,
                                data: serde_json::from_str(row.get_ref(4)?.as_str()?).map_err(
                                    |e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)),
                                )?,
                            })
                        },
                    )?;
                    for row in rows {
                        out.push(row?);
                    }
                }
                None => {
                    let mut stmt = conn.prepare_cached(
                        "SELECT id, kind, name, file_path, data FROM graph_entities
                         WHERE kind=?1
                           AND json_extract(data, '$.from_agent') = ?2
                           AND json_extract(data, '$.to_agent') = ?3
                           AND json_extract(data, '$.project_id') IS NULL",
                    )?;
                    let rows = stmt.query_map(
                        params![EntityType::Handoff.as_str(), from_agent, to_agent],
                        |row| {
                            Ok(GraphEntity {
                                id: row.get(0)?,
                                kind: row.get(1)?,
                                name: row.get(2)?,
                                file_path: row.get(3)?,
                                data: serde_json::from_str(row.get_ref(4)?.as_str()?).map_err(
                                    |e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)),
                                )?,
                            })
                        },
                    )?;
                    for row in rows {
                        out.push(row?);
                    }
                }
            }
            Ok(out)
        })
    }
}
