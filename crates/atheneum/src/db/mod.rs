//! Atheneum's SQL "payload" layer.
//!
//! The graph (sqlitegraph) holds relationships; this module holds typed
//! data tables — agents, reasoning_logs, tool_calls (Stage 11b), then
//! tasks/requirements/blockers (Stage 11c) and discoveries/wiki_pages/
//! journal_sections (Stage 11d).
//!
//! Migrations are forward-only and applied automatically on
//! [`crate::graph::AtheneumGraph::open`] / `open_in_memory`. Each
//! migration is wrapped in a transaction; on failure the open call
//! errors out so a partially-migrated DB can't accidentally be used.

use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;
use serde_json::Value;

pub(crate) fn json_to_string(v: &Value) -> Result<String> {
    serde_json::to_string(v).map_err(|e| anyhow::anyhow!("JSON serialization failed: {}", e))
}

pub mod chat;
pub mod claims;
pub mod evidence;
pub mod execution;
pub mod hook_compat;
pub mod knowledge;
pub mod memory;
pub mod memory_router;
pub mod planning;
pub mod transcripts;
pub mod wiki_fts;

type Migration = fn(&rusqlite::Transaction<'_>) -> Result<()>;

/// Registered migrations, in apply order. Each entry is
/// `(version, short_name, body)`.
pub(crate) const MIGRATIONS: &[(u32, &str, Migration)] = &[
    (1, "execution-domain", execution::migrate_v1_execution),
    (2, "planning-domain", planning::migrate_v2_planning),
    (3, "knowledge-domain", knowledge::migrate_v3_knowledge),
    (4, "evidence-domain", evidence::migrate_v4_evidence),
    (5, "hook-compat", hook_compat::migrate_v5_hook_compat),
    (6, "transcript-imports", transcripts::migrate_v6_transcripts),
    (7, "memory-domain", memory::migrate_v7_memory),
    (
        8,
        "planning-archive-fix",
        planning::migrate_v8_planning_archive_fix,
    ),
    (9, "wiki-fts", wiki_fts::migrate_v9_wiki_fts),
    (10, "wiki-fts-path", wiki_fts::migrate_v10_wiki_fts_path),
    (
        11,
        "discoveries-session-id",
        knowledge::migrate_v11_discoveries_session,
    ),
    (12, "chat-columns-fts", chat::migrate_v12_chat_columns_fts),
    (
        13,
        "discovery-context-snapshot",
        knowledge::migrate_v13_discovery_context_snapshot,
    ),
    (14, "grounded-claims", claims::migrate_v14_grounded_claims),
    (
        15,
        "memory-router-domain",
        memory_router::migrate_v15_memory_router,
    ),
];

/// Apply any pending migrations to the connection. Idempotent — already-
/// applied versions are skipped via the `atheneum_schema_version` table.
///
/// Takes `&Connection` rather than `&mut` so we can drive it from
/// sqlitegraph's pool, which exposes shared references. Uses
/// `unchecked_transaction` internally — that's the rusqlite escape hatch
/// for cases where mutable connection access isn't available but
/// transactions are still needed.
pub fn run_migrations(conn: &Connection) -> Result<Vec<u32>> {
    run_migrations_up_to(conn, u32::MAX)
}

/// Apply any pending migrations up to `target_version` to the connection.
pub fn run_migrations_up_to(conn: &Connection, target_version: u32) -> Result<Vec<u32>> {
    ensure_version_table(conn)?;
    let applied = applied_versions(conn)?;
    let mut applied_now = Vec::new();
    for (version, name, migrate) in MIGRATIONS {
        if *version > target_version {
            continue;
        }
        if applied.contains(version) {
            continue;
        }
        let tx = conn.unchecked_transaction()?;
        migrate(&tx)
            .map_err(|e| anyhow::anyhow!("migration {} ({}) failed: {}", version, name, e))?;
        tx.execute(
            "INSERT INTO atheneum_schema_version (version, applied_at) VALUES (?1, ?2)",
            rusqlite::params![*version as i64, Utc::now().to_rfc3339()],
        )?;
        tx.commit()?;
        applied_now.push(*version);
    }
    let current = applied_versions(conn)?.into_iter().max().unwrap_or(0);
    conn.execute_batch(&format!("PRAGMA user_version = {};", current))?;

    Ok(applied_now)
}

pub const LATEST_SCHEMA_VERSION: u32 = 15;

/// Read the effective schema version from the database.
/// Inspects both `atheneum_schema_version` and `PRAGMA user_version` and returns the max.
pub fn get_schema_version(conn: &Connection) -> Result<u32> {
    let pragma_version: u32 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap_or(0);
    let table_version = if conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='atheneum_schema_version'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|c| c > 0)
        .unwrap_or(false)
    {
        conn.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM atheneum_schema_version",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|v| v as u32)
        .unwrap_or(0)
    } else {
        0
    };
    Ok(std::cmp::max(pragma_version, table_version))
}

