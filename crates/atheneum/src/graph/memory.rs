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
use super::hashing::content_hash_excluding;
use super::{AtheneumGraph, EntityType, MemoryPreview};

type MemoryEntry = (String, String, String, f64, String);
type ScoredMemoryRef<'a> = (f64, &'a MemoryEntry);

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
        tags: Option<&[String]>,
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
        if let (Some(tags), Some(obj)) = (tags, proposed_data.as_object_mut()) {
            obj.insert("tags".to_string(), json!(tags));
        }

        let content_hash = content_hash_excluding(
            &proposed_data,
            &["created_at", "updated_at", "sql_id", "content_hash"],
        )?;
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

        let disambiguation = self
            .resolve(
                &format!("{key} {content}"),
                0.3,
                project_id,
                Some(EntityType::Memory.as_str()),
            )
            .ok();

        Ok(MemoryPreview {
            proposed_key: key.to_string(),
            proposed_data,
            content_hash,
            exact_matches,
            candidate_matches,
            disambiguation,
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
        tags: Option<&[String]>,
    ) -> Result<i64> {
        let now = Utc::now().to_rfc3339();

        // Check for existing memory by composite key (key, scope, project_id).
        let existing_id = super::with_graph_conn(&self.inner, |conn| {
            let mut stmt = if project_id.is_some() {
                conn.prepare_cached(
                    "SELECT id FROM graph_entities
                     WHERE kind = ?1 AND name = ?2
                       AND json_extract(data, '$.scope') = ?3
                       AND json_extract(data, '$.project_id') = ?4",
                )?
            } else {
                conn.prepare_cached(
                    "SELECT id FROM graph_entities
                     WHERE kind = ?1 AND name = ?2
                       AND json_extract(data, '$.scope') = ?3
                       AND json_extract(data, '$.project_id') IS NULL",
                )?
            };
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

                self.runtime.record_memory_row_repair();
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
            if let (Some(tags), Some(obj)) = (tags, data.as_object_mut()) {
                obj.insert("tags".to_string(), json!(tags));
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
        if let (Some(tags), Some(obj)) = (tags, data.as_object_mut()) {
            obj.insert("tags".to_string(), json!(tags));
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

    /// Patch an existing Memory entity in place. Only fields set on `patch` are
    /// written; the rest are preserved. An empty patch is a no-op and returns
    /// the id of the existing memory. Recomputes the content hash, mirrors the
    /// change into the `memory_entries` SQL table (same dual-write path as
    /// [`store_memory`](Self::store_memory)), re-indexes the entity for search,
    /// and invalidates the memory cache.
    ///
    /// Errors with [`AtheneumError::EntityNotFound`] if `id` does not refer to
    /// a kind=`Memory` entity.
    pub fn update_memory(&self, id: i64, patch: &super::MemoryPatch) -> Result<i64> {
        let entity = self.get_entity(id).map_err(|_| super::AtheneumError::EntityNotFound(id))?;
        if entity.kind != EntityType::Memory.as_str() {
            return Err(super::AtheneumError::EntityNotFound(id).into());
        }

        // Empty patch: no-op, return current id.
        if patch.is_empty() {
            return Ok(id);
        }

        let mut data = entity.data.clone();
        let obj = data
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("Memory {} data is not a JSON object", id))?;

        let key = obj
            .get("key")
            .and_then(|v| v.as_str())
            .map(String::from)
            .ok_or_else(|| anyhow::anyhow!("Memory {} missing 'key' field", id))?;
        let scope = obj
            .get("scope")
            .and_then(|v| v.as_str())
            .map(String::from)
            .ok_or_else(|| anyhow::anyhow!("Memory {} missing 'scope' field", id))?;
        let project_id = obj
            .get("project_id")
            .and_then(|v| v.as_str())
            .map(String::from);

        let mut content = obj
            .get("content")
            .and_then(|v| v.as_str())
            .map(String::from)
            .ok_or_else(|| anyhow::anyhow!("Memory {} missing 'content' field", id))?;
        let mut confidence = obj
            .get("confidence")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| anyhow::anyhow!("Memory {} missing 'confidence' field", id))?;

        if let Some(ref new_content) = patch.content {
            content = new_content.clone();
        }
        if let Some(importance) = patch.importance {
            // Mirror store_memory's 1..10 importance → 0..1 confidence mapping.
            confidence = (importance as f64 / 10.0).clamp(0.0, 1.0);
        }

        // Tags: merge by default, or replace when replace_tags is set.
        let merged_tags: Option<Vec<String>> = if let Some(ref new_tags) = patch.tags {
            if patch.replace_tags {
                Some(new_tags.clone())
            } else {
                let mut existing: Vec<String> = obj
                    .get("tags")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                for t in new_tags {
                    if !existing.iter().any(|e| e == t) {
                        existing.push(t.clone());
                    }
                }
                Some(existing)
            }
        } else {
            obj.get("tags")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        };

        // Preserve created_at + sql_id.
        let created_at = obj
            .get("created_at")
            .and_then(|v| v.as_str())
            .map(String::from)
            .ok_or_else(|| anyhow::anyhow!("Memory {} missing 'created_at' field", id))?;
        let sql_id = obj.get("sql_id").and_then(|v| v.as_i64()).unwrap_or(0);
        let now = Utc::now().to_rfc3339();

        // Mirror into the memory_entries SQL table (same dual-write path as
        // store_memory). If the row is missing — e.g. from a legacy import —
        // insert it rather than silently losing the write.
        self.with_raw_connection(|conn| {
            let updated = conn.execute(
                "UPDATE memory_entries
                 SET content = ?1, confidence = ?2, updated_at = ?3
                 WHERE key = ?4 AND scope = ?5
                   AND COALESCE(project_id, '') = COALESCE(?6, '')",
                params![&content, confidence, &now, &key, &scope, project_id.as_deref()],
            )?;
            if updated == 0 {
                self.runtime.record_memory_row_repair();
                conn.execute(
                    "INSERT INTO memory_entries
                        (key, scope, content, confidence, project_id, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![&key, &scope, &content, confidence, project_id.as_deref(), &created_at, &now],
                )?;
            }
            Ok::<_, anyhow::Error>(())
        })?;

        // Rebuild the entity data, recompute hash, write back.
        let mut new_data = json!({
            "sql_id": sql_id,
            "key": key,
            "scope": scope,
            "content": content,
            "confidence": confidence,
            "created_at": created_at,
            "updated_at": now,
        });
        if let (Some(pid), Some(o)) = (project_id.as_deref(), new_data.as_object_mut()) {
            o.insert("project_id".to_string(), Value::String(pid.to_string()));
        }
        if let (Some(tags), Some(o)) = (merged_tags.as_ref(), new_data.as_object_mut()) {
            o.insert("tags".to_string(), json!(tags));
        }
        let hash = super::hashing::content_hash_excluding(
            &new_data,
            &["created_at", "updated_at", "sql_id", "content_hash"],
        )?;
        if let Some(o) = new_data.as_object_mut() {
            o.insert("content_hash".to_string(), Value::String(hash));
        }

        self.update_entity_data(id, &new_data)?;

        let indexed = GraphEntity {
            id,
            kind: EntityType::Memory.as_str().to_string(),
            name: key,
            file_path: None,
            data: new_data.clone(),
        };
        if let Err(e) = self.add_entity_to_search_index(&indexed) {
            eprintln!("[atheneum] memory re-index warning: {}", e);
        }

        self.runtime.record_memory_write();
        self.runtime.bump_generation(CacheDomain::Memory);
        Ok(id)
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
                    let mut stmt = conn.prepare_cached(
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
                    let mut stmt = conn.prepare_cached(
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
                    let mut stmt = conn.prepare_cached(
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
                    let mut stmt = conn.prepare_cached(
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

    /// List memory entries for a scope with pagination.
    ///
    /// This is the primary implementation; `list_memory` is a compatibility
    /// wrapper that caches the full result set.
    pub fn list_memory_page(
        &self,
        scope: Option<&str>,
        project_id: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<GraphEntity>> {
        let lim = limit as i64;
        let off = offset as i64;
        super::with_graph_conn(&self.inner, |conn| {
            let mut out = Vec::new();
            match (scope, project_id) {
                (Some(s), Some(pid)) => {
                    let mut stmt = conn.prepare_cached(
                        "SELECT id, kind, name, file_path, data FROM graph_entities
                         WHERE kind = ?1
                           AND json_extract(data, '$.scope') = ?2
                           AND json_extract(data, '$.project_id') = ?3
                         LIMIT ?4 OFFSET ?5",
                    )?;
                    let rows = stmt.query_map(
                        params![EntityType::Memory.as_str(), s, pid, lim, off],
                        row_to_entity,
                    )?;
                    for row in rows {
                        out.push(row?);
                    }
                }
                (Some(s), None) => {
                    let mut stmt = conn.prepare_cached(
                        "SELECT id, kind, name, file_path, data FROM graph_entities
                         WHERE kind = ?1
                           AND json_extract(data, '$.scope') = ?2
                         LIMIT ?3 OFFSET ?4",
                    )?;
                    let rows = stmt.query_map(
                        params![EntityType::Memory.as_str(), s, lim, off],
                        row_to_entity,
                    )?;
                    for row in rows {
                        out.push(row?);
                    }
                }
                (None, Some(pid)) => {
                    let mut stmt = conn.prepare_cached(
                        "SELECT id, kind, name, file_path, data FROM graph_entities
                         WHERE kind = ?1
                           AND json_extract(data, '$.project_id') = ?2
                         LIMIT ?3 OFFSET ?4",
                    )?;
                    let rows = stmt.query_map(
                        params![EntityType::Memory.as_str(), pid, lim, off],
                        row_to_entity,
                    )?;
                    for row in rows {
                        out.push(row?);
                    }
                }
                (None, None) => {
                    let mut stmt = conn.prepare_cached(
                        "SELECT id, kind, name, file_path, data FROM graph_entities
                         WHERE kind = ?1
                         LIMIT ?2 OFFSET ?3",
                    )?;
                    let rows = stmt.query_map(
                        params![EntityType::Memory.as_str(), lim, off],
                        row_to_entity,
                    )?;
                    for row in rows {
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

        let out = self.list_memory_page(scope, project_id, 0, usize::MAX)?;
        self.runtime.cache_store(
            cache_key,
            CacheDomain::Memory,
            QueryCacheValue::Entities(out.clone()),
        );
        Ok(out)
    }

    /// Compose a token-bounded bootstrap packet: recent memories + session digest.
    ///
    /// Memories are scored by relevance to recent discoveries (graph-aware) and
    /// recency, rather than purely by `updated_at`. Returns JSON:
    /// ```json
    /// {
    ///   "memories": [{"key": "...", "content": "...", "confidence": 1.0, "scope": "...", "updated_at": "..."}],
    ///   "session_digest": "...",
    ///   "token_estimate": 123,
    ///   "relevance_context": ["term1", "term2", ...]
    /// }
    /// ```
    pub fn compose_memory_bootstrap(
        &self,
        project: Option<&str>,
        token_budget: usize,
        last_sessions: i64,
    ) -> Result<Value> {
        const CHARS_PER_TOKEN: usize = 4;

        let mem_budget = token_budget / 2;
        let digest_budget = token_budget - mem_budget;

        // Build relevance set from recent discoveries' targets.
        // Split targets on whitespace, underscores, hyphens into indexable terms.
        let (relevance_terms, top_context_terms) = self.with_raw_connection(|conn| {
            let targets: Vec<String> = if let Some(pid) = project {
                let mut s = conn.prepare_cached(
                    "SELECT target FROM discoveries
                     WHERE COALESCE(project_id, '') = ?1
                     ORDER BY created_at DESC LIMIT 20",
                )?;
                let mut rows = Vec::new();
                for v in s
                    .query_map(rusqlite::params![pid], |r| r.get::<_, String>(0))?
                    .flatten()
                {
                    rows.push(v);
                }
                rows
            } else {
                let mut s = conn.prepare_cached(
                    "SELECT target FROM discoveries ORDER BY created_at DESC LIMIT 20",
                )?;
                let mut rows = Vec::new();
                for v in s.query_map([], |r| r.get::<_, String>(0))?.flatten() {
                    rows.push(v);
                }
                rows
            };

            let mut term_set: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut ordered: Vec<String> = Vec::new();
            for target in &targets {
                for part in target.split(|c: char| c.is_whitespace() || c == '_' || c == '-') {
                    let t = part.to_lowercase();
                    if t.len() > 2 && term_set.insert(t.clone()) {
                        ordered.push(t);
                    }
                }
            }
            let top10: Vec<String> = ordered.into_iter().take(10).collect();
            Ok((term_set, top10))
        })?;

        // BFS on graph_edges (depth 2) from recent Decision/Discovery nodes.
        // Builds a set of memory keys that are graph-connected to recent activity.
        let graph_boosted_keys: std::collections::HashSet<String> = self
            .with_raw_connection(|conn| {
                // Step 1: seed nodes — most recent Decision/Discovery entities.
                let mut seed_ids: Vec<i64> = Vec::new();
                {
                    let mut s = conn.prepare_cached(
                        "SELECT id FROM graph_entities
                         WHERE kind IN ('Decision','Discovery')
                         ORDER BY id DESC LIMIT 30",
                    )?;
                    for v in s.query_map([], |r| r.get::<_, i64>(0))?.flatten() {
                        seed_ids.push(v);
                    }
                }

                if seed_ids.is_empty() {
                    return Ok(std::collections::HashSet::new());
                }

                // Step 2: BFS depth 2 across led_to/caused_by/related_to edges.
                let mut visited: std::collections::HashSet<i64> =
                    seed_ids.iter().copied().collect();
                let mut frontier: Vec<i64> = seed_ids;
                for _ in 0..2 {
                    if frontier.is_empty() || visited.len() >= 200 {
                        break;
                    }
                    let mut next_frontier: Vec<i64> = Vec::new();
                    for node_id in &frontier {
                        let mut s = conn.prepare_cached(
                            "SELECT from_id, to_id FROM graph_edges
                             WHERE (from_id = ?1 OR to_id = ?1)
                               AND edge_type IN ('led_to','caused_by','related_to')",
                        )?;
                        for (from, to) in s
                            .query_map(rusqlite::params![node_id], |r| {
                                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
                            })?
                            .flatten()
                        {
                            for neighbor in [from, to] {
                                if visited.insert(neighbor) {
                                    next_frontier.push(neighbor);
                                    if visited.len() >= 200 {
                                        break;
                                    }
                                }
                            }
                            if visited.len() >= 200 {
                                break;
                            }
                        }
                        if visited.len() >= 200 {
                            break;
                        }
                    }
                    frontier = next_frontier;
                }

                // Step 3: find Memory nodes in the visited set, extract their keys.
                let mut boosted: std::collections::HashSet<String> =
                    std::collections::HashSet::new();
                for entity_id in &visited {
                    let mut s =
                        conn.prepare_cached("SELECT kind, data FROM graph_entities WHERE id = ?1")?;
                    let mut rows: Vec<(String, String)> = Vec::new();
                    for v in s
                        .query_map(rusqlite::params![entity_id], |r| {
                            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                        })?
                        .flatten()
                    {
                        rows.push(v);
                    }
                    for (kind, data_str) in rows {
                        if kind == "Memory" {
                            if let Ok(data) = serde_json::from_str::<Value>(&data_str) {
                                if let Some(key) = data.get("key").and_then(|v| v.as_str()) {
                                    boosted.insert(key.to_string());
                                }
                            }
                        }
                    }
                }
                Ok(boosted)
            })
            .unwrap_or_default();

        // Fetch all candidate memories — exclude session-scoped turn-by-turn
        // chat logs (scope LIKE 'session:%') which are low-confidence noise that
        // crowds out durable memories in the token budget. Keep durable scopes:
        // memory, reference, user, project, finding, preference, risk.
        let memories: Vec<MemoryEntry> = self.with_raw_connection(|conn| {
            let mut out = Vec::new();
            if let Some(pid) = project {
                let mut s = conn.prepare_cached(
                    "SELECT key, scope, content, confidence, COALESCE(updated_at, created_at)
                         FROM memory_entries
                         WHERE COALESCE(project_id, '') = ?1
                         AND scope NOT LIKE 'session:%'
                         LIMIT 200",
                )?;
                for row in s.query_map(rusqlite::params![pid], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, f64>(3)?,
                        r.get::<_, String>(4)?,
                    ))
                })? {
                    out.push(row?);
                }
            } else {
                let mut s = conn.prepare_cached(
                    "SELECT key, scope, content, confidence, COALESCE(updated_at, created_at)
                         FROM memory_entries
                         WHERE scope NOT LIKE 'session:%'
                         LIMIT 200",
                )?;
                for row in s.query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, f64>(3)?,
                        r.get::<_, String>(4)?,
                    ))
                })? {
                    out.push(row?);
                }
            }
            Ok(out)
        })?;

        // Score each memory: relevance_hits + recency_bonus.
        let now_ts = chrono::Utc::now();
        let mut scored: Vec<ScoredMemoryRef<'_>> = memories
            .iter()
            .map(|m| {
                let (key, _scope, content, _confidence, updated_at) = m;
                let base_relevance: f64 = if relevance_terms.is_empty() {
                    0.0
                } else {
                    let searchable = format!("{} {}", key, content).to_lowercase();
                    let hits = relevance_terms
                        .iter()
                        .filter(|t| searchable.contains(t.as_str()))
                        .count();
                    hits as f64
                };
                let recency_bonus = chrono::DateTime::parse_from_rfc3339(updated_at)
                    .ok()
                    .map(|dt| {
                        let days = now_ts
                            .signed_duration_since(dt.with_timezone(&chrono::Utc))
                            .num_days()
                            .max(0) as f64;
                        1.0 / (days + 1.0)
                    })
                    .unwrap_or(0.0);
                let graph_boost = if graph_boosted_keys.contains(key.as_str()) {
                    2.0
                } else {
                    0.0
                };
                (base_relevance + recency_bonus + graph_boost, m)
            })
            .collect();

        // Sort descending by score; fall back to original order on equal scores.
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        // Fit top-scored memories into budget, skipping oversized entries.
        const MAX_CONTENT_CHARS: usize = 1200; // ~300 tokens per entry cap
        let mut mem_tokens_used = 0usize;
        let mut mem_out = Vec::new();
        let mut graph_connected_count = 0usize;
        for (_score, (key, scope, content, confidence, updated_at)) in &scored {
            let truncated_content = if content.len() > MAX_CONTENT_CHARS {
                &content[..MAX_CONTENT_CHARS]
            } else {
                content.as_str()
            };
            let entry_chars =
                key.len() + scope.len() + truncated_content.len() + updated_at.len() + 30;
            let entry_tokens = entry_chars.div_ceil(CHARS_PER_TOKEN);
            if mem_tokens_used + entry_tokens > mem_budget {
                continue; // skip oversized, don't stop — smaller entries may still fit
            }
            mem_tokens_used += entry_tokens;
            let is_graph_connected = graph_boosted_keys.contains(key.as_str());
            if is_graph_connected {
                graph_connected_count += 1;
            }
            mem_out.push(json!({
                "key": key,
                "scope": scope,
                "content": truncated_content,
                "confidence": confidence,
                "updated_at": updated_at,
                "graph_connected": is_graph_connected,
            }));
        }

        // Compose session digest (text form, token-bounded).
        let digest_text = self
            .compose_digest(project, last_sessions, digest_budget)
            .unwrap_or_default();
        let digest_tokens = digest_text.len().div_ceil(CHARS_PER_TOKEN);

        Ok(json!({
            "memories": mem_out,
            "session_digest": digest_text,
            "token_estimate": mem_tokens_used + digest_tokens,
            "relevance_context": top_context_terms,
            "memories_graph_connected": graph_connected_count,
        }))
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
