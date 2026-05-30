//! Stage 11f: v5 migration — make tool_calls.reasoning_log_id nullable.
//!
//! The original v1 schema required reasoning_log_id NOT NULL, which assumed
//! every tool call originated from a reasoning trace. Hook-originated calls
//! (from envoy-hook) have no associated reasoning log. This migration
//! recreates the table with reasoning_log_id as nullable so direct session
//! recording works without a synthetic log entry.

use anyhow::Result;
use rusqlite::Transaction;

pub fn migrate_v5_hook_compat(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS tool_calls_v5 (
             id               INTEGER PRIMARY KEY,
             reasoning_log_id INTEGER REFERENCES reasoning_logs(id),
             tool_name        TEXT NOT NULL,
             args             TEXT NOT NULL,
             project_id       TEXT,
             created_at       TEXT NOT NULL,
             session_id       TEXT REFERENCES sessions(session_id)
         );

         INSERT INTO tool_calls_v5 (id, reasoning_log_id, tool_name, args, project_id, created_at, session_id)
         SELECT id, reasoning_log_id, tool_name, args, project_id, created_at, session_id
         FROM tool_calls;

         DROP TABLE tool_calls;
         ALTER TABLE tool_calls_v5 RENAME TO tool_calls;

         CREATE INDEX IF NOT EXISTS tool_calls_log_idx      ON tool_calls(reasoning_log_id);
         CREATE INDEX IF NOT EXISTS tool_calls_tool_name_idx ON tool_calls(tool_name);
         CREATE INDEX IF NOT EXISTS tool_calls_project_idx  ON tool_calls(project_id);
         CREATE INDEX IF NOT EXISTS tool_calls_session_idx  ON tool_calls(session_id);",
    )?;
    Ok(())
}
