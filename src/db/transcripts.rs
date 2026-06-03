use anyhow::Result;
use rusqlite::Transaction;

pub fn migrate_v6_transcripts(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS transcript_imports (
            source_key         TEXT PRIMARY KEY,
            session_id         TEXT NOT NULL REFERENCES sessions(session_id),
            tool               TEXT NOT NULL,
            transcript_path    TEXT NOT NULL,
            offset             INTEGER NOT NULL DEFAULT 0,
            prompt_sequence    INTEGER NOT NULL DEFAULT 0,
            tool_sequence      INTEGER NOT NULL DEFAULT 0,
            file_access_sequence INTEGER NOT NULL DEFAULT 0,
            file_write_sequence  INTEGER NOT NULL DEFAULT 0,
            file_inode         INTEGER,
            file_mtime_ns      INTEGER,
            imported_at        TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_transcript_imports_session ON transcript_imports(session_id);
        CREATE INDEX IF NOT EXISTS idx_transcript_imports_path ON transcript_imports(transcript_path);",
    )?;
    Ok(())
}
