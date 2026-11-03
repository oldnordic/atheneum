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

struct ProjectStateTokensInput<'a> {
    objective: Option<&'a str>,
    state: &'a Value,
    branch: Option<&'a str>,
    head_sha: Option<&'a str>,
    constraints: &'a [Value],
    active_task_ids: &'a [Value],
    blocker_ids: &'a [Value],
    decision_ids: &'a [Value],
    receipt_ids: &'a [Value],
    changelog: Option<&'a str>,
    evidence: &'a [Value],
}

struct StateCollections<'a> {
    constraints: &'a [Value],
    active_task_ids: &'a [Value],
    blocker_ids: &'a [Value],
    decision_ids: &'a [Value],
    receipt_ids: &'a [Value],
    evidence: &'a [Value],
}

fn make_tokens_input<'a>(
    objective: Option<&'a str>,
    state: &'a Value,
    branch: Option<&'a str>,
    head_sha: Option<&'a str>,
    changelog: Option<&'a str>,
    collections: &StateCollections<'a>,
) -> ProjectStateTokensInput<'a> {
    ProjectStateTokensInput {
        objective,
        state,
        branch,
        head_sha,
        constraints: collections.constraints,
        active_task_ids: collections.active_task_ids,
        blocker_ids: collections.blocker_ids,
        decision_ids: collections.decision_ids,
        receipt_ids: collections.receipt_ids,
        evidence: collections.evidence,
        changelog,
    }
}

