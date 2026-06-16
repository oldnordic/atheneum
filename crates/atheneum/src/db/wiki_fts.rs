//! Stage 11f: Full-text search over wiki pages.
//!
//! Adds an FTS5 virtual table `wiki_pages_fts` backed by `wiki_pages`
//! rows, with triggers to keep the index in sync. Backfills existing
//! pages and creates real WikiPage graph entities for any rows that are
//! only present in SQL (stubs or missing entirely).

use anyhow::Result;
use rusqlite::Transaction;

pub fn migrate_v9_wiki_fts(tx: &Transaction<'_>) -> Result<()> {
    migrate_wiki_fts_with_columns(tx, &["title", "body"])
}

pub fn migrate_v10_wiki_fts_path(tx: &Transaction<'_>) -> Result<()> {
    migrate_wiki_fts_with_columns(tx, &["title", "body", "path"])
}

fn migrate_wiki_fts_with_columns(tx: &Transaction<'_>, columns: &[&str]) -> Result<()> {
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
        DROP TRIGGER IF EXISTS wiki_pages_fts_update;",
    )?;

    let col_list = columns.join(", ");
    let create_sql = format!(
        "CREATE VIRTUAL TABLE wiki_pages_fts USING fts5(
            {col_list},
            content='wiki_pages',
            content_rowid='id'
        );"
    );
    tx.execute(&create_sql, [])?;

    // SQLite triggers cannot use bound variables; reference new/old columns literally.
    let new_vals = columns
        .iter()
        .map(|c| format!("new.{}", c))
        .collect::<Vec<_>>()
        .join(", ");
    let old_vals = columns
        .iter()
        .map(|c| format!("old.{}", c))
        .collect::<Vec<_>>()
        .join(", ");
    let insert_cols = columns.join(", ");

    let trigger_insert = format!(
        "CREATE TRIGGER IF NOT EXISTS wiki_pages_fts_insert AFTER INSERT ON wiki_pages BEGIN
            INSERT INTO wiki_pages_fts (rowid, {insert_cols})
            VALUES (new.id, {new_vals});
        END;"
    );
    let trigger_delete = format!(
        "CREATE TRIGGER IF NOT EXISTS wiki_pages_fts_delete AFTER DELETE ON wiki_pages BEGIN
            INSERT INTO wiki_pages_fts (wiki_pages_fts, rowid, {insert_cols})
            VALUES ('delete', old.id, {old_vals});
        END;"
    );
    let trigger_update = format!(
        "CREATE TRIGGER IF NOT EXISTS wiki_pages_fts_update AFTER UPDATE ON wiki_pages BEGIN
            INSERT INTO wiki_pages_fts (wiki_pages_fts, rowid, {insert_cols})
            VALUES ('delete', old.id, {old_vals});
            INSERT INTO wiki_pages_fts (rowid, {insert_cols})
            VALUES (new.id, {new_vals});
        END;"
    );

    tx.execute_batch(&format!(
        "{}\n{}\n{}\nCREATE INDEX IF NOT EXISTS wiki_pages_path_idx ON wiki_pages(path);",
        trigger_insert, trigger_delete, trigger_update
    ))?;

    backfill_wiki_fts(tx, columns)?;
    Ok(())
}

fn backfill_wiki_fts(tx: &Transaction<'_>, columns: &[&str]) -> Result<()> {
    // Only rebuild if the FTS table is empty; otherwise triggers kept it current.
    let existing: i64 = tx.query_row("SELECT COUNT(*) FROM wiki_pages_fts LIMIT 1", [], |r| {
        r.get(0)
    })?;
    if existing > 0 {
        return Ok(());
    }

    let select_cols = columns.join(", ");
    let insert_cols = columns.join(", ");
    let placeholders: Vec<String> = columns
        .iter()
        .enumerate()
        .map(|(i, _)| format!("?{}", i + 2))
        .collect();
    let insert_vals = placeholders.join(", ");

    let mut stmt = tx.prepare(&format!(
        "SELECT id, {select_cols} FROM wiki_pages ORDER BY id"
    ))?;
    let rows: Vec<(i64, Vec<Option<String>>)> = stmt
        .query_map([], |r| {
            let id: i64 = r.get(0)?;
            let mut vals = Vec::with_capacity(columns.len());
            for i in 0..columns.len() {
                vals.push(r.get::<_, Option<String>>(i + 1)?);
            }
            Ok((id, vals))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);

    let sql =
        format!("INSERT INTO wiki_pages_fts (rowid, {insert_cols}) VALUES (?1, {insert_vals})");
    for (id, vals) in rows {
        let mut params: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(vals.len() + 1);
        params.push(&id);
        let owned: Vec<Option<String>> = vals;
        for v in &owned {
            params.push(v as &dyn rusqlite::ToSql);
        }
        tx.execute(&sql, params.as_slice())?;
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
