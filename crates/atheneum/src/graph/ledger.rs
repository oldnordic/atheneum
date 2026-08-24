//! Ledger export/import for cross-store reconciliation.
//!
//! `export_ledger` streams Discovery / Memory / Task entities out of a store
//! as NDJSON (one [`LedgerRecord`] per line, full fidelity). `import_ledger`
//! replays that NDJSON into another store **through the normal entity-creation
//! code paths** ([`AtheneumGraph::store_discovery_in_project`],
//! [`AtheneumGraph::store_memory`], [`AtheneumGraph::create_task`] +
//! [`AtheneumGraph::update_task_status`]) — never raw SQL inserts — so edges,
//! events, and FTS invariants hold exactly as they do for live stores.
//!
//! Dedup key is `(kind, agent, target, content_hash)`: a record whose
//! quadruple already exists in the target is skipped, not re-stored. Hashes
//! are computed with [`content_hash_excluding`] over [`VOLATILE_KEYS`], so a
//! re-stored record collides with the original despite fresh timestamps and
//! new sql_ids.
//!
//! Both functions tolerate a live WAL target: the graph pool already sets
//! `busy_timeout = 5000` (see [`AtheneumGraph::open`]), and import wraps every
//! store call in a busy-retry loop for lock contention that outlasts it.

use std::collections::{BTreeMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::hashing::content_hash_excluding;
use super::{AtheneumGraph, EntityType, KanbanStatus};

/// Keys stripped before hashing: anything a re-store refreshes (timestamps,
/// auto-assigned sql_id, the hash field itself). Stripping them keeps the
/// hash of a re-stored record identical to the original store's hash.
const VOLATILE_KEYS: &[&str] = &[
    "timestamp",
    "sql_id",
    "content_hash",
    "created_at",
    "updated_at",
    "status_updated_at",
];

/// Ledger record kinds, selectable via `--kinds discoveries,memories,tasks`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerKind {
    Discovery,
    Memory,
    Task,
}

impl LedgerKind {
    pub fn all() -> Vec<LedgerKind> {
        vec![LedgerKind::Discovery, LedgerKind::Memory, LedgerKind::Task]
    }

    /// Parse a comma-separated `--kinds` value (`discoveries,memories,tasks`).
    pub fn parse_list(value: &str) -> Result<Vec<LedgerKind>> {
        let mut kinds = Vec::new();
        for part in value.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let kind = match part.to_ascii_lowercase().as_str() {
                "discovery" | "discoveries" => LedgerKind::Discovery,
                "memory" | "memories" => LedgerKind::Memory,
                "task" | "tasks" => LedgerKind::Task,
                other => anyhow::bail!(
                    "unknown ledger kind '{}'; valid: discoveries,memories,tasks",
                    other
                ),
            };
            if !kinds.contains(&kind) {
                kinds.push(kind);
            }
        }
        if kinds.is_empty() {
            anyhow::bail!("--kinds must name at least one of discoveries,memories,tasks");
        }
        Ok(kinds)
    }

    fn entity_kind(&self) -> &'static str {
        match self {
            LedgerKind::Discovery => EntityType::Discovery.as_str(),
            LedgerKind::Memory => EntityType::Memory.as_str(),
            LedgerKind::Task => EntityType::Task.as_str(),
        }
    }

    fn label(&self) -> &'static str {
        match self {
            LedgerKind::Discovery => "discovery",
            LedgerKind::Memory => "memory",
            LedgerKind::Task => "task",
        }
    }

    fn from_label(label: &str) -> Option<LedgerKind> {
        match label {
            "discovery" => Some(LedgerKind::Discovery),
            "memory" => Some(LedgerKind::Memory),
            "task" => Some(LedgerKind::Task),
            _ => None,
        }
    }
}

