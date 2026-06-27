//! Migration v12 — `chat-columns-fts`: generated columns + FTS5 over
//! `graph_entities` chat rows (`ReasoningLog` + `ToolCall`).
//!
//! These tests prove four things, all grounded against the real insert path
//! (`record_session` → `record_evidence_prompt` / `record_evidence_tool_call`)
//! and the real migration body (`db::chat::migrate_v12_chat_columns_fts`):
//!
//! 1. A fresh in-memory DB migrates through the current schema head and still
//!    has the v12 generated columns, composite indexes, `entity_fts` virtual
//!    table, and four sync triggers.
//! 2. The generated columns (`session_id`, `sequence`, `role`, `content_text`)
//!    populate automatically from `graph_entities.data` JSON when chat rows are
//!    inserted via the existing evidence path — i.e. migration v12 required
//!    **zero** insert-path Rust changes.
//! 3. The FTS5 external-content table is kept in sync for chat kinds only
//!    (insert / delete / the two split update triggers), and session/sequence
//!    lookups use the composite index rather than a table scan.
//! 4. Re-opening an already-migrated DB is idempotent (the applied migrations
//!    stay stamped, and the schema plus indexed rows survive).
//!
//! The live (`~/.local/share/atheneum/atheneum.db`) v11→v12 upgrade is exercised
//! out-of-band in Phase 6 with the envoy stopped; touching a live, WAL-active
//! DB from a test would be unsafe, so it is intentionally not covered here.

use atheneum::graph::{AtheneumGraph, PromptParams, SessionParams, ToolCallParams};
use rusqlite::params;
use serde_json::json;

const CURRENT_SCHEMA_VERSION: i64 = 13;

/// Minimal `SessionParams` for the synthetic session `sess_v12`.
fn session_params() -> SessionParams {
    SessionParams {
        session_id: "sess_v12".to_string(),
        agent_name: "agent_v12".to_string(),
        project: "atheneum".to_string(),
        tool: "claude-code".to_string(),
        trigger: "test".to_string(),
        model: Some("test-model".to_string()),
        git_branch: Some("main".to_string()),
        git_head: Some("deadbeef".to_string()),
        parent_session_id: None,
        relations: vec![],
    }
}

/// A `PromptParams` at `sequence` carrying `content_summary` (the text that
/// ends up in the `content_text` generated column + the FTS index).
fn prompt_params(sequence: i64, content_summary: &str) -> PromptParams {
    PromptParams {
        session_id: "sess_v12".to_string(),
        role: "assistant".to_string(),
        sequence,
        content_summary: Some(content_summary.to_string()),
        source: Some("test".to_string()),
        input_hash: format!("in-{sequence}"),
        input_tokens: Some(10),
        output_hash: Some(format!("out-{sequence}")),
        output_tokens: Some(20),
        latency_ms: Some(5),
        model: Some("test-model".to_string()),
        cost_usd: Some(0.001),
        relations: vec![],
    }
}

/// A `ToolCallParams` at `sequence` for `tool_name` / `tool_category`.
fn tool_call_params(sequence: Option<i64>, tool_name: &str, tool_category: &str) -> ToolCallParams {
    ToolCallParams {
        session_id: "sess_v12".to_string(),
        tool_name: tool_name.to_string(),
        sequence,
        source: Some("test".to_string()),
        tool_version: Some("0.1".to_string()),
        input_hash: Some("tool-in".to_string()),
        input_summary: Some("tool input".to_string()),
        output_hash: Some("tool-out".to_string()),
        output_summary: Some("tool output".to_string()),
        exit_status: "success".to_string(),
        latency_ms: 7,
        input_tokens_est: Some(30),
        tool_category: tool_category.to_string(),
        relations: vec![],
    }
}

/// Collect the `detail` column of an `EXPLAIN QUERY PLAN` into one string.
fn explain_query_plan(
    conn: &rusqlite::Connection,
    sql: &str,
    args: &[&dyn rusqlite::ToSql],
) -> String {
    let mut stmt = conn.prepare(sql).expect("prepare EQP");
    let rows = stmt
        .query_map(args, |r| r.get::<_, String>(3))
        .expect("query EQP");
    let mut out = Vec::new();
    for row in rows {
        out.push(row.expect("EQP row"));
    }
    out.join("\n")
}

