//! Stage 11b: Execution-domain SQL tables.
//!
//! - `agents` — one row per agent name (unique).
//! - `reasoning_logs` — one row per thought.
//! - `tool_calls` — one row per tool invocation, FK → reasoning_logs.
//!
//! The migration also backfills legacy `graph_entities` rows of the
//! matching kinds so existing on-disk DBs survive the upgrade.

use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Transaction};
use serde_json::Value;

/// Create the execution-domain tables and backfill any legacy data
/// already in `graph_entities`.
pub fn migrate_v1_execution(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS agents (
             id INTEGER PRIMARY KEY,
             name TEXT NOT NULL UNIQUE,
             project_id TEXT,
             metadata TEXT,
             created_at TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS agents_project_idx ON agents(project_id);

         CREATE TABLE IF NOT EXISTS reasoning_logs (
             id INTEGER PRIMARY KEY,
             agent_id INTEGER NOT NULL REFERENCES agents(id),
             content TEXT NOT NULL,
             project_id TEXT,
             metadata TEXT,
             created_at TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS reasoning_logs_agent_idx ON reasoning_logs(agent_id);
         CREATE INDEX IF NOT EXISTS reasoning_logs_project_idx ON reasoning_logs(project_id);
         CREATE INDEX IF NOT EXISTS reasoning_logs_created_at_idx ON reasoning_logs(created_at);

         CREATE TABLE IF NOT EXISTS tool_calls (
             id INTEGER PRIMARY KEY,
             reasoning_log_id INTEGER NOT NULL REFERENCES reasoning_logs(id),
             tool_name TEXT NOT NULL,
             args TEXT NOT NULL,
             project_id TEXT,
             created_at TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS tool_calls_log_idx ON tool_calls(reasoning_log_id);
         CREATE INDEX IF NOT EXISTS tool_calls_tool_name_idx ON tool_calls(tool_name);
         CREATE INDEX IF NOT EXISTS tool_calls_project_idx ON tool_calls(project_id);",
    )?;

    backfill_agents(tx)?;
    backfill_reasoning_logs(tx)?;
    backfill_tool_calls(tx)?;

    Ok(())
}

fn backfill_agents(tx: &Transaction<'_>) -> Result<()> {
    // Pull Agent graph entities that haven't been backfilled yet (no sql_id
    // in data). For each, insert into agents and stamp sql_id back onto the
    // graph entity.
    let mut stmt = tx.prepare(
        "SELECT id, name, data FROM graph_entities
         WHERE kind = 'Agent'
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
        let project_id = data.get("project_id").and_then(|v| v.as_str());
        // INSERT OR IGNORE so concurrent backfills (or partial state from a
        // previous failed run) don't fail on the UNIQUE(name) constraint.
        tx.execute(
            "INSERT OR IGNORE INTO agents (name, project_id, metadata, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                name,
                project_id,
                serde_json::to_string(&data).unwrap_or_default(),
                Utc::now().to_rfc3339()
            ],
        )?;
        let sql_id: i64 = tx.query_row(
            "SELECT id FROM agents WHERE name = ?1",
            params![name],
            |r| r.get(0),
        )?;
        stamp_sql_id(tx, entity_id, &data, sql_id)?;
    }
    Ok(())
}

fn backfill_reasoning_logs(tx: &Transaction<'_>) -> Result<()> {
    let mut stmt = tx.prepare(
        "SELECT id, data FROM graph_entities
         WHERE kind = 'ReasoningLog'
           AND (data IS NULL OR json_extract(data, '$.sql_id') IS NULL)",
    )?;
    let rows: Vec<(i64, String)> = stmt
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);

    for (entity_id, data_str) in rows {
        let data: Value = serde_json::from_str(&data_str).unwrap_or(Value::Null);
        let agent_name = data
            .get("agent")
            .and_then(|v| v.as_str())
            .unwrap_or("_legacy");
        let content = data.get("content").and_then(|v| v.as_str()).unwrap_or("");
        let project_id = data.get("project_id").and_then(|v| v.as_str());

        // Ensure the parent agent exists (handles dangling references).
        tx.execute(
            "INSERT OR IGNORE INTO agents (name, project_id, metadata, created_at)
             VALUES (?1, NULL, '{}', ?2)",
            params![agent_name, Utc::now().to_rfc3339()],
        )?;
        let agent_id: i64 = tx.query_row(
            "SELECT id FROM agents WHERE name = ?1",
            params![agent_name],
            |r| r.get(0),
        )?;

        tx.execute(
            "INSERT INTO reasoning_logs
                (agent_id, content, project_id, metadata, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                agent_id,
                content,
                project_id,
                serde_json::to_string(&data).unwrap_or_default(),
                data.get("timestamp")
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .unwrap_or_else(|| Utc::now().to_rfc3339()),
            ],
        )?;
        let sql_id = tx.last_insert_rowid();
        stamp_sql_id(tx, entity_id, &data, sql_id)?;
    }
    Ok(())
}

fn backfill_tool_calls(tx: &Transaction<'_>) -> Result<()> {
    let mut stmt = tx.prepare(
        "SELECT id, data FROM graph_entities
         WHERE kind = 'ToolCall'
           AND (data IS NULL OR json_extract(data, '$.sql_id') IS NULL)",
    )?;
    let rows: Vec<(i64, String)> = stmt
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);

    for (entity_id, data_str) in rows {
        let data: Value = serde_json::from_str(&data_str).unwrap_or(Value::Null);
        let tool_name = data.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
        let args = data.get("args").cloned().unwrap_or(Value::Null);
        let project_id = data.get("project_id").and_then(|v| v.as_str());

        // Resolve parent reasoning_log via the Called edge that points to
        // this ToolCall entity.
        let log_sql_id: Option<i64> = tx
            .query_row(
                "SELECT json_extract(le.data, '$.sql_id')
                 FROM graph_edges e
                 JOIN graph_entities le ON le.id = e.from_id
                 WHERE e.to_id = ?1
                   AND e.edge_type = 'called'
                   AND le.kind = 'ReasoningLog'
                 LIMIT 1",
                params![entity_id],
                |r| r.get::<_, Option<i64>>(0),
            )
            .ok()
            .flatten();

        // If we can't resolve, skip — leave the legacy graph_entity in place
        // without an sql_id; aggregation queries simply won't see it.
        let Some(log_sql_id) = log_sql_id else {
            continue;
        };

        tx.execute(
            "INSERT INTO tool_calls
                (reasoning_log_id, tool_name, args, project_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                log_sql_id,
                tool_name,
                serde_json::to_string(&args).unwrap_or_default(),
                project_id,
                data.get("timestamp")
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .unwrap_or_else(|| Utc::now().to_rfc3339()),
            ],
        )?;
        let sql_id = tx.last_insert_rowid();
        stamp_sql_id(tx, entity_id, &data, sql_id)?;
    }
    Ok(())
}

/// Patch the graph_entity row's data to include `sql_id` so subsequent
/// runs of the backfill skip it.
fn stamp_sql_id(
    tx: &Transaction<'_>,
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
        params![serde_json::to_string(&data)?, entity_id],
    )?;
    Ok(())
}