/// One NDJSON ledger line: full fidelity for cross-store replay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerRecord {
    pub kind: String,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub discovery_type: Option<String>,
    pub target: String,
    #[serde(default)]
    pub project_id: Option<String>,
    pub metadata: Value,
    pub content_hash: String,
    #[serde(default)]
    pub created_at: Option<String>,
}

/// Import outcome counts, printed exactly by the CLI.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LedgerCounts {
    pub merged: u64,
    pub skipped: u64,
    pub failed: u64,
}

/// Dedup key for one ledger record or existing entity.
type DedupKey = (String, Option<String>, String, String);

/// Hash an entity/record data body the same way on both sides of the merge:
/// prefer a stored `content_hash`, else compute one over the body minus
/// volatile keys.
fn body_hash(data: &Value) -> Result<String> {
    if let Some(stored) = data.get("content_hash").and_then(|v| v.as_str()) {
        return Ok(stored.to_string());
    }
    content_hash_excluding(data, VOLATILE_KEYS)
}

/// Extract the dedup key for an existing entity of `kind` from its stored
/// `(name, data)`.
fn entity_dedup_key(kind: LedgerKind, name: &str, data: &Value) -> Result<DedupKey> {
    let hash = body_hash(data)?;
    match kind {
        LedgerKind::Discovery => {
            let agent = data.get("agent").and_then(|v| v.as_str()).map(String::from);
            let target = data
                .get("target")
                .and_then(|v| v.as_str())
                .unwrap_or(name)
                .to_string();
            Ok((kind.label().to_string(), agent, target, hash))
        }
        LedgerKind::Memory | LedgerKind::Task => {
            Ok((kind.label().to_string(), None, name.to_string(), hash))
        }
    }
}

/// Extract the dedup key for an incoming ledger record.
fn record_dedup_key(record: &LedgerRecord) -> Result<DedupKey> {
    let kind = LedgerKind::from_label(&record.kind)
        .ok_or_else(|| anyhow::anyhow!("unknown ledger record kind '{}'", record.kind))?;
    let hash = if record.metadata.is_object() {
        body_hash(&record.metadata)?
    } else {
        record.content_hash.clone()
    };
    Ok((
        kind.label().to_string(),
        record.agent.clone(),
        record.target.clone(),
        hash,
    ))
}

/// Load every existing `(kind, agent, target, content_hash)` quadruple from
/// the target store in a single pass, so per-record dedup is O(1) instead of
/// a full `json_extract` scan per record.
fn load_existing_keys(graph: &AtheneumGraph) -> Result<HashSet<DedupKey>> {
    super::with_graph_conn(&graph.inner, |conn| {
        let mut stmt = conn.prepare_cached(
            "SELECT kind, name, data FROM graph_entities
             WHERE kind IN (?1, ?2, ?3)",
        )?;
        let mut rows = stmt.query(rusqlite::params![
            LedgerKind::Discovery.entity_kind(),
            LedgerKind::Memory.entity_kind(),
            LedgerKind::Task.entity_kind(),
        ])?;
        let mut keys = HashSet::new();
        while let Some(row) = rows.next()? {
            let kind_str: String = row.get(0)?;
            let name: String = row.get(1)?;
            let data_str: String = row.get(2)?;
            let data: Value = serde_json::from_str(&data_str)
                .with_context(|| format!("parse entity data for '{}'", name))?;
            let kind = match kind_str.as_str() {
                "Discovery" => LedgerKind::Discovery,
                "Memory" => LedgerKind::Memory,
                "Task" => LedgerKind::Task,
                _ => continue,
            };
            keys.insert(entity_dedup_key(kind, &name, &data)?);
        }
        Ok(keys)
    })
}

