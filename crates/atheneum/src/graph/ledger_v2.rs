//! Memory Router Phase 3 — Ledger extension, canonical NDJSON envelope, and import adapters.
//!
//! Provides:
//! - Canonical NDJSON schema for memory router records (Memory, Source, Artifact, Receipt, ExternalRef).
//! - Conflict-safe ledger import routing memory records through `upsert_memory_item`.
//! - Dry-run import with zero database mutations.
//! - Audit map generation recording merged/skipped/conflict/failed records.
//! - Hermes Markdown memory adapter.
//! - Bookkeeper Markdown export adapter.

use std::collections::{BTreeMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use regex::Regex;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::memory_router::{
    ConflictStrategy, LinkExternalMemoryParams, RecordArtifactParams, RecordReceiptParams,
    RecordSourceParams, UpsertMemoryItemParams, UpsertMemoryOutcome,
};
use super::AtheneumGraph;

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryRecordV2 {
    pub key: String,
    pub content: String,
    #[serde(default = "default_memory_scope")]
    pub scope: String,
    #[serde(default = "default_memory_kind")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default = "default_confidence")]
    pub confidence: f64,
    #[serde(default = "default_status")]
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_uri: Option<String>,
    #[serde(default)]
    pub evidence: Vec<Value>,
    pub content_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_estimate: Option<usize>,
    #[serde(default = "default_sensitivity")]
    pub sensitivity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_seconds: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_entity_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceRecordV2 {
    pub uri: String,
    pub source_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retrieved_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credibility: Option<f64>,
    #[serde(default = "default_sensitivity")]
    pub sensitivity: String,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArtifactRecordV2 {
    pub path: String,
    pub sha256: String,
    pub artifact_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_sha: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
    #[serde(default = "default_sensitivity")]
    pub sensitivity: String,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReceiptRecordV2 {
    pub verdict: String,
    pub verifier: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_graph_entity_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_sample_redacted: Option<String>,
    #[serde(default)]
    pub evidence: Vec<Value>,
    #[serde(default = "default_sensitivity")]
    pub sensitivity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExternalRefRecordV2 {
    pub external_store: String,
    pub external_id: String,
    pub atheneum_graph_entity_id: i64,
    pub atheneum_kind: String,
    pub content_hash: String,
    pub sync_state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_synced_at: Option<String>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "record_type", rename_all = "snake_case")]
pub enum CanonicalMemoryRecord {
    Memory(MemoryRecordV2),
    Source(SourceRecordV2),
    Artifact(ArtifactRecordV2),
    Receipt(ReceiptRecordV2),
    ExternalRef(ExternalRefRecordV2),
}

impl CanonicalMemoryRecord {
    pub fn sensitivity(&self) -> &str {
        match self {
            CanonicalMemoryRecord::Memory(m) => &m.sensitivity,
            CanonicalMemoryRecord::Source(s) => &s.sensitivity,
            CanonicalMemoryRecord::Artifact(a) => &a.sensitivity,
            CanonicalMemoryRecord::Receipt(r) => &r.sensitivity,
            CanonicalMemoryRecord::ExternalRef(_) => "normal",
        }
    }

    pub fn record_type(&self) -> &'static str {
        match self {
            CanonicalMemoryRecord::Memory(_) => "memory",
            CanonicalMemoryRecord::Source(_) => "source",
            CanonicalMemoryRecord::Artifact(_) => "artifact",
            CanonicalMemoryRecord::Receipt(_) => "receipt",
            CanonicalMemoryRecord::ExternalRef(_) => "external_ref",
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerV2Counts {
    pub merged: u64,
    pub skipped: u64,
    pub conflicts: u64,
    pub failed: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HermesAdoptSummary {
    pub status: String,
    pub records_extracted: usize,
    pub files_scanned: usize,
    pub out: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookkeeperAdoptSummary {
    pub status: String,
    pub records_extracted: usize,
    pub source_file: String,
    pub out: String,
}

fn decode_base64_str(input: &str) -> Option<String> {
    let mut bytes = Vec::new();
    let mut buf = 0u32;
    let mut bits = 0u32;
    for &b in input.trim().as_bytes() {
        let val = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            _ => continue,
        };
        buf = (buf << 6) | (val as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            bytes.push(((buf >> bits) & 0xff) as u8);
        }
    }
    String::from_utf8(bytes).ok()
}

fn collect_md_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_md_files(&path, out)?;
        } else if path.is_file() {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            if !name.starts_with('.') && path.extension().and_then(|e| e.to_str()) == Some("md") {
                out.push(path);
            }
        }
    }
    Ok(())
}

fn compute_sha256(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn write_map_entry(mut out: impl Write, entry: &Value) -> Result<()> {
    serde_json::to_writer(&mut out, entry)?;
    out.write_all(b"\n")?;
    Ok(())
}

/// Retry a store call when SQLite reports lock contention.
fn with_busy_retry<T>(mut f: impl FnMut() -> Result<T>) -> Result<T> {
    const MAX_ATTEMPTS: u32 = 5;
    let mut attempt = 0u32;
    loop {
        match f() {
            Ok(value) => return Ok(value),
            Err(err) => {
                let msg = format!("{:#}", err).to_ascii_lowercase();
                let busy = msg.contains("database is locked")
                    || msg.contains("database busy")
                    || msg.contains("sqlite_busy")
                    || msg.contains("database table is locked");
                if busy && attempt + 1 < MAX_ATTEMPTS {
                    attempt += 1;
                    std::thread::sleep(std::time::Duration::from_millis(250 * u64::from(attempt)));
                    continue;
                }
                return Err(err);
            }
        }
    }
}

/// Export ledger records out of a store as canonical NDJSON (one record per line).
/// Invariant: Records with sensitivity 'secret' are excluded from export.
pub fn export_ledger_v2(
    graph: &AtheneumGraph,
    kinds: Option<&[&str]>,
    until: Option<&str>,
    mut out: impl Write,
) -> Result<BTreeMap<String, u64>> {
    let all_kinds = ["memory", "source", "artifact", "receipt", "external_ref"];
    let selected_kinds: Vec<String> = match kinds {
        Some(k) if !k.is_empty() => k
            .iter()
            .map(|s| s.trim().to_ascii_lowercase())
            .map(|s| match s.as_str() {
                "memories" => "memory".to_string(),
                "sources" => "source".to_string(),
                "artifacts" => "artifact".to_string(),
                "receipts" => "receipt".to_string(),
                "external_refs" | "external_ref" => "external_ref".to_string(),
                other => other.to_string(),
            })
            .collect(),
        _ => all_kinds.iter().map(|s| s.to_string()).collect(),
    };

    let mut counts = BTreeMap::new();
    let until_owned = until.map(String::from);

    // 1. Memory entries
    if selected_kinds.contains(&"memory".to_string()) {
        let n = graph.with_raw_connection(|conn| {
            let (sql, with_until) = if until_owned.is_some() {
                (
                    "SELECT id, graph_entity_id, key, scope, content, confidence, project_id,
                            created_at, updated_at, kind, title, summary, status, source_uri,
                            evidence_json, content_hash, token_estimate, sensitivity, valid_from,
                            valid_to, ttl_seconds, supersedes_graph_entity_id
                     FROM memory_entries
                     WHERE status != 'superseded'
                       AND LOWER(sensitivity) != 'secret'
                       AND datetime(created_at) < datetime(?1)
                     ORDER BY id",
                    true,
                )
            } else {
                (
                    "SELECT id, graph_entity_id, key, scope, content, confidence, project_id,
                            created_at, updated_at, kind, title, summary, status, source_uri,
                            evidence_json, content_hash, token_estimate, sensitivity, valid_from,
                            valid_to, ttl_seconds, supersedes_graph_entity_id
                     FROM memory_entries
                     WHERE status != 'superseded'
                       AND LOWER(sensitivity) != 'secret'
                     ORDER BY id",
                    false,
                )
            };

            let mut stmt = conn.prepare(sql)?;
            let mut count = 0u64;
            let mut rows = if with_until {
                stmt.query(params![until_owned.as_deref().unwrap_or_default()])?
            } else {
                stmt.query([])?
            };

            while let Some(row) = rows.next()? {
                let _id: i64 = row.get(0)?;
                let graph_entity_id: Option<i64> = row.get(1)?;
                let key: String = row.get(2)?;
                let scope: String = row.get(3)?;
                let content: String = row.get(4)?;
                let confidence: f64 = row.get(5)?;
                let project_id: Option<String> = row.get(6)?;
                let _created_at: String = row.get(7)?;
                let _updated_at: Option<String> = row.get(8)?;
                let kind: String = row.get(9)?;
                let title: Option<String> = row.get(10)?;
                let summary: Option<String> = row.get(11)?;
                let status: String = row.get(12)?;
                let source_uri: Option<String> = row.get(13)?;
                let evidence_str: String = row.get(14)?;
                let content_hash_opt: Option<String> = row.get(15)?;
                let token_estimate: Option<usize> =
                    row.get::<_, Option<i64>>(16)?.map(|n| n as usize);
                let sensitivity: String = row.get(17)?;
                let valid_from: Option<String> = row.get(18)?;
                let valid_to: Option<String> = row.get(19)?;
                let ttl_seconds: Option<i64> = row.get(20)?;

                let evidence: Vec<Value> = serde_json::from_str(&evidence_str).unwrap_or_default();
                let content_hash = content_hash_opt.unwrap_or_else(|| compute_sha256(&content));

                let rec = CanonicalMemoryRecord::Memory(MemoryRecordV2 {
                    key,
                    content,
                    scope,
                    kind,
                    title,
                    summary,
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
                    tags: None,
                    graph_entity_id,
                });

                serde_json::to_writer(&mut out, &rec)?;
                out.write_all(b"\n")?;
                count += 1;
            }
            Ok(count)
        })?;
        counts.insert("memory".to_string(), n);
    }

    // 2. Sources
    if selected_kinds.contains(&"source".to_string()) {
        let n = graph.with_raw_connection(|conn| {
            let (sql, with_until) = if until_owned.is_some() {
                (
                    "SELECT uri, source_type, project_id, content_hash, title, summary,
                            author, published_at, retrieved_at, credibility, sensitivity, metadata_json
                     FROM sources
                     WHERE LOWER(sensitivity) != 'secret'
                       AND datetime(created_at) < datetime(?1)
                     ORDER BY id",
                    true,
                )
            } else {
                (
                    "SELECT uri, source_type, project_id, content_hash, title, summary,
                            author, published_at, retrieved_at, credibility, sensitivity, metadata_json
                     FROM sources
                     WHERE LOWER(sensitivity) != 'secret'
                     ORDER BY id",
                    false,
                )
            };

            let mut stmt = conn.prepare(sql)?;
            let mut count = 0u64;
            let mut rows = if with_until {
                stmt.query(params![until_owned.as_deref().unwrap_or_default()])?
            } else {
                stmt.query([])?
            };

            while let Some(row) = rows.next()? {
                let uri: String = row.get(0)?;
                let source_type: String = row.get(1)?;
                let project_id: Option<String> = row.get(2)?;
                let content_hash: Option<String> = row.get(3)?;
                let title: Option<String> = row.get(4)?;
                let summary: Option<String> = row.get(5)?;
                let author: Option<String> = row.get(6)?;
                let published_at: Option<String> = row.get(7)?;
                let retrieved_at: Option<String> = row.get(8)?;
                let credibility: Option<f64> = row.get(9)?;
                let sensitivity: String = row.get(10)?;
                let meta_str: String = row.get(11)?;
                let metadata: Value = serde_json::from_str(&meta_str).unwrap_or_default();

                let rec = CanonicalMemoryRecord::Source(SourceRecordV2 {
                    uri,
                    source_type,
                    project_id,
                    content_hash,
                    title,
                    summary,
                    author,
                    published_at,
                    retrieved_at,
                    credibility,
                    sensitivity,
                    metadata,
                });
                serde_json::to_writer(&mut out, &rec)?;
                out.write_all(b"\n")?;
                count += 1;
            }
            Ok(count)
        })?;
        counts.insert("source".to_string(), n);
    }

    // 3. Artifacts
    if selected_kinds.contains(&"artifact".to_string()) {
        let n = graph.with_raw_connection(|conn| {
            let (sql, with_until) = if until_owned.is_some() {
                (
                    "SELECT path, sha256, artifact_kind, project_id, task_id, bytes, commit_sha,
                            created_by, sensitivity, metadata_json
                     FROM artifacts
                     WHERE LOWER(sensitivity) != 'secret'
                       AND datetime(created_at) < datetime(?1)
                     ORDER BY id",
                    true,
                )
            } else {
                (
                    "SELECT path, sha256, artifact_kind, project_id, task_id, bytes, commit_sha,
                            created_by, sensitivity, metadata_json
                     FROM artifacts
                     WHERE LOWER(sensitivity) != 'secret'
                     ORDER BY id",
                    false,
                )
            };

            let mut stmt = conn.prepare(sql)?;
            let mut count = 0u64;
            let mut rows = if with_until {
                stmt.query(params![until_owned.as_deref().unwrap_or_default()])?
            } else {
                stmt.query([])?
            };

            while let Some(row) = rows.next()? {
                let path: String = row.get(0)?;
                let sha256: String = row.get(1)?;
                let artifact_kind: String = row.get(2)?;
                let project_id: Option<String> = row.get(3)?;
                let task_id: Option<i64> = row.get(4)?;
                let bytes: Option<i64> = row.get(5)?;
                let commit_sha: Option<String> = row.get(6)?;
                let created_by: Option<String> = row.get(7)?;
                let sensitivity: String = row.get(8)?;
                let meta_str: String = row.get(9)?;
                let metadata: Value = serde_json::from_str(&meta_str).unwrap_or_default();

                let rec = CanonicalMemoryRecord::Artifact(ArtifactRecordV2 {
                    path,
                    sha256,
                    artifact_kind,
                    project_id,
                    task_id,
                    bytes,
                    commit_sha,
                    created_by,
                    sensitivity,
                    metadata,
                });
                serde_json::to_writer(&mut out, &rec)?;
                out.write_all(b"\n")?;
                count += 1;
            }
            Ok(count)
        })?;
        counts.insert("artifact".to_string(), n);
    }

    // 4. Receipts
    if selected_kinds.contains(&"receipt".to_string()) {
        let n = graph.with_raw_connection(|conn| {
            let (sql, with_until) = if until_owned.is_some() {
                (
                    "SELECT verdict, verifier, project_id, task_id, artifact_graph_entity_id, session_id,
                            command, exit_code, duration_ms, output_hash, output_sample_redacted,
                            evidence_json, sensitivity
                     FROM receipts
                     WHERE LOWER(sensitivity) != 'secret'
                       AND datetime(created_at) < datetime(?1)
                     ORDER BY id",
                    true,
                )
            } else {
                (
                    "SELECT verdict, verifier, project_id, task_id, artifact_graph_entity_id, session_id,
                            command, exit_code, duration_ms, output_hash, output_sample_redacted,
                            evidence_json, sensitivity
                     FROM receipts
                     WHERE LOWER(sensitivity) != 'secret'
                     ORDER BY id",
                    false,
                )
            };

            let mut stmt = conn.prepare(sql)?;
            let mut count = 0u64;
            let mut rows = if with_until {
                stmt.query(params![until_owned.as_deref().unwrap_or_default()])?
            } else {
                stmt.query([])?
            };

            while let Some(row) = rows.next()? {
                let verdict: String = row.get(0)?;
                let verifier: String = row.get(1)?;
                let project_id: Option<String> = row.get(2)?;
                let task_id: Option<i64> = row.get(3)?;
                let artifact_graph_entity_id: Option<i64> = row.get(4)?;
                let session_id: Option<String> = row.get(5)?;
                let command: Option<String> = row.get(6)?;
                let exit_code: Option<i32> = row.get(7)?;
                let duration_ms: Option<i64> = row.get(8)?;
                let output_hash: Option<String> = row.get(9)?;
                let output_sample_redacted: Option<String> = row.get(10)?;
                let evidence_str: String = row.get(11)?;
                let sensitivity: String = row.get(12)?;
                let evidence: Vec<Value> = serde_json::from_str(&evidence_str).unwrap_or_default();

                let rec = CanonicalMemoryRecord::Receipt(ReceiptRecordV2 {
                    verdict,
                    verifier,
                    project_id,
                    task_id,
                    artifact_graph_entity_id,
                    session_id,
                    command,
                    exit_code,
                    duration_ms,
                    output_hash,
                    output_sample_redacted,
                    evidence,
                    sensitivity,
                });
                serde_json::to_writer(&mut out, &rec)?;
                out.write_all(b"\n")?;
                count += 1;
            }
            Ok(count)
        })?;
        counts.insert("receipt".to_string(), n);
    }

    // 5. External memory refs
    if selected_kinds.contains(&"external_ref".to_string()) {
        let n = graph.with_raw_connection(|conn| {
            let (sql, with_until) = if until_owned.is_some() {
                (
                    "SELECT external_store, external_id, atheneum_graph_entity_id, atheneum_kind,
                            content_hash, sync_state, last_synced_at, metadata_json
                     FROM external_memory_refs
                     WHERE datetime(last_synced_at) < datetime(?1)
                     ORDER BY id",
                    true,
                )
            } else {
                (
                    "SELECT external_store, external_id, atheneum_graph_entity_id, atheneum_kind,
                            content_hash, sync_state, last_synced_at, metadata_json
                     FROM external_memory_refs
                     ORDER BY id",
                    false,
                )
            };

            let mut stmt = conn.prepare(sql)?;
            let mut count = 0u64;
            let mut rows = if with_until {
                stmt.query(params![until_owned.as_deref().unwrap_or_default()])?
            } else {
                stmt.query([])?
            };

            while let Some(row) = rows.next()? {
                let external_store: String = row.get(0)?;
                let external_id: String = row.get(1)?;
                let atheneum_graph_entity_id: i64 = row.get(2)?;
                let atheneum_kind: String = row.get(3)?;
                let content_hash: String = row.get(4)?;
                let sync_state: String = row.get(5)?;
                let last_synced_at: Option<String> = row.get(6)?;
                let meta_str: String = row.get(7)?;
                let metadata: Value = serde_json::from_str(&meta_str).unwrap_or_default();

                let rec = CanonicalMemoryRecord::ExternalRef(ExternalRefRecordV2 {
                    external_store,
                    external_id,
                    atheneum_graph_entity_id,
                    atheneum_kind,
                    content_hash,
                    sync_state,
                    last_synced_at,
                    metadata,
                });
                serde_json::to_writer(&mut out, &rec)?;
                out.write_all(b"\n")?;
                count += 1;
            }
            Ok(count)
        })?;
        counts.insert("external_ref".to_string(), n);
    }

    Ok(counts)
}

/// Parse a raw line into CanonicalMemoryRecord or adapt from legacy/flexible shapes.
fn parse_incoming_record(line: &str) -> Result<CanonicalMemoryRecord> {
    if let Ok(rec) = serde_json::from_str::<CanonicalMemoryRecord>(line) {
        return Ok(rec);
    }

    let val: Value = serde_json::from_str(line).context("parse JSON")?;
    let record_type = val
        .get("record_type")
        .or_else(|| val.get("kind"))
        .and_then(|v| v.as_str())
        .unwrap_or("memory");

    match record_type {
        "memory" | "decision" | "lesson" | "preference" | "failure" | "correction"
        | "tool_quirk" | "convention" => {
            let key = val
                .get("key")
                .or_else(|| val.get("target"))
                .and_then(|v| v.as_str())
                .unwrap_or("unnamed_key")
                .to_string();
            let content = val
                .get("content")
                .or_else(|| val.get("metadata").and_then(|m| m.get("content")))
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("memory record missing content"))?
                .to_string();
            let scope = val
                .get("scope")
                .or_else(|| val.get("metadata").and_then(|m| m.get("scope")))
                .and_then(|v| v.as_str())
                .unwrap_or("user")
                .to_string();
            let kind = val
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("memory")
                .to_string();
            let title = val.get("title").and_then(|v| v.as_str()).map(String::from);
            let summary = val
                .get("summary")
                .and_then(|v| v.as_str())
                .map(String::from);
            let confidence = val
                .get("confidence")
                .and_then(|v| v.as_f64())
                .unwrap_or(1.0);
            let status = val
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("active")
                .to_string();
            let project_id = val
                .get("project_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            let source_uri = val
                .get("source_uri")
                .and_then(|v| v.as_str())
                .map(String::from);
            let evidence = val
                .get("evidence")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let content_hash = val
                .get("content_hash")
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or_else(|| compute_sha256(&content));
            let sensitivity = val
                .get("sensitivity")
                .and_then(|v| v.as_str())
                .unwrap_or("normal")
                .to_string();

            Ok(CanonicalMemoryRecord::Memory(MemoryRecordV2 {
                key,
                content,
                scope,
                kind,
                title,
                summary,
                confidence,
                status,
                project_id,
                source_uri,
                evidence,
                content_hash,
                token_estimate: None,
                sensitivity,
                valid_from: val
                    .get("valid_from")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                valid_to: val
                    .get("valid_to")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                ttl_seconds: val.get("ttl_seconds").and_then(|v| v.as_i64()),
                tags: None,
                graph_entity_id: val.get("graph_entity_id").and_then(|v| v.as_i64()),
            }))
        }
        "source" => {
            let uri = val
                .get("uri")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("source record missing uri"))?
                .to_string();
            let source_type = val
                .get("source_type")
                .and_then(|v| v.as_str())
                .unwrap_or("web")
                .to_string();
            let project_id = val
                .get("project_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            let content_hash = val
                .get("content_hash")
                .and_then(|v| v.as_str())
                .map(String::from);
            let title = val.get("title").and_then(|v| v.as_str()).map(String::from);
            let summary = val
                .get("summary")
                .and_then(|v| v.as_str())
                .map(String::from);
            let author = val.get("author").and_then(|v| v.as_str()).map(String::from);
            let published_at = val
                .get("published_at")
                .and_then(|v| v.as_str())
                .map(String::from);
            let retrieved_at = val
                .get("retrieved_at")
                .and_then(|v| v.as_str())
                .map(String::from);
            let credibility = val.get("credibility").and_then(|v| v.as_f64());
            let sensitivity = val
                .get("sensitivity")
                .and_then(|v| v.as_str())
                .unwrap_or("normal")
                .to_string();
            let metadata = val.get("metadata").cloned().unwrap_or_else(|| json!({}));

            Ok(CanonicalMemoryRecord::Source(SourceRecordV2 {
                uri,
                source_type,
                project_id,
                content_hash,
                title,
                summary,
                author,
                published_at,
                retrieved_at,
                credibility,
                sensitivity,
                metadata,
            }))
        }
        "artifact" => {
            let path = val
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("artifact missing path"))?
                .to_string();
            let sha256 = val
                .get("sha256")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("artifact missing sha256"))?
                .to_string();
            let artifact_kind = val
                .get("artifact_kind")
                .and_then(|v| v.as_str())
                .unwrap_or("file")
                .to_string();
            let project_id = val
                .get("project_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            let task_id = val.get("task_id").and_then(|v| v.as_i64());
            let bytes = val.get("bytes").and_then(|v| v.as_i64());
            let commit_sha = val
                .get("commit_sha")
                .and_then(|v| v.as_str())
                .map(String::from);
            let created_by = val
                .get("created_by")
                .and_then(|v| v.as_str())
                .map(String::from);
            let sensitivity = val
                .get("sensitivity")
                .and_then(|v| v.as_str())
                .unwrap_or("normal")
                .to_string();
            let metadata = val.get("metadata").cloned().unwrap_or_else(|| json!({}));

            Ok(CanonicalMemoryRecord::Artifact(ArtifactRecordV2 {
                path,
                sha256,
                artifact_kind,
                project_id,
                task_id,
                bytes,
                commit_sha,
                created_by,
                sensitivity,
                metadata,
            }))
        }
        "receipt" => {
            let verdict = val
                .get("verdict")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("receipt missing verdict"))?
                .to_string();
            let verifier = val
                .get("verifier")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("receipt missing verifier"))?
                .to_string();
            let project_id = val
                .get("project_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            let task_id = val.get("task_id").and_then(|v| v.as_i64());
            let artifact_graph_entity_id =
                val.get("artifact_graph_entity_id").and_then(|v| v.as_i64());
            let session_id = val
                .get("session_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            let command = val
                .get("command")
                .and_then(|v| v.as_str())
                .map(String::from);
            let exit_code = val
                .get("exit_code")
                .and_then(|v| v.as_i64())
                .map(|c| c as i32);
            let duration_ms = val.get("duration_ms").and_then(|v| v.as_i64());
            let output_hash = val
                .get("output_hash")
                .and_then(|v| v.as_str())
                .map(String::from);
            let output_sample_redacted = val
                .get("output_sample_redacted")
                .and_then(|v| v.as_str())
                .map(String::from);
            let evidence = val
                .get("evidence")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let sensitivity = val
                .get("sensitivity")
                .and_then(|v| v.as_str())
                .unwrap_or("normal")
                .to_string();

            Ok(CanonicalMemoryRecord::Receipt(ReceiptRecordV2 {
                verdict,
                verifier,
                project_id,
                task_id,
                artifact_graph_entity_id,
                session_id,
                command,
                exit_code,
                duration_ms,
                output_hash,
                output_sample_redacted,
                evidence,
                sensitivity,
            }))
        }
        "external_ref" => {
            let external_store = val
                .get("external_store")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("external_ref missing external_store"))?
                .to_string();
            let external_id = val
                .get("external_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("external_ref missing external_id"))?
                .to_string();
            let atheneum_graph_entity_id = val
                .get("atheneum_graph_entity_id")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let atheneum_kind = val
                .get("atheneum_kind")
                .and_then(|v| v.as_str())
                .unwrap_or("Memory")
                .to_string();
            let content_hash = val
                .get("content_hash")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let sync_state = val
                .get("sync_state")
                .and_then(|v| v.as_str())
                .unwrap_or("linked")
                .to_string();
            let last_synced_at = val
                .get("last_synced_at")
                .and_then(|v| v.as_str())
                .map(String::from);
            let metadata = val.get("metadata").cloned().unwrap_or_else(|| json!({}));

            Ok(CanonicalMemoryRecord::ExternalRef(ExternalRefRecordV2 {
                external_store,
                external_id,
                atheneum_graph_entity_id,
                atheneum_kind,
                content_hash,
                sync_state,
                last_synced_at,
                metadata,
            }))
        }
        other => anyhow::bail!("unknown or unsupported record kind '{}'", other),
    }
}

/// Import ledger records from NDJSON file into store through conflict-safe APIs.
/// Dry run performs zero mutations.
/// Records with sensitivity=secret are rejected at import.
pub fn import_ledger_v2(
    graph: &AtheneumGraph,
    path: &Path,
    dry_run: bool,
    conflict_strategy: ConflictStrategy,
    mut map_out: impl Write,
) -> Result<LedgerV2Counts> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("open ledger file {}", path.display()))?;
    let reader = BufReader::new(file);

    let mut counts = LedgerV2Counts::default();

    // Track simulated keys seen during dry run or within this file
    let mut dry_run_seen_memories: HashSet<(String, String, Option<String>)> = HashSet::new();
    let mut dry_run_seen_sources: HashSet<(Option<String>, String)> = HashSet::new();
    let mut dry_run_seen_artifacts: HashSet<(Option<String>, String, String)> = HashSet::new();
    let mut dry_run_seen_external_refs: HashSet<(String, String)> = HashSet::new();

    for (idx, line_res) in reader.lines().enumerate() {
        let line_no = idx + 1;
        let line = line_res.with_context(|| format!("read line {}", line_no))?;
        if line.trim().is_empty() {
            continue;
        }

        let record = match parse_incoming_record(&line) {
            Ok(rec) => rec,
            Err(err) => {
                counts.failed += 1;
                write_map_entry(
                    &mut map_out,
                    &json!({
                        "line": line_no,
                        "status": "failed",
                        "error": format!("parse error: {}", err),
                    }),
                )?;
                continue;
            }
        };

        // Invariant: sensitivity=secret records rejected at ingest
        if record.sensitivity().eq_ignore_ascii_case("secret") {
            counts.failed += 1;
            write_map_entry(
                &mut map_out,
                &json!({
                    "line": line_no,
                    "record_type": record.record_type(),
                    "status": "failed",
                    "error": "sensitivity=secret records are rejected at import",
                }),
            )?;
            continue;
        }

        match record {
            CanonicalMemoryRecord::Memory(m) => {
                let mem_key = (m.key.clone(), m.scope.clone(), m.project_id.clone());

                if dry_run {
                    let existing: Option<(i64, Option<i64>, Option<String>)> = graph
                        .with_raw_connection(|conn| {
                            let mut stmt = conn.prepare(
                                "SELECT id, graph_entity_id, content_hash
                             FROM memory_entries
                             WHERE key = ?1 AND scope = ?2
                               AND COALESCE(project_id, '') = COALESCE(?3, '')
                               AND status != 'superseded'
                             ORDER BY id DESC LIMIT 1",
                            )?;
                            let row = stmt
                                .query_row(params![m.key, m.scope, m.project_id.as_deref()], |r| {
                                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                                })
                                .ok();
                            Ok(row)
                        })?;

                    if dry_run_seen_memories.contains(&mem_key) {
                        counts.skipped += 1;
                        write_map_entry(
                            &mut map_out,
                            &json!({
                                "line": line_no,
                                "record_type": "memory",
                                "target": m.key,
                                "content_hash": m.content_hash,
                                "status": "skipped",
                                "reason": "duplicate entry in import file",
                            }),
                        )?;
                        continue;
                    }

                    match existing {
                        None => {
                            counts.merged += 1;
                            dry_run_seen_memories.insert(mem_key);
                            write_map_entry(
                                &mut map_out,
                                &json!({
                                    "line": line_no,
                                    "record_type": "memory",
                                    "target": m.key,
                                    "content_hash": m.content_hash,
                                    "status": "would_merge",
                                    "new_id": Value::Null,
                                }),
                            )?;
                        }
                        Some((_sql_id, ge_id, hash_opt)) => {
                            let ex_hash = hash_opt.unwrap_or_default();
                            if ex_hash == m.content_hash {
                                counts.skipped += 1;
                                write_map_entry(
                                    &mut map_out,
                                    &json!({
                                        "line": line_no,
                                        "record_type": "memory",
                                        "target": m.key,
                                        "content_hash": m.content_hash,
                                        "status": "skipped",
                                        "existing_id": ge_id,
                                    }),
                                )?;
                            } else if conflict_strategy == ConflictStrategy::Revision {
                                counts.merged += 1;
                                dry_run_seen_memories.insert(mem_key);
                                write_map_entry(
                                    &mut map_out,
                                    &json!({
                                        "line": line_no,
                                        "record_type": "memory",
                                        "target": m.key,
                                        "content_hash": m.content_hash,
                                        "status": "would_revision",
                                        "supersedes_id": ge_id,
                                    }),
                                )?;
                            } else {
                                counts.conflicts += 1;
                                write_map_entry(
                                    &mut map_out,
                                    &json!({
                                        "line": line_no,
                                        "record_type": "memory",
                                        "target": m.key,
                                        "content_hash": m.content_hash,
                                        "existing_content_hash": ex_hash,
                                        "existing_id": ge_id,
                                        "status": "conflict",
                                        "reason": "key collision with different content hash",
                                    }),
                                )?;
                            }
                        }
                    }
                } else {
                    let params = UpsertMemoryItemParams {
                        key: m.key.clone(),
                        content: m.content.clone(),
                        scope: m.scope.clone(),
                        kind: m.kind.clone(),
                        title: m.title.clone(),
                        summary: m.summary.clone(),
                        confidence: m.confidence,
                        project_id: m.project_id.clone(),
                        status: m.status.clone(),
                        source_uri: m.source_uri.clone(),
                        evidence: m.evidence.clone(),
                        content_hash: Some(m.content_hash.clone()),
                        token_estimate: m.token_estimate,
                        sensitivity: m.sensitivity.clone(),
                        valid_from: m.valid_from.clone(),
                        valid_to: m.valid_to.clone(),
                        ttl_seconds: m.ttl_seconds,
                        tags: m.tags.clone(),
                        conflict_strategy,
                    };

                    match with_busy_retry(|| graph.upsert_memory_item(&params)) {
                        Ok(UpsertMemoryOutcome::Created {
                            graph_entity_id,
                            content_hash,
                            ..
                        }) => {
                            counts.merged += 1;
                            write_map_entry(
                                &mut map_out,
                                &json!({
                                    "line": line_no,
                                    "record_type": "memory",
                                    "target": m.key,
                                    "content_hash": content_hash,
                                    "status": "merged",
                                    "new_id": graph_entity_id,
                                }),
                            )?;
                        }
                        Ok(UpsertMemoryOutcome::Unchanged {
                            graph_entity_id,
                            content_hash,
                            ..
                        }) => {
                            counts.skipped += 1;
                            write_map_entry(
                                &mut map_out,
                                &json!({
                                    "line": line_no,
                                    "record_type": "memory",
                                    "target": m.key,
                                    "content_hash": content_hash,
                                    "status": "skipped",
                                    "existing_id": graph_entity_id,
                                }),
                            )?;
                        }
                        Ok(UpsertMemoryOutcome::RevisionCreated {
                            graph_entity_id,
                            supersedes_graph_entity_id,
                            content_hash,
                            ..
                        }) => {
                            counts.merged += 1;
                            write_map_entry(
                                &mut map_out,
                                &json!({
                                    "line": line_no,
                                    "record_type": "memory",
                                    "target": m.key,
                                    "content_hash": content_hash,
                                    "status": "revision_created",
                                    "new_id": graph_entity_id,
                                    "supersedes_id": supersedes_graph_entity_id,
                                }),
                            )?;
                        }
                        Ok(UpsertMemoryOutcome::Conflict {
                            existing_graph_entity_id,
                            existing_content_hash,
                            incoming_content_hash,
                            reason,
                        }) => {
                            counts.conflicts += 1;
                            write_map_entry(
                                &mut map_out,
                                &json!({
                                    "line": line_no,
                                    "record_type": "memory",
                                    "target": m.key,
                                    "content_hash": incoming_content_hash,
                                    "existing_content_hash": existing_content_hash,
                                    "existing_id": existing_graph_entity_id,
                                    "status": "conflict",
                                    "reason": reason,
                                }),
                            )?;
                        }
                        Err(err) => {
                            counts.failed += 1;
                            write_map_entry(
                                &mut map_out,
                                &json!({
                                    "line": line_no,
                                    "record_type": "memory",
                                    "target": m.key,
                                    "content_hash": m.content_hash,
                                    "status": "failed",
                                    "error": format!("{:#}", err),
                                }),
                            )?;
                        }
                    }
                }
            }
            CanonicalMemoryRecord::Source(s) => {
                let src_key = (s.project_id.clone(), s.uri.clone());

                if dry_run {
                    let existing: Option<i64> = graph.with_raw_connection(|conn| {
                        let mut stmt = conn.prepare(
                            "SELECT graph_entity_id FROM sources
                             WHERE COALESCE(project_id, '') = COALESCE(?1, '')
                               AND uri = ?2 LIMIT 1",
                        )?;
                        let row = stmt
                            .query_row(params![s.project_id.as_deref(), s.uri], |r| r.get(0))
                            .ok();
                        Ok(row)
                    })?;

                    if existing.is_some() || dry_run_seen_sources.contains(&src_key) {
                        counts.skipped += 1;
                        write_map_entry(
                            &mut map_out,
                            &json!({
                                "line": line_no,
                                "record_type": "source",
                                "target": s.uri,
                                "status": "skipped",
                                "existing_id": existing,
                            }),
                        )?;
                    } else {
                        counts.merged += 1;
                        dry_run_seen_sources.insert(src_key);
                        write_map_entry(
                            &mut map_out,
                            &json!({
                                "line": line_no,
                                "record_type": "source",
                                "target": s.uri,
                                "status": "would_merge",
                            }),
                        )?;
                    }
                } else {
                    let params = RecordSourceParams {
                        project_id: s.project_id.clone(),
                        source_type: s.source_type.clone(),
                        uri: s.uri.clone(),
                        content_hash: s.content_hash.clone(),
                        title: s.title.clone(),
                        summary: s.summary.clone(),
                        author: s.author.clone(),
                        published_at: s.published_at.clone(),
                        retrieved_at: s.retrieved_at.clone(),
                        credibility: s.credibility,
                        sensitivity: s.sensitivity.clone(),
                        metadata: s.metadata.clone(),
                    };

                    match with_busy_retry(|| graph.record_source(&params)) {
                        Ok(super::RecordSourceOutcome::Created {
                            graph_entity_id, ..
                        }) => {
                            counts.merged += 1;
                            write_map_entry(
                                &mut map_out,
                                &json!({
                                    "line": line_no,
                                    "record_type": "source",
                                    "target": s.uri,
                                    "status": "merged",
                                    "new_id": graph_entity_id,
                                }),
                            )?;
                        }
                        Ok(super::RecordSourceOutcome::Unchanged {
                            graph_entity_id, ..
                        }) => {
                            counts.skipped += 1;
                            write_map_entry(
                                &mut map_out,
                                &json!({
                                    "line": line_no,
                                    "record_type": "source",
                                    "target": s.uri,
                                    "status": "skipped",
                                    "existing_id": graph_entity_id,
                                }),
                            )?;
                        }
                        Err(err) => {
                            counts.failed += 1;
                            write_map_entry(
                                &mut map_out,
                                &json!({
                                    "line": line_no,
                                    "record_type": "source",
                                    "target": s.uri,
                                    "status": "failed",
                                    "error": format!("{:#}", err),
                                }),
                            )?;
                        }
                    }
                }
            }
            CanonicalMemoryRecord::Artifact(a) => {
                let art_key = (a.project_id.clone(), a.path.clone(), a.sha256.clone());

                if dry_run {
                    let existing: Option<i64> = graph.with_raw_connection(|conn| {
                        let mut stmt = conn.prepare(
                            "SELECT graph_entity_id FROM artifacts
                             WHERE COALESCE(project_id, '') = COALESCE(?1, '')
                               AND path = ?2
                               AND sha256 = ?3 LIMIT 1",
                        )?;
                        let row = stmt
                            .query_row(params![a.project_id.as_deref(), a.path, a.sha256], |r| {
                                r.get(0)
                            })
                            .ok();
                        Ok(row)
                    })?;

                    if existing.is_some() || dry_run_seen_artifacts.contains(&art_key) {
                        counts.skipped += 1;
                        write_map_entry(
                            &mut map_out,
                            &json!({
                                "line": line_no,
                                "record_type": "artifact",
                                "target": a.path,
                                "sha256": a.sha256,
                                "status": "skipped",
                                "existing_id": existing,
                            }),
                        )?;
                    } else {
                        counts.merged += 1;
                        dry_run_seen_artifacts.insert(art_key);
                        write_map_entry(
                            &mut map_out,
                            &json!({
                                "line": line_no,
                                "record_type": "artifact",
                                "target": a.path,
                                "sha256": a.sha256,
                                "status": "would_merge",
                            }),
                        )?;
                    }
                } else {
                    let params = RecordArtifactParams {
                        project_id: a.project_id.clone(),
                        task_id: a.task_id,
                        artifact_kind: a.artifact_kind.clone(),
                        path: a.path.clone(),
                        sha256: a.sha256.clone(),
                        bytes: a.bytes,
                        commit_sha: a.commit_sha.clone(),
                        created_by: a.created_by.clone(),
                        sensitivity: a.sensitivity.clone(),
                        metadata: a.metadata.clone(),
                    };

                    match with_busy_retry(|| graph.record_artifact(&params)) {
                        Ok(super::RecordArtifactOutcome::Created {
                            graph_entity_id, ..
                        }) => {
                            counts.merged += 1;
                            write_map_entry(
                                &mut map_out,
                                &json!({
                                    "line": line_no,
                                    "record_type": "artifact",
                                    "target": a.path,
                                    "sha256": a.sha256,
                                    "status": "merged",
                                    "new_id": graph_entity_id,
                                }),
                            )?;
                        }
                        Ok(super::RecordArtifactOutcome::Unchanged {
                            graph_entity_id, ..
                        }) => {
                            counts.skipped += 1;
                            write_map_entry(
                                &mut map_out,
                                &json!({
                                    "line": line_no,
                                    "record_type": "artifact",
                                    "target": a.path,
                                    "sha256": a.sha256,
                                    "status": "skipped",
                                    "existing_id": graph_entity_id,
                                }),
                            )?;
                        }
                        Err(err) => {
                            counts.failed += 1;
                            write_map_entry(
                                &mut map_out,
                                &json!({
                                    "line": line_no,
                                    "record_type": "artifact",
                                    "target": a.path,
                                    "sha256": a.sha256,
                                    "status": "failed",
                                    "error": format!("{:#}", err),
                                }),
                            )?;
                        }
                    }
                }
            }
            CanonicalMemoryRecord::Receipt(r) => {
                if dry_run {
                    counts.merged += 1;
                    write_map_entry(
                        &mut map_out,
                        &json!({
                            "line": line_no,
                            "record_type": "receipt",
                            "verifier": r.verifier,
                            "verdict": r.verdict,
                            "status": "would_merge",
                        }),
                    )?;
                } else {
                    let params = RecordReceiptParams {
                        project_id: r.project_id.clone(),
                        task_id: r.task_id,
                        artifact_graph_entity_id: r.artifact_graph_entity_id,
                        session_id: r.session_id.clone(),
                        verdict: r.verdict.clone(),
                        verifier: r.verifier.clone(),
                        command: r.command.clone(),
                        exit_code: r.exit_code,
                        duration_ms: r.duration_ms,
                        output_hash: r.output_hash.clone(),
                        output_sample_redacted: r.output_sample_redacted.clone(),
                        evidence: r.evidence.clone(),
                        sensitivity: r.sensitivity.clone(),
                    };

                    match with_busy_retry(|| graph.record_receipt(&params)) {
                        Ok(super::RecordReceiptOutcome::Created {
                            graph_entity_id, ..
                        }) => {
                            counts.merged += 1;
                            write_map_entry(
                                &mut map_out,
                                &json!({
                                    "line": line_no,
                                    "record_type": "receipt",
                                    "verifier": r.verifier,
                                    "verdict": r.verdict,
                                    "status": "merged",
                                    "new_id": graph_entity_id,
                                }),
                            )?;
                        }
                        Err(err) => {
                            counts.failed += 1;
                            write_map_entry(
                                &mut map_out,
                                &json!({
                                    "line": line_no,
                                    "record_type": "receipt",
                                    "verifier": r.verifier,
                                    "verdict": r.verdict,
                                    "status": "failed",
                                    "error": format!("{:#}", err),
                                }),
                            )?;
                        }
                    }
                }
            }
            CanonicalMemoryRecord::ExternalRef(e) => {
                let ext_key = (e.external_store.clone(), e.external_id.clone());

                if dry_run {
                    let existing: Option<i64> = graph.with_raw_connection(|conn| {
                        let mut stmt = conn.prepare(
                            "SELECT graph_entity_id FROM external_memory_refs
                             WHERE external_store = ?1 AND external_id = ?2 LIMIT 1",
                        )?;
                        let row = stmt
                            .query_row(params![e.external_store, e.external_id], |r| r.get(0))
                            .ok();
                        Ok(row)
                    })?;

                    if existing.is_some() || dry_run_seen_external_refs.contains(&ext_key) {
                        counts.skipped += 1;
                        write_map_entry(
                            &mut map_out,
                            &json!({
                                "line": line_no,
                                "record_type": "external_ref",
                                "external_store": e.external_store,
                                "external_id": e.external_id,
                                "status": "skipped",
                                "existing_id": existing,
                            }),
                        )?;
                    } else {
                        counts.merged += 1;
                        dry_run_seen_external_refs.insert(ext_key);
                        write_map_entry(
                            &mut map_out,
                            &json!({
                                "line": line_no,
                                "record_type": "external_ref",
                                "external_store": e.external_store,
                                "external_id": e.external_id,
                                "status": "would_merge",
                            }),
                        )?;
                    }
                } else {
                    let params = LinkExternalMemoryParams {
                        external_store: e.external_store.clone(),
                        external_id: e.external_id.clone(),
                        atheneum_graph_entity_id: e.atheneum_graph_entity_id,
                        atheneum_kind: e.atheneum_kind.clone(),
                        content_hash: e.content_hash.clone(),
                        sync_state: e.sync_state.clone(),
                        last_synced_at: e.last_synced_at.clone(),
                        metadata: e.metadata.clone(),
                    };

                    match with_busy_retry(|| graph.link_external_memory(&params)) {
                        Ok(super::LinkExternalMemoryOutcome::Created {
                            graph_entity_id, ..
                        }) => {
                            counts.merged += 1;
                            write_map_entry(
                                &mut map_out,
                                &json!({
                                    "line": line_no,
                                    "record_type": "external_ref",
                                    "external_store": e.external_store,
                                    "external_id": e.external_id,
                                    "status": "merged",
                                    "new_id": graph_entity_id,
                                }),
                            )?;
                        }
                        Ok(super::LinkExternalMemoryOutcome::Unchanged {
                            graph_entity_id, ..
                        }) => {
                            counts.skipped += 1;
                            write_map_entry(
                                &mut map_out,
                                &json!({
                                    "line": line_no,
                                    "record_type": "external_ref",
                                    "external_store": e.external_store,
                                    "external_id": e.external_id,
                                    "status": "skipped",
                                    "existing_id": graph_entity_id,
                                }),
                            )?;
                        }
                        Err(err) => {
                            counts.failed += 1;
                            write_map_entry(
                                &mut map_out,
                                &json!({
                                    "line": line_no,
                                    "record_type": "external_ref",
                                    "external_store": e.external_store,
                                    "external_id": e.external_id,
                                    "status": "failed",
                                    "error": format!("{:#}", err),
                                }),
                            )?;
                        }
                    }
                }
            }
        }
    }

    Ok(counts)
}

/// Convert Hermes markdown memory files into canonical NDJSON.
pub fn adopt_hermes_markdown(input_dir: &Path, mut out: impl Write) -> Result<HermesAdoptSummary> {
    if !input_dir.exists() {
        bail!("input directory does not exist: {}", input_dir.display());
    }

    let section_split_re = Regex::new(r"(?m)(?:^|\n)\s*§\s*(?:\n|$)")?;
    let meta_comment_re = Regex::new(r"<!--(.*?)-->")?;
    let attr_re = Regex::new(r#"(\w+)=["']?([^\s,>"']+)["']?"#)?;
    let tag_re = Regex::new(r"(?s)^\[([a-zA-Z0-9_-]+)\]\s*(.*)$")?;

    let mut files: Vec<PathBuf> = Vec::new();
    collect_md_files(input_dir, &mut files)?;
    files.sort();

    let mut records_extracted = 0usize;
    let files_scanned = files.len();

    for file_path in &files {
        let content = std::fs::read_to_string(file_path)
            .with_context(|| format!("read {}", file_path.display()))?;
        let filename = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let source_uri = file_path.to_string_lossy().to_string();

        let (default_scope, default_kind) = if filename.eq_ignore_ascii_case("user.md") {
            ("user".to_string(), "preference".to_string())
        } else if filename.eq_ignore_ascii_case("failures.md") {
            ("agent".to_string(), "lesson".to_string())
        } else {
            ("agent".to_string(), "memory".to_string())
        };

        // Determine project from parent directory if inside projects-memory/<project>/
        let default_project = file_path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .and_then(|name| {
                if name != "pi-hermes-memory" && name != "fixtures" && name != "hermes" {
                    Some(name.to_string())
                } else {
                    None
                }
            });

        let raw_sections: Vec<&str> = section_split_re.split(&content).collect();

        for raw in raw_sections {
            let raw_clean = raw.trim();
            if raw_clean.is_empty() {
                continue;
            }

            let mut metadata: BTreeMap<String, String> = BTreeMap::new();
            let entry_text = if let Some(m) = meta_comment_re.captures(raw_clean) {
                if let Some(comment_body) = m.get(1) {
                    for cap in attr_re.captures_iter(comment_body.as_str()) {
                        if let (Some(k), Some(v)) = (cap.get(1), cap.get(2)) {
                            metadata.insert(k.as_str().to_string(), v.as_str().to_string());
                        }
                    }
                }
                meta_comment_re
                    .replace_all(raw_clean, "")
                    .trim()
                    .to_string()
            } else {
                raw_clean.to_string()
            };

            if entry_text.is_empty() {
                continue;
            }

            // Project inference: project64 or project attr, else default
            let mut project_id = default_project.clone();
            if let Some(p64) = metadata.get("project64") {
                if let Some(s) = decode_base64_str(p64) {
                    project_id = Some(s.trim().to_string());
                }
            } else if let Some(p) = metadata.get("project") {
                project_id = Some(p.trim().to_string());
            }

            // Kind and title inference
            let (kind, title, text) = if let Some(caps) = tag_re.captures(&entry_text) {
                let tag = caps
                    .get(1)
                    .map(|m| m.as_str().to_lowercase())
                    .unwrap_or_default();
                let rest = caps.get(2).map(|m| m.as_str().trim()).unwrap_or_default();

                let k = match tag.as_str() {
                    "failure" => "failure",
                    "correction" => "correction",
                    "tool-quirk" => "tool_quirk",
                    "insight" => "insight",
                    "convention" => "convention",
                    "decision" | "design" => "decision",
                    "gate" => "gate",
                    other => other,
                }
                .to_string();

                let title_candidate = if let Some(before) = rest.split(" — ").next() {
                    before.trim()
                } else if let Some(before) = rest.split('\n').next() {
                    before.trim()
                } else {
                    rest
                };

                let t = if let Some(first_sent) = title_candidate.split(". ").next() {
                    first_sent.trim()
                } else {
                    &title_candidate[..title_candidate.len().min(96)]
                };

                (k, t.to_string(), entry_text.clone())
            } else {
                let first_line = entry_text.split('\n').next().unwrap_or(&entry_text).trim();
                let t = if let Some(first_sent) = first_line.split(". ").next() {
                    first_sent.trim()
                } else {
                    &first_line[..first_line.len().min(96)]
                };
                (default_kind.clone(), t.to_string(), entry_text.clone())
            };

            let content_hash = compute_sha256(&text);
            let stem = file_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("hermes");
            let key = format!("hermes:{}:{}", stem, &content_hash[..16]);
            let created = metadata.get("created").cloned();
            let sensitivity = metadata
                .get("sensitivity")
                .cloned()
                .unwrap_or_else(|| "normal".to_string());

            let rec = CanonicalMemoryRecord::Memory(MemoryRecordV2 {
                key,
                content: text,
                scope: default_scope.clone(),
                kind,
                title: Some(title),
                summary: None,
                confidence: 1.0,
                status: "active".to_string(),
                project_id,
                source_uri: Some(source_uri.clone()),
                evidence: vec![json!({
                    "source": "hermes_markdown_adapter",
                    "file": source_uri,
                })],
                content_hash,
                token_estimate: None,
                sensitivity,
                valid_from: created,
                valid_to: None,
                ttl_seconds: None,
                tags: None,
                graph_entity_id: None,
            });

            serde_json::to_writer(&mut out, &rec)?;
            out.write_all(b"\n")?;
            records_extracted += 1;
        }
    }

    Ok(HermesAdoptSummary {
        status: "ok".to_string(),
        records_extracted,
        files_scanned,
        out: "".to_string(),
    })
}

/// Convert Bookkeeper markdown export format into canonical NDJSON.
pub fn adopt_bookkeeper_markdown(
    input_path: &Path,
    mut out: impl Write,
) -> Result<BookkeeperAdoptSummary> {
    if !input_path.exists() {
        bail!("input file does not exist: {}", input_path.display());
    }

    let content = std::fs::read_to_string(input_path)
        .with_context(|| format!("read {}", input_path.display()))?;
    let section_split_re = Regex::new(r"(?m)(?:^|\n)\s*§\s*(?:\n|$)")?;
    let meta_comment_re = Regex::new(r"<!--(.*?)-->")?;
    let attr_re = Regex::new(r#"(\w+)=["']?([^\s,>"']+)["']?"#)?;
    let tag_re = Regex::new(r"(?s)^\[([a-zA-Z0-9_-]+)\]\s*(.*)$")?;

    let raw_sections: Vec<&str> = section_split_re.split(&content).collect();
    let mut records_extracted = 0usize;
    let source_uri = input_path.to_string_lossy().to_string();

    for raw in raw_sections {
        let raw_clean = raw.trim();
        if raw_clean.is_empty() {
            continue;
        }

        let mut metadata: BTreeMap<String, String> = BTreeMap::new();
        let entry_text = if let Some(m) = meta_comment_re.captures(raw_clean) {
            if let Some(comment_body) = m.get(1) {
                for cap in attr_re.captures_iter(comment_body.as_str()) {
                    if let (Some(k), Some(v)) = (cap.get(1), cap.get(2)) {
                        metadata.insert(k.as_str().to_string(), v.as_str().to_string());
                    }
                }
            }
            meta_comment_re
                .replace_all(raw_clean, "")
                .trim()
                .to_string()
        } else {
            raw_clean.to_string()
        };

        if entry_text.is_empty() {
            continue;
        }

        let (kind, title, body_text) = if let Some(caps) = tag_re.captures(&entry_text) {
            let tag = caps
                .get(1)
                .map(|m| m.as_str().to_lowercase())
                .unwrap_or_default();
            let rest = caps.get(2).map(|m| m.as_str().trim()).unwrap_or_default();

            let title_cand = rest.split('\n').next().unwrap_or(rest).trim();
            let body = if let Some(pos) = rest.find('\n') {
                rest[pos + 1..].trim()
            } else {
                rest
            };
            (tag, title_cand.to_string(), body.to_string())
        } else {
            let title_cand = entry_text.split('\n').next().unwrap_or(&entry_text).trim();
            (
                "lesson".to_string(),
                title_cand.to_string(),
                entry_text.clone(),
            )
        };

        // Decode project
        let mut project_id = None;
        if let Some(p64) = metadata.get("project64") {
            if let Some(s) = decode_base64_str(p64) {
                project_id = Some(s.trim().to_string());
            }
        } else if let Some(p) = metadata.get("project") {
            project_id = Some(p.trim().to_string());
        }

        let content_hash = compute_sha256(&format!("{}\n{}", title, body_text));
        let record_id = metadata
            .get("id")
            .or_else(|| metadata.get("logical_id"))
            .cloned()
            .unwrap_or_else(|| format!("{}:{}", kind, &content_hash[..16]));

        let key = format!("bookkeeper:{}", record_id);
        let created = metadata.get("created").cloned();
        let sensitivity = metadata
            .get("sensitivity")
            .cloned()
            .unwrap_or_else(|| "normal".to_string());

        let rec = CanonicalMemoryRecord::Memory(MemoryRecordV2 {
            key,
            content: body_text,
            scope: "agent".to_string(),
            kind,
            title: Some(title),
            summary: None,
            confidence: 1.0,
            status: "active".to_string(),
            project_id,
            source_uri: Some(format!("{}#id={}", source_uri, record_id)),
            evidence: vec![json!({
                "source": "bookkeeper_markdown_adapter",
                "file": source_uri,
                "record_id": record_id,
            })],
            content_hash,
            token_estimate: None,
            sensitivity,
            valid_from: created,
            valid_to: None,
            ttl_seconds: None,
            tags: None,
            graph_entity_id: None,
        });

        serde_json::to_writer(&mut out, &rec)?;
        out.write_all(b"\n")?;
        records_extracted += 1;
    }

    Ok(BookkeeperAdoptSummary {
        status: "ok".to_string(),
        records_extracted,
        source_file: source_uri,
        out: "".to_string(),
    })
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_phase3_ndjson_roundtrip() {
        let mem = CanonicalMemoryRecord::Memory(MemoryRecordV2 {
            key: "test-key".into(),
            content: "test content".into(),
            scope: "agent".into(),
            kind: "memory".into(),
            title: Some("Test Title".into()),
            summary: Some("Test Summary".into()),
            confidence: 0.95,
            status: "active".into(),
            project_id: Some("test-project".into()),
            source_uri: Some("file:///test/path.rs".into()),
            evidence: vec![json!({"file": "path.rs", "line": 42})],
            content_hash: "abcd1234efgh".into(),
            token_estimate: Some(15),
            sensitivity: "normal".into(),
            valid_from: Some("2026-11-01T00:00:00Z".into()),
            valid_to: None,
            ttl_seconds: None,
            tags: Some(vec!["rust".into(), "testing".into()]),
            graph_entity_id: Some(101),
        });

        let src = CanonicalMemoryRecord::Source(SourceRecordV2 {
            uri: "https://example.com/doc".into(),
            source_type: "web".into(),
            project_id: Some("test-project".into()),
            content_hash: Some("src-hash-1".into()),
            title: Some("Doc Title".into()),
            summary: Some("Doc Summary".into()),
            author: Some("Author".into()),
            published_at: Some("2026-10-01".into()),
            retrieved_at: Some("2026-11-01".into()),
            credibility: Some(0.9),
            sensitivity: "normal".into(),
            metadata: json!({"tag": "official"}),
        });

        let serialized_mem = serde_json::to_string(&mem).unwrap();
        let parsed_mem: CanonicalMemoryRecord = serde_json::from_str(&serialized_mem).unwrap();
        assert_eq!(mem, parsed_mem);

        let serialized_src = serde_json::to_string(&src).unwrap();
        let parsed_src: CanonicalMemoryRecord = serde_json::from_str(&serialized_src).unwrap();
        assert_eq!(src, parsed_src);

        // Verify export_ledger_v2 can export to writer
        let graph = AtheneumGraph::open_in_memory().unwrap();
        let mut buf = Vec::new();
        export_ledger_v2(&graph, None, None, &mut buf).unwrap();
    }

    #[test]
    fn test_phase3_conflict_safe_import_both_polarities() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let graph = AtheneumGraph::open(&db_path).unwrap();

        let mem1 = CanonicalMemoryRecord::Memory(MemoryRecordV2 {
            key: "cache-config".into(),
            content: "initial value A".into(),
            scope: "agent".into(),
            kind: "memory".into(),
            title: Some("Cache Config".into()),
            summary: None,
            confidence: 1.0,
            status: "active".into(),
            project_id: Some("proj-a".into()),
            source_uri: None,
            evidence: vec![],
            content_hash: "hash_A".into(),
            token_estimate: None,
            sensitivity: "normal".into(),
            valid_from: None,
            valid_to: None,
            ttl_seconds: None,
            tags: None,
            graph_entity_id: None,
        });

        let ndjson_path1 = dir.path().join("mem1.ndjson");
        std::fs::write(
            &ndjson_path1,
            format!("{}\n", serde_json::to_string(&mem1).unwrap()),
        )
        .unwrap();

        // Polarity A: Initial import merges
        let mut map_buf = Vec::new();
        let counts = import_ledger_v2(
            &graph,
            &ndjson_path1,
            false,
            ConflictStrategy::Conflict,
            &mut map_buf,
        )
        .unwrap();
        assert_eq!(counts.merged, 1);
        assert_eq!(counts.skipped, 0);
        assert_eq!(counts.conflicts, 0);

        // Same content hash replay -> skipped (idempotent)
        let mut map_buf2 = Vec::new();
        let counts_replay = import_ledger_v2(
            &graph,
            &ndjson_path1,
            false,
            ConflictStrategy::Conflict,
            &mut map_buf2,
        )
        .unwrap();
        assert_eq!(counts_replay.merged, 0);
        assert_eq!(counts_replay.skipped, 1);
        assert_eq!(counts_replay.conflicts, 0);

        // Polarity B (Conflict): Colliding key with different content hash and Conflict strategy
        let mut mem2 = mem1.clone();
        if let CanonicalMemoryRecord::Memory(ref mut m) = mem2 {
            m.content = "colliding value B".into();
            m.content_hash = "hash_B".into();
        }
        let ndjson_path2 = dir.path().join("mem2.ndjson");
        std::fs::write(
            &ndjson_path2,
            format!("{}\n", serde_json::to_string(&mem2).unwrap()),
        )
        .unwrap();

        let mut map_buf3 = Vec::new();
        let counts_conflict = import_ledger_v2(
            &graph,
            &ndjson_path2,
            false,
            ConflictStrategy::Conflict,
            &mut map_buf3,
        )
        .unwrap();
        assert_eq!(counts_conflict.conflicts, 1);
        assert_eq!(counts_conflict.merged, 0);

        // Original content remains preserved
        let items = graph
            .query_memory("cache-config", Some("agent"), Some("proj-a"), false)
            .unwrap();
        assert_eq!(
            items[0].data.get("content").and_then(|v| v.as_str()),
            Some("initial value A")
        );

        // Polarity C (Revision): Colliding key with Revision strategy creates revision
        let mut map_buf4 = Vec::new();
        let counts_rev = import_ledger_v2(
            &graph,
            &ndjson_path2,
            false,
            ConflictStrategy::Revision,
            &mut map_buf4,
        )
        .unwrap();
        assert_eq!(counts_rev.merged, 1);
        assert_eq!(counts_rev.conflicts, 0);

        let items_rev = graph
            .query_memory("cache-config", Some("agent"), Some("proj-a"), false)
            .unwrap();
        assert_eq!(
            items_rev[0].data.get("content").and_then(|v| v.as_str()),
            Some("colliding value B")
        );
    }

    #[test]
    fn test_phase3_dry_run_no_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("dry_run_test.db");
        {
            let graph = AtheneumGraph::open(&db_path).unwrap();
            graph
                .store_memory("key1", "val1", "user", 1.0, None, None)
                .unwrap();
        }

        let sha_before = {
            let bytes = std::fs::read(&db_path).unwrap();
            format!("{:x}", Sha256::digest(&bytes))
        };

        let graph = AtheneumGraph::open(&db_path).unwrap();

        let mem = CanonicalMemoryRecord::Memory(MemoryRecordV2 {
            key: "key2".into(),
            content: "val2".into(),
            scope: "user".into(),
            kind: "memory".into(),
            title: None,
            summary: None,
            confidence: 1.0,
            status: "active".into(),
            project_id: None,
            source_uri: None,
            evidence: vec![],
            content_hash: "hash_val2".into(),
            token_estimate: None,
            sensitivity: "normal".into(),
            valid_from: None,
            valid_to: None,
            ttl_seconds: None,
            tags: None,
            graph_entity_id: None,
        });

        let ndjson_path = dir.path().join("dry_run.ndjson");
        std::fs::write(
            &ndjson_path,
            format!("{}\n", serde_json::to_string(&mem).unwrap()),
        )
        .unwrap();

        let mut map_buf = Vec::new();
        let counts = import_ledger_v2(
            &graph,
            &ndjson_path,
            true,
            ConflictStrategy::Conflict,
            &mut map_buf,
        )
        .unwrap();
        assert_eq!(counts.merged, 1); // would merge
        assert_eq!(counts.skipped, 0);

        drop(graph);

        let sha_after = {
            let bytes = std::fs::read(&db_path).unwrap();
            format!("{:x}", Sha256::digest(&bytes))
        };

        assert_eq!(
            sha_before, sha_after,
            "DB SHA256 must be identical before and after dry run"
        );
    }

    #[test]
    fn test_phase3_idempotent_replay() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("replay_test.db");
        let graph = AtheneumGraph::open(&db_path).unwrap();

        let mem = CanonicalMemoryRecord::Memory(MemoryRecordV2 {
            key: "replay-key".into(),
            content: "replay-val".into(),
            scope: "user".into(),
            kind: "memory".into(),
            title: None,
            summary: None,
            confidence: 1.0,
            status: "active".into(),
            project_id: None,
            source_uri: None,
            evidence: vec![],
            content_hash: "hash_replay".into(),
            token_estimate: None,
            sensitivity: "normal".into(),
            valid_from: None,
            valid_to: None,
            ttl_seconds: None,
            tags: None,
            graph_entity_id: None,
        });

        let src = CanonicalMemoryRecord::Source(SourceRecordV2 {
            uri: "https://example.com/source1".into(),
            source_type: "doc".into(),
            project_id: None,
            content_hash: Some("hash_src".into()),
            title: Some("Source Title".into()),
            summary: None,
            author: None,
            published_at: None,
            retrieved_at: None,
            credibility: None,
            sensitivity: "normal".into(),
            metadata: json!({}),
        });

        let ndjson_path = dir.path().join("batch.ndjson");
        let lines = format!(
            "{}\n{}\n",
            serde_json::to_string(&mem).unwrap(),
            serde_json::to_string(&src).unwrap()
        );
        std::fs::write(&ndjson_path, lines).unwrap();

        // Pass 1: Merged
        let mut map1 = Vec::new();
        let counts1 = import_ledger_v2(
            &graph,
            &ndjson_path,
            false,
            ConflictStrategy::Conflict,
            &mut map1,
        )
        .unwrap();
        assert_eq!(counts1.merged, 2);
        assert_eq!(counts1.skipped, 0);

        // Pass 2: Exact same file replayed -> Zero new rows, all skipped
        let mut map2 = Vec::new();
        let counts2 = import_ledger_v2(
            &graph,
            &ndjson_path,
            false,
            ConflictStrategy::Conflict,
            &mut map2,
        )
        .unwrap();
        assert_eq!(counts2.merged, 0);
        assert_eq!(counts2.skipped, 2);
    }

    #[test]
    fn test_phase3_secret_exclusion_and_rejection() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("secret_test.db");
        let graph = AtheneumGraph::open(&db_path).unwrap();

        // 1. Secret record rejection at import
        let secret_mem = CanonicalMemoryRecord::Memory(MemoryRecordV2 {
            key: "secret-key".into(),
            content: "secret credentials".into(),
            scope: "user".into(),
            kind: "memory".into(),
            title: None,
            summary: None,
            confidence: 1.0,
            status: "active".into(),
            project_id: None,
            source_uri: None,
            evidence: vec![],
            content_hash: "hash_sec".into(),
            token_estimate: None,
            sensitivity: "secret".into(),
            valid_from: None,
            valid_to: None,
            ttl_seconds: None,
            tags: None,
            graph_entity_id: None,
        });

        let ndjson_path = dir.path().join("secret.ndjson");
        std::fs::write(
            &ndjson_path,
            format!("{}\n", serde_json::to_string(&secret_mem).unwrap()),
        )
        .unwrap();

        let mut map_buf = Vec::new();
        let counts = import_ledger_v2(
            &graph,
            &ndjson_path,
            false,
            ConflictStrategy::Conflict,
            &mut map_buf,
        )
        .unwrap();
        assert_eq!(counts.failed, 1);
        assert_eq!(counts.merged, 0);

        let map_str = String::from_utf8(map_buf).unwrap();
        assert!(map_str.contains("sensitivity") && map_str.contains("secret"));

        // 2. Secret record exclusion during export
        let mut export_buf = Vec::new();
        export_ledger_v2(&graph, None, None, &mut export_buf).unwrap();
        let exported_str = String::from_utf8(export_buf).unwrap();
        assert!(!exported_str.contains("secret credentials"));
    }

    #[test]
    fn test_phase3_hermes_fixture_parse() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let fixture_dir =
            manifest_dir.join("../../tasks/atheneum-memory-router/phase3/fixtures/hermes");
        let dir = tempfile::tempdir().unwrap();
        let out_ndjson = dir.path().join("hermes.ndjson");

        let summary =
            adopt_hermes_markdown(&fixture_dir, std::fs::File::create(&out_ndjson).unwrap())
                .unwrap();
        assert!(summary.records_extracted >= 4);

        let content = std::fs::read_to_string(&out_ndjson).unwrap();
        let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines.len(), summary.records_extracted);

        // Verify preservation of source URI, content hash, kind, scope, project
        let mut found_llama_proj = false;
        let mut found_user_scope = false;
        for line in lines {
            let rec: CanonicalMemoryRecord = serde_json::from_str(line).unwrap();
            match rec {
                CanonicalMemoryRecord::Memory(m) => {
                    assert!(!m.content_hash.is_empty());
                    assert!(m.source_uri.is_some());
                    if m.project_id.as_deref() == Some("llama-rs") {
                        found_llama_proj = true;
                    }
                    if m.scope == "user" {
                        found_user_scope = true;
                    }
                }
                _ => panic!("Expected Memory record from Hermes markdown"),
            }
        }
        assert!(
            found_llama_proj,
            "Hermes adapter should decode project64 to llama-rs"
        );
        assert!(
            found_user_scope,
            "Hermes adapter should set scope=user for USER.md"
        );
    }

    #[test]
    fn test_phase3_bookkeeper_fixture_parse() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let fixture_file = manifest_dir
            .join("../../tasks/atheneum-memory-router/phase3/fixtures/bookkeeper/export.md");
        let dir = tempfile::tempdir().unwrap();
        let out_ndjson = dir.path().join("bookkeeper.ndjson");

        let summary =
            adopt_bookkeeper_markdown(&fixture_file, std::fs::File::create(&out_ndjson).unwrap())
                .unwrap();
        assert_eq!(summary.records_extracted, 2);

        let content = std::fs::read_to_string(&out_ndjson).unwrap();
        let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines.len(), 2);

        for line in lines {
            let rec: CanonicalMemoryRecord = serde_json::from_str(line).unwrap();
            match rec {
                CanonicalMemoryRecord::Memory(m) => {
                    assert!(!m.content_hash.is_empty());
                    assert!(m.source_uri.is_some());
                    assert!(m.title.is_some());
                    assert_eq!(m.project_id.as_deref(), Some("atheneum"));
                    assert!(!m.evidence.is_empty());
                }
                _ => panic!("Expected Memory record from Bookkeeper export"),
            }
        }
    }
}