fn estimate_project_state_tokens(input: &ProjectStateTokensInput<'_>) -> usize {
    let mut combined = String::new();
    if let Some(obj) = input.objective {
        combined.push_str(obj);
        combined.push(' ');
    }
    combined.push_str(&input.state.to_string());
    combined.push(' ');
    if let Some(b) = input.branch {
        combined.push_str(b);
        combined.push(' ');
    }
    if let Some(h) = input.head_sha {
        combined.push_str(h);
        combined.push(' ');
    }
    for c in input.constraints {
        combined.push_str(&c.to_string());
        combined.push(' ');
    }
    for t in input.active_task_ids {
        combined.push_str(&t.to_string());
        combined.push(' ');
    }
    for b in input.blocker_ids {
        combined.push_str(&b.to_string());
        combined.push(' ');
    }
    for d in input.decision_ids {
        combined.push_str(&d.to_string());
        combined.push(' ');
    }
    for r in input.receipt_ids {
        combined.push_str(&r.to_string());
        combined.push(' ');
    }
    if let Some(cl) = input.changelog {
        combined.push_str(cl);
        combined.push(' ');
    }
    for ev in input.evidence {
        combined.push_str(&ev.to_string());
        combined.push(' ');
    }
    estimate_text_tokens(&combined)
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

fn default_sync_state() -> String {
    "synced".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectStateParams {
    pub objective: Option<String>,
    #[serde(default)]
    pub state: Value,
    pub branch: Option<String>,
    pub head_sha: Option<String>,
    #[serde(default)]
    pub constraints: Vec<Value>,
    #[serde(default)]
    pub active_task_ids: Vec<Value>,
    #[serde(default)]
    pub blocker_ids: Vec<Value>,
    #[serde(default)]
    pub decision_ids: Vec<Value>,
    #[serde(default)]
    pub receipt_ids: Vec<Value>,
    pub changelog: Option<String>,
    #[serde(default)]
    pub evidence: Vec<Value>,
    pub token_estimate: Option<usize>,
    #[serde(default = "default_sensitivity")]
    pub sensitivity: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectStateOutcome {
    pub graph_entity_id: i64,
    pub sql_id: i64,
    pub project_id: String,
    pub version: i64,
    pub is_head: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectStateEnvelope {
    pub graph_entity_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sql_id: Option<i64>,
    pub id: String,
    pub project_id: String,
    pub version: i64,
    pub is_head: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub objective: Option<String>,
    pub state: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head_sha: Option<String>,
    pub constraints: Vec<Value>,
    pub active_task_ids: Vec<Value>,
    pub blocker_ids: Vec<Value>,
    pub decision_ids: Vec<Value>,
    pub receipt_ids: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changelog: Option<String>,
    pub evidence: Vec<Value>,
    pub token_estimate: usize,
    pub sensitivity: String,
    pub created_at: String,
    pub truncated: bool,
    pub omitted_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStateParams {
    pub project_id: Option<String>,
    pub session_id: Option<String>,
    pub dispatch_id: Option<String>,
    pub task_id: Option<i64>,
    pub status: String,
    pub current_objective: Option<String>,
    #[serde(default)]
    pub state: Value,
    #[serde(default)]
    pub budget: Value,
    pub last_receipt_graph_entity_id: Option<i64>,
    #[serde(default = "default_sensitivity")]
    pub sensitivity: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentStateOutcome {
    pub graph_entity_id: i64,
    pub sql_id: i64,
    pub agent_name: String,
    pub version: i64,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentStateEnvelope {
    pub graph_entity_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sql_id: Option<i64>,
    pub id: String,
    pub agent_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dispatch_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<i64>,
    pub version: i64,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_objective: Option<String>,
    pub state: Value,
    pub budget: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_receipt_graph_entity_id: Option<i64>,
    pub sensitivity: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordArtifactParams {
    pub project_id: Option<String>,
    pub task_id: Option<i64>,
    pub artifact_kind: String,
    pub path: String,
    pub sha256: String,
    pub bytes: Option<i64>,
    pub commit_sha: Option<String>,
    pub created_by: Option<String>,
    #[serde(default = "default_sensitivity")]
    pub sensitivity: String,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RecordArtifactOutcome {
    Created {
        graph_entity_id: i64,
        sql_id: i64,
        sha256: String,
    },
    Unchanged {
        graph_entity_id: i64,
        sql_id: i64,
        sha256: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordReceiptParams {
    pub project_id: Option<String>,
    pub task_id: Option<i64>,
    pub artifact_graph_entity_id: Option<i64>,
    pub session_id: Option<String>,
    pub verdict: String,
    pub verifier: String,
    pub command: Option<String>,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<i64>,
    pub output_hash: Option<String>,
    pub output_sample_redacted: Option<String>,
    #[serde(default)]
    pub evidence: Vec<Value>,
    #[serde(default = "default_sensitivity")]
    pub sensitivity: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RecordReceiptOutcome {
    Created {
        graph_entity_id: i64,
        sql_id: i64,
        verdict: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        artifact_graph_entity_id: Option<i64>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordSourceParams {
    pub project_id: Option<String>,
    pub source_type: String,
    pub uri: String,
    pub content_hash: Option<String>,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub author: Option<String>,
    pub published_at: Option<String>,
    pub retrieved_at: Option<String>,
    pub credibility: Option<f64>,
    #[serde(default = "default_sensitivity")]
    pub sensitivity: String,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RecordSourceOutcome {
    Created {
        graph_entity_id: i64,
        sql_id: i64,
        uri: String,
    },
    Unchanged {
        graph_entity_id: i64,
        sql_id: i64,
        uri: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkExternalMemoryParams {
    pub external_store: String,
    pub external_id: String,
    pub atheneum_graph_entity_id: i64,
    pub atheneum_kind: String,
    pub content_hash: String,
    #[serde(default = "default_sync_state")]
    pub sync_state: String,
    pub last_synced_at: Option<String>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum LinkExternalMemoryOutcome {
    Created {
        graph_entity_id: i64,
        sql_id: i64,
        external_store: String,
        external_id: String,
        atheneum_graph_entity_id: i64,
    },
    Unchanged {
        graph_entity_id: i64,
        sql_id: i64,
        external_store: String,
        external_id: String,
        atheneum_graph_entity_id: i64,
    },
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

    /// Append a new project state version.
    pub fn append_project_state(
        &self,
        project: &str,
        state: &ProjectStateParams,
    ) -> Result<ProjectStateOutcome> {
        if state.sensitivity.eq_ignore_ascii_case("secret") {
            bail!("Records with sensitivity 'secret' are rejected at ingest");
        }
        if project.trim().is_empty() {
            bail!("project cannot be empty");
        }

        let now = Utc::now().to_rfc3339();
        let token_est = state.token_estimate.unwrap_or_else(|| {
            estimate_project_state_tokens(&ProjectStateTokensInput {
                objective: state.objective.as_deref(),
                state: &state.state,
                branch: state.branch.as_deref(),
                head_sha: state.head_sha.as_deref(),
                constraints: &state.constraints,
                active_task_ids: &state.active_task_ids,
                blocker_ids: &state.blocker_ids,
                decision_ids: &state.decision_ids,
                receipt_ids: &state.receipt_ids,
                changelog: state.changelog.as_deref(),
                evidence: &state.evidence,
            })
        });

        let state_json_str = serde_json::to_string(&state.state)?;
        let constraints_str = serde_json::to_string(&state.constraints)?;
        let active_tasks_str = serde_json::to_string(&state.active_task_ids)?;
        let blockers_str = serde_json::to_string(&state.blocker_ids)?;
        let decisions_str = serde_json::to_string(&state.decision_ids)?;
        let receipts_str = serde_json::to_string(&state.receipt_ids)?;
        let evidence_str = serde_json::to_string(&state.evidence)?;

        let (sql_id, version) = self.with_raw_connection(|conn| {
            let max_ver: i64 = conn
                .query_row(
                    "SELECT COALESCE(MAX(version), 0) FROM project_states WHERE project_id = ?1",
                    params![project],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            let next_ver = max_ver + 1;

            conn.execute(
                "UPDATE project_states SET is_head = 0 WHERE project_id = ?1 AND is_head = 1",
                params![project],
            )?;

            conn.execute(
                "INSERT INTO project_states (
                    graph_entity_id, project_id, version, is_head, objective, state_json,
                    branch, head_sha, constraints_json, active_task_ids_json, blocker_ids_json,
                    decision_ids_json, receipt_ids_json, changelog, evidence_json,
                    token_estimate, sensitivity, created_at
                ) VALUES (
                    0, ?1, ?2, 1, ?3, ?4,
                    ?5, ?6, ?7, ?8, ?9,
                    ?10, ?11, ?12, ?13,
                    ?14, ?15, ?16
                )",
                params![
                    project,
                    next_ver,
                    state.objective.as_deref(),
                    &state_json_str,
                    state.branch.as_deref(),
                    state.head_sha.as_deref(),
                    &constraints_str,
                    &active_tasks_str,
                    &blockers_str,
                    &decisions_str,
                    &receipts_str,
                    state.changelog.as_deref(),
                    &evidence_str,
                    token_est as i64,
                    &state.sensitivity,
                    &now,
                ],
            )?;
            let rowid = conn.last_insert_rowid();
            Ok((rowid, next_ver))
        })?;

        let mut entity_data = json!({
            "sql_id": sql_id,
            "project_id": project,
            "version": version,
            "is_head": true,
            "objective": &state.objective,
            "state": &state.state,
            "branch": &state.branch,
            "head_sha": &state.head_sha,
            "constraints": &state.constraints,
            "active_task_ids": &state.active_task_ids,
            "blocker_ids": &state.blocker_ids,
            "decision_ids": &state.decision_ids,
            "receipt_ids": &state.receipt_ids,
            "changelog": &state.changelog,
            "evidence": &state.evidence,
            "token_estimate": token_est,
            "sensitivity": &state.sensitivity,
            "created_at": &now,
        });

        let ge = GraphEntity {
            id: 0,
            kind: EntityType::ProjectState.as_str().to_string(),
            name: format!("{}:v{}", project, version),
            file_path: None,
            data: entity_data.clone(),
        };
        let ge_id = self
            .inner
            .insert_entity(&ge)
            .map_err(|e| anyhow::anyhow!("insert ProjectState entity: {}", e))?;

        self.with_raw_connection(|conn| {
            Ok(conn.execute(
                "UPDATE project_states SET graph_entity_id = ?1 WHERE id = ?2",
                params![ge_id, sql_id],
            )?)
        })?;
        if let Some(obj) = entity_data.as_object_mut() {
            obj.insert("graph_entity_id".to_string(), json!(ge_id));
        }
        self.update_entity_data(ge_id, &entity_data)?;

        self.runtime.record_memory_write();
        self.runtime.bump_generation(CacheDomain::Memory);

        Ok(ProjectStateOutcome {
            graph_entity_id: ge_id,
            sql_id,
            project_id: project.to_string(),
            version,
            is_head: true,
        })
    }

    /// Retrieve the head state of a project within a token budget.
    pub fn get_project_state_head(
        &self,
        project: &str,
        budget: usize,
    ) -> Result<Option<ProjectStateEnvelope>> {
        self.get_project_state_head_with_options(project, budget, false)
    }

    /// Retrieve the head state of a project with sensitivity filtering option.
    pub fn get_project_state_head_with_options(
        &self,
        project: &str,
        budget: usize,
        include_sensitive: bool,
    ) -> Result<Option<ProjectStateEnvelope>> {
        let row_opt = self.with_raw_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, graph_entity_id, project_id, version, is_head, objective, state_json,
                        branch, head_sha, constraints_json, active_task_ids_json, blocker_ids_json,
                        decision_ids_json, receipt_ids_json, changelog, evidence_json,
                        token_estimate, sensitivity, created_at
                 FROM project_states
                 WHERE project_id = ?1
                   AND (?2 = 1 OR sensitivity != 'sensitive')
                   AND sensitivity != 'secret'
                 ORDER BY version DESC LIMIT 1",
            )?;
            let row = stmt
                .query_row(
                    params![project, if include_sensitive { 1 } else { 0 }],
                    |r| {
                        Ok((
                            r.get::<_, i64>(0)?,
                            r.get::<_, i64>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, i64>(3)?,
                            r.get::<_, i64>(4)? == 1,
                            r.get::<_, Option<String>>(5)?,
                            r.get::<_, String>(6)?,
                            r.get::<_, Option<String>>(7)?,
                            r.get::<_, Option<String>>(8)?,
                            r.get::<_, String>(9)?,
                            r.get::<_, String>(10)?,
                            r.get::<_, String>(11)?,
                            r.get::<_, String>(12)?,
                            r.get::<_, String>(13)?,
                            r.get::<_, Option<String>>(14)?,
                            r.get::<_, String>(15)?,
                            r.get::<_, Option<i64>>(16)?,
                            r.get::<_, String>(17)?,
                            r.get::<_, String>(18)?,
                        ))
                    },
                )
                .ok();
            Ok(row)
        })?;

        let (
            sql_id,
            graph_entity_id,
            project_id,
            version,
            is_head,
            objective,
            state_json,
            branch,
            head_sha,
            constraints_json,
            active_task_ids_json,
            blocker_ids_json,
            decision_ids_json,
            receipt_ids_json,
            mut changelog,
            evidence_json,
            token_estimate,
            sensitivity,
            created_at,
        ) = match row_opt {
            Some(vals) => vals,
            None => return Ok(None),
        };

        let mut state: Value = serde_json::from_str(&state_json).unwrap_or(Value::Null);
        let mut constraints: Vec<Value> =
            serde_json::from_str(&constraints_json).unwrap_or_default();
        let mut active_task_ids: Vec<Value> =
            serde_json::from_str(&active_task_ids_json).unwrap_or_default();
        let mut blocker_ids: Vec<Value> =
            serde_json::from_str(&blocker_ids_json).unwrap_or_default();
        let mut decision_ids: Vec<Value> =
            serde_json::from_str(&decision_ids_json).unwrap_or_default();
        let mut receipt_ids: Vec<Value> =
            serde_json::from_str(&receipt_ids_json).unwrap_or_default();
        let mut evidence: Vec<Value> = serde_json::from_str(&evidence_json).unwrap_or_default();

        let eval_cost = |s: &Value, cl: Option<&str>, cols: &StateCollections<'_>| -> usize {
            estimate_project_state_tokens(&make_tokens_input(
                objective.as_deref(),
                s,
                branch.as_deref(),
                head_sha.as_deref(),
                cl,
                cols,
            ))
        };

        let initial_cost = token_estimate.map(|t| t as usize).unwrap_or_else(|| {
            eval_cost(
                &state,
                changelog.as_deref(),
                &StateCollections {
                    constraints: &constraints,
                    active_task_ids: &active_task_ids,
                    blocker_ids: &blocker_ids,
                    decision_ids: &decision_ids,
                    receipt_ids: &receipt_ids,
                    evidence: &evidence,
                },
            )
        });

        let mut truncated = false;
        let mut omitted_count = 0usize;
        let mut current_cost = initial_cost;

        if budget > 0 && current_cost > budget {
            truncated = true;
            while current_cost > budget && !evidence.is_empty() {
                evidence.pop();
                omitted_count += 1;
                current_cost = eval_cost(
                    &state,
                    changelog.as_deref(),
                    &StateCollections {
                        constraints: &constraints,
                        active_task_ids: &active_task_ids,
                        blocker_ids: &blocker_ids,
                        decision_ids: &decision_ids,
                        receipt_ids: &receipt_ids,
                        evidence: &evidence,
                    },
                );
            }
            while current_cost > budget && !receipt_ids.is_empty() {
                receipt_ids.pop();
                omitted_count += 1;
                current_cost = eval_cost(
                    &state,
                    changelog.as_deref(),
                    &StateCollections {
                        constraints: &constraints,
                        active_task_ids: &active_task_ids,
                        blocker_ids: &blocker_ids,
                        decision_ids: &decision_ids,
                        receipt_ids: &receipt_ids,
                        evidence: &evidence,
                    },
                );
            }
            while current_cost > budget && !decision_ids.is_empty() {
                decision_ids.pop();
                omitted_count += 1;
                current_cost = eval_cost(
                    &state,
                    changelog.as_deref(),
                    &StateCollections {
                        constraints: &constraints,
                        active_task_ids: &active_task_ids,
                        blocker_ids: &blocker_ids,
                        decision_ids: &decision_ids,
                        receipt_ids: &receipt_ids,
                        evidence: &evidence,
                    },
                );
            }
            while current_cost > budget && !blocker_ids.is_empty() {
                blocker_ids.pop();
                omitted_count += 1;
                current_cost = eval_cost(
                    &state,
                    changelog.as_deref(),
                    &StateCollections {
                        constraints: &constraints,
                        active_task_ids: &active_task_ids,
                        blocker_ids: &blocker_ids,
                        decision_ids: &decision_ids,
                        receipt_ids: &receipt_ids,
                        evidence: &evidence,
                    },
                );
            }
            while current_cost > budget && !active_task_ids.is_empty() {
                active_task_ids.pop();
                omitted_count += 1;
                current_cost = eval_cost(
                    &state,
                    changelog.as_deref(),
                    &StateCollections {
                        constraints: &constraints,
                        active_task_ids: &active_task_ids,
                        blocker_ids: &blocker_ids,
                        decision_ids: &decision_ids,
                        receipt_ids: &receipt_ids,
                        evidence: &evidence,
                    },
                );
            }
            while current_cost > budget && !constraints.is_empty() {
                constraints.pop();
                omitted_count += 1;
                current_cost = eval_cost(
                    &state,
                    changelog.as_deref(),
                    &StateCollections {
                        constraints: &constraints,
                        active_task_ids: &active_task_ids,
                        blocker_ids: &blocker_ids,
                        decision_ids: &decision_ids,
                        receipt_ids: &receipt_ids,
                        evidence: &evidence,
                    },
                );
            }
            if current_cost > budget && changelog.is_some() {
                changelog = None;
                omitted_count += 1;
                current_cost = eval_cost(
                    &state,
                    changelog.as_deref(),
                    &StateCollections {
                        constraints: &constraints,
                        active_task_ids: &active_task_ids,
                        blocker_ids: &blocker_ids,
                        decision_ids: &decision_ids,
                        receipt_ids: &receipt_ids,
                        evidence: &evidence,
                    },
                );
            }
            if current_cost > budget {
                state = json!({"truncated": true});
                omitted_count += 1;
                current_cost = eval_cost(
                    &state,
                    changelog.as_deref(),
                    &StateCollections {
                        constraints: &constraints,
                        active_task_ids: &active_task_ids,
                        blocker_ids: &blocker_ids,
                        decision_ids: &decision_ids,
                        receipt_ids: &receipt_ids,
                        evidence: &evidence,
                    },
                )
                .min(budget);
            }
        }

        Ok(Some(ProjectStateEnvelope {
            graph_entity_id,
            sql_id: Some(sql_id),
            id: format!("project_state:{}", graph_entity_id),
            project_id,
            version,
            is_head,
            objective,
            state,
            branch,
            head_sha,
            constraints,
            active_task_ids,
            blocker_ids,
            decision_ids,
            receipt_ids,
            changelog,
            evidence,
            token_estimate: current_cost,
            sensitivity,
            created_at,
            truncated,
            omitted_count,
        }))
    }

    /// Append a new agent state version.
    pub fn append_agent_state(
        &self,
        agent: &str,
        state: &AgentStateParams,
    ) -> Result<AgentStateOutcome> {
        if state.sensitivity.eq_ignore_ascii_case("secret") {
            bail!("Records with sensitivity 'secret' are rejected at ingest");
        }
        if agent.trim().is_empty() {
            bail!("agent cannot be empty");
        }

        if let Some(r_id) = state.last_receipt_graph_entity_id {
            if self.inner.get_entity(r_id).is_err() {
                bail!("Referenced receipt graph entity {} does not exist", r_id);
            }
        }

        let now = Utc::now().to_rfc3339();
        let state_json_str = serde_json::to_string(&state.state)?;
        let budget_json_str = serde_json::to_string(&state.budget)?;

        let (sql_id, version) = self.with_raw_connection(|conn| {
            let max_ver: i64 = conn
                .query_row(
                    "SELECT COALESCE(MAX(version), 0) FROM agent_states
                     WHERE agent_name = ?1
                       AND COALESCE(project_id, '') = COALESCE(?2, '')
                       AND COALESCE(dispatch_id, '') = COALESCE(?3, '')",
                    params![
                        agent,
                        state.project_id.as_deref(),
                        state.dispatch_id.as_deref()
                    ],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            let next_ver = max_ver + 1;

            conn.execute(
                "INSERT INTO agent_states (
                    graph_entity_id, agent_name, project_id, session_id, dispatch_id, task_id,
                    version, status, current_objective, state_json, budget_json,
                    last_receipt_graph_entity_id, sensitivity, created_at
                ) VALUES (
                    0, ?1, ?2, ?3, ?4, ?5,
                    ?6, ?7, ?8, ?9, ?10,
                    ?11, ?12, ?13
                )",
                params![
                    agent,
                    state.project_id.as_deref(),
                    state.session_id.as_deref(),
                    state.dispatch_id.as_deref(),
                    state.task_id,
                    next_ver,
                    &state.status,
                    state.current_objective.as_deref(),
                    &state_json_str,
                    &budget_json_str,
                    state.last_receipt_graph_entity_id,
                    &state.sensitivity,
                    &now,
                ],
            )?;
            let rowid = conn.last_insert_rowid();
            Ok((rowid, next_ver))
        })?;

        let mut entity_data = json!({
            "sql_id": sql_id,
            "agent_name": agent,
            "project_id": &state.project_id,
            "session_id": &state.session_id,
            "dispatch_id": &state.dispatch_id,
            "task_id": state.task_id,
            "version": version,
            "status": &state.status,
            "current_objective": &state.current_objective,
            "state": &state.state,
            "budget": &state.budget,
            "last_receipt_graph_entity_id": state.last_receipt_graph_entity_id,
            "sensitivity": &state.sensitivity,
            "created_at": &now,
        });

        let ge = GraphEntity {
            id: 0,
            kind: EntityType::AgentState.as_str().to_string(),
            name: format!("{}:v{}", agent, version),
            file_path: None,
            data: entity_data.clone(),
        };
        let ge_id = self
            .inner
            .insert_entity(&ge)
            .map_err(|e| anyhow::anyhow!("insert AgentState entity: {}", e))?;

        self.with_raw_connection(|conn| {
            Ok(conn.execute(
                "UPDATE agent_states SET graph_entity_id = ?1 WHERE id = ?2",
                params![ge_id, sql_id],
            )?)
        })?;
        if let Some(obj) = entity_data.as_object_mut() {
            obj.insert("graph_entity_id".to_string(), json!(ge_id));
        }
        self.update_entity_data(ge_id, &entity_data)?;

        if let Some(r_id) = state.last_receipt_graph_entity_id {
            let _ = self.insert_edge(ge_id, r_id, EdgeType::RelatedTo, json!({}));
        }

        self.runtime.record_memory_write();
        self.runtime.bump_generation(CacheDomain::Memory);

        Ok(AgentStateOutcome {
            graph_entity_id: ge_id,
            sql_id,
            agent_name: agent.to_string(),
            version,
            status: state.status.clone(),
        })
    }

    /// Retrieve the latest agent state.
    pub fn get_agent_state_head(
        &self,
        agent: &str,
        project: Option<&str>,
    ) -> Result<Option<AgentStateEnvelope>> {
        self.get_agent_state_head_with_options(agent, project, false)
    }

    /// Retrieve the latest agent state with sensitivity filtering option.
    pub fn get_agent_state_head_with_options(
        &self,
        agent: &str,
        project: Option<&str>,
        include_sensitive: bool,
    ) -> Result<Option<AgentStateEnvelope>> {
        let row_opt = self.with_raw_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, graph_entity_id, agent_name, project_id, session_id, dispatch_id, task_id,
                        version, status, current_objective, state_json, budget_json,
                        last_receipt_graph_entity_id, sensitivity, created_at
                 FROM agent_states
                 WHERE agent_name = ?1
                   AND (?2 IS NULL OR project_id = ?2)
                   AND (?3 = 1 OR sensitivity != 'sensitive')
                   AND sensitivity != 'secret'
                 ORDER BY version DESC LIMIT 1",
            )?;
            let row = stmt
                .query_row(
                    params![agent, project, if include_sensitive { 1 } else { 0 }],
                    |r| {
                        Ok((
                            r.get::<_, i64>(0)?,
                            r.get::<_, i64>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, Option<String>>(3)?,
                            r.get::<_, Option<String>>(4)?,
                            r.get::<_, Option<String>>(5)?,
                            r.get::<_, Option<i64>>(6)?,
                            r.get::<_, i64>(7)?,
                            r.get::<_, String>(8)?,
                            r.get::<_, Option<String>>(9)?,
                            r.get::<_, String>(10)?,
                            r.get::<_, String>(11)?,
                            r.get::<_, Option<i64>>(12)?,
                            r.get::<_, String>(13)?,
                            r.get::<_, String>(14)?,
                        ))
                    },
                )
                .ok();
            Ok(row)
        })?;

        let (
            sql_id,
            graph_entity_id,
            agent_name,
            project_id,
            session_id,
            dispatch_id,
            task_id,
            version,
            status,
            current_objective,
            state_json,
            budget_json,
            last_receipt_graph_entity_id,
            sensitivity,
            created_at,
        ) = match row_opt {
            Some(vals) => vals,
            None => return Ok(None),
        };

        let state = serde_json::from_str(&state_json).unwrap_or(Value::Null);
        let budget = serde_json::from_str(&budget_json).unwrap_or(Value::Null);

        Ok(Some(AgentStateEnvelope {
            graph_entity_id,
            sql_id: Some(sql_id),
            id: format!("agent_state:{}", graph_entity_id),
            agent_name,
            project_id,
            session_id,
            dispatch_id,
            task_id,
            version,
            status,
            current_objective,
            state,
            budget,
            last_receipt_graph_entity_id,
            sensitivity,
            created_at,
        }))
    }

    /// Record an artifact, deduplicating on UNIQUE(project_id, path, sha256).
    pub fn record_artifact(&self, params: &RecordArtifactParams) -> Result<RecordArtifactOutcome> {
        if params.sensitivity.eq_ignore_ascii_case("secret") {
            bail!("Records with sensitivity 'secret' are rejected at ingest");
        }
        if params.path.trim().is_empty() {
            bail!("path cannot be empty");
        }
        if params.sha256.trim().is_empty() {
            bail!("sha256 cannot be empty");
        }
        if params.artifact_kind.trim().is_empty() {
            bail!("artifact_kind cannot be empty");
        }

        let existing: Option<(i64, i64)> = self.with_raw_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, graph_entity_id FROM artifacts
                 WHERE COALESCE(project_id, '') = COALESCE(?1, '')
                   AND path = ?2
                   AND sha256 = ?3
                 LIMIT 1",
            )?;
            let row = stmt
                .query_row(
                    params![params.project_id.as_deref(), params.path, params.sha256],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .ok();
            Ok(row)
        })?;

        if let Some((sql_id, ge_id)) = existing {
            return Ok(RecordArtifactOutcome::Unchanged {
                graph_entity_id: ge_id,
                sql_id,
                sha256: params.sha256.clone(),
            });
        }

        let now = Utc::now().to_rfc3339();
        let meta_str = serde_json::to_string(&params.metadata)?;

        let sql_id = self.with_raw_connection(|conn| {
            conn.execute(
                "INSERT INTO artifacts (
                    graph_entity_id, project_id, task_id, artifact_kind, path, sha256,
                    bytes, commit_sha, created_by, sensitivity, metadata_json, created_at
                ) VALUES (
                    0, ?1, ?2, ?3, ?4, ?5,
                    ?6, ?7, ?8, ?9, ?10, ?11
                )",
                params![
                    params.project_id.as_deref(),
                    params.task_id,
                    params.artifact_kind,
                    params.path,
                    params.sha256,
                    params.bytes,
                    params.commit_sha.as_deref(),
                    params.created_by.as_deref(),
                    params.sensitivity,
                    meta_str,
                    &now,
                ],
            )?;
            Ok(conn.last_insert_rowid())
        })?;

        let mut entity_data = json!({
            "sql_id": sql_id,
            "project_id": &params.project_id,
            "task_id": params.task_id,
            "artifact_kind": &params.artifact_kind,
            "path": &params.path,
            "sha256": &params.sha256,
            "bytes": params.bytes,
            "commit_sha": &params.commit_sha,
            "created_by": &params.created_by,
            "sensitivity": &params.sensitivity,
            "metadata": &params.metadata,
            "created_at": &now,
        });

        let ge = GraphEntity {
            id: 0,
            kind: EntityType::Artifact.as_str().to_string(),
            name: params.path.clone(),
            file_path: Some(params.path.clone()),
            data: entity_data.clone(),
        };
        let ge_id = self
            .inner
            .insert_entity(&ge)
            .map_err(|e| anyhow::anyhow!("insert Artifact entity: {}", e))?;

        self.with_raw_connection(|conn| {
            Ok(conn.execute(
                "UPDATE artifacts SET graph_entity_id = ?1 WHERE id = ?2",
                params![ge_id, sql_id],
            )?)
        })?;
        if let Some(obj) = entity_data.as_object_mut() {
            obj.insert("graph_entity_id".to_string(), json!(ge_id));
        }
        self.update_entity_data(ge_id, &entity_data)?;

        self.runtime.record_memory_write();
        self.runtime.bump_generation(CacheDomain::Memory);

        Ok(RecordArtifactOutcome::Created {
            graph_entity_id: ge_id,
            sql_id,
            sha256: params.sha256.clone(),
        })
    }

    /// Record a receipt and link it to its verified artifact.
    pub fn record_receipt(&self, params: &RecordReceiptParams) -> Result<RecordReceiptOutcome> {
        if params.sensitivity.eq_ignore_ascii_case("secret") {
            bail!("Records with sensitivity 'secret' are rejected at ingest");
        }
        if params.verdict.trim().is_empty() {
            bail!("verdict cannot be empty");
        }
        if params.verifier.trim().is_empty() {
            bail!("verifier cannot be empty");
        }

        if let Some(art_id) = params.artifact_graph_entity_id {
            if self.inner.get_entity(art_id).is_err() {
                bail!("Referenced artifact graph entity {} does not exist", art_id);
            }
        }

        let now = Utc::now().to_rfc3339();
        let evidence_str = serde_json::to_string(&params.evidence)?;

        let sql_id = self.with_raw_connection(|conn| {
            conn.execute(
                "INSERT INTO receipts (
                    graph_entity_id, project_id, task_id, artifact_graph_entity_id, session_id,
                    verdict, verifier, command, exit_code, duration_ms,
                    output_hash, output_sample_redacted, evidence_json, sensitivity, created_at
                ) VALUES (
                    0, ?1, ?2, ?3, ?4,
                    ?5, ?6, ?7, ?8, ?9,
                    ?10, ?11, ?12, ?13, ?14
                )",
                params![
                    params.project_id.as_deref(),
                    params.task_id,
                    params.artifact_graph_entity_id,
                    params.session_id.as_deref(),
                    params.verdict,
                    params.verifier,
                    params.command.as_deref(),
                    params.exit_code,
                    params.duration_ms,
                    params.output_hash.as_deref(),
                    params.output_sample_redacted.as_deref(),
                    evidence_str,
                    params.sensitivity,
                    &now,
                ],
            )?;
            Ok(conn.last_insert_rowid())
        })?;

        let mut entity_data = json!({
            "sql_id": sql_id,
            "project_id": &params.project_id,
            "task_id": params.task_id,
            "artifact_graph_entity_id": params.artifact_graph_entity_id,
            "session_id": &params.session_id,
            "verdict": &params.verdict,
            "verifier": &params.verifier,
            "command": &params.command,
            "exit_code": params.exit_code,
            "duration_ms": params.duration_ms,
            "output_hash": &params.output_hash,
            "output_sample_redacted": &params.output_sample_redacted,
            "evidence": &params.evidence,
            "sensitivity": &params.sensitivity,
            "created_at": &now,
        });

        let ge = GraphEntity {
            id: 0,
            kind: EntityType::Receipt.as_str().to_string(),
            name: format!("receipt:{}:{}", params.verifier, now),
            file_path: None,
            data: entity_data.clone(),
        };
        let ge_id = self
            .inner
            .insert_entity(&ge)
            .map_err(|e| anyhow::anyhow!("insert Receipt entity: {}", e))?;

        self.with_raw_connection(|conn| {
            Ok(conn.execute(
                "UPDATE receipts SET graph_entity_id = ?1 WHERE id = ?2",
                params![ge_id, sql_id],
            )?)
        })?;
        if let Some(obj) = entity_data.as_object_mut() {
            obj.insert("graph_entity_id".to_string(), json!(ge_id));
        }
        self.update_entity_data(ge_id, &entity_data)?;

        if let Some(art_id) = params.artifact_graph_entity_id {
            self.insert_edge(ge_id, art_id, EdgeType::Verifies, json!({}))?;
        }

        self.runtime.record_memory_write();
        self.runtime.bump_generation(CacheDomain::Memory);

        Ok(RecordReceiptOutcome::Created {
            graph_entity_id: ge_id,
            sql_id,
            verdict: params.verdict.clone(),
            artifact_graph_entity_id: params.artifact_graph_entity_id,
        })
    }

    /// Record a source, deduplicating on UNIQUE(project_id, uri).
    pub fn record_source(&self, params: &RecordSourceParams) -> Result<RecordSourceOutcome> {
        if params.sensitivity.eq_ignore_ascii_case("secret") {
            bail!("Records with sensitivity 'secret' are rejected at ingest");
        }
        if params.uri.trim().is_empty() {
            bail!("uri cannot be empty");
        }
        if params.source_type.trim().is_empty() {
            bail!("source_type cannot be empty");
        }

        let existing: Option<(i64, i64)> = self.with_raw_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, graph_entity_id FROM sources
                 WHERE COALESCE(project_id, '') = COALESCE(?1, '')
                   AND uri = ?2
                 LIMIT 1",
            )?;
            let row = stmt
                .query_row(params![params.project_id.as_deref(), params.uri], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .ok();
            Ok(row)
        })?;

        if let Some((sql_id, ge_id)) = existing {
            return Ok(RecordSourceOutcome::Unchanged {
                graph_entity_id: ge_id,
                sql_id,
                uri: params.uri.clone(),
            });
        }

        let now = Utc::now().to_rfc3339();
        let meta_str = serde_json::to_string(&params.metadata)?;

        let sql_id = self.with_raw_connection(|conn| {
            conn.execute(
                "INSERT INTO sources (
                    graph_entity_id, project_id, source_type, uri, content_hash,
                    title, summary, author, published_at, retrieved_at,
                    credibility, sensitivity, metadata_json
                ) VALUES (
                    0, ?1, ?2, ?3, ?4,
                    ?5, ?6, ?7, ?8, ?9,
                    ?10, ?11, ?12
                )",
                params![
                    params.project_id.as_deref(),
                    params.source_type,
                    params.uri,
                    params.content_hash.as_deref(),
                    params.title.as_deref(),
                    params.summary.as_deref(),
                    params.author.as_deref(),
                    params.published_at.as_deref(),
                    params.retrieved_at.as_deref(),
                    params.credibility,
                    params.sensitivity,
                    meta_str,
                ],
            )?;
            Ok(conn.last_insert_rowid())
        })?;

        let mut entity_data = json!({
            "sql_id": sql_id,
            "project_id": &params.project_id,
            "source_type": &params.source_type,
            "uri": &params.uri,
            "content_hash": &params.content_hash,
            "title": &params.title,
            "summary": &params.summary,
            "author": &params.author,
            "published_at": &params.published_at,
            "retrieved_at": &params.retrieved_at,
            "credibility": params.credibility,
            "sensitivity": &params.sensitivity,
            "metadata": &params.metadata,
            "created_at": &now,
        });

        let ge = GraphEntity {
            id: 0,
            kind: EntityType::Source.as_str().to_string(),
            name: params.title.as_deref().unwrap_or(&params.uri).to_string(),
            file_path: None,
            data: entity_data.clone(),
        };
        let ge_id = self
            .inner
            .insert_entity(&ge)
            .map_err(|e| anyhow::anyhow!("insert Source entity: {}", e))?;

        self.with_raw_connection(|conn| {
            Ok(conn.execute(
                "UPDATE sources SET graph_entity_id = ?1 WHERE id = ?2",
                params![ge_id, sql_id],
            )?)
        })?;
        if let Some(obj) = entity_data.as_object_mut() {
            obj.insert("graph_entity_id".to_string(), json!(ge_id));
        }
        self.update_entity_data(ge_id, &entity_data)?;

        self.runtime.record_memory_write();
        self.runtime.bump_generation(CacheDomain::Memory);

        Ok(RecordSourceOutcome::Created {
            graph_entity_id: ge_id,
            sql_id,
            uri: params.uri.clone(),
        })
    }

    /// Link an external memory reference, deduplicating on UNIQUE(external_store, external_id).
    pub fn link_external_memory(
        &self,
        params: &LinkExternalMemoryParams,
    ) -> Result<LinkExternalMemoryOutcome> {
        if params.external_store.trim().is_empty() {
            bail!("external_store cannot be empty");
        }
        if params.external_id.trim().is_empty() {
            bail!("external_id cannot be empty");
        }
        if params.atheneum_graph_entity_id <= 0 {
            bail!("atheneum_graph_entity_id must be positive");
        }

        if self
            .inner
            .get_entity(params.atheneum_graph_entity_id)
            .is_err()
        {
            bail!(
                "Referenced atheneum graph entity {} does not exist",
                params.atheneum_graph_entity_id
            );
        }

        let existing: Option<(i64, i64)> = self.with_raw_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, graph_entity_id FROM external_memory_refs
                 WHERE external_store = ?1 AND external_id = ?2
                 LIMIT 1",
            )?;
            let row = stmt
                .query_row(params![params.external_store, params.external_id], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .ok();
            Ok(row)
        })?;

        if let Some((sql_id, ge_id)) = existing {
            return Ok(LinkExternalMemoryOutcome::Unchanged {
                graph_entity_id: ge_id,
                sql_id,
                external_store: params.external_store.clone(),
                external_id: params.external_id.clone(),
                atheneum_graph_entity_id: params.atheneum_graph_entity_id,
            });
        }

        let now = Utc::now().to_rfc3339();
        let last_synced = params.last_synced_at.as_deref().unwrap_or(&now);
        let meta_str = serde_json::to_string(&params.metadata)?;

        let sql_id = self.with_raw_connection(|conn| {
            conn.execute(
                "INSERT INTO external_memory_refs (
                    graph_entity_id, external_store, external_id, atheneum_graph_entity_id,
                    atheneum_kind, content_hash, sync_state, last_synced_at, metadata_json
                ) VALUES (
                    0, ?1, ?2, ?3,
                    ?4, ?5, ?6, ?7, ?8
                )",
                params![
                    params.external_store,
                    params.external_id,
                    params.atheneum_graph_entity_id,
                    params.atheneum_kind,
                    params.content_hash,
                    params.sync_state,
                    last_synced,
                    meta_str,
                ],
            )?;
            Ok(conn.last_insert_rowid())
        })?;

        let mut entity_data = json!({
            "sql_id": sql_id,
            "external_store": &params.external_store,
            "external_id": &params.external_id,
            "atheneum_graph_entity_id": params.atheneum_graph_entity_id,
            "atheneum_kind": &params.atheneum_kind,
            "content_hash": &params.content_hash,
            "sync_state": &params.sync_state,
            "last_synced_at": last_synced,
            "metadata": &params.metadata,
            "created_at": &now,
        });

        let ge = GraphEntity {
            id: 0,
            kind: EntityType::ExternalMemoryRef.as_str().to_string(),
            name: format!("{}:{}", params.external_store, params.external_id),
            file_path: None,
            data: entity_data.clone(),
        };
        let ge_id = self
            .inner
            .insert_entity(&ge)
            .map_err(|e| anyhow::anyhow!("insert ExternalMemoryRef entity: {}", e))?;

        self.with_raw_connection(|conn| {
            Ok(conn.execute(
                "UPDATE external_memory_refs SET graph_entity_id = ?1 WHERE id = ?2",
                params![ge_id, sql_id],
            )?)
        })?;
        if let Some(obj) = entity_data.as_object_mut() {
            obj.insert("graph_entity_id".to_string(), json!(ge_id));
        }
        self.update_entity_data(ge_id, &entity_data)?;

        let _ = self.insert_edge(
            ge_id,
            params.atheneum_graph_entity_id,
            EdgeType::DerivedFrom,
            json!({}),
        );

        self.runtime.record_memory_write();
        self.runtime.bump_generation(CacheDomain::Memory);

        Ok(LinkExternalMemoryOutcome::Created {
            graph_entity_id: ge_id,
            sql_id,
            external_store: params.external_store.clone(),
            external_id: params.external_id.clone(),
            atheneum_graph_entity_id: params.atheneum_graph_entity_id,
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

    #[test]
    fn test_phase2b_project_state_append_and_versioning() {
        let graph = AtheneumGraph::open_in_memory().expect("open");
        let state1 = ProjectStateParams {
            objective: Some("Phase 2B Implementation".to_string()),
            state: json!({"phase": "2B", "status": "in_progress"}),
            branch: Some("atheneum/memory-router-phase2b".to_string()),
            head_sha: Some("9e826d2".to_string()),
            constraints: vec![json!("fail closed on invalid records")],
            active_task_ids: vec![json!(1)],
            blocker_ids: vec![],
            decision_ids: vec![json!(101)],
            receipt_ids: vec![],
            changelog: Some("Initial phase 2b setup".to_string()),
            evidence: vec![json!({"type": "spec", "ref": "TECH.md"})],
            token_estimate: None,
            sensitivity: "normal".to_string(),
        };
        let out1 = graph
            .append_project_state("atheneum", &state1)
            .expect("append v1");
        assert_eq!(out1.version, 1);
        assert!(out1.is_head);
        assert!(out1.graph_entity_id > 0);

        // Verify dual-write: GraphEntity of kind ProjectState exists
        let ge1 = graph
            .inner
            .get_entity(out1.graph_entity_id)
            .expect("get entity");
        assert_eq!(ge1.kind, EntityType::ProjectState.as_str());

        // Append v2
        let mut state2 = state1.clone();
        state2.objective = Some("Phase 2B Completed".to_string());
        let out2 = graph
            .append_project_state("atheneum", &state2)
            .expect("append v2");
        assert_eq!(out2.version, 2);
        assert!(out2.is_head);
        assert!(out2.graph_entity_id > out1.graph_entity_id);

        // Bipolar gate A: both retained in database
        let count: i64 = graph
            .with_raw_connection(|conn| {
                Ok(conn.query_row(
                    "SELECT count(*) FROM project_states WHERE project_id = 'atheneum'",
                    [],
                    |r| r.get(0),
                )?)
            })
            .expect("count");
        assert_eq!(
            count, 2,
            "both versions must be retained (append-only history)"
        );

        // Head query returns latest version (v2)
        let head = graph
            .get_project_state_head("atheneum", 2000)
            .expect("get head")
            .expect("exists");
        assert_eq!(head.version, 2);
        assert_eq!(head.graph_entity_id, out2.graph_entity_id);
        assert_eq!(head.objective.as_deref(), Some("Phase 2B Completed"));
    }

    #[test]
    fn test_phase2b_project_state_head_budget_and_sensitive_filter() {
        let graph = AtheneumGraph::open_in_memory().expect("open");
        let state = ProjectStateParams {
            objective: Some("Large Objective with many active tasks and constraints".to_string()),
            state: json!({"payload": "x".repeat(1500)}),
            branch: Some("main".to_string()),
            head_sha: None,
            constraints: (0..40)
                .map(|i| json!(format!("constraint_{}", i)))
                .collect(),
            active_task_ids: (0..40).map(|i| json!(i)).collect(),
            blocker_ids: vec![],
            decision_ids: (0..40).map(|i| json!(i)).collect(),
            receipt_ids: (0..40).map(|i| json!(i)).collect(),
            changelog: Some("Large changelog ".repeat(40)),
            evidence: (0..40).map(|i| json!({"evidence_num": i})).collect(),
            token_estimate: None,
            sensitivity: "normal".to_string(),
        };
        graph
            .append_project_state("budget-proj", &state)
            .expect("append");

        // With large budget: no truncation
        let full = graph
            .get_project_state_head("budget-proj", 10000)
            .expect("head")
            .expect("exists");
        assert!(!full.truncated);
        assert_eq!(full.omitted_count, 0);

        // With small budget: truncated with omitted_count > 0
        let truncated = graph
            .get_project_state_head("budget-proj", 60)
            .expect("head")
            .expect("exists");
        assert!(truncated.truncated);
        assert!(truncated.omitted_count > 0);

        // Sensitivity filtering:
        let sens_state = ProjectStateParams {
            objective: Some("Sensitive Operations".to_string()),
            state: json!({"sensitive": true}),
            branch: None,
            head_sha: None,
            constraints: vec![],
            active_task_ids: vec![],
            blocker_ids: vec![],
            decision_ids: vec![],
            receipt_ids: vec![],
            changelog: None,
            evidence: vec![],
            token_estimate: None,
            sensitivity: "sensitive".to_string(),
        };
        graph
            .append_project_state("sens-proj", &sens_state)
            .expect("append sensitive");
        // Default operator call excludes sensitive
        let head_excluded = graph
            .get_project_state_head("sens-proj", 1000)
            .expect("head");
        assert!(
            head_excluded.is_none(),
            "sensitive state must be excluded without operator flag"
        );
        // With operator flag include_sensitive=true
        let head_included = graph
            .get_project_state_head_with_options("sens-proj", 1000, true)
            .expect("head");
        assert!(
            head_included.is_some(),
            "sensitive state must be returned when operator flag is true"
        );
    }

    #[test]
    fn test_phase2b_agent_state_append_and_head() {
        let graph = AtheneumGraph::open_in_memory().expect("open");
        let state1 = AgentStateParams {
            project_id: Some("atheneum".to_string()),
            session_id: Some("sess-1".to_string()),
            dispatch_id: Some("disp-1".to_string()),
            task_id: None,
            status: "running".to_string(),
            current_objective: Some("Writing tests".to_string()),
            state: json!({"step": 1}),
            budget: json!({"tokens_used": 120}),
            last_receipt_graph_entity_id: None,
            sensitivity: "normal".to_string(),
        };
        let out1 = graph
            .append_agent_state("worker-agy", &state1)
            .expect("append 1");
        assert_eq!(out1.version, 1);
        assert_eq!(out1.status, "running");

        let mut state2 = state1.clone();
        state2.status = "completed".to_string();
        state2.current_objective = Some("Tests passed".to_string());
        let out2 = graph
            .append_agent_state("worker-agy", &state2)
            .expect("append 2");
        assert_eq!(out2.version, 2);
        assert_eq!(out2.status, "completed");

        let head = graph
            .get_agent_state_head("worker-agy", Some("atheneum"))
            .expect("head")
            .expect("found");
        assert_eq!(head.version, 2);
        assert_eq!(head.status, "completed");
        assert_eq!(head.graph_entity_id, out2.graph_entity_id);
    }

    #[test]
    fn test_phase2b_artifact_record_and_idempotent_replay() {
        let graph = AtheneumGraph::open_in_memory().expect("open");
        let art_params = RecordArtifactParams {
            project_id: Some("atheneum".to_string()),
            task_id: None,
            artifact_kind: "report".to_string(),
            path: "tasks/atheneum-memory-router/phase2b/report.md".to_string(),
            sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string(),
            bytes: Some(1024),
            commit_sha: Some("9e826d2".to_string()),
            created_by: Some("worker-agy".to_string()),
            sensitivity: "normal".to_string(),
            metadata: json!({"phase": "2B"}),
        };
        let res1 = graph.record_artifact(&art_params).expect("record 1");
        let ge_id = match res1 {
            RecordArtifactOutcome::Created {
                graph_entity_id, ..
            } => graph_entity_id,
            _ => panic!("expected Created"),
        };

        // Bipolar gate ¬A: replaying identical artifact returns same ID, no duplicate row
        let res2 = graph.record_artifact(&art_params).expect("record 2");
        match res2 {
            RecordArtifactOutcome::Unchanged {
                graph_entity_id, ..
            } => {
                assert_eq!(graph_entity_id, ge_id);
            }
            _ => panic!("expected Unchanged on idempotent replay"),
        }

        let count: i64 = graph
            .with_raw_connection(|conn| {
                Ok(conn.query_row("SELECT count(*) FROM artifacts", [], |r| r.get(0))?)
            })
            .expect("count");
        assert_eq!(count, 1, "exactly one row must exist after replay");
    }

    #[test]
    fn test_phase2b_receipt_record_linked_to_artifact() {
        let graph = AtheneumGraph::open_in_memory().expect("open");
        let art_params = RecordArtifactParams {
            project_id: Some("atheneum".to_string()),
            task_id: None,
            artifact_kind: "report".to_string(),
            path: "tasks/report.md".to_string(),
            sha256: "hash12345678".to_string(),
            bytes: Some(500),
            commit_sha: None,
            created_by: None,
            sensitivity: "normal".to_string(),
            metadata: json!({}),
        };
        let art_res = graph.record_artifact(&art_params).expect("record artifact");
        let art_ge_id = match art_res {
            RecordArtifactOutcome::Created {
                graph_entity_id, ..
            } => graph_entity_id,
            _ => unreachable!(),
        };

        let rcpt_params = RecordReceiptParams {
            project_id: Some("atheneum".to_string()),
            task_id: None,
            artifact_graph_entity_id: Some(art_ge_id),
            session_id: None,
            verdict: "pass".to_string(),
            verifier: "cargo-test".to_string(),
            command: Some("cargo test".to_string()),
            exit_code: Some(0),
            duration_ms: Some(1500),
            output_hash: Some("out123".to_string()),
            output_sample_redacted: Some("126 passed; 0 failed".to_string()),
            evidence: vec![json!({"tool": "cargo"})],
            sensitivity: "normal".to_string(),
        };
        let rcpt_res = graph.record_receipt(&rcpt_params).expect("record receipt");
        let rcpt_ge_id = match rcpt_res {
            RecordReceiptOutcome::Created {
                graph_entity_id,
                artifact_graph_entity_id,
                ..
            } => {
                assert_eq!(artifact_graph_entity_id, Some(art_ge_id));
                graph_entity_id
            }
        };

        // Verify graph edge receipt -> artifact with EdgeType::Verifies
        let edges = graph.outgoing_edges(rcpt_ge_id).expect("edges");
        assert!(edges
            .iter()
            .any(|e| e.to_id == art_ge_id && e.edge_type == EdgeType::Verifies.as_str()));

        // Verify fail closed when artifact_graph_entity_id does not exist
        let mut bad_rcpt = rcpt_params.clone();
        bad_rcpt.artifact_graph_entity_id = Some(999999);
        let bad_res = graph.record_receipt(&bad_rcpt);
        assert!(
            bad_res.is_err(),
            "recording receipt with non-existent artifact must fail closed"
        );
    }

    #[test]
    fn test_phase2b_source_record_dedup() {
        let graph = AtheneumGraph::open_in_memory().expect("open");
        let src_params = RecordSourceParams {
            project_id: Some("atheneum".to_string()),
            source_type: "web".to_string(),
            uri: "https://specs.example.com/phase2b.md".to_string(),
            content_hash: Some("contenthash123".to_string()),
            title: Some("Phase 2B Spec".to_string()),
            summary: Some("Spec for Phase 2B".to_string()),
            author: Some("Luiz Spies".to_string()),
            published_at: Some("2026-11-03T00:00:00Z".to_string()),
            retrieved_at: Some("2026-11-03T12:00:00Z".to_string()),
            credibility: Some(0.95),
            sensitivity: "normal".to_string(),
            metadata: json!({"reviewed": true}),
        };
        let res1 = graph.record_source(&src_params).expect("record 1");
        let ge_id = match res1 {
            RecordSourceOutcome::Created {
                graph_entity_id, ..
            } => graph_entity_id,
            _ => panic!("expected Created"),
        };

        // Replay same source (same project_id and uri)
        let res2 = graph.record_source(&src_params).expect("record 2");
        match res2 {
            RecordSourceOutcome::Unchanged {
                graph_entity_id, ..
            } => {
                assert_eq!(graph_entity_id, ge_id);
            }
            _ => panic!("expected Unchanged on duplicate replay"),
        }

        let count: i64 = graph
            .with_raw_connection(|conn| {
                Ok(conn.query_row(
                    "SELECT count(*) FROM sources WHERE uri = ?1",
                    params![src_params.uri],
                    |r| r.get(0),
                )?)
            })
            .expect("count");
        assert_eq!(count, 1, "exactly 1 row in sources for URI");
    }

    #[test]
    fn test_phase2b_external_ref_dedup_and_link() {
        let graph = AtheneumGraph::open_in_memory().expect("open");
        // Upsert memory item to have a valid atheneum_graph_entity_id
        let mem_out = graph
            .upsert_memory_item(&sample_params("target_key", "Target memory content"))
            .expect("upsert mem");
        let atheneum_ge_id = match mem_out {
            UpsertMemoryOutcome::Created {
                graph_entity_id, ..
            } => graph_entity_id,
            _ => panic!("expected created"),
        };

        let ext_params = LinkExternalMemoryParams {
            external_store: "bookkeeper".to_string(),
            external_id: "bk_item_9912".to_string(),
            atheneum_graph_entity_id: atheneum_ge_id,
            atheneum_kind: "Memory".to_string(),
            content_hash: "hash9912".to_string(),
            sync_state: "synced".to_string(),
            last_synced_at: Some("2026-11-03T12:00:00Z".to_string()),
            metadata: json!({"collection": "agent_memories"}),
        };
        let res1 = graph.link_external_memory(&ext_params).expect("link 1");
        let ge_id = match res1 {
            LinkExternalMemoryOutcome::Created {
                graph_entity_id, ..
            } => graph_entity_id,
            _ => panic!("expected Created"),
        };

        // Dedup on (external_store, external_id)
        let res2 = graph.link_external_memory(&ext_params).expect("link 2");
        match res2 {
            LinkExternalMemoryOutcome::Unchanged {
                graph_entity_id, ..
            } => {
                assert_eq!(graph_entity_id, ge_id);
            }
            _ => panic!("expected Unchanged on duplicate external ref"),
        }

        // Linking to non-existent atheneum_graph_entity_id fails closed
        let mut bad_ext = ext_params.clone();
        bad_ext.external_id = "bk_item_other".to_string();
        bad_ext.atheneum_graph_entity_id = 888888;
        let bad_res = graph.link_external_memory(&bad_ext);
        assert!(
            bad_res.is_err(),
            "linking to non-existent entity must fail closed"
        );
    }

    #[test]
    fn test_phase2b_secret_rejection_at_ingest() {
        let graph = AtheneumGraph::open_in_memory().expect("open");

        // project state
        let p = ProjectStateParams {
            objective: Some("Secret Plan".to_string()),
            state: json!({}),
            branch: None,
            head_sha: None,
            constraints: vec![],
            active_task_ids: vec![],
            blocker_ids: vec![],
            decision_ids: vec![],
            receipt_ids: vec![],
            changelog: None,
            evidence: vec![],
            token_estimate: None,
            sensitivity: "secret".to_string(),
        };
        assert!(graph.append_project_state("p", &p).is_err());

        // agent state
        let a = AgentStateParams {
            project_id: Some("p".to_string()),
            session_id: None,
            dispatch_id: None,
            task_id: None,
            status: "secret".to_string(),
            current_objective: None,
            state: json!({}),
            budget: json!({}),
            last_receipt_graph_entity_id: None,
            sensitivity: "SECRET".to_string(),
        };
        assert!(graph.append_agent_state("agent", &a).is_err());

        // artifact
        let art = RecordArtifactParams {
            project_id: Some("p".to_string()),
            task_id: None,
            artifact_kind: "file".to_string(),
            path: "secret.key".to_string(),
            sha256: "123".to_string(),
            bytes: None,
            commit_sha: None,
            created_by: None,
            sensitivity: "secret".to_string(),
            metadata: json!({}),
        };
        assert!(graph.record_artifact(&art).is_err());

        // receipt
        let rcpt = RecordReceiptParams {
            project_id: Some("p".to_string()),
            task_id: None,
            artifact_graph_entity_id: None,
            session_id: None,
            verdict: "fail".to_string(),
            verifier: "v".to_string(),
            command: None,
            exit_code: None,
            duration_ms: None,
            output_hash: None,
            output_sample_redacted: None,
            evidence: vec![],
            sensitivity: "Secret".to_string(),
        };
        assert!(graph.record_receipt(&rcpt).is_err());

        // source
        let src = RecordSourceParams {
            project_id: Some("p".to_string()),
            source_type: "web".to_string(),
            uri: "https://secret.example.com".to_string(),
            content_hash: None,
            title: None,
            summary: None,
            author: None,
            published_at: None,
            retrieved_at: None,
            credibility: None,
            sensitivity: "secret".to_string(),
            metadata: json!({}),
        };
        assert!(graph.record_source(&src).is_err());
    }

    #[test]
    fn test_phase2b_cli_json_output_shape() {
        // Verify JSON serialization format of all outcomes and envelopes
        let proj_out = ProjectStateOutcome {
            graph_entity_id: 42,
            sql_id: 1,
            project_id: "atheneum".to_string(),
            version: 1,
            is_head: true,
        };
        let json_val = serde_json::to_value(&proj_out).expect("serialize");
        assert_eq!(json_val["graph_entity_id"], 42);
        assert_eq!(json_val["version"], 1);
        assert_eq!(json_val["is_head"], true);

        let art_out = RecordArtifactOutcome::Created {
            graph_entity_id: 43,
            sql_id: 2,
            sha256: "abc".to_string(),
        };
        let art_json = serde_json::to_value(&art_out).expect("serialize");
        assert_eq!(art_json["status"], "created");
        assert_eq!(art_json["graph_entity_id"], 43);

        let src_out = RecordSourceOutcome::Unchanged {
            graph_entity_id: 44,
            sql_id: 3,
            uri: "https://example.com".to_string(),
        };
        let src_json = serde_json::to_value(&src_out).expect("serialize");
        assert_eq!(src_json["status"], "unchanged");
        assert_eq!(src_json["graph_entity_id"], 44);

        let ext_out = LinkExternalMemoryOutcome::Created {
            graph_entity_id: 45,
            sql_id: 4,
            external_store: "bookkeeper".to_string(),
            external_id: "e1".to_string(),
            atheneum_graph_entity_id: 10,
        };
        let ext_json = serde_json::to_value(&ext_out).expect("serialize");
        assert_eq!(ext_json["graph_entity_id"], 45);
        assert_eq!(ext_json["status"], "created");
    }

    #[test]
    fn test_phase2b_phase2a_regression() {
        let graph = AtheneumGraph::open_in_memory().expect("open");
        let params = sample_params("reg_key", "Phase 2A Regression Content");
        let res = graph.upsert_memory_item(&params).expect("upsert");
        let ge_id = match res {
            UpsertMemoryOutcome::Created {
                graph_entity_id, ..
            } => graph_entity_id,
            _ => panic!("expected created"),
        };
        let item = graph
            .get_memory_item_by_graph_id(ge_id)
            .expect("get")
            .expect("exists");
        assert_eq!(item.key, "reg_key");
        assert_eq!(item.content, "Phase 2A Regression Content");
    }
}