/// Stream ledger records out of a store as NDJSON, one record per line.
///
/// `until` (RFC3339) bounds which records leave the source: only records
/// created strictly before the boundary are exported. The comparison runs
/// through SQLite's `datetime()` so mixed RFC3339 offsets (`Z` vs `+00:00`)
/// compare correctly. Returns per-kind record counts.
pub fn export_ledger(
    graph: &AtheneumGraph,
    kinds: &[LedgerKind],
    until: Option<&str>,
    mut out: impl Write,
) -> Result<BTreeMap<String, u64>> {
    let mut counts = BTreeMap::new();
    for kind in kinds {
        // Discoveries timestamp as `data.timestamp` (store_discovery);
        // memories and tasks as `data.created_at`.
        let ts_field = match kind {
            LedgerKind::Discovery => "timestamp",
            LedgerKind::Memory | LedgerKind::Task => "created_at",
        };
        let until_owned = until.map(String::from);
        let entity_kind = kind.entity_kind();
        let kind_label = kind.label();
        let written = super::with_graph_conn(&graph.inner, |conn| {
            let (sql, with_until) = if until_owned.is_some() {
                (
                    format!(
                        "SELECT name, data FROM graph_entities
                         WHERE kind = ?1
                           AND datetime(json_extract(data, '$.{}')) < datetime(?2)
                         ORDER BY id",
                        ts_field
                    ),
                    true,
                )
            } else {
                (
                    "SELECT name, data FROM graph_entities WHERE kind = ?1 ORDER BY id".to_string(),
                    false,
                )
            };
            // Stream row-by-row: the source store can hold hundreds of
            // thousands of ledger entities, so collecting is not an option.
            let mut stmt = conn.prepare(&sql)?;
            let mut n = 0u64;
            {
                let mut rows = if with_until {
                    let boundary = until_owned.as_deref().unwrap_or_default();
                    stmt.query(rusqlite::params![entity_kind, boundary])?
                } else {
                    stmt.query(rusqlite::params![entity_kind])?
                };
                while let Some(row) = rows.next()? {
                    let name: String = row.get(0)?;
                    let data_str: String = row.get(1)?;
                    let data: Value = serde_json::from_str(&data_str)
                        .with_context(|| format!("parse entity data for '{}'", name))?;
                    let record = entity_to_record(*kind, &name, data)?;
                    serde_json::to_writer(&mut out, &record)?;
                    out.write_all(b"\n")?;
                    n += 1;
                }
            }
            Ok(n)
        })?;
        counts.insert(kind_label.to_string(), written);
    }
    Ok(counts)
}

fn entity_to_record(kind: LedgerKind, name: &str, data: Value) -> Result<LedgerRecord> {
    let hash = body_hash(&data)?;
    let (agent, discovery_type, target, created_at) = match kind {
        LedgerKind::Discovery => (
            data.get("agent").and_then(|v| v.as_str()).map(String::from),
            data.get("discovery_type")
                .and_then(|v| v.as_str())
                .map(String::from),
            data.get("target")
                .and_then(|v| v.as_str())
                .unwrap_or(name)
                .to_string(),
            data.get("timestamp")
                .and_then(|v| v.as_str())
                .map(String::from),
        ),
        LedgerKind::Memory | LedgerKind::Task => (
            None,
            None,
            name.to_string(),
            data.get("created_at")
                .and_then(|v| v.as_str())
                .map(String::from),
        ),
    };
    let project_id = data
        .get("project_id")
        .and_then(|v| v.as_str())
        .map(String::from);
    Ok(LedgerRecord {
        kind: kind.label().to_string(),
        agent,
        discovery_type,
        target,
        project_id,
        metadata: data,
        content_hash: hash,
        created_at,
    })
}

/// Retry a store call when the live WAL target reports lock contention.
/// `busy_timeout = 5000` absorbs short contention; this absorbs the rest.
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

