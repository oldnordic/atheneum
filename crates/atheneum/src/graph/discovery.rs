use anyhow::Result;
use chrono::Utc;
use rusqlite::params;
use rusqlite::params_from_iter;
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
        let session_id_s = metadata
            .get("session_id")
            .and_then(|v| v.as_str())
            .map(String::from);
        let metadata_str = super::json_to_string(&metadata)?;
        let created_at = Utc::now().to_rfc3339();
        let sql_id = self.with_raw_connection(|conn| {
            conn.execute(
                "INSERT INTO discoveries
                    (agent_name, discovery_type, target, project_id, session_id, metadata, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    agent_s,
                    discovery_type_s,
                    target_s,
                    project_id_s,
                    session_id_s,
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

        // Link this discovery into its session thread (observed_in → Session,
        // caused_by/led_to chain to the prior same-session decision). Best-effort:
        // a missing Session entity or no prior decision is not an error.
        if let Some(sid) = session_id_s.as_deref() {
            if let Err(e) = self.link_discovery_thread(discovery_id, sid, project_id_s.as_deref()) {
                eprintln!("[atheneum] thread-link warning: {}", e);
            }
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

    /// Link a freshly-stored Discovery into its session thread.
    ///
    /// Creates up to three edges (all best-effort, none are errors if absent):
    /// - `observed_in → Session` — to the most-recent Session entity whose
    ///   `data.session_id` matches. Multiple Session entities per session_id
    ///   are tolerated; the highest-id one is used.
    /// - `caused_by → prior` — to the most-recent earlier same-session
    ///   Discovery or ReasoningLog. "Earlier" is by entity `id`, which is
    ///   `AUTOINCREMENT` and therefore reflects insert (chronological) order
    ///   within a session. `graph_entities` has no `created_at` column, so id
    ///   ordering is the only deterministic chronological signal available.
    /// - `led_to` — the inverse edge (`prior → this`) for cheap outward walks.
    ///
    /// If no prior same-session decision exists, this discovery is a thread
    /// root and only the `observed_in` edge (if a Session entity exists) is
    /// created. Per Open Decision #2 (ReasoningLog has no decision-tag field),
    /// the chain is gated to entities that already carry a `session_id` in
    /// their data; ReasoningLogs are chain *anchors* but are never auto-linked
    /// from `store_discovery` because ingest has no decision signal.
    fn link_discovery_thread(
        &self,
        discovery_id: i64,
        session_id: &str,
        project_id: Option<&str>,
    ) -> Result<()> {
        const REASONING_LOG_KIND: &str = "ReasoningLog";

        // Resolve the Session entity id and the prior chain-anchor id in one
        // connection trip. Each returns Option<i64> (None = no such entity).
        let (session_entity_id, prior_id) = self.with_raw_connection(|conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT id FROM graph_entities
                 WHERE kind = ?1 AND json_extract(data, '$.session_id') = ?2
                 ORDER BY id DESC LIMIT 1",
            )?;
            let rows = stmt.query_map(
                rusqlite::params![EntityType::Session.as_str(), session_id],
                |row| row.get::<_, i64>(0),
            )?;
            let mut session_ids: Vec<i64> = Vec::new();
            for r in rows {
                session_ids.push(r?);
            }

            let mut stmt2 = conn.prepare_cached(
                "SELECT id FROM graph_entities
                 WHERE kind IN (?1, ?2)
                   AND json_extract(data, '$.session_id') = ?3
                   AND id < ?4
                 ORDER BY id DESC LIMIT 1",
            )?;
            let rows2 = stmt2.query_map(
                rusqlite::params![
                    EntityType::Discovery.as_str(),
                    REASONING_LOG_KIND,
                    session_id,
                    discovery_id
                ],
                |row| row.get::<_, i64>(0),
            )?;
            let mut prior_ids: Vec<i64> = Vec::new();
            for r in rows2 {
                prior_ids.push(r?);
            }
            Ok::<(Option<i64>, Option<i64>), anyhow::Error>((session_ids.pop(), prior_ids.pop()))
        })?;

        let prov = ProvenanceData::new("store_discovery").to_value();
        let mut edge_data = serde_json::Map::new();
        edge_data.insert("provenance".to_string(), prov);
        edge_data.insert(
            "session_id".to_string(),
            Value::String(session_id.to_string()),
        );
        if let Some(pid) = project_id {
            edge_data.insert("project_id".to_string(), Value::String(pid.to_string()));
        }
        let edge_data = Value::Object(edge_data);

        if let Some(sid) = session_entity_id {
            self.insert_edge(discovery_id, sid, EdgeType::ObservedIn, edge_data.clone())?;
        }

        if let Some(prior) = prior_id {
            // caused_by: this discovery was triggered by the prior decision.
            self.insert_edge(discovery_id, prior, EdgeType::CausedBy, edge_data.clone())?;
            // led_to: the prior decision led to this one (inverse, for outward walks).
            self.insert_edge(prior, discovery_id, EdgeType::LedTo, edge_data.clone())?;
        }

        Ok(())
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

    pub fn recent_discoveries(
        &self,
        project_id: Option<&str>,
        agent: Option<&str>,
        session_id: Option<&str>,
        discovery_type: Option<&str>,
        limit: i64,
    ) -> Result<Vec<GraphEntity>> {
        let project_id = project_id.map(|s| s.to_string());
        let agent = agent.map(|s| s.to_string());
        let session_id = session_id.map(|s| s.to_string());
        let discovery_type = discovery_type.map(|s| s.to_string());

        super::with_graph_conn(&self.inner, move |conn| {
            // Build the WHERE clause with bare `?` placeholders only and bind
            // params in order via `params_from_iter`. This avoids the fragile
            // positional-index bookkeeping the old fixed-`?N` form required when
            // adding optional filters (`session_id`, `discovery_type`).
            let mut sql = String::from(
                "SELECT id, kind, name, file_path, data FROM graph_entities
                 WHERE kind = ?",
            );
            let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::with_capacity(1 + 2 + 1 + 1 + 1);
            params.push(Box::new(EntityType::Discovery.as_str().to_string()));
            if let Some(pid) = &project_id {
                // `project_id` OR `project` — same value bound twice.
                sql.push_str(
                    " AND (json_extract(data, '$.project_id') = ?
                           OR json_extract(data, '$.project') = ?)",
                );
                params.push(Box::new(pid.clone()));
                params.push(Box::new(pid.clone()));
            }
            if let Some(agent) = &agent {
                sql.push_str(" AND json_extract(data, '$.agent') = ?");
                params.push(Box::new(agent.clone()));
            }
            if let Some(sid) = &session_id {
                sql.push_str(" AND json_extract(data, '$.session_id') = ?");
                params.push(Box::new(sid.clone()));
            }
            if let Some(dty) = &discovery_type {
                sql.push_str(" AND json_extract(data, '$.discovery_type') = ?");
                params.push(Box::new(dty.clone()));
            }
            sql.push_str(" ORDER BY id DESC LIMIT ?");
            params.push(Box::new(limit));

            let mut stmt = conn.prepare_cached(&sql)?;
            let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
                Ok(GraphEntity {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    name: row.get(2)?,
                    file_path: row.get(3)?,
                    data: serde_json::from_str(row.get_ref(4)?.as_str()?)
                        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
                })
            })?;

            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }

    /// Dedup guard for live decision capture (Phase 4 watcher) and the
    /// post-hoc extractor (Phase 3). Returns true iff a Decision row already
    /// exists for this exact `(session_id, sequence, target, source)` quadruple
    /// — the dedup key named in `CHAT_DECISION_PLAN.md` ("both layers check
    /// before insert"). `source` distinguishes the capturing layer
    /// (`"askuser"` / `"exitplan"` / `"taskcreate"` / `"todowrite"` for the
    /// live watcher, `"llm-extract"` for the post-hoc extractor), so a decision
    /// captured live and re-extracted post-hoc with a different `source` is
    /// intentionally NOT collapsed here — that cross-layer double is the
    /// plan's documented tradeoff, not a bug.
    ///
    /// Queries the indexed `discoveries` table (`session_id`, `target`,
    /// `discovery_type` columns) rather than `graph_entities` so the check is
    /// index-backed, not a full scan + `json_extract`.
    pub fn decision_exists(
        &self,
        session_id: &str,
        sequence: i64,
        target: &str,
        source: &str,
    ) -> Result<bool> {
        let session_id = session_id.to_string();
        let target = target.to_string();
        let source = source.to_string();
        super::with_graph_conn(&self.inner, move |conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT 1 FROM discoveries
                 WHERE session_id = ?
                   AND target = ?
                   AND discovery_type = 'Decision'
                   AND json_extract(metadata, '$.source') = ?
                   AND json_extract(metadata, '$.sequence') = ?
                 LIMIT 1",
            )?;
            let exists = stmt.exists(params![session_id, target, source, sequence])?;
            Ok(exists)
        })
    }

    /// Dedup guard for the cooperative-skill and manual `/decision` capture
    /// paths (Phase 5), which have no stable transcript `sequence` to key on.
    /// Returns true iff a Decision row already exists for this exact
    /// `(session_id, target, source, chosen)` quadruple — the dedup key that
    /// means "the same choice was already recorded" when the capturing layer
    /// is a Claude Code skill or a manual command rather than a transcript
    /// scan.
    ///
    /// This is intentionally a *different* key from [`Self::decision_exists`]:
    /// the live watcher (Phase 4) and post-hoc extractor (Phase 3) key on
    /// `sequence` because transcript turns have a stable order; a skill that
    /// re-fires on the same architectural choice has no stable sequence, so a
    /// sequence key would let the duplicate through. Keying on `chosen` makes
    /// "re-fire the same decision" collapse correctly within the skill layer.
    /// The two layers use different `source` values (`"skill"` vs
    /// `"askuser"`/`"exitplan"`/`"taskcreate"`/`"todowrite"`/`"llm-extract"`),
    /// so a decision captured live and again via the skill is intentionally
    /// not collapsed here — that cross-layer double is the plan's documented
    /// tradeoff, not a bug.
    ///
    /// Queries the indexed `discoveries` table (`session_id`, `target`,
    /// `discovery_type` columns); `source` and `chosen` live in the metadata
    /// JSON, so those two predicates are `json_extract` scans over the
    /// already-session/target-filtered rows (bounded by the index).
    pub fn decision_exists_chosen(
        &self,
        session_id: &str,
        target: &str,
        source: &str,
        chosen: &str,
    ) -> Result<bool> {
        let session_id = session_id.to_string();
        let target = target.to_string();
        let source = source.to_string();
        let chosen = chosen.to_string();
        super::with_graph_conn(&self.inner, move |conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT 1 FROM discoveries
                 WHERE session_id = ?
                   AND target = ?
                   AND discovery_type = 'Decision'
                   AND json_extract(metadata, '$.source') = ?
                   AND json_extract(metadata, '$.chosen') = ?
                 LIMIT 1",
            )?;
            let exists = stmt.exists(params![session_id, target, source, chosen])?;
            Ok(exists)
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
