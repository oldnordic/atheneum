//! Memory Router Phase 2A — Canonical memory envelope APIs and routing read path.
//!
//! Provides conflict-safe upsert, get-by-graph-id, budgeted route queries,
//! and explicit memory expansion.

use anyhow::{bail, Result};
use chrono::Utc;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlitegraph::GraphEntity;

use super::cache::CacheDomain;
use super::hashing::content_hash_excluding;
use super::{AtheneumGraph, EdgeType, EntityType};

fn default_memory_scope() -> String {
    "user".to_string()
}

fn default_memory_kind() -> String {
    "memory".to_string()
}

fn default_confidence() -> f64 {
    1.0
}

fn default_status() -> String {
    "active".to_string()
}

fn default_sensitivity() -> String {
    "normal".to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConflictStrategy {
    #[default]
    Revision,
    Conflict,
}

impl ConflictStrategy {
    pub fn from_str_opt(s: Option<&str>) -> Self {
        match s.map(str::to_lowercase).as_deref() {
            Some("conflict") => ConflictStrategy::Conflict,
            _ => ConflictStrategy::Revision,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemoryRouteDepth {
    #[default]
    Summary,
    Bundle,
    Expand,
}

impl MemoryRouteDepth {
    pub fn from_str_opt(s: Option<&str>) -> Self {
        match s.map(str::to_lowercase).as_deref() {
            Some("bundle") => MemoryRouteDepth::Bundle,
            Some("expand") => MemoryRouteDepth::Expand,
            _ => MemoryRouteDepth::Summary,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertMemoryItemParams {
    pub key: String,
    pub content: String,
    #[serde(default = "default_memory_scope")]
    pub scope: String,
    #[serde(default = "default_memory_kind")]
    pub kind: String,
    pub title: Option<String>,
    pub summary: Option<String>,
    #[serde(default = "default_confidence")]
    pub confidence: f64,
    pub project_id: Option<String>,
    #[serde(default = "default_status")]
    pub status: String,
    pub source_uri: Option<String>,
    #[serde(default)]
    pub evidence: Vec<Value>,
    pub content_hash: Option<String>,
    pub token_estimate: Option<usize>,
    #[serde(default = "default_sensitivity")]
    pub sensitivity: String,
    pub valid_from: Option<String>,
    pub valid_to: Option<String>,
    pub ttl_seconds: Option<i64>,
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub conflict_strategy: ConflictStrategy,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum UpsertMemoryOutcome {
    Created {
        graph_entity_id: i64,
        sql_id: i64,
        content_hash: String,
    },
    Unchanged {
        graph_entity_id: i64,
        sql_id: i64,
        content_hash: String,
    },
    RevisionCreated {
        graph_entity_id: i64,
        sql_id: i64,
        supersedes_graph_entity_id: i64,
        content_hash: String,
    },
    Conflict {
        existing_graph_entity_id: i64,
        existing_content_hash: String,
        incoming_content_hash: String,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryItemEnvelope {
    pub graph_entity_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sql_id: Option<i64>,
    pub id: String,
    pub key: String,
    pub scope: String,
    pub kind: String,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub content: String,
    pub confidence: f64,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_uri: Option<String>,
    pub evidence: Vec<Value>,
    pub content_hash: Option<String>,
    pub token_estimate: usize,
    pub sensitivity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl_seconds: Option<i64>,
    pub created_at: String,
    pub updated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supersedes_graph_entity_id: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoryRouteFilters {
    pub project: Option<String>,
    pub kinds: Option<Vec<String>>,
    pub status: Option<String>,
    pub confidence_floor: Option<f64>,
    pub evidence_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRouteItem {
    pub id: String,
    pub graph_entity_id: i64,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    pub score: f32,
    pub evidence_state: String,
    pub token_cost: usize,
    pub expandable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRouteResponse {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    pub budget_tokens: usize,
    pub budget_used: usize,
    pub depth: MemoryRouteDepth,
    pub items: Vec<MemoryRouteItem>,
    pub omitted: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryExpandResponse {
    pub budget_tokens: usize,
    pub budget_used: usize,
    pub items: Vec<MemoryItemEnvelope>,
    pub omitted: usize,
    pub warnings: Vec<String>,
}

fn estimate_text_tokens(text: &str) -> usize {
    let chars = text.chars().count();
    (chars / 4).max(1)
}

fn compute_memory_content_hash(params: &UpsertMemoryItemParams) -> Result<String> {
    if let Some(ref h) = params.content_hash {
        return Ok(h.clone());
    }
    let val = json!({
        "key": &params.key,
        "scope": &params.scope,
        "content": &params.content,
        "kind": &params.kind,
        "title": &params.title,
        "summary": &params.summary,
        "project_id": &params.project_id,
        "evidence": &params.evidence,
        "sensitivity": &params.sensitivity,
    });
    content_hash_excluding(
        &val,
        &[
            "created_at",
            "updated_at",
            "sql_id",
            "content_hash",
            "graph_entity_id",
            "supersedes_graph_entity_id",
        ],
    )
}

impl AtheneumGraph {
    /// Upsert a memory item with conflict safety.
    pub fn upsert_memory_item(
        &self,
        params: &UpsertMemoryItemParams,
    ) -> Result<UpsertMemoryOutcome> {
        // Invariant: sensitivity=secret records rejected at ingest
        if params.sensitivity.eq_ignore_ascii_case("secret") {
            bail!("Records with sensitivity 'secret' are rejected at ingest");
        }

        let now = Utc::now().to_rfc3339();
        let content_hash = compute_memory_content_hash(params)?;
        let token_est = params.token_estimate.unwrap_or_else(|| {
            let combined = format!(
                "{} {} {}",
                params.title.as_deref().unwrap_or_default(),
                params.summary.as_deref().unwrap_or_default(),
                params.content
            );
            estimate_text_tokens(&combined)
        });
        let evidence_json_str = serde_json::to_string(&params.evidence)?;

        // Find any existing active record with same logical key (key, scope, project_id)
        let existing: Option<(i64, Option<i64>, Option<String>, String)> = self
            .with_raw_connection(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, graph_entity_id, content_hash, content
                 FROM memory_entries
                 WHERE key = ?1 AND scope = ?2
                   AND COALESCE(project_id, '') = COALESCE(?3, '')
                   AND status != 'superseded'
                 ORDER BY id DESC LIMIT 1",
                )?;
                let row = stmt
                    .query_row(
                        params![params.key, params.scope, params.project_id.as_deref()],
                        |r| {
                            Ok((
                                r.get::<_, i64>(0)?,
                                r.get::<_, Option<i64>>(1)?,
                                r.get::<_, Option<String>>(2)?,
                                r.get::<_, String>(3)?,
                            ))
                        },
                    )
                    .ok();
                Ok(row)
            })?;

        if let Some((existing_sql_id, existing_ge_id_opt, existing_hash_opt, _existing_content)) =
            existing
        {
            // Find or fallback to the graph entity ID
            let existing_ge_id = if let Some(ge_id) = existing_ge_id_opt {
                ge_id
            } else {
                // Look up in graph_entities
                let found_id = super::with_graph_conn(&self.inner, |conn| {
                    let mut stmt = conn.prepare(
                        "SELECT id FROM graph_entities
                         WHERE kind = ?1 AND name = ?2
                           AND json_extract(data, '$.scope') = ?3
                           AND COALESCE(json_extract(data, '$.project_id'), '') = COALESCE(?4, '')
                           AND json_extract(data, '$.superseded_at') IS NULL
                         ORDER BY id DESC LIMIT 1",
                    )?;
                    let id: Option<i64> = stmt
                        .query_row(
                            params![
                                EntityType::Memory.as_str(),
                                params.key,
                                params.scope,
                                params.project_id.as_deref()
                            ],
                            |r| r.get(0),
                        )
                        .ok();
                    Ok(id)
                })?;
                found_id.unwrap_or(0)
            };

            let existing_hash = existing_hash_opt.unwrap_or_default();

            // Polarity 1: Same content hash = Idempotent replay (no new row)
            if existing_hash == content_hash {
                return Ok(UpsertMemoryOutcome::Unchanged {
                    graph_entity_id: existing_ge_id,
                    sql_id: existing_sql_id,
                    content_hash,
                });
            }

            // Polarity 2: Different content hash = Conflict or Revision (never silently overwrite)
            if params.conflict_strategy == ConflictStrategy::Conflict {
                return Ok(UpsertMemoryOutcome::Conflict {
                    existing_graph_entity_id: existing_ge_id,
                    existing_content_hash: existing_hash,
                    incoming_content_hash: content_hash,
                    reason: format!(
                        "Memory item with key '{}' already exists with different content hash",
                        params.key
                    ),
                });
            }

            // Strategy is Revision: create a new revision row and link supersedes_graph_entity_id
            // 1. Mark existing SQL row as superseded
            self.with_raw_connection(|conn| {
                Ok(conn.execute(
                    "UPDATE memory_entries SET status = 'superseded', updated_at = ?1 WHERE id = ?2",
                    params![&now, existing_sql_id],
                )?)
            })?;

            // 2. Insert new revision SQL row
            let new_sql_id = self.with_raw_connection(|conn| {
                conn.execute(
                    "INSERT INTO memory_entries (
                        key, scope, content, confidence, project_id, created_at, updated_at,
                        kind, title, summary, status, source_uri, evidence_json, content_hash,
                        token_estimate, sensitivity, valid_from, valid_to, ttl_seconds,
                        supersedes_graph_entity_id
                    ) VALUES (
                        ?1, ?2, ?3, ?4, ?5, ?6, ?6,
                        ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                        ?14, ?15, ?16, ?17, ?18,
                        ?19
                    )",
                    params![
                        params.key,
                        params.scope,
                        params.content,
                        params.confidence,
                        params.project_id.as_deref(),
                        &now,
                        params.kind,
                        params.title.as_deref(),
                        params.summary.as_deref(),
                        params.status,
                        params.source_uri.as_deref(),
                        &evidence_json_str,
                        &content_hash,
                        token_est as i64,
                        params.sensitivity,
                        params.valid_from.as_deref(),
                        params.valid_to.as_deref(),
                        params.ttl_seconds,
                        existing_ge_id,
                    ],
                )?;
                Ok(conn.last_insert_rowid())
            })?;

            // 3. Create new GraphEntity for the revision
            let mut entity_data = json!({
                "sql_id": new_sql_id,
                "key": &params.key,
                "scope": &params.scope,
                "kind": &params.kind,
                "title": &params.title,
                "summary": &params.summary,
                "content": &params.content,
                "confidence": params.confidence,
                "status": &params.status,
                "source_uri": &params.source_uri,
                "evidence": &params.evidence,
                "content_hash": &content_hash,
                "token_estimate": token_est,
                "sensitivity": &params.sensitivity,
                "valid_from": &params.valid_from,
                "valid_to": &params.valid_to,
                "ttl_seconds": params.ttl_seconds,
                "supersedes_graph_entity_id": existing_ge_id,
                "created_at": &now,
                "updated_at": &now,
            });
            if let (Some(pid), Some(obj)) = (&params.project_id, entity_data.as_object_mut()) {
                obj.insert("project_id".to_string(), Value::String(pid.clone()));
            }
            if let (Some(tags), Some(obj)) = (&params.tags, entity_data.as_object_mut()) {
                obj.insert("tags".to_string(), json!(tags));
            }

            let new_ge = GraphEntity {
                id: 0,
                kind: EntityType::Memory.as_str().to_string(),
                name: params.key.clone(),
                file_path: None,
                data: entity_data.clone(),
            };
            let new_ge_id = self
                .inner
                .insert_entity(&new_ge)
                .map_err(|e| anyhow::anyhow!("insert revision Memory entity: {}", e))?;

            // 4. Update SQL row and entity data with new_ge_id
            self.with_raw_connection(|conn| {
                Ok(conn.execute(
                    "UPDATE memory_entries SET graph_entity_id = ?1 WHERE id = ?2",
                    params![new_ge_id, new_sql_id],
                )?)
            })?;
            if let Some(obj) = entity_data.as_object_mut() {
                obj.insert("graph_entity_id".to_string(), json!(new_ge_id));
            }
            self.update_entity_data(new_ge_id, &entity_data)?;

            // 5. Mark old GraphEntity as superseded
            if existing_ge_id > 0 {
                if let Ok(mut old_ge) = self.get_entity(existing_ge_id) {
                    if let Some(obj) = old_ge.data.as_object_mut() {
                        obj.insert("superseded_at".to_string(), json!(now));
                        obj.insert("superseded_by".to_string(), json!(new_ge_id));
                        obj.insert("status".to_string(), json!("superseded"));
                    }
                    let _ = self.update_entity_data(existing_ge_id, &old_ge.data);
                }
                // Add graph edges
                let _ = self.inner.insert_edge(&sqlitegraph::GraphEdge {
                    id: 0,
                    from_id: new_ge_id,
                    to_id: existing_ge_id,
                    edge_type: EdgeType::Supersedes.as_str().to_string(),
                    data: json!({"at": now}),
                });
                let _ = self.inner.insert_edge(&sqlitegraph::GraphEdge {
                    id: 0,
                    from_id: existing_ge_id,
                    to_id: new_ge_id,
                    edge_type: EdgeType::SupersededBy.as_str().to_string(),
                    data: json!({"at": now}),
                });
            }

            // Index in search
            let indexed = GraphEntity {
                id: new_ge_id,
                kind: EntityType::Memory.as_str().to_string(),
                name: params.key.clone(),
                file_path: None,
                data: entity_data,
            };
            let _ = self.add_entity_to_search_index(&indexed);

            self.runtime.record_memory_write();
            self.runtime.bump_generation(CacheDomain::Memory);

            return Ok(UpsertMemoryOutcome::RevisionCreated {
                graph_entity_id: new_ge_id,
                sql_id: new_sql_id,
                supersedes_graph_entity_id: existing_ge_id,
                content_hash,
            });
        }

        // New record insertion:
        // 1. Insert SQL row
        let sql_id = self.with_raw_connection(|conn| {
            conn.execute(
                "INSERT INTO memory_entries (
                    key, scope, content, confidence, project_id, created_at, updated_at,
                    kind, title, summary, status, source_uri, evidence_json, content_hash,
                    token_estimate, sensitivity, valid_from, valid_to, ttl_seconds
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?6,
                    ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                    ?14, ?15, ?16, ?17, ?18
                )",
                params![
                    params.key,
                    params.scope,
                    params.content,
                    params.confidence,
                    params.project_id.as_deref(),
                    &now,
                    params.kind,
                    params.title.as_deref(),
                    params.summary.as_deref(),
                    params.status,
                    params.source_uri.as_deref(),
                    &evidence_json_str,
                    &content_hash,
                    token_est as i64,
                    params.sensitivity,
                    params.valid_from.as_deref(),
                    params.valid_to.as_deref(),
                    params.ttl_seconds,
                ],
            )?;
            Ok(conn.last_insert_rowid())
        })?;

        // 2. Insert GraphEntity
        let mut entity_data = json!({
            "sql_id": sql_id,
            "key": &params.key,
            "scope": &params.scope,
            "kind": &params.kind,
            "title": &params.title,
            "summary": &params.summary,
            "content": &params.content,
            "confidence": params.confidence,
            "status": &params.status,
            "source_uri": &params.source_uri,
            "evidence": &params.evidence,
            "content_hash": &content_hash,
            "token_estimate": token_est,
            "sensitivity": &params.sensitivity,
            "valid_from": &params.valid_from,
            "valid_to": &params.valid_to,
            "ttl_seconds": params.ttl_seconds,
            "created_at": &now,
            "updated_at": &now,
        });
        if let (Some(pid), Some(obj)) = (&params.project_id, entity_data.as_object_mut()) {
            obj.insert("project_id".to_string(), Value::String(pid.clone()));
        }
        if let (Some(tags), Some(obj)) = (&params.tags, entity_data.as_object_mut()) {
            obj.insert("tags".to_string(), json!(tags));
        }

        let ge = GraphEntity {
            id: 0,
            kind: EntityType::Memory.as_str().to_string(),
            name: params.key.clone(),
            file_path: None,
            data: entity_data.clone(),
        };
        let ge_id = self
            .inner
            .insert_entity(&ge)
            .map_err(|e| anyhow::anyhow!("insert Memory entity: {}", e))?;

        // 3. Link graph_entity_id in SQL row and entity data
        self.with_raw_connection(|conn| {
            Ok(conn.execute(
                "UPDATE memory_entries SET graph_entity_id = ?1 WHERE id = ?2",
                params![ge_id, sql_id],
            )?)
        })?;
        if let Some(obj) = entity_data.as_object_mut() {
            obj.insert("graph_entity_id".to_string(), json!(ge_id));
        }
        self.update_entity_data(ge_id, &entity_data)?;

        // Index in search
        let indexed = GraphEntity {
            id: ge_id,
            kind: EntityType::Memory.as_str().to_string(),
            name: params.key.clone(),
            file_path: None,
            data: entity_data,
        };
        let _ = self.add_entity_to_search_index(&indexed);

        self.runtime.record_memory_write();
        self.runtime.bump_generation(CacheDomain::Memory);

        Ok(UpsertMemoryOutcome::Created {
            graph_entity_id: ge_id,
            sql_id,
            content_hash,
        })
    }

    /// Retrieve a canonical memory item envelope by its graph entity ID.
    pub fn get_memory_item_by_graph_id(
        &self,
        graph_entity_id: i64,
    ) -> Result<Option<MemoryItemEnvelope>> {
        let sql_item: Option<MemoryItemEnvelope> = self.with_raw_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, key, scope, content, confidence, project_id, created_at, updated_at,
                        kind, title, summary, status, source_uri, evidence_json, content_hash,
                        token_estimate, sensitivity, valid_from, valid_to, ttl_seconds,
                        graph_entity_id, supersedes_graph_entity_id
                 FROM memory_entries
                 WHERE graph_entity_id = ?1
                 ORDER BY id DESC LIMIT 1",
            )?;
            let item = stmt
                .query_row(params![graph_entity_id], |r| {
                    let id: i64 = r.get(0)?;
                    let key: String = r.get(1)?;
                    let scope: String = r.get(2)?;
                    let content: String = r.get(3)?;
                    let confidence: f64 = r.get(4)?;
                    let project_id: Option<String> = r.get(5)?;
                    let created_at: String = r.get(6)?;
                    let updated_at: Option<String> = r.get(7)?;
                    let kind: Option<String> = r.get(8)?;
                    let title: Option<String> = r.get(9)?;
                    let summary: Option<String> = r.get(10)?;
                    let status: Option<String> = r.get(11)?;
                    let source_uri: Option<String> = r.get(12)?;
                    let evidence_str: Option<String> = r.get(13)?;
                    let content_hash: Option<String> = r.get(14)?;
                    let token_estimate: Option<i64> = r.get(15)?;
                    let sensitivity: Option<String> = r.get(16)?;
                    let valid_from: Option<String> = r.get(17)?;
                    let valid_to: Option<String> = r.get(18)?;
                    let ttl_seconds: Option<i64> = r.get(19)?;
                    let ge_id: Option<i64> = r.get(20)?;
                    let supersedes: Option<i64> = r.get(21)?;

                    let evidence: Vec<Value> = evidence_str
                        .as_deref()
                        .and_then(|s| serde_json::from_str(s).ok())
                        .unwrap_or_default();

                    let eff_ge_id = ge_id.unwrap_or(graph_entity_id);
                    let token_est = token_estimate
                        .map(|t| t as usize)
                        .unwrap_or_else(|| estimate_text_tokens(&content));

                    Ok(MemoryItemEnvelope {
                        graph_entity_id: eff_ge_id,
                        sql_id: Some(id),
                        id: format!("memory:{}", eff_ge_id),
                        key,
                        scope,
                        kind: kind.unwrap_or_else(|| "memory".to_string()),
                        title,
                        summary,
                        content,
                        confidence,
                        status: status.unwrap_or_else(|| "active".to_string()),
                        project_id,
                        source_uri,
                        evidence,
                        content_hash,
                        token_estimate: token_est,
                        sensitivity: sensitivity.unwrap_or_else(|| "normal".to_string()),
                        valid_from,
                        valid_to,
                        ttl_seconds,
                        created_at,
                        updated_at,
                        supersedes_graph_entity_id: supersedes,
                    })
                })
                .ok();
            Ok(item)
        })?;

        if let Some(item) = sql_item {
            return Ok(Some(item));
        }

        // Fallback: check graph_entities directly
        if let Ok(ge) = self.get_entity(graph_entity_id) {
            let data = &ge.data;
            let key = data
                .get("key")
                .and_then(|v| v.as_str())
                .unwrap_or(&ge.name)
                .to_string();
            let scope = data
                .get("scope")
                .and_then(|v| v.as_str())
                .unwrap_or("user")
                .to_string();
            let kind = data
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("memory")
                .to_string();
            let title = data
                .get("title")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let summary = data
                .get("summary")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let content = data
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let confidence = data
                .get("confidence")
                .and_then(|v| v.as_f64())
                .unwrap_or(1.0);
            let status = data
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("active")
                .to_string();
            let project_id = data
                .get("project_id")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let source_uri = data
                .get("source_uri")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let evidence: Vec<Value> = data
                .get("evidence")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let content_hash = data
                .get("content_hash")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let token_estimate = data
                .get("token_estimate")
                .and_then(|v| v.as_u64())
                .map(|t| t as usize)
                .unwrap_or_else(|| estimate_text_tokens(&content));
            let sensitivity = data
                .get("sensitivity")
                .and_then(|v| v.as_str())
                .unwrap_or("normal")
                .to_string();
            let valid_from = data
                .get("valid_from")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let valid_to = data
                .get("valid_to")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let ttl_seconds = data.get("ttl_seconds").and_then(|v| v.as_i64());
            let created_at = data
                .get("created_at")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let updated_at = data
                .get("updated_at")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let supersedes = data
                .get("supersedes_graph_entity_id")
                .and_then(|v| v.as_i64());
            let sql_id = data.get("sql_id").and_then(|v| v.as_i64());

            return Ok(Some(MemoryItemEnvelope {
                graph_entity_id,
                sql_id,
                id: format!("memory:{}", graph_entity_id),
                key,
                scope,
                kind,
                title,
                summary,
                content,
                confidence,
                status,
                project_id,
                source_uri,
                evidence,
                content_hash,
                token_estimate,
                sensitivity,
                valid_from,
                valid_to,
                ttl_seconds,
                created_at,
                updated_at,
                supersedes_graph_entity_id: supersedes,
            }));
        }

        Ok(None)
    }

    /// Route memories based on query text, filters, depth, and token budget.
    pub fn route_memory(
        &self,
        query: &str,
        filters: &MemoryRouteFilters,
        budget: usize,
        depth: MemoryRouteDepth,
    ) -> Result<MemoryRouteResponse> {
        self.runtime.record_memory_query();

        // 1. Load candidate memories from SQL
        let candidates: Vec<MemoryItemEnvelope> = self.with_raw_connection(|conn| {
            let mut sql = String::from(
                "SELECT id, key, scope, content, confidence, project_id, created_at, updated_at,
                        kind, title, summary, status, source_uri, evidence_json, content_hash,
                        token_estimate, sensitivity, valid_from, valid_to, ttl_seconds,
                        graph_entity_id, supersedes_graph_entity_id
                 FROM memory_entries
                 WHERE (sensitivity IS NULL OR LOWER(sensitivity) != 'secret')",
            );

            if let Some(ref st) = filters.status {
                sql.push_str(&format!(" AND status = '{}'", st.replace('\'', "''")));
            } else {
                sql.push_str(" AND (status IS NULL OR status != 'superseded')");
            }

            if let Some(ref p) = filters.project {
                sql.push_str(&format!(" AND project_id = '{}'", p.replace('\'', "''")));
            }

            if let Some(floor) = filters.confidence_floor {
                sql.push_str(&format!(" AND confidence >= {}", floor));
            }

            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map([], |r| {
                let id: i64 = r.get(0)?;
                let key: String = r.get(1)?;
                let scope: String = r.get(2)?;
                let content: String = r.get(3)?;
                let confidence: f64 = r.get(4)?;
                let project_id: Option<String> = r.get(5)?;
                let created_at: String = r.get(6)?;
                let updated_at: Option<String> = r.get(7)?;
                let kind: Option<String> = r.get(8)?;
                let title: Option<String> = r.get(9)?;
                let summary: Option<String> = r.get(10)?;
                let status: Option<String> = r.get(11)?;
                let source_uri: Option<String> = r.get(12)?;
                let evidence_str: Option<String> = r.get(13)?;
                let content_hash: Option<String> = r.get(14)?;
                let token_estimate: Option<i64> = r.get(15)?;
                let sensitivity: Option<String> = r.get(16)?;
                let valid_from: Option<String> = r.get(17)?;
                let valid_to: Option<String> = r.get(18)?;
                let ttl_seconds: Option<i64> = r.get(19)?;
                let ge_id: Option<i64> = r.get(20)?;
                let supersedes: Option<i64> = r.get(21)?;

                let evidence: Vec<Value> = evidence_str
                    .as_deref()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or_default();

                let eff_ge_id = ge_id.unwrap_or(id);
                let token_est = token_estimate
                    .map(|t| t as usize)
                    .unwrap_or_else(|| estimate_text_tokens(&content));

                Ok(MemoryItemEnvelope {
                    graph_entity_id: eff_ge_id,
                    sql_id: Some(id),
                    id: format!("memory:{}", eff_ge_id),
                    key,
                    scope,
                    kind: kind.unwrap_or_else(|| "memory".to_string()),
                    title,
                    summary,
                    content,
                    confidence,
                    status: status.unwrap_or_else(|| "active".to_string()),
                    project_id,
                    source_uri,
                    evidence,
                    content_hash,
                    token_estimate: token_est,
                    sensitivity: sensitivity.unwrap_or_else(|| "normal".to_string()),
                    valid_from,
                    valid_to,
                    ttl_seconds,
                    created_at,
                    updated_at,
                    supersedes_graph_entity_id: supersedes,
                })
            })?;

            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })?;

        // 2. Filter candidates by kind and evidence requirements
        let query_tokens: Vec<String> = query
            .to_lowercase()
            .split_whitespace()
            .map(str::to_string)
            .filter(|t| !t.is_empty())
            .collect();

        let mut scored_items: Vec<(f32, MemoryItemEnvelope)> = Vec::new();

        for item in candidates {
            // Sensitivity invariant
            if item.sensitivity.eq_ignore_ascii_case("secret") {
                continue;
            }

            // Kinds filter
            if let Some(ref kinds) = filters.kinds {
                if !kinds.iter().any(|k| k.eq_ignore_ascii_case(&item.kind)) {
                    continue;
                }
            }

            // Evidence filter
            let has_evidence = !item.evidence.is_empty();
            if filters.evidence_required && !has_evidence {
                continue;
            }

            // Score calculation
            let score = if query_tokens.is_empty() || query == "*" {
                item.confidence as f32
            } else {
                let searchable_text = format!(
                    "{} {} {} {}",
                    item.key.to_lowercase(),
                    item.title.as_deref().unwrap_or_default().to_lowercase(),
                    item.summary.as_deref().unwrap_or_default().to_lowercase(),
                    item.content.to_lowercase()
                );
                let matches = query_tokens
                    .iter()
                    .filter(|t| searchable_text.contains(*t))
                    .count();
                if matches == 0 {
                    continue; // Skip zero-match candidates when query is specified
                }
                (matches as f32 / query_tokens.len() as f32).min(1.0)
            };

            scored_items.push((score, item));
        }

        // 3. Sort by score DESC, then updated_at / created_at DESC
        scored_items.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.1.updated_at.cmp(&a.1.updated_at))
                .then_with(|| b.1.created_at.cmp(&a.1.created_at))
        });

        // 4. Token budgeting and depth transformation
        let mut budget_used = 0usize;
        let mut items = Vec::new();
        let mut omitted = 0usize;

        for (score, item) in scored_items {
            let evidence_state = if item.evidence.is_empty() {
                "evidence_missing".to_string()
            } else {
                "present".to_string()
            };

            let (excerpt, content, token_cost) = match depth {
                MemoryRouteDepth::Summary => {
                    let cost = estimate_text_tokens(&format!(
                        "{} {} {} {}",
                        item.key,
                        item.kind,
                        item.title.as_deref().unwrap_or_default(),
                        item.summary.as_deref().unwrap_or_default()
                    ));
                    (None, None, cost)
                }
                MemoryRouteDepth::Bundle => {
                    let excerpt_text = item.content.chars().take(200).collect::<String>();
                    let cost = estimate_text_tokens(&format!(
                        "{} {} {} {} {}",
                        item.key,
                        item.kind,
                        item.title.as_deref().unwrap_or_default(),
                        item.summary.as_deref().unwrap_or_default(),
                        excerpt_text
                    ));
                    (Some(excerpt_text), None, cost)
                }
                MemoryRouteDepth::Expand => {
                    let cost = estimate_text_tokens(&format!(
                        "{} {} {} {} {}",
                        item.key,
                        item.kind,
                        item.title.as_deref().unwrap_or_default(),
                        item.summary.as_deref().unwrap_or_default(),
                        item.content
                    ));
                    (None, Some(item.content.clone()), cost)
                }
            };

            if budget_used + token_cost <= budget {
                budget_used += token_cost;
                items.push(MemoryRouteItem {
                    id: item.id,
                    graph_entity_id: item.graph_entity_id,
                    kind: item.kind,
                    title: item.title,
                    summary: item.summary,
                    excerpt,
                    content,
                    score,
                    evidence_state,
                    token_cost,
                    expandable: true,
                });
            } else {
                omitted += 1;
            }
        }

        Ok(MemoryRouteResponse {
            query: query.to_string(),
            project: filters.project.clone(),
            budget_tokens: budget,
            budget_used,
            depth,
            items,
            omitted,
            warnings: vec![],
        })
    }

    /// Expand memory items by graph entity IDs up to token budget.
    pub fn expand_memory(&self, ids: &[String], budget: usize) -> Result<MemoryExpandResponse> {
        let mut budget_used = 0usize;
        let mut items = Vec::new();
        let mut omitted = 0usize;
        let mut warnings = Vec::new();

        for raw_id in ids {
            let clean = raw_id.trim();
            let parsed_id: Option<i64> = if let Some(stripped) = clean.strip_prefix("memory:") {
                stripped.parse().ok()
            } else {
                clean.parse().ok()
            };

            let ge_id = match parsed_id {
                Some(id) => id,
                None => {
                    warnings.push(format!("Invalid memory ID format: '{}'", raw_id));
                    continue;
                }
            };

            match self.get_memory_item_by_graph_id(ge_id)? {
                Some(item) => {
                    if item.sensitivity.eq_ignore_ascii_case("secret") {
                        continue;
                    }
                    let cost = item.token_estimate;
                    if budget_used + cost <= budget {
                        budget_used += cost;
                        items.push(item);
                    } else {
                        omitted += 1;
                    }
                }
                None => {
                    warnings.push(format!("Memory item with id '{}' not found", raw_id));
                }
            }
        }

        Ok(MemoryExpandResponse {
            budget_tokens: budget,
            budget_used,
            items,
            omitted,
            warnings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{AtheneumGraph, EntityType};
    use serde_json::json;

    fn sample_params(key: &str, content: &str) -> UpsertMemoryItemParams {
        UpsertMemoryItemParams {
            key: key.to_string(),
            content: content.to_string(),
            scope: "project".to_string(),
            kind: "decision".to_string(),
            title: Some("SQLite Persistence".to_string()),
            summary: Some("Chose SQLite for persistence".to_string()),
            confidence: 1.0,
            project_id: Some("atheneum".to_string()),
            status: "active".to_string(),
            source_uri: Some("specs/memory-router/TECH.md".to_string()),
            evidence: vec![json!({"type": "spec", "ref": "TECH.md"})],
            content_hash: None,
            token_estimate: None,
            sensitivity: "normal".to_string(),
            valid_from: None,
            valid_to: None,
            ttl_seconds: None,
            tags: Some(vec!["storage".to_string(), "sqlite".to_string()]),
            conflict_strategy: ConflictStrategy::Revision,
        }
    }

    #[test]
    fn test_upsert_memory_item_registration_and_dual_write() {
        let graph = AtheneumGraph::open_in_memory().expect("open");
        let params = sample_params("arch_choice", "Use SQLite for all graph state");

        let outcome = graph.upsert_memory_item(&params).expect("upsert");
        let (ge_id, sql_id, hash) = match outcome {
            UpsertMemoryOutcome::Created {
                graph_entity_id,
                sql_id,
                content_hash,
            } => (graph_entity_id, sql_id, content_hash),
            other => panic!("expected Created outcome, got {:?}", other),
        };

        assert!(ge_id > 0);
        assert!(sql_id > 0);
        assert!(!hash.is_empty());

        // Verify GraphEntity
        let entity = graph.get_entity(ge_id).expect("get_entity");
        assert_eq!(entity.kind, EntityType::Memory.as_str());
        assert_eq!(entity.name, "arch_choice");
        assert_eq!(
            entity.data.get("kind").and_then(|v| v.as_str()),
            Some("decision")
        );
        assert_eq!(
            entity.data.get("sql_id").and_then(|v| v.as_i64()),
            Some(sql_id)
        );
        assert_eq!(
            entity.data.get("content_hash").and_then(|v| v.as_str()),
            Some(hash.as_str())
        );

        // Verify SQL row in memory_entries
        let (sql_key, sql_content, sql_ge_id, sql_status): (String, String, i64, String) = graph
            .with_raw_connection(|conn| {
                Ok(conn.query_row(
                    "SELECT key, content, graph_entity_id, status FROM memory_entries WHERE id = ?1",
                    params![sql_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )?)
            })
            .expect("query sql row");

        assert_eq!(sql_key, "arch_choice");
        assert_eq!(sql_content, "Use SQLite for all graph state");
        assert_eq!(sql_ge_id, ge_id);
        assert_eq!(sql_status, "active");
    }

    #[test]
    fn test_upsert_conflict_safety_same_hash_idempotent_replay() {
        let graph = AtheneumGraph::open_in_memory().expect("open");
        let params = sample_params("replay_test", "Constant content across calls");

        let first = graph.upsert_memory_item(&params).expect("first upsert");
        let (ge1, sql1, hash1) = match first {
            UpsertMemoryOutcome::Created {
                graph_entity_id,
                sql_id,
                content_hash,
            } => (graph_entity_id, sql_id, content_hash),
            other => panic!("expected Created, got {:?}", other),
        };

        // Replay with identical params
        let second = graph.upsert_memory_item(&params).expect("second upsert");
        match second {
            UpsertMemoryOutcome::Unchanged {
                graph_entity_id,
                sql_id,
                content_hash,
            } => {
                assert_eq!(graph_entity_id, ge1);
                assert_eq!(sql_id, sql1);
                assert_eq!(content_hash, hash1);
            }
            other => panic!("expected Unchanged, got {:?}", other),
        }

        // Verify row count remains 1 in memory_entries
        let row_count: i64 = graph
            .with_raw_connection(|conn| {
                Ok(conn.query_row(
                    "SELECT count(*) FROM memory_entries WHERE key = 'replay_test'",
                    [],
                    |r| r.get(0),
                )?)
            })
            .expect("count");
        assert_eq!(
            row_count, 1,
            "idempotent replay must not create duplicate SQL rows"
        );
    }

    #[test]
    fn test_upsert_conflict_safety_different_hash_conflict_outcome() {
        let graph = AtheneumGraph::open_in_memory().expect("open");
        let params = sample_params("conflict_key", "Initial architecture choice: SQLite");
        let first = graph.upsert_memory_item(&params).expect("initial upsert");
        let ge1 = match first {
            UpsertMemoryOutcome::Created {
                graph_entity_id, ..
            } => graph_entity_id,
            other => panic!("expected Created, got {:?}", other),
        };

        // Same logical key (key, scope, project_id) but different content, with ConflictStrategy::Conflict
        let mut modified = sample_params("conflict_key", "Conflicting choice: PostgreSQL");
        modified.conflict_strategy = ConflictStrategy::Conflict;

        let outcome = graph
            .upsert_memory_item(&modified)
            .expect("upsert on conflict");
        match outcome {
            UpsertMemoryOutcome::Conflict {
                existing_graph_entity_id,
                existing_content_hash,
                incoming_content_hash,
                ..
            } => {
                assert_eq!(existing_graph_entity_id, ge1);
                assert_ne!(existing_content_hash, incoming_content_hash);
            }
            other => panic!("expected Conflict outcome, got {:?}", other),
        }

        // Ensure original content was NOT overwritten
        let original_content: String = graph
            .with_raw_connection(|conn| {
                Ok(conn.query_row(
                    "SELECT content FROM memory_entries WHERE graph_entity_id = ?1",
                    params![ge1],
                    |r| r.get(0),
                )?)
            })
            .expect("get original");
        assert_eq!(original_content, "Initial architecture choice: SQLite");
    }

    #[test]
    fn test_upsert_conflict_safety_different_hash_revision_created() {
        let graph = AtheneumGraph::open_in_memory().expect("open");
        let params = sample_params("revision_key", "Revision 1 content");
        let first = graph.upsert_memory_item(&params).expect("first");
        let (ge1, _sql1, _hash1) = match first {
            UpsertMemoryOutcome::Created {
                graph_entity_id,
                sql_id,
                content_hash,
            } => (graph_entity_id, sql_id, content_hash),
            other => panic!("expected Created, got {:?}", other),
        };

        // Incoming with different content, default Revision strategy
        let mut revision_params =
            sample_params("revision_key", "Revision 2 content: updated facts");
        revision_params.conflict_strategy = ConflictStrategy::Revision;

        let second = graph.upsert_memory_item(&revision_params).expect("second");
        let (ge2, sql2, supersedes) = match second {
            UpsertMemoryOutcome::RevisionCreated {
                graph_entity_id,
                sql_id,
                supersedes_graph_entity_id,
                ..
            } => (graph_entity_id, sql_id, supersedes_graph_entity_id),
            other => panic!("expected RevisionCreated, got {:?}", other),
        };

        assert_ne!(ge1, ge2);
        assert_eq!(supersedes, ge1);

        // Check original row still exists and was marked superseded
        let (status1, content1): (String, String) = graph
            .with_raw_connection(|conn| {
                Ok(conn.query_row(
                    "SELECT status, content FROM memory_entries WHERE graph_entity_id = ?1",
                    params![ge1],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?)
            })
            .expect("original row");
        assert_eq!(status1, "superseded");
        assert_eq!(content1, "Revision 1 content", "original content preserved");

        // Check new row has supersedes_graph_entity_id
        let (status2, supersedes2, content2): (String, Option<i64>, String) = graph
            .with_raw_connection(|conn| {
                Ok(conn.query_row(
                    "SELECT status, supersedes_graph_entity_id, content FROM memory_entries WHERE id = ?1",
                    params![sql2],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )?)
            })
            .expect("new row");
        assert_eq!(status2, "active");
        assert_eq!(supersedes2, Some(ge1));
        assert_eq!(content2, "Revision 2 content: updated facts");

        // Check graph entity supersession metadata
        let old_entity = graph.get_entity(ge1).expect("old ge");
        assert!(old_entity.data.get("superseded_at").is_some());
        assert_eq!(
            old_entity
                .data
                .get("superseded_by")
                .and_then(|v| v.as_i64()),
            Some(ge2)
        );
    }

    #[test]
    fn test_get_memory_item_by_graph_id() {
        let graph = AtheneumGraph::open_in_memory().expect("open");
        let params = sample_params("get_by_id_test", "Full content to retrieve by graph ID");
        let outcome = graph.upsert_memory_item(&params).expect("upsert");
        let ge_id = match outcome {
            UpsertMemoryOutcome::Created {
                graph_entity_id, ..
            } => graph_entity_id,
            other => panic!("expected Created, got {:?}", other),
        };

        let env = graph
            .get_memory_item_by_graph_id(ge_id)
            .expect("get")
            .expect("found");
        assert_eq!(env.graph_entity_id, ge_id);
        assert_eq!(env.id, format!("memory:{}", ge_id));
        assert_eq!(env.key, "get_by_id_test");
        assert_eq!(env.content, "Full content to retrieve by graph ID");
        assert_eq!(env.title.as_deref(), Some("SQLite Persistence"));
        assert_eq!(env.status, "active");
        assert_eq!(env.evidence.len(), 1);

        // Non-existent ID returns None
        let missing = graph
            .get_memory_item_by_graph_id(999999)
            .expect("missing check");
        assert!(missing.is_none());
    }

    #[test]
    fn test_route_memory_summary_depth() {
        let graph = AtheneumGraph::open_in_memory().expect("open");

        // Item 1: with evidence
        let mut p1 = sample_params("route_test_1", "Kernel property verification routine");
        p1.title = Some("Kernel Verification".to_string());
        p1.summary = Some("Math properties of recurrent kernel verified".to_string());
        p1.evidence = vec![json!({"file": "verify_math_formulas.py"})];
        graph.upsert_memory_item(&p1).expect("p1");

        // Item 2: without evidence (evidence_missing)
        let mut p2 = sample_params("route_test_2", "Informal note about kernel speed");
        p2.title = Some("Kernel Speed Note".to_string());
        p2.summary = Some("Informal observations on memory bandwidth".to_string());
        p2.evidence = vec![];
        graph.upsert_memory_item(&p2).expect("p2");

        let res = graph
            .route_memory(
                "Kernel",
                &MemoryRouteFilters::default(),
                1000,
                MemoryRouteDepth::Summary,
            )
            .expect("route");

        assert_eq!(res.depth, MemoryRouteDepth::Summary);
        assert_eq!(res.items.len(), 2);

        let item1 = res
            .items
            .iter()
            .find(|i| i.title.as_deref() == Some("Kernel Verification"))
            .expect("item1");
        assert_eq!(item1.evidence_state, "present");
        assert!(
            item1.excerpt.is_none(),
            "summary depth must not have excerpt"
        );
        assert!(
            item1.content.is_none(),
            "summary depth must not have content"
        );
        assert!(item1.expandable);

        let item2 = res
            .items
            .iter()
            .find(|i| i.title.as_deref() == Some("Kernel Speed Note"))
            .expect("item2");
        assert_eq!(item2.evidence_state, "evidence_missing");
    }

    #[test]
    fn test_route_and_expand_budget_truncation_omitted_count() {
        let graph = AtheneumGraph::open_in_memory().expect("open");

        let mut ids = Vec::new();
        for i in 1..=5 {
            let mut p = sample_params(
                &format!("budget_key_{}", i),
                &format!(
                    "This is candidate item {} with substantial content for budgeting tests.",
                    i
                ),
            );
            p.title = Some(format!("Item {}", i));
            p.summary = Some(format!("Summary of candidate item {}", i));
            let outcome = graph.upsert_memory_item(&p).expect("insert");
            if let UpsertMemoryOutcome::Created {
                graph_entity_id, ..
            } = outcome
            {
                ids.push(format!("memory:{}", graph_entity_id));
            }
        }
        assert_eq!(ids.len(), 5);

        // Route with tight budget (50 tokens) — each item summary costs ~25-35 tokens
        let route_res = graph
            .route_memory(
                "budget",
                &MemoryRouteFilters::default(),
                50,
                MemoryRouteDepth::Summary,
            )
            .expect("route");
        assert!(
            route_res.items.len() < 5,
            "items must be truncated to budget"
        );
        assert_eq!(
            route_res.items.len() + route_res.omitted,
            5,
            "omitted count must accurately reflect untruncated candidates"
        );
        assert!(route_res.budget_used <= 50);

        // Expand with tight budget (40 tokens)
        let expand_res = graph.expand_memory(&ids, 40).expect("expand");
        assert!(
            expand_res.items.len() < 5,
            "expanded items must be truncated to budget"
        );
        assert_eq!(
            expand_res.items.len() + expand_res.omitted,
            5,
            "expand omitted count must account for all requested items"
        );
        assert!(expand_res.budget_used <= 40);
    }

    #[test]
    fn test_expand_memory_explicit_ids() {
        let graph = AtheneumGraph::open_in_memory().expect("open");
        let p1 = sample_params("expand_1", "Detailed body for item one");
        let p2 = sample_params("expand_2", "Detailed body for item two");

        let outcome1 = graph.upsert_memory_item(&p1).expect("p1");
        let outcome2 = graph.upsert_memory_item(&p2).expect("p2");

        let ge1 = match outcome1 {
            UpsertMemoryOutcome::Created {
                graph_entity_id, ..
            } => graph_entity_id,
            other => panic!("expected Created, got {:?}", other),
        };
        let ge2 = match outcome2 {
            UpsertMemoryOutcome::Created {
                graph_entity_id, ..
            } => graph_entity_id,
            other => panic!("expected Created, got {:?}", other),
        };

        // Expand both using explicit IDs ("memory:<id>" and bare "<id>")
        let res = graph
            .expand_memory(&[format!("memory:{}", ge1), format!("{}", ge2)], 2000)
            .expect("expand");

        assert_eq!(res.items.len(), 2);
        assert_eq!(res.omitted, 0);

        let item1 = res
            .items
            .iter()
            .find(|i| i.graph_entity_id == ge1)
            .expect("item1");
        assert_eq!(item1.content, "Detailed body for item one");

        let item2 = res
            .items
            .iter()
            .find(|i| i.graph_entity_id == ge2)
            .expect("item2");
        assert_eq!(item2.content, "Detailed body for item two");
    }

    #[test]
    fn test_secret_rejection_at_ingest_and_exclusion() {
        let graph = AtheneumGraph::open_in_memory().expect("open");

        // Ingest of secret sensitivity must be rejected
        let mut secret_params =
            sample_params("secret_cred", "AWS_SECRET_ACCESS_KEY=AKIAIOSFODNN7EXAMPLE");
        secret_params.sensitivity = "secret".to_string();

        let result = graph.upsert_memory_item(&secret_params);
        assert!(
            result.is_err(),
            "upserting sensitivity=secret must be rejected at ingest"
        );

        // Normal memory passes
        let normal_params = sample_params("public_fact", "Public non-secret memory");
        let outcome = graph
            .upsert_memory_item(&normal_params)
            .expect("normal upsert");
        let ge_id = match outcome {
            UpsertMemoryOutcome::Created {
                graph_entity_id, ..
            } => graph_entity_id,
            other => panic!("expected Created, got {:?}", other),
        };

        // If a secret record somehow existed in DB (e.g. legacy), route and expand must exclude it
        graph
            .with_raw_connection(|conn| {
                Ok(conn.execute(
                    "INSERT INTO memory_entries (key, scope, content, sensitivity, status, created_at)
                     VALUES ('injected_secret', 'project', 'secret-val', 'secret', 'active', '2026-11-03T00:00:00Z')",
                    [],
                )?)
            })
            .expect("inject legacy secret");

        let route_res = graph
            .route_memory(
                "secret",
                &MemoryRouteFilters::default(),
                1000,
                MemoryRouteDepth::Summary,
            )
            .expect("route");
        assert!(
            route_res.items.iter().all(|i| i.kind != "secret"),
            "secret records must never be returned by route"
        );

        let expand_res = graph
            .expand_memory(&[format!("memory:{}", ge_id)], 1000)
            .expect("expand");
        assert!(expand_res.items.iter().all(|i| i.sensitivity != "secret"));
    }

    #[test]
    fn test_existing_memory_methods_and_cli_unchanged() {
        let graph = AtheneumGraph::open_in_memory().expect("open");

        // Existing store_memory
        let id = graph
            .store_memory("legacy_key", "Legacy content", "user", 1.0, None, None)
            .expect("store_memory");
        assert!(id > 0);

        // Existing query_memory
        let queried = graph
            .query_memory("legacy_key", Some("user"), None, false)
            .expect("query_memory");
        assert_eq!(queried.len(), 1);
        assert_eq!(queried[0].name, "legacy_key");

        // Existing list_memory
        let listed = graph.list_memory(Some("user"), None).expect("list_memory");
        assert!(!listed.is_empty());

        // Existing update_memory
        let patch = crate::MemoryPatch {
            content: Some("Updated legacy content".to_string()),
            importance: None,
            tags: None,
            replace_tags: false,
        };
        let updated_id = graph.update_memory(id, &patch).expect("update_memory");
        assert_eq!(updated_id, id);
    }
}