/// Replay a ledger NDJSON file into `graph` through the normal store paths.
///
/// Every line is deduped on `(kind, agent, target, content_hash)` against the
/// target store (preloaded in one pass) and against earlier lines of the same
/// file. With `dry_run = true` nothing is stored — counts report what a real
/// run *would* do and the target is not mutated.
///
/// Each processed line yields one audit-map entry on `map_out`
/// (`{line, kind, target, content_hash, status, new_id?, error?}`), the
/// per-record old-hash → new-id map the spec requires.
pub fn import_ledger(
    graph: &AtheneumGraph,
    path: &Path,
    dry_run: bool,
    mut map_out: impl Write,
) -> Result<LedgerCounts> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("open ledger file {}", path.display()))?;
    let reader = BufReader::new(file);

    let mut existing = load_existing_keys(graph)?;
    let mut counts = LedgerCounts::default();

    for (idx, line) in reader.lines().enumerate() {
        let line_no = idx + 1;
        let line = line.with_context(|| format!("read line {}", line_no))?;
        if line.trim().is_empty() {
            continue;
        }

        let record: LedgerRecord = match serde_json::from_str(&line) {
            Ok(record) => record,
            Err(err) => {
                counts.failed += 1;
                write_map_entry(
                    &mut map_out,
                    &serde_json::json!({
                        "line": line_no,
                        "status": "failed",
                        "error": format!("parse: {}", err),
                    }),
                )?;
                continue;
            }
        };

        let key = match record_dedup_key(&record) {
            Ok(key) => key,
            Err(err) => {
                counts.failed += 1;
                write_map_entry(
                    &mut map_out,
                    &serde_json::json!({
                        "line": line_no,
                        "kind": record.kind,
                        "target": record.target,
                        "content_hash": record.content_hash,
                        "status": "failed",
                        "error": format!("{:#}", err),
                    }),
                )?;
                continue;
            }
        };

        if existing.contains(&key) {
            counts.skipped += 1;
            write_map_entry(
                &mut map_out,
                &serde_json::json!({
                    "line": line_no,
                    "kind": record.kind,
                    "target": record.target,
                    "content_hash": record.content_hash,
                    "status": "skipped",
                }),
            )?;
            continue;
        }

        if dry_run {
            counts.merged += 1;
            existing.insert(key);
            write_map_entry(
                &mut map_out,
                &serde_json::json!({
                    "line": line_no,
                    "kind": record.kind,
                    "target": record.target,
                    "content_hash": record.content_hash,
                    "status": "would_merge",
                    "new_id": Value::Null,
                }),
            )?;
            continue;
        }

        match with_busy_retry(|| store_record(graph, &record)) {
            Ok(new_id) => {
                counts.merged += 1;
                existing.insert(key);
                write_map_entry(
                    &mut map_out,
                    &serde_json::json!({
                        "line": line_no,
                        "kind": record.kind,
                        "target": record.target,
                        "content_hash": record.content_hash,
                        "status": "merged",
                        "new_id": new_id,
                    }),
                )?;
            }
            Err(err) => {
                counts.failed += 1;
                write_map_entry(
                    &mut map_out,
                    &serde_json::json!({
                        "line": line_no,
                        "kind": record.kind,
                        "target": record.target,
                        "content_hash": record.content_hash,
                        "status": "failed",
                        "error": format!("{:#}", err),
                    }),
                )?;
            }
        }
    }

    Ok(counts)
}

fn write_map_entry(mut out: impl Write, entry: &Value) -> Result<()> {
    serde_json::to_writer(&mut out, entry)?;
    out.write_all(b"\n")?;
    Ok(())
}