/// Assert the v12 schema objects exist on a freshly opened DB.
#[test]
fn fresh_db_has_v12_schema() {
    let graph = AtheneumGraph::open_in_memory().expect("open");

    let (columns, objects, version): (Vec<String>, Vec<String>, i64) = graph
        .with_raw_connection(|conn| {
            // PRAGMA table_info hides VIRTUAL generated columns; table_xinfo
            // lists them (with a non-zero `hidden` flag).
            let cols: Vec<String> = conn
                .prepare("PRAGMA table_xinfo(graph_entities)")?
                .query_map([], |r| r.get::<_, String>(1))?
                .filter_map(|r| r.ok())
                .collect();
            let objs: Vec<String> = conn
                .prepare("SELECT name FROM sqlite_master WHERE name IN ('entity_fts','idx_entities_session_seq','idx_entities_session_role_seq','entity_fts_ai','entity_fts_ad','entity_fts_au_old','entity_fts_au_new') ORDER BY name")?
                .query_map([], |r| r.get::<_, String>(0))?
                .filter_map(|r| r.ok())
                .collect();
            let v: i64 = conn.query_row(
                "SELECT MAX(version) FROM atheneum_schema_version",
                [],
                |r| r.get(0),
            )?;
            Ok::<_, anyhow::Error>((cols, objs, v))
        })
        .expect("query schema");

    for required in ["session_id", "sequence", "role", "content_text"] {
        assert!(
            columns.iter().any(|c| c == required),
            "missing generated column `{required}`; have: {columns:?}"
        );
    }
    for required in [
        "entity_fts",
        "idx_entities_session_seq",
        "idx_entities_session_role_seq",
        "entity_fts_ai",
        "entity_fts_ad",
        "entity_fts_au_old",
        "entity_fts_au_new",
    ] {
        assert!(
            objects.iter().any(|o| o == required),
            "missing v12 object `{required}`; have: {objects:?}"
        );
    }
    assert_eq!(
        version, CURRENT_SCHEMA_VERSION,
        "schema_version should be stamped to the current migration head"
    );
}

/// Zero insert-path change: a ReasoningLog written via the existing evidence
/// path populates the generated columns and the FTS index automatically.
#[test]
fn reasoning_log_generated_columns_populate_via_record_path() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph.record_session(session_params()).expect("session");
    graph
        .record_evidence_prompt(prompt_params(1, "chose HNSW over brute force"))
        .expect("prompt");

    let (sid, seq, role, content_text, fts_hit): (String, i64, String, String, i64) = graph
        .with_raw_connection(|conn| {
            let (sid, seq, role, content_text): (String, i64, String, String) = conn.query_row(
                "SELECT session_id, sequence, role, content_text
                 FROM graph_entities
                 WHERE kind='ReasoningLog' AND session_id=?1
                 ORDER BY sequence ASC LIMIT 1",
                params!["sess_v12"],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                },
            )?;
            let fts: i64 = conn.query_row(
                "SELECT count(*) FROM entity_fts WHERE entity_fts MATCH ?1",
                params!["HNSW"],
                |r| r.get(0),
            )?;
            Ok::<_, anyhow::Error>((sid, seq, role, content_text, fts))
        })
        .expect("query reasoning");

    assert_eq!(sid, "sess_v12");
    assert_eq!(seq, 1);
    assert_eq!(role, "assistant");
    assert!(
        content_text.contains("HNSW"),
        "content_text should derive from content_summary; got `{content_text}`"
    );
    assert_eq!(fts_hit, 1, "ReasoningLog should be indexed in entity_fts");
}

