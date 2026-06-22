//! Chat-navigation schema (migration v12): generated columns + FTS5 over
//! `graph_entities` chat rows (`ReasoningLog` + `ToolCall`).
//!
//! `session_id`, `sequence`, `role`, and `content_text` are VIRTUAL GENERATED
//! columns derived from `graph_entities.data` JSON, so the existing insert path
//! (`record_evidence_prompt`, `record_evidence_tool_call`) keeps working
//! unchanged — no Rust insert-path edit is needed. A composite index backs
//! session/sequence queries; `entity_fts` (FTS5, external-content on
//! `graph_entities`) backs `--search`. Sync triggers keep `entity_fts` aligned
//! for chat kinds only.
//!
//! Grounded against the live schema (verified 2026-06-23): ReasoningLog.data
//! carries `session_id`/`role`/`sequence`/`content_summary`; ToolCall.data
//! carries `session_id`/`sequence`/`tool_name`/`tool_category` (it has no
//! `content_summary`, so `content_text` falls back to `tool_name` +
//! `tool_category` for tool rows). `content_text` also coalesces `content`
//! for the audit-schema variant of ReasoningLog.

use anyhow::{anyhow, bail, Result};

use rusqlite::Connection;

/// Minimum SQLite supporting generated columns (3.31.0, released 2020-10).
const MIN_GENERATED_COLUMN: (u32, u32) = (3, 31);

/// Migration v12 — `chat-columns-fts`.
///
/// Idempotency: `run_migrations` stamps v12 in `atheneum_schema_version` and
/// skips already-applied versions, so this body runs once per DB. The schema
/// delta is wrapped in the caller's transaction; on error it rolls back (no
/// partial columns/indexes). Index/trigger/table statements additionally use
/// `IF NOT EXISTS` so a manual re-run after a rolled-back attempt is harmless.
pub(crate) fn migrate_v12_chat_columns_fts(tx: &rusqlite::Transaction<'_>) -> Result<()> {
    ensure_sqlite_supports_generated_columns(tx)?;

    // The four generated columns derive from `data` JSON. VIRTUAL = computed
    // on read, indexable, zero storage. The composite indexes store the
    // derived values, so session/sequence lookups never recompute.
    tx.execute_batch(
        r#"
        ALTER TABLE graph_entities ADD COLUMN session_id TEXT
            GENERATED ALWAYS AS (json_extract(data, '$.session_id')) VIRTUAL;
        ALTER TABLE graph_entities ADD COLUMN sequence INTEGER
            GENERATED ALWAYS AS (json_extract(data, '$.sequence')) VIRTUAL;
        ALTER TABLE graph_entities ADD COLUMN role TEXT
            GENERATED ALWAYS AS (json_extract(data, '$.role')) VIRTUAL;
        ALTER TABLE graph_entities ADD COLUMN content_text TEXT
            GENERATED ALWAYS AS (
                coalesce(json_extract(data, '$.content_summary'), '') || ' ' ||
                coalesce(json_extract(data, '$.content'), '') || ' ' ||
                coalesce(json_extract(data, '$.tool_name'), '') || ' ' ||
                coalesce(json_extract(data, '$.tool_category'), '')
            ) VIRTUAL;

        CREATE INDEX IF NOT EXISTS idx_entities_session_seq
            ON graph_entities(session_id, sequence);
        CREATE INDEX IF NOT EXISTS idx_entities_session_role_seq
            ON graph_entities(session_id, role, sequence);

        -- FTS5 over chat-turn content. External-content table = graph_entities;
        -- content lives in the FTS index, kept in sync by the triggers below.
        -- Only chat kinds are indexed (keeps the index small).
        CREATE VIRTUAL TABLE IF NOT EXISTS entity_fts USING fts5(
            content_text,
            content='graph_entities', content_rowid='id',
            tokenize='porter unicode61'
        );

        -- Backfill existing chat rows.
        INSERT INTO entity_fts(rowid, content_text)
            SELECT id, content_text FROM graph_entities
            WHERE kind IN ('ReasoningLog', 'ToolCall');

        -- Sync triggers (FTS5 external content does not auto-sync). The insert
        -- and delete triggers fire only for chat kinds. The update path is split
        -- into two triggers so a kind transition is handled correctly: remove
        -- the old entry iff the old row was chat, add the new entry iff the new
        -- row is chat. (atheneum never transitions a row's kind in practice, but
        -- the split makes the invariant explicit rather than relying on that.)
        CREATE TRIGGER IF NOT EXISTS entity_fts_ai AFTER INSERT ON graph_entities
            WHEN new.kind IN ('ReasoningLog', 'ToolCall')
            BEGIN
                INSERT INTO entity_fts(rowid, content_text)
                    VALUES (new.id, new.content_text);
            END;
        CREATE TRIGGER IF NOT EXISTS entity_fts_ad AFTER DELETE ON graph_entities
            WHEN old.kind IN ('ReasoningLog', 'ToolCall')
            BEGIN
                INSERT INTO entity_fts(entity_fts, rowid, content_text)
                    VALUES ('delete', old.id, old.content_text);
            END;
        CREATE TRIGGER IF NOT EXISTS entity_fts_au_old AFTER UPDATE ON graph_entities
            WHEN old.kind IN ('ReasoningLog', 'ToolCall')
            BEGIN
                INSERT INTO entity_fts(entity_fts, rowid, content_text)
                    VALUES ('delete', old.id, old.content_text);
            END;
        CREATE TRIGGER IF NOT EXISTS entity_fts_au_new AFTER UPDATE ON graph_entities
            WHEN new.kind IN ('ReasoningLog', 'ToolCall')
            BEGIN
                INSERT INTO entity_fts(rowid, content_text)
                    VALUES (new.id, new.content_text);
            END;
        "#,
    )?;

    Ok(())
}

/// Refuse to migrate if the SQLite is too old for generated columns. Returns
/// a clear, actionable error rather than letting the `ALTER` fail with a
/// generic message. `conn` is the migration transaction (Deref-coerced).
fn ensure_sqlite_supports_generated_columns(conn: &Connection) -> Result<()> {
    let version: String = conn.query_row("SELECT sqlite_version()", [], |row| row.get(0))?;
    let mut parts = version.split('.');
    let major = parts.next().and_then(|s| s.parse::<u32>().ok());
    let minor = parts.next().and_then(|s| s.parse::<u32>().ok());
    match (major, minor) {
        (Some(maj), Some(min)) if (maj, min) >= MIN_GENERATED_COLUMN => Ok(()),
        (Some(maj), Some(min)) => bail!(
            "atheneum migration v12 needs SQLite >= {}.{} for generated columns; \
             this database reports {} ({}.{}). Upgrade the system SQLite, or enable \
             the rusqlite 'bundled' feature in atheneum's Cargo.toml.",
            MIN_GENERATED_COLUMN.0,
            MIN_GENERATED_COLUMN.1,
            version,
            maj,
            min
        ),
        _ => Err(anyhow!("cannot parse sqlite_version() output: {}", version)),
    }
}