/// Store one ledger record through the same code paths the CLI store
/// subcommands use. Timestamps and sql_ids are re-assigned by the store
/// paths; content identity is preserved (and verified by the dedup hash).
fn store_record(graph: &AtheneumGraph, record: &LedgerRecord) -> Result<i64> {
    match LedgerKind::from_label(&record.kind) {
        Some(LedgerKind::Discovery) => {
            let agent = record
                .agent
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("discovery record missing agent"))?;
            let discovery_type = record
                .discovery_type
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("discovery record missing discovery_type"))?;
            graph.store_discovery_in_project(
                agent,
                discovery_type,
                &record.target,
                record.project_id.as_deref(),
                record.metadata.clone(),
            )
        }
        Some(LedgerKind::Memory) => {
            let meta = &record.metadata;
            let key = meta
                .get("key")
                .and_then(|v| v.as_str())
                .unwrap_or(&record.target);
            let scope = meta
                .get("scope")
                .and_then(|v| v.as_str())
                .unwrap_or("agent");
            let content = meta
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("memory record missing content"))?;
            let confidence = meta
                .get("confidence")
                .and_then(|v| v.as_f64())
                .unwrap_or(1.0);
            let tags: Option<Vec<String>> = meta.get("tags").and_then(|v| {
                v.as_array().map(|items| {
                    items
                        .iter()
                        .filter_map(|t| t.as_str().map(String::from))
                        .collect()
                })
            });
            graph.store_memory(
                key,
                content,
                scope,
                confidence,
                record.project_id.as_deref(),
                tags.as_deref(),
            )
        }
        Some(LedgerKind::Task) => {
            let description = record.metadata.get("description").and_then(|v| v.as_str());
            let task_id =
                graph.create_task(&record.target, description, record.project_id.as_deref())?;
            // Restore the source status through the normal update path
            // (create_task always starts at KanbanStatus::Todo).
            if let Some(status_str) = record.metadata.get("status").and_then(|v| v.as_str()) {
                if let Some(status) = KanbanStatus::parse(status_str) {
                    if status != KanbanStatus::Todo {
                        graph.update_task_status(task_id, status)?;
                    }
                }
            }
            Ok(task_id)
        }
        None => anyhow::bail!("unknown ledger record kind '{}'", record.kind),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_list_accepts_plural_and_singular() {
        let kinds = LedgerKind::parse_list("discoveries,memory,tasks").unwrap();
        assert_eq!(
            kinds,
            vec![LedgerKind::Discovery, LedgerKind::Memory, LedgerKind::Task]
        );
    }

    #[test]
    fn parse_list_rejects_unknown_kind() {
        assert!(LedgerKind::parse_list("discoveries,widgets").is_err());
        assert!(LedgerKind::parse_list("").is_err());
    }

    #[test]
    fn export_import_roundtrip_preserves_content_hashes() {
        let source = AtheneumGraph::open_in_memory().unwrap();
        source
            .store_discovery(
                "test-agent",
                "bug_found",
                "http_handler",
                json!({"detail": "connection pool leak"}),
            )
            .unwrap();
        source
            .store_memory("user-lang", "prefers Rust", "user", 0.9, None, None)
            .unwrap();
        source
            .create_task(
                "reconcile ledgers",
                Some("merge legacy store"),
                Some("atheneum"),
            )
            .unwrap();

        let mut ndjson: Vec<u8> = Vec::new();
        let counts = export_ledger(&source, &LedgerKind::all(), None, &mut ndjson).unwrap();
        assert_eq!(counts.get("discovery"), Some(&1));
        assert_eq!(counts.get("memory"), Some(&1));
        assert_eq!(counts.get("task"), Some(&1));

        // Import into an empty in-memory target through the file path API.
        let dir = tempfile::tempdir().unwrap();
        let ledger_path = dir.path().join("ledger.ndjson");
        std::fs::write(&ledger_path, &ndjson).unwrap();

        let target = AtheneumGraph::open_in_memory().unwrap();
        let mut map: Vec<u8> = Vec::new();
        let import = import_ledger(&target, &ledger_path, false, &mut map).unwrap();
        assert_eq!(
            import,
            LedgerCounts {
                merged: 3,
                skipped: 0,
                failed: 0,
            }
        );

        // Re-export the target: content_hash sets must be equal.
        let mut re_exported: Vec<u8> = Vec::new();
        export_ledger(&target, &LedgerKind::all(), None, &mut re_exported).unwrap();
        let hashes_of = |bytes: &[u8]| -> HashSet<String> {
            String::from_utf8(bytes.to_vec())
                .unwrap()
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| {
                    let record: LedgerRecord = serde_json::from_str(l).unwrap();
                    record.content_hash
                })
                .collect()
        };
        assert_eq!(hashes_of(&ndjson), hashes_of(&re_exported));
    }

    #[test]
    fn import_skips_records_already_present() {
        let source = AtheneumGraph::open_in_memory().unwrap();
        source
            .store_discovery("test-agent", "bug_found", "http_handler", json!({"n": 1}))
            .unwrap();
        source
            .store_discovery("test-agent", "bug_found", "db_layer", json!({"n": 2}))
            .unwrap();

        let mut ndjson: Vec<u8> = Vec::new();
        export_ledger(&source, &[LedgerKind::Discovery], None, &mut ndjson).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let ledger_path = dir.path().join("ledger.ndjson");
        std::fs::write(&ledger_path, &ndjson).unwrap();

        let target = AtheneumGraph::open_in_memory().unwrap();
        // Pre-store one of the two records identically.
        target
            .store_discovery("test-agent", "bug_found", "http_handler", json!({"n": 1}))
            .unwrap();

        let mut map: Vec<u8> = Vec::new();
        let counts = import_ledger(&target, &ledger_path, false, &mut map).unwrap();
        assert_eq!(
            counts,
            LedgerCounts {
                merged: 1,
                skipped: 1,
                failed: 0,
            }
        );

        // A second import of the same file skips everything.
        let mut map2: Vec<u8> = Vec::new();
        let counts2 = import_ledger(&target, &ledger_path, false, &mut map2).unwrap();
        assert_eq!(
            counts2,
            LedgerCounts {
                merged: 0,
                skipped: 2,
                failed: 0,
            }
        );
    }

    #[test]
    fn dry_run_reports_without_mutating() {
        let source = AtheneumGraph::open_in_memory().unwrap();
        source
            .store_discovery("test-agent", "bug_found", "http_handler", json!({"n": 1}))
            .unwrap();
        let mut ndjson: Vec<u8> = Vec::new();
        export_ledger(&source, &[LedgerKind::Discovery], None, &mut ndjson).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let ledger_path = dir.path().join("ledger.ndjson");
        std::fs::write(&ledger_path, &ndjson).unwrap();

        let target = AtheneumGraph::open_in_memory().unwrap();
        let mut map: Vec<u8> = Vec::new();
        let counts = import_ledger(&target, &ledger_path, true, &mut map).unwrap();
        assert_eq!(
            counts,
            LedgerCounts {
                merged: 1,
                skipped: 0,
                failed: 0,
            }
        );

        // Target must still be empty: the dry run stored nothing.
        let mut re_exported: Vec<u8> = Vec::new();
        let after = export_ledger(&target, &LedgerKind::all(), None, &mut re_exported).unwrap();
        assert_eq!(after.get("discovery"), Some(&0));
        assert_eq!(after.get("memory"), Some(&0));
        assert_eq!(after.get("task"), Some(&0));
    }

    #[test]
    fn export_until_excludes_later_records() {
        let graph = AtheneumGraph::open_in_memory().unwrap();
        graph
            .store_discovery("test-agent", "bug_found", "old_target", json!({"n": 1}))
            .unwrap();

        // Everything in this test store is timestamped "now", so a boundary
        // in the past exports nothing and a boundary in the future exports all.
        let mut none: Vec<u8> = Vec::new();
        let counts = export_ledger(
            &graph,
            &[LedgerKind::Discovery],
            Some("2000-01-01T00:00:00Z"),
            &mut none,
        )
        .unwrap();
        assert_eq!(counts.get("discovery"), Some(&0));

        let mut all: Vec<u8> = Vec::new();
        let counts = export_ledger(
            &graph,
            &[LedgerKind::Discovery],
            Some("2999-01-01T00:00:00Z"),
            &mut all,
        )
        .unwrap();
        assert_eq!(counts.get("discovery"), Some(&1));
    }
}
