//! Stage 11c: Planning-domain SQL tables.
//!
//! - `tasks` — kanban tasks with CHECK-constrained status.
//! - `requirements` — UNMET/MET, FK → tasks.
//! - `blockers` — DEPENDENCY/BUG/INFO_GAP, FK → tasks.
//!
//! Backfill ports legacy Task/Requirement/Blocker graph_entities into
//! the new tables. The parent task is resolved from `data.task_id`
//! which is a graph_entity_id in pre-11c data — we map that to the
//! new `tasks.id` via the sql_id pointer we just wrote.

use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Transaction};
use serde_json::Value;

pub fn migrate_v2_planning(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS tasks (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             description TEXT,
             status TEXT NOT NULL DEFAULT 'TODO'
                 CHECK(status IN ('TODO','IN_PROGRESS','DONE','BLOCKED')),
             project_id TEXT,
             metadata TEXT,
             created_at TEXT NOT NULL,
             status_updated_at TEXT
         );
         CREATE INDEX IF NOT EXISTS tasks_project_idx ON tasks(project_id);
         CREATE INDEX IF NOT EXISTS tasks_status_idx ON tasks(status);
         CREATE INDEX IF NOT EXISTS tasks_title_idx ON tasks(title);

         CREATE TABLE IF NOT EXISTS requirements (
             id INTEGER PRIMARY KEY,
             task_id INTEGER NOT NULL REFERENCES tasks(id),
             statement TEXT NOT NULL,
             status TEXT NOT NULL DEFAULT 'UNMET' CHECK(status IN ('MET','UNMET')),
             verification_method TEXT,
             created_at TEXT NOT NULL,
             met_at TEXT
         );
         CREATE INDEX IF NOT EXISTS requirements_task_idx ON requirements(task_id);

         CREATE TABLE IF NOT EXISTS blockers (
             id INTEGER PRIMARY KEY,
             task_id INTEGER NOT NULL REFERENCES tasks(id),
             description TEXT NOT NULL,
             blocker_type TEXT NOT NULL
                 CHECK(blocker_type IN ('DEPENDENCY','BUG','INFO_GAP')),
             resolved_at TEXT,
             created_at TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS blockers_task_idx ON blockers(task_id);",
    )?;

    backfill_tasks(tx)?;
    backfill_requirements_and_blockers(tx)?;

    Ok(())
}

fn backfill_tasks(tx: &Transaction<'_>) -> Result<()> {
    let mut stmt = tx.prepare(
        "SELECT id, data FROM graph_entities
         WHERE kind = 'Task'
           AND (data IS NULL OR json_extract(data, '$.sql_id') IS NULL)",
    )?;
    let rows: Vec<(i64, String)> = stmt
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);

    for (entity_id, data_str) in rows {
        let data: Value = serde_json::from_str(&data_str).unwrap_or(Value::Null);
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let description = data.get("description").and_then(|v| v.as_str());
        let status = data
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("TODO");
        let project_id = data.get("project_id").and_then(|v| v.as_str());
        let status_updated_at = data
            .get("status_updated_at")
            .and_then(|v| v.as_str())
            .map(String::from);
        let created_at = data
            .get("created_at")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| Utc::now().to_rfc3339());

        tx.execute(
            "INSERT INTO tasks
                (title, description, status, project_id, metadata, created_at, status_updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                title,
                description,
                status,
                project_id,
                super::json_to_string(&data)?,
                created_at,
                status_updated_at,
            ],
        )?;
        let sql_id = tx.last_insert_rowid();
        super::stamp_sql_id(tx, entity_id, &data, sql_id)?;
    }
    Ok(())
}

fn backfill_requirements_and_blockers(tx: &Transaction<'_>) -> Result<()> {
    // Helper: pre-11c data carried `task_id` as the parent's graph_entity_id.
    // Convert to the new tasks.id via the sql_id pointer we stamped above.
    let resolve_task_sql_id =
        |tx: &Transaction<'_>, parent_entity_id: i64| -> Result<Option<i64>> {
            let row = tx
                .query_row(
                    "SELECT json_extract(data, '$.sql_id') FROM graph_entities WHERE id = ?1",
                    params![parent_entity_id],
                    |r| r.get::<_, Option<i64>>(0),
                )
                .ok()
                .flatten();
            Ok(row)
        };

    // Requirements
    let mut stmt = tx.prepare(
        "SELECT id, data FROM graph_entities
         WHERE kind = 'Requirement'
           AND (data IS NULL OR json_extract(data, '$.sql_id') IS NULL)",
    )?;
    let rows: Vec<(i64, String)> = stmt
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);
    for (entity_id, data_str) in rows {
        let data: Value = serde_json::from_str(&data_str).unwrap_or(Value::Null);
        let parent_entity_id = data.get("task_id").and_then(|v| v.as_i64());
        let task_sql_id = match parent_entity_id {
            Some(eid) => resolve_task_sql_id(tx, eid)?,
            None => None,
        };
        let Some(task_sql_id) = task_sql_id else {
            continue;
        };
        let statement = data.get("statement").and_then(|v| v.as_str()).unwrap_or("");
        let status = data
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("UNMET");
        let verification_method = data.get("verification_method").and_then(|v| v.as_str());
        let met_at = data.get("met_at").and_then(|v| v.as_str());
        let created_at = data
            .get("created_at")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| Utc::now().to_rfc3339());

        tx.execute(
            "INSERT INTO requirements
                (task_id, statement, status, verification_method, created_at, met_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                task_sql_id,
                statement,
                status,
                verification_method,
                created_at,
                met_at,
            ],
        )?;
        let sql_id = tx.last_insert_rowid();
        super::stamp_sql_id(tx, entity_id, &data, sql_id)?;
    }

    // Blockers
    let mut stmt = tx.prepare(
        "SELECT id, data FROM graph_entities
         WHERE kind = 'Blocker'
           AND (data IS NULL OR json_extract(data, '$.sql_id') IS NULL)",
    )?;
    let rows: Vec<(i64, String)> = stmt
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);
    for (entity_id, data_str) in rows {
        let data: Value = serde_json::from_str(&data_str).unwrap_or(Value::Null);
        let parent_entity_id = data.get("task_id").and_then(|v| v.as_i64());
        let task_sql_id = match parent_entity_id {
            Some(eid) => resolve_task_sql_id(tx, eid)?,
            None => None,
        };
        let Some(task_sql_id) = task_sql_id else {
            continue;
        };
        let description = data
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let blocker_type = data
            .get("blocker_type")
            .and_then(|v| v.as_str())
            .unwrap_or("INFO_GAP");
        let resolved_at = data.get("resolved_at").and_then(|v| v.as_str());
        let created_at = data
            .get("created_at")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| Utc::now().to_rfc3339());

        tx.execute(
            "INSERT INTO blockers
                (task_id, description, blocker_type, resolved_at, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                task_sql_id,
                description,
                blocker_type,
                resolved_at,
                created_at,
            ],
        )?;
        let sql_id = tx.last_insert_rowid();
        super::stamp_sql_id(tx, entity_id, &data, sql_id)?;
    }

    Ok(())
}
