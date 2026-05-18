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

pub mod execution;

type Migration = fn(&rusqlite::Transaction<'_>) -> Result<()>;

/// Registered migrations, in apply order. Each entry is
/// `(version, short_name, body)`.
const MIGRATIONS: &[(u32, &str, Migration)] =
    &[(1, "execution-domain", execution::migrate_v1_execution)];

/// Apply any pending migrations to the connection. Idempotent — already-
/// applied versions are skipped via the `atheneum_schema_version` table.
///
/// Takes `&Connection` rather than `&mut` so we can drive it from
/// sqlitegraph's pool, which exposes shared references. Uses
/// `unchecked_transaction` internally — that's the rusqlite escape hatch
/// for cases where mutable connection access isn't available but
/// transactions are still needed.
pub(crate) fn run_migrations(conn: &Connection) -> Result<()> {
    ensure_version_table(conn)?;
    let applied = applied_versions(conn)?;
    for (version, name, migrate) in MIGRATIONS {
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
    }
    Ok(())
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