/// Zero insert-path change: a ToolCall written via the existing evidence path
/// populates `session_id`/`sequence` (role is NULL, content_text falls back to
/// tool_name + tool_category) and is indexed in FTS.
#[test]
fn tool_call_generated_columns_populate_via_record_path() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph.record_session(session_params()).expect("session");
    graph
        .record_evidence_tool_call(tool_call_params(Some(2), "Write", "edit"))
        .expect("tool call");

    let (sid, seq, role, content_text, fts_tool, fts_cat): (
        String,
        Option<i64>,
        Option<String>,
        String,
        i64,
        i64,
    ) = graph
        .with_raw_connection(|conn| {
            let (sid, seq, role, content_text): (String, Option<i64>, Option<String>, String) =
                conn.query_row(
                    "SELECT session_id, sequence, role, content_text
                 FROM graph_entities
                 WHERE kind='ToolCall' AND session_id=?1
                 ORDER BY id DESC LIMIT 1",
                    params!["sess_v12"],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, Option<i64>>(1)?,
                            r.get::<_, Option<String>>(2)?,
                            r.get::<_, String>(3)?,
                        ))
                    },
                )?;
            let fts_tool: i64 = conn.query_row(
                "SELECT count(*) FROM entity_fts WHERE entity_fts MATCH ?1",
                params!["Write"],
                |r| r.get(0),
            )?;
            let fts_cat: i64 = conn.query_row(
                "SELECT count(*) FROM entity_fts WHERE entity_fts MATCH ?1",
                params!["edit"],
                |r| r.get(0),
            )?;
            Ok::<_, anyhow::Error>((sid, seq, role, content_text, fts_tool, fts_cat))
        })
        .expect("query tool call");

    assert_eq!(sid, "sess_v12");
    assert_eq!(seq, Some(2));
    assert_eq!(
        role, None,
        "ToolCall has no role; generated column must be NULL"
    );
    assert!(
        content_text.contains("Write") && content_text.contains("edit"),
        "content_text should fall back to tool_name + tool_category; got `{content_text}`"
    );
    assert!(fts_tool >= 1, "tool_name should be FTS-searchable");
    assert!(fts_cat >= 1, "tool_category should be FTS-searchable");
}

/// Session/sequence lookups use the composite index, not a table scan.
#[test]
fn session_sequence_query_uses_index() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph.record_session(session_params()).expect("session");
    graph
        .record_evidence_prompt(prompt_params(1, "alpha"))
        .expect("prompt");
    graph
        .record_evidence_prompt(prompt_params(2, "beta"))
        .expect("prompt");

    let plan: String = graph
        .with_raw_connection(|conn| {
            Ok::<_, anyhow::Error>(explain_query_plan(
                conn,
                "EXPLAIN QUERY PLAN
                 SELECT id FROM graph_entities
                 WHERE session_id=?1
                 ORDER BY sequence",
                &[&"sess_v12" as &dyn rusqlite::ToSql],
            ))
        })
        .expect("eqp");

    assert!(
        plan.contains("idx_entities_session_seq"),
        "session/sequence lookup should use idx_entities_session_seq; plan:\n{plan}"
    );
    assert!(
        !plan.to_lowercase().contains("scan graph_entities"),
        "session/sequence lookup must not table-scan graph_entities; plan:\n{plan}"
    );
}

/// FTS MATCH is served by the `entity_fts` virtual table (FTS5), not a B-tree.
#[test]
fn fts_match_uses_fts5() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph.record_session(session_params()).expect("session");
    graph
        .record_evidence_prompt(prompt_params(1, "indexed token"))
        .expect("prompt");

    let plan: String = graph
        .with_raw_connection(|conn| {
            Ok::<_, anyhow::Error>(explain_query_plan(
                conn,
                "EXPLAIN QUERY PLAN
                 SELECT rowid FROM entity_fts WHERE entity_fts MATCH ?1",
                &[&"indexed" as &dyn rusqlite::ToSql],
            ))
        })
        .expect("eqp fts");

    assert!(
        plan.contains("entity_fts"),
        "FTS MATCH should be served by entity_fts; plan:\n{plan}"
    );
}

