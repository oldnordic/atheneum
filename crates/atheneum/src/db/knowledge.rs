//! Stage 11d: Knowledge-domain SQL tables.
//!
//! - `discoveries` — agent findings, indexed on target/project_id.
//! - `wiki_pages` — ingested markdown pages, UNIQUE by path.
//! - `journal_sections` — Logseq-style journal segments, UNIQUE by (path, section_index).
//!
//! Backfill ports legacy Discovery / WikiPage / JournalSection
//! graph_entities into the new SQL tables and stamps `sql_id` back
//! onto each pointer node.

use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Transaction};
use serde_json::Value;

pub fn migrate_v3_knowledge(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS discoveries (
             id INTEGER PRIMARY KEY,
             agent_name TEXT NOT NULL,
             discovery_type TEXT NOT NULL,
             target TEXT NOT NULL,
             project_id TEXT,
             metadata TEXT,
             created_at TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS discoveries_target_idx ON discoveries(target);
         CREATE INDEX IF NOT EXISTS discoveries_agent_idx ON discoveries(agent_name);
         CREATE INDEX IF NOT EXISTS discoveries_project_idx ON discoveries(project_id);
         CREATE INDEX IF NOT EXISTS discoveries_type_idx ON discoveries(discovery_type);

         CREATE TABLE IF NOT EXISTS wiki_pages (
             id INTEGER PRIMARY KEY,
             path TEXT NOT NULL UNIQUE,
             title TEXT,
             content_hash TEXT,
             body TEXT,
             wikilinks TEXT,
             project_id TEXT,
             metadata TEXT,
             created_at TEXT NOT NULL,
             updated_at TEXT
         );
         CREATE INDEX IF NOT EXISTS wiki_pages_project_idx ON wiki_pages(project_id);

         CREATE TABLE IF NOT EXISTS journal_sections (
             id INTEGER PRIMARY KEY,
             path TEXT NOT NULL,
             section_index INTEGER NOT NULL,
             time TEXT,
             title TEXT NOT NULL,
             body TEXT,
             kanban_updates TEXT,
             wikilinks TEXT,
             project_id TEXT,
             metadata TEXT,
             created_at TEXT NOT NULL,
             UNIQUE(path, section_index)
         );
         CREATE INDEX IF NOT EXISTS journal_sections_project_idx ON journal_sections(project_id);
         CREATE INDEX IF NOT EXISTS journal_sections_path_idx ON journal_sections(path);",
    )?;

    backfill_discoveries(tx)?;
    backfill_wiki_pages(tx)?;
    backfill_journal_sections(tx)?;

    Ok(())
}

/// v11: add `session_id` to `discoveries` so findings can be attributed to
/// the session that produced them (used by `session-digest`). The column is
/// nullable — legacy rows and discoveries stored without a session keep
/// `NULL`. Existing rows that already carry `session_id` inside their
/// `metadata` JSON are backfilled so no data is lost.
///
/// SQLite has no `ADD COLUMN IF NOT EXISTS`, so the column presence is
/// checked via `PRAGMA table_info` before the `ALTER TABLE`. This makes the
/// migration safe to re-run on a partially-applied DB.
pub fn migrate_v11_discoveries_session(tx: &Transaction<'_>) -> Result<()> {
    let has_column: bool = {
        let mut stmt = tx.prepare("PRAGMA table_info(discoveries)")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
        let mut found = false;
        for row in rows {
            if row? == "session_id" {
                found = true;
            }
        }
        found
    };

    if !has_column {
        tx.execute_batch(
            "ALTER TABLE discoveries ADD COLUMN session_id TEXT;
             CREATE INDEX IF NOT EXISTS discoveries_session_idx ON discoveries(session_id);",
        )?;
    } else {
        // Index may still be missing if a prior run added the column but
        // crashed before the index. CREATE IF NOT EXISTS makes this safe.
        tx.execute_batch(
            "CREATE INDEX IF NOT EXISTS discoveries_session_idx ON discoveries(session_id);",
        )?;
    }

    // Backfill from metadata JSON for rows that already recorded a session.
    tx.execute(
        "UPDATE discoveries
         SET session_id = json_extract(metadata, '$.session_id')
         WHERE session_id IS NULL
           AND json_extract(metadata, '$.session_id') IS NOT NULL",
        [],
    )?;

    Ok(())
}