/// Retrieve pending migration versions for this database.
pub fn get_pending_migrations(conn: &Connection) -> Result<Vec<u32>> {
    let applied = if conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='atheneum_schema_version'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|c| c > 0)
        .unwrap_or(false)
    {
        applied_versions(conn)?
    } else {
        std::collections::HashSet::new()
    };

    let mut pending = Vec::new();
    for (version, _, _) in MIGRATIONS {
        if !applied.contains(version) {
            pending.push(*version);
        }
    }
    Ok(pending)
}

fn ensure_version_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS atheneum_schema_version (
            version INTEGER PRIMARY KEY NOT NULL,
            applied_at TEXT NOT NULL
         );",
    )?;
    Ok(())
}

fn applied_versions(conn: &Connection) -> Result<std::collections::HashSet<u32>> {
    let mut stmt = conn.prepare("SELECT version FROM atheneum_schema_version")?;
    let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
    let mut out = std::collections::HashSet::new();
    for row in rows {
        out.insert(row? as u32);
    }
    Ok(out)
}

/// Shared helper: stamp `sql_id` onto a `graph_entities.data` JSON blob.
/// Used by every domain's backfill code (execution, planning, …) so re-
/// running the migration on the same row is a no-op.
pub(crate) fn stamp_sql_id(
    tx: &rusqlite::Transaction<'_>,
    entity_id: i64,
    existing_data: &Value,
    sql_id: i64,
) -> Result<()> {
    let mut data = existing_data.clone();
    if let Some(obj) = data.as_object_mut() {
        obj.insert("sql_id".to_string(), Value::Number(sql_id.into()));
    }
    tx.execute(
        "UPDATE graph_entities SET data = ?1 WHERE id = ?2",
        rusqlite::params![serde_json::to_string(&data)?, entity_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_version_and_pending_migrations() {
        let graph = sqlitegraph::SqliteGraph::open_in_memory().unwrap();
        let conn = graph.pool.direct_connection().unwrap();
        // Fresh DB: no atheneum_schema_version table yet
        assert_eq!(get_schema_version(conn).unwrap(), 0);

        // Apply migrations up to 14
        let applied = run_migrations_up_to(conn, 14).unwrap();
        assert_eq!(applied.len(), 14);
        assert_eq!(get_schema_version(conn).unwrap(), 14);

        let pending = get_pending_migrations(conn).unwrap();
        assert_eq!(pending, vec![15]);

        // Apply remaining migrations
        let applied_rem = run_migrations(conn).unwrap();
        assert_eq!(applied_rem, vec![15]);
        assert_eq!(get_schema_version(conn).unwrap(), 15);

        let pending_after = get_pending_migrations(conn).unwrap();
        assert!(pending_after.is_empty());
    }

    #[test]
    fn test_schema_version_newer_pragma() {
        let graph = sqlitegraph::SqliteGraph::open_in_memory().unwrap();
        let conn = graph.pool.direct_connection().unwrap();
        run_migrations_up_to(conn, 14).unwrap();
        conn.execute_batch("PRAGMA user_version = 99;").unwrap();
        assert_eq!(get_schema_version(conn).unwrap(), 99);
    }
}