/// The split UPDATE triggers handle a kind transition correctly: updating a
/// non-chat row to a chat kind adds it to `entity_fts`; updating a chat row to
/// a non-chat kind removes it. (atheneum never transitions a row's kind in
/// practice, but the split makes the invariant explicit.)
#[test]
fn update_trigger_split_handles_kind_transition() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let data_reasoning = json!({
        "session_id": "sess_tr",
        "role": "assistant",
        "sequence": 1,
        "content_summary": "transition marker"
    });
    let data_file = json!({"path": "src/x.rs"});

    let id: i64 = graph
        .with_raw_connection(|conn| {
            conn.execute(
                "INSERT INTO graph_entities (kind, name, data) VALUES ('File', 'f1', ?1)",
                params![serde_json::to_string(&data_file)?],
            )?;
            Ok::<_, anyhow::Error>(conn.last_insert_rowid())
        })
        .expect("insert file");

    // File → ReasoningLog: au_old skips (old kind not chat), au_new adds to FTS.
    graph
        .with_raw_connection(|conn| {
            conn.execute(
                "UPDATE graph_entities SET kind='ReasoningLog', data=?1 WHERE id=?2",
                params![serde_json::to_string(&data_reasoning)?, id],
            )?;
            Ok::<_, anyhow::Error>(())
        })
        .expect("update to reasoning");

    let fts_after_up: i64 = graph
        .with_raw_connection(|conn| {
            Ok::<_, anyhow::Error>(conn.query_row(
                "SELECT count(*) FROM entity_fts WHERE entity_fts MATCH ?1",
                params!["transition"],
                |r| r.get(0),
            )?)
        })
        .expect("count fts after up");
    assert_eq!(
        fts_after_up, 1,
        "ReasoningLog transition should add the row to entity_fts"
    );

    // ReasoningLog → File: au_old removes from FTS, au_new skips (new kind not chat).
    graph
        .with_raw_connection(|conn| {
            conn.execute(
                "UPDATE graph_entities SET kind='File', data=?1 WHERE id=?2",
                params![serde_json::to_string(&data_file)?, id],
            )?;
            Ok::<_, anyhow::Error>(())
        })
        .expect("update to file");

    let fts_after_down: i64 = graph
        .with_raw_connection(|conn| {
            Ok::<_, anyhow::Error>(conn.query_row(
                "SELECT count(*) FROM entity_fts WHERE entity_fts MATCH ?1",
                params!["transition"],
                |r| r.get(0),
            )?)
        })
        .expect("count fts after down");
    assert_eq!(
        fts_after_down, 0,
        "File transition should remove the row from entity_fts"
    );
}

/// The delete trigger removes chat rows from `entity_fts`.
#[test]
fn delete_trigger_removes_chat_from_fts() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph.record_session(session_params()).expect("session");
    graph
        .record_evidence_prompt(prompt_params(1, "deletable token"))
        .expect("prompt");

    let before: i64 = graph
        .with_raw_connection(|conn| {
            Ok::<_, anyhow::Error>(conn.query_row(
                "SELECT count(*) FROM entity_fts WHERE entity_fts MATCH ?1",
                params!["deletable"],
                |r| r.get(0),
            )?)
        })
        .expect("count before delete");
    assert_eq!(before, 1);

    graph
        .with_raw_connection(|conn| {
            conn.execute(
                "DELETE FROM graph_entities WHERE kind='ReasoningLog' AND session_id=?1",
                params!["sess_v12"],
            )?;
            Ok::<_, anyhow::Error>(())
        })
        .expect("delete reasoning");

    let after: i64 = graph
        .with_raw_connection(|conn| {
            Ok::<_, anyhow::Error>(conn.query_row(
                "SELECT count(*) FROM entity_fts WHERE entity_fts MATCH ?1",
                params!["deletable"],
                |r| r.get(0),
            )?)
        })
        .expect("count after delete");
    assert_eq!(
        after, 0,
        "delete trigger should remove the row from entity_fts"
    );
}

/// Re-opening an already-migrated DB is idempotent: the schema head stays
/// stamped, and the schema objects plus indexed rows survive the close/reopen.
#[test]
fn reopen_is_idempotent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("migrate_v12.db");

    {
        let graph = AtheneumGraph::open(&db_path).expect("open 1");
        graph.record_session(session_params()).expect("session");
        graph
            .record_evidence_prompt(prompt_params(1, "persisted token"))
            .expect("prompt");
    }

    // Reopen — all migrations are already stamped, so re-open must be a no-op.
    let graph = AtheneumGraph::open(&db_path).expect("open 2");

    let (version, fts_hit, cols_ok): (i64, i64, bool) = graph
        .with_raw_connection(|conn| {
            let v: i64 = conn.query_row(
                "SELECT MAX(version) FROM atheneum_schema_version",
                [],
                |r| r.get(0),
            )?;
            let fts: i64 = conn.query_row(
                "SELECT count(*) FROM entity_fts WHERE entity_fts MATCH ?1",
                params!["persisted"],
                |r| r.get(0),
            )?;
            let has_cols: bool = conn.query_row(
                "SELECT count(*) FROM pragma_table_xinfo('graph_entities') WHERE name IN ('session_id','sequence','role','content_text')",
                [],
                |r| r.get::<_, i64>(0),
            )? == 4;
            Ok::<_, anyhow::Error>((v, fts, has_cols))
        })
        .expect("reopen query");

    assert_eq!(
        version, CURRENT_SCHEMA_VERSION,
        "schema_version should still be at the current migration head after reopen"
    );
    assert!(cols_ok, "generated columns should survive reopen");
    assert_eq!(
        fts_hit, 1,
        "FTS row should survive reopen (no re-backfill dupe)"
    );
}