fn backfill_discoveries(tx: &Transaction<'_>) -> Result<()> {
    let mut stmt = tx.prepare(
        "SELECT id, data FROM graph_entities
         WHERE kind = 'Discovery'
           AND (data IS NULL OR json_extract(data, '$.sql_id') IS NULL)",
    )?;
    let rows: Vec<(i64, String)> = stmt
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);

    for (entity_id, data_str) in rows {
        let data: Value = serde_json::from_str(&data_str).unwrap_or(Value::Null);
        let agent = data
            .get("agent")
            .and_then(|v| v.as_str())
            .unwrap_or("_legacy");
        let discovery_type = data
            .get("discovery_type")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let target = data.get("target").and_then(|v| v.as_str()).unwrap_or("");
        let project_id = data.get("project_id").and_then(|v| v.as_str());
        let created_at = data
            .get("timestamp")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| Utc::now().to_rfc3339());

        tx.execute(
            "INSERT INTO discoveries
                (agent_name, discovery_type, target, project_id, metadata, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                agent,
                discovery_type,
                target,
                project_id,
                super::json_to_string(&data)?,
                created_at,
            ],
        )?;
        let sql_id = tx.last_insert_rowid();
        super::stamp_sql_id(tx, entity_id, &data, sql_id)?;
    }
    Ok(())
}

fn backfill_wiki_pages(tx: &Transaction<'_>) -> Result<()> {
    let mut stmt = tx.prepare(
        "SELECT id, name, data FROM graph_entities
         WHERE kind = 'WikiPage'
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
        let path = data
            .get("path")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or(name);
        let title = data.get("title").and_then(|v| v.as_str());
        let content_hash = data.get("content_hash").and_then(|v| v.as_str());
        let body = data.get("body").and_then(|v| v.as_str());
        let wikilinks = data
            .get("wikilinks")
            .map(super::json_to_string)
            .transpose()?;
        let project_id = data.get("project_id").and_then(|v| v.as_str());
        let now = Utc::now().to_rfc3339();

        // INSERT OR IGNORE — if path already exists, we leave it (a future
        // sync will overwrite via ingest_wiki_page).
        tx.execute(
            "INSERT OR IGNORE INTO wiki_pages
                (path, title, content_hash, body, wikilinks, project_id, metadata, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                path,
                title,
                content_hash,
                body,
                wikilinks,
                project_id,
                super::json_to_string(&data)?,
                now,
                now,
            ],
        )?;
        let sql_id: i64 = tx.query_row(
            "SELECT id FROM wiki_pages WHERE path = ?1",
            params![path],
            |r| r.get(0),
        )?;
        super::stamp_sql_id(tx, entity_id, &data, sql_id)?;
    }
    Ok(())
}

fn backfill_journal_sections(tx: &Transaction<'_>) -> Result<()> {
    let mut stmt = tx.prepare(
        "SELECT id, data FROM graph_entities
         WHERE kind = 'JournalSection'
           AND (data IS NULL OR json_extract(data, '$.sql_id') IS NULL)",
    )?;
    let rows: Vec<(i64, String)> = stmt
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);

    for (entity_id, data_str) in rows {
        let data: Value = serde_json::from_str(&data_str).unwrap_or(Value::Null);
        let path = data.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let section_index = data
            .get("section_index")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let time = data.get("time").and_then(|v| v.as_str());
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let body = data.get("body").and_then(|v| v.as_str());
        let kanban_updates = data
            .get("kanban_updates")
            .map(super::json_to_string)
            .transpose()?;
        let wikilinks = data
            .get("wikilinks")
            .map(super::json_to_string)
            .transpose()?;
        let project_id = data.get("project_id").and_then(|v| v.as_str());
        let created_at = Utc::now().to_rfc3339();

        tx.execute(
            "INSERT OR IGNORE INTO journal_sections
                (path, section_index, time, title, body, kanban_updates, wikilinks,
                 project_id, metadata, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                path,
                section_index,
                time,
                title,
                body,
                kanban_updates,
                wikilinks,
                project_id,
                super::json_to_string(&data)?,
                created_at,
            ],
        )?;
        let sql_id: i64 = tx.query_row(
            "SELECT id FROM journal_sections WHERE path = ?1 AND section_index = ?2",
            params![path, section_index],
            |r| r.get(0),
        )?;
        super::stamp_sql_id(tx, entity_id, &data, sql_id)?;
    }
    Ok(())
}
