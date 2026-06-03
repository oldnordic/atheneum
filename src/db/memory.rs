//! Stage 11e: Memory-domain SQL table.
//!
//! `memory_entries` holds stable facts (user preferences, project conventions)
//! distinct from `discoveries` (agent insights) and `wiki_pages` (documents).

use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Transaction};
use serde_json::Value;

pub fn migrate_v7_memory(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS memory_entries (
             id INTEGER PRIMARY KEY,
             key TEXT NOT NULL,
             scope TEXT NOT NULL,
             content TEXT NOT NULL,
             confidence REAL DEFAULT 1.0,
             project_id TEXT,
             created_at TEXT NOT NULL,
             updated_at TEXT
         );
         CREATE INDEX IF NOT EXISTS memory_key_idx ON memory_entries(key);
         CREATE INDEX IF NOT EXISTS memory_scope_idx ON memory_entries(scope);
         CREATE INDEX IF NOT EXISTS memory_project_idx ON memory_entries(project_id);
         CREATE INDEX IF NOT EXISTS memory_scope_key_idx ON memory_entries(scope, key);",
    )?;

    backfill_memories(tx)?;

    Ok(())
}

fn backfill_memories(tx: &Transaction<'_>) -> Result<()> {
    let mut stmt = tx.prepare(
        "SELECT id, name, data FROM graph_entities
         WHERE kind = 'Memory'
           AND (data IS NULL OR json_extract(data, '$.sql_id') IS NULL)",
    )?;
    let rows: Vec<(i64, String, String)> = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);

    for (entity_id, name, data_str) in rows {
        let data: Value = serde_json::from_str(&data_str).unwrap_or(Value::Null);
        let key = data.get("key").and_then(|v| v.as_str()).unwrap_or(&name);
        let scope = data
            .get("scope")
            .and_then(|v| v.as_str())
            .unwrap_or("project");
        let content = data.get("content").and_then(|v| v.as_str()).unwrap_or("");
        let confidence = data
            .get("confidence")
            .and_then(|v| v.as_f64())
            .unwrap_or(1.0);
        let project_id = data.get("project_id").and_then(|v| v.as_str());
        let created_at = data
            .get("created_at")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| Utc::now().to_rfc3339());

        tx.execute(
            "INSERT INTO memory_entries (key, scope, content, confidence, project_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![key, scope, content, confidence, project_id, created_at],
        )?;
        let sql_id = tx.last_insert_rowid();

        crate::db::stamp_sql_id(tx, entity_id, &data, sql_id)?;
    }

    Ok(())
}
