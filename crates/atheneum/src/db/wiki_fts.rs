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

/// Per-open self-healing check for the `wiki_pages_fts` virtual table.
///
/// The FTS5 shadow tables (`wiki_pages_fts_data`, `_idx`, …) can be left in an
/// internally inconsistent state when a process other than the running binary
/// writes them — e.g. the system `python3`/`sqlite3` CLI, or a different
/// SQLite build touching the table between atheneum runs. A corrupt FTS table
/// surfaces as "database disk image is malformed" / "vtable constructor failed"
/// on the next read or write (sync-wiki, search-wiki, backfill-wiki).
///
/// Migrations v9/v10 recreate the table once, but that does not protect against
/// corruption introduced *after* they ran. This probe runs on every open and
/// recreates the index if it is unreadable, so atheneum always starts from a
/// consistent index regardless of what touched the DB in between.
///
/// # Why this takes a path, not a connection
///
/// A corrupt FTS virtual table cannot be dropped or recreated on the *same*
/// connection that has already loaded its (broken) schema: SQLite caches the
/// vtable registration for the connection's lifetime, so even a successful
/// `writable_schema` purge of `sqlite_master` leaves the connection reporting
/// "table already exists". The recovery therefore opens its own fresh
/// connections for each phase (probe, purge, recreate, rebuild); their schema
/// caches are clean. Call this *before* the connection pool is created so the
/// pool's connections open onto an already-consistent DB.
///
/// # Recovery details
///
/// After recreating the FTS5 table with the correct schema, we run a full
/// `delete-all` / repopulate / `rebuild` cycle. This is necessary because a
/// plain row-by-row backfill can leave the FTS5 shadow tables in a state where
/// subsequent `UPDATE` triggers (fired by `INSERT ... ON CONFLICT DO UPDATE` on
/// pre-existing `wiki_pages` rows) report "database disk image is malformed".
/// The rebuild finalizes the shadow-table invariants.
pub fn ensure_wiki_fts_healthy(path: &std::path::Path) -> Result<()> {
    // Each phase uses its own fresh connection with the same pragmas the pool
    // sets (WAL + busy_timeout), so a healed table is visible to the pool's
    // first open rather than stranded in a WAL frame the pool can't see.
    fn open_wal(path: &std::path::Path) -> Result<rusqlite::Connection> {
        let conn = rusqlite::Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_millis(5000))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        Ok(conn)
    }

    // Phase 1 — probe on a throwaway connection.
    let healthy = {
        let conn = open_wal(path)?;
        // If the table doesn't exist at all, this is a fresh DB — leave it to
        // the migrations; only recover when the table *exists* but is broken.
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='wiki_pages_fts')",
                [],
                |r| r.get(0),
            )
            .unwrap_or(false);
        if !exists {
            return Ok(());
        }
        conn.query_row("SELECT COUNT(*) FROM wiki_pages_fts LIMIT 1", [], |r| {
            r.get::<_, i64>(0)
        })
        .is_ok()
    };
    if healthy {
        return Ok(());
    }

    // Phase 2 — purge the corrupt virtual table and ALL its shadow tables /
    // triggers directly from sqlite_master. A normal `DROP TABLE` is unusable
    // here because dropping a virtual table invokes the module's constructor,
    // which fails on the corrupt shadow tables. Editing sqlite_master directly
    // bypasses the module entirely.
    {
        let conn = open_wal(path)?;
        conn.execute_batch(
            "PRAGMA writable_schema=ON;
             DELETE FROM sqlite_master WHERE name='wiki_pages_fts';
             DELETE FROM sqlite_master WHERE name LIKE 'wiki_pages_fts\\_%' ESCAPE '\\';
             PRAGMA writable_schema=OFF;",
        )?;
    }

    // Phase 3 — recreate the table + triggers on a fresh connection (schema
    // cache clean). The migration helper backfills an empty table; since we
    // just purged everything, the table is empty and the migration backfill is
    // a no-op.
    {
        let conn = open_wal(path)?;
        let tx = conn.unchecked_transaction()?;
        migrate_wiki_fts_with_columns(&tx, &["title", "body", "path"])?;
        tx.commit()?;
    }

    // Phase 4 — fully rebuild the index on yet another fresh connection. This
    // must run *after* the create transaction commits and on a connection that
    // did not participate in the create, otherwise FTS5's internal shadow
    // tables can be left in an inconsistent state that breaks UPDATE triggers.
    {
        let conn = open_wal(path)?;
        let tx = conn.unchecked_transaction()?;
        // `delete-all` is the FTS5 special command to empty the index without
        // touching `wiki_pages`. It is idempotent and safe even on a fresh table.
        tx.execute(
            "INSERT INTO wiki_pages_fts(wiki_pages_fts) VALUES('delete-all')",
            [],
        )?;
        // Repopulate from the actual `wiki_pages` rows.
        tx.execute(
            "INSERT INTO wiki_pages_fts (rowid, title, body, path)
             SELECT id, title, body, path FROM wiki_pages",
            [],
        )?;
        // Finalize shadow-table consistency.
        tx.execute(
            "INSERT INTO wiki_pages_fts(wiki_pages_fts) VALUES('rebuild')",
            [],
        )?;
        tx.commit()?;
        let _ = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
    }
    Ok(())
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
