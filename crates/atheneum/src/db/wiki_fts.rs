//! Stage 11f: Full-text search over wiki pages.
//!
//! Adds an FTS5 virtual table `wiki_pages_fts` backed by `wiki_pages`
//! rows, with triggers to keep the index in sync. Backfills existing
//! pages and creates real WikiPage graph entities for any rows that are
//! only present in SQL (stubs or missing entirely).

use anyhow::Result;
use rusqlite::{params, Transaction};

pub fn migrate_v9_wiki_fts(tx: &Transaction<'_>) -> Result<()> {
    // SQLite's FTS5 module format can differ across SQLite versions. If the
    // existing virtual table was created by a newer/different SQLite build, the
    // running binary may see it as "malformed" on writes. Recreate it with the
    // SQLite version that is actually opening this connection.
    tx.execute_batch(
        "DROP TABLE IF EXISTS wiki_pages_fts;
        DROP TABLE IF EXISTS wiki_pages_fts_data;
        DROP TABLE IF EXISTS wiki_pages_fts_idx;
        DROP TABLE IF EXISTS wiki_pages_fts_config;
        DROP TABLE IF EXISTS wiki_pages_fts_docsize;
        DROP TABLE IF EXISTS wiki_pages_fts_content;
        DROP TRIGGER IF EXISTS wiki_pages_fts_insert;
        DROP TRIGGER IF EXISTS wiki_pages_fts_delete;
        DROP TRIGGER IF EXISTS wiki_pages_fts_update;

        CREATE VIRTUAL TABLE wiki_pages_fts USING fts5(
            title,
            body,
            content='wiki_pages',
            content_rowid='id'
        );

        -- Keep the FTS index in sync with the table
        CREATE TRIGGER IF NOT EXISTS wiki_pages_fts_insert AFTER INSERT ON wiki_pages BEGIN
            INSERT INTO wiki_pages_fts (rowid, title, body)
            VALUES (new.id, new.title, new.body);
        END;

        CREATE TRIGGER IF NOT EXISTS wiki_pages_fts_delete AFTER DELETE ON wiki_pages BEGIN
            INSERT INTO wiki_pages_fts (wiki_pages_fts, rowid, title, body)
            VALUES ('delete', old.id, old.title, old.body);
        END;

        CREATE TRIGGER IF NOT EXISTS wiki_pages_fts_update AFTER UPDATE ON wiki_pages BEGIN
            INSERT INTO wiki_pages_fts (wiki_pages_fts, rowid, title, body)
            VALUES ('delete', old.id, old.title, old.body);
            INSERT INTO wiki_pages_fts (rowid, title, body)
            VALUES (new.id, new.title, new.body);
        END;

        CREATE INDEX IF NOT EXISTS wiki_pages_path_idx ON wiki_pages(path);",
    )?;

    backfill_wiki_fts(tx)?;
    Ok(())
}

fn backfill_wiki_fts(tx: &Transaction<'_>) -> Result<()> {
    // Only rebuild if the FTS table is empty; otherwise triggers kept it current.
    let existing: i64 = tx.query_row("SELECT COUNT(*) FROM wiki_pages_fts LIMIT 1", [], |r| {
        r.get(0)
    })?;
    if existing > 0 {
        return Ok(());
    }

    let mut stmt = tx.prepare("SELECT id, title, body FROM wiki_pages ORDER BY id")?;
    let rows: Vec<(i64, Option<String>, Option<String>)> = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);

    for (id, title, body) in rows {
        tx.execute(
            "INSERT INTO wiki_pages_fts (rowid, title, body) VALUES (?1, ?2, ?3)",
            params![id, title, body],
        )?;
    }

    // Direct INSERTs into an empty FTS5 table can leave the index in an
    // inconsistent state until a rebuild. This is idempotent and fast for
    // modest page counts.
    tx.execute(
        "INSERT INTO wiki_pages_fts(wiki_pages_fts) VALUES('rebuild')",
        [],
    )?;

    Ok(())
}
