//! Phase 4 — live decision watcher detector tests.
//!
//! Synthetic transcripts with known `AskUserQuestion` (tool_use + matching
//! tool_result), `ExitPlanMode` (with `allowedPrompts`), and `TaskCreate`
//! lines. Asserts the Tier-1 detector emits the right `Decision` rows with
//! the right `source` / `chosen` / `alternatives` / `rationale`, that the
//! session-scope filter holds, and that re-scanning the same bytes is
//! idempotent (offset cache + `decision_exists` dedup).

use std::fs;
use std::path::{Path, PathBuf};

use atheneum::graph::{watch_decisions, AtheneumGraph, WatchConfig};
use serde_json::{json, Value};

/// One JSONL record. `type` + `message` + top-level fields a real transcript
/// carries. Content blocks are passed through verbatim.
fn record(rec_type: &str, content: Vec<Value>) -> String {
    json!({
        "type": rec_type,
        "message": { "role": rec_type, "content": content },
        "sessionId": "sess_watch",
    })
    .to_string()
}

fn text_block(text: &str) -> Value {
    json!({ "type": "text", "text": text })
}

fn tool_use(id: &str, name: &str, input: Value) -> Value {
    json!({ "type": "tool_use", "id": id, "name": name, "input": input })
}

fn tool_result(id: &str, content: &str) -> Value {
    json!({ "type": "tool_result", "tool_use_id": id, "content": content })
}

/// Write a transcript file under `<tmp>/.claude/projects/<proj>/<sid>.jsonl`.
fn write_transcript(tmp: &Path, sid: &str, lines: &[String]) -> PathBuf {
    let proj_dir = tmp.join(".claude").join("projects").join("proj_watch");
    fs::create_dir_all(&proj_dir).unwrap();
    let path = proj_dir.join(format!("{sid}.jsonl"));
    let mut body = String::new();
    for line in lines {
        body.push_str(line);
        body.push('\n');
    }
    fs::write(&path, body).unwrap();
    path
}

fn run_once(db: &Path, config_dir: &Path) -> atheneum::graph::WatchStats {
    let graph = AtheneumGraph::open(db).unwrap();
    let config = WatchConfig {
        config_dirs: vec![config_dir.to_path_buf()],
        interval: std::time::Duration::from_secs(1),
        project: Some("watchtest".to_string()),
        agent: "claude".to_string(),
        dry_run: false,
        once: true,
    };
    watch_decisions(&graph, &config).unwrap()
}

fn decision_rows(db: &Path, session_id: &str) -> Vec<Value> {
    let graph = AtheneumGraph::open(db).unwrap();
    let rows = graph
        .recent_discoveries(None, None, Some(session_id), 1000)
        .unwrap();
    rows.into_iter().map(|e| e.data).collect::<Vec<_>>()
}

#[test]
fn askuser_question_emits_one_decision_with_chosen_label() {
    let tmp = tempfile_dir();
    let db = tmp.join("watch.db");
    let config_dir = tmp.join(".claude");

    let tool_use = tool_use(
        "call_auq1",
        "AskUserQuestion",
        json!({
            "questions": [{
                "question": "Which index?",
                "header": "Index",
                "multiSelect": false,
                "options": [
                    { "label": "HNSW", "description": "fast ANN" },
                    { "label": "brute", "description": "exact but slow" }
                ]
            }]
        }),
    );
    let result = tool_result(
        "call_auq1",
        r#"Your questions have been answered: "Which index?"="HNSW". You can now continue."#,
    );
    let lines = vec![
        record("assistant", vec![text_block("asking"), tool_use]),
        record("user", vec![result]),
    ];
    write_transcript(&tmp, "sess_watch", &lines);

    let stats = run_once(&db, &config_dir);
    assert_eq!(
        stats.decisions_emitted, 1,
        "exactly one decision: {:?}",
        stats
    );
    assert_eq!(stats.decisions_skipped, 0);

    let rows = decision_rows(&db, "sess_watch");
    assert_eq!(rows.len(), 1);
    let d = &rows[0];
    assert_eq!(d["source"], json!("askuser"));
    assert_eq!(d["chosen"], json!("HNSW"));
    assert_eq!(d["target"], json!("Index"), "target = header");
    assert_eq!(d["alternatives"], json!(["HNSW", "brute"]));
    assert_eq!(
        d["rationale"],
        json!("fast ANN"),
        "rationale = chosen option's description"
    );
}

#[test]
fn exitplan_mode_emits_decision_only_when_allowed_prompts_present() {
    let tmp = tempfile_dir();
    let db = tmp.join("watch.db");
    let config_dir = tmp.join(".claude");

    // With allowedPrompts → one Decision.
    let with_prompts = tool_use(
        "call_ep1",
        "ExitPlanMode",
        json!({ "allowedPrompts": ["cargo build", "cargo test"] }),
    );
    // Empty input → no Decision (100% precision: no structured signal).
    let empty = tool_use("call_ep2", "ExitPlanMode", json!({}));
    let lines = vec![
        record("assistant", vec![text_block("plan a"), with_prompts]),
        record("assistant", vec![text_block("plan b"), empty]),
    ];
    write_transcript(&tmp, "sess_watch", &lines);

    let stats = run_once(&db, &config_dir);
    assert_eq!(
        stats.decisions_emitted, 1,
        "only the allowedPrompts one: {:?}",
        stats
    );

    let rows = decision_rows(&db, "sess_watch");
    assert_eq!(rows.len(), 1);
    let d = &rows[0];
    assert_eq!(d["source"], json!("exitplan"));
    assert_eq!(d["chosen"], json!("proceed"));
    assert_eq!(d["target"], json!("plan-approval"));
    assert_eq!(d["alternatives"], json!(["cargo build", "cargo test"]));
}

#[test]
fn taskcreate_emits_decision_with_subject_as_target() {
    let tmp = tempfile_dir();
    let db = tmp.join("watch.db");
    let config_dir = tmp.join(".claude");

    let tu = tool_use(
        "call_tc1",
        "TaskCreate",
        json!({
            "subject": "Create scorer schema",
            "description": "tables in src/graph/scorer/schema.rs",
            "activeForm": "Creating scorer schema"
        }),
    );
    let lines = vec![record("assistant", vec![text_block("planning"), tu])];
    write_transcript(&tmp, "sess_watch", &lines);

    let stats = run_once(&db, &config_dir);
    assert_eq!(stats.decisions_emitted, 1, "{:?}", stats);

    let rows = decision_rows(&db, "sess_watch");
    assert_eq!(rows.len(), 1);
    let d = &rows[0];
    assert_eq!(d["source"], json!("taskcreate"));
    assert_eq!(d["target"], json!("Create scorer schema"));
    assert_eq!(d["chosen"], json!("Create scorer schema"));
    assert_eq!(
        d["rationale"],
        json!("tables in src/graph/scorer/schema.rs")
    );
}

#[test]
fn todowrite_emits_only_for_new_tasks() {
    let tmp = tempfile_dir();
    let db = tmp.join("watch.db");
    let config_dir = tmp.join(".claude");

    // First TodoWrite with two new tasks → two decisions.
    let tw1 = tool_use(
        "call_tw1",
        "TodoWrite",
        json!({
            "todos": [
                { "content": "task A", "status": "pending" },
                { "content": "task B", "status": "pending" }
            ]
        }),
    );
    // Second TodoWrite rewrites the list (A again + C new) → only C is new.
    let tw2 = tool_use(
        "call_tw2",
        "TodoWrite",
        json!({
            "todos": [
                { "content": "task A", "status": "completed" },
                { "content": "task C", "status": "pending" }
            ]
        }),
    );
    let lines = vec![
        record("assistant", vec![text_block("first"), tw1]),
        record("assistant", vec![text_block("second"), tw2]),
    ];
    write_transcript(&tmp, "sess_watch", &lines);

    let stats = run_once(&db, &config_dir);
    // A, B (first call) + C (second call, A is a re-write) = 3 emitted.
    assert_eq!(stats.decisions_emitted, 3, "{:?}", stats);

    let rows = decision_rows(&db, "sess_watch");
    let sources: Vec<&str> = rows.iter().map(|d| d["source"].as_str().unwrap()).collect();
    assert!(sources.iter().all(|s| *s == "todowrite"));
    assert_eq!(rows.len(), 3);
}

#[test]
fn rescan_same_bytes_is_idempotent() {
    let tmp = tempfile_dir();
    let db = tmp.join("watch.db");
    let config_dir = tmp.join(".claude");

    let tu = tool_use(
        "call_tc_idem",
        "TaskCreate",
        json!({ "subject": "do the thing", "description": "desc" }),
    );
    let lines = vec![record("assistant", vec![text_block("go"), tu])];
    write_transcript(&tmp, "sess_watch", &lines);

    let s1 = run_once(&db, &config_dir);
    assert_eq!(s1.decisions_emitted, 1);
    let s2 = run_once(&db, &config_dir);
    // Second scan: offset cache already past the bytes → nothing new, nothing
    // re-emitted. Even if it were re-read, decision_exists dedup catches it.
    assert_eq!(
        s2.decisions_emitted, 0,
        "second scan must emit nothing: {:?}",
        s2
    );
    assert_eq!(
        decision_rows(&db, "sess_watch").len(),
        1,
        "row count stable at 1"
    );
}

#[test]
fn decisions_are_session_scoped() {
    let tmp = tempfile_dir();
    let db = tmp.join("watch.db");
    let config_dir = tmp.join(".claude");

    let tu = tool_use(
        "call_tc_scope",
        "TaskCreate",
        json!({ "subject": "scoped task", "description": "" }),
    );
    let lines = vec![record("assistant", vec![text_block("x"), tu])];
    write_transcript(&tmp, "sess_watch", &lines);

    let _ = run_once(&db, &config_dir);
    // The captured decision carries session_id = sess_watch (file stem).
    let rows = decision_rows(&db, "sess_watch");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["session_id"], json!("sess_watch"));
    // A different session_id filter returns nothing.
    let graph = AtheneumGraph::open(&db).unwrap();
    let other = graph
        .recent_discoveries(None, None, Some("sess_other"), 1000)
        .unwrap();
    assert!(other.is_empty());
}

#[test]
fn partial_final_line_is_tolerated_not_fabricated() {
    let tmp = tempfile_dir();
    let db = tmp.join("watch.db");
    let config_dir = tmp.join(".claude");

    let tu = tool_use(
        "call_tc_partial",
        "TaskCreate",
        json!({ "subject": "finished task", "description": "" }),
    );
    let complete_line = record("assistant", vec![text_block("x"), tu]);

    // One complete line, then a partial line with no trailing newline (a live
    // stream mid-write). The partial bytes must not parse, must not crash, and
    // must not fabricate a decision — only the complete line is captured.
    let proj_dir = tmp.join(".claude").join("projects").join("proj_watch");
    fs::create_dir_all(&proj_dir).unwrap();
    let path = proj_dir.join("sess_watch.jsonl");
    fs::write(
        &path,
        format!("{complete_line}\n{{\"type\":\"assistant\",\"message\":"),
    )
    .unwrap();

    let stats = run_once(&db, &config_dir);
    assert_eq!(
        stats.decisions_emitted, 1,
        "only the complete line: {:?}",
        stats
    );
    assert_eq!(decision_rows(&db, "sess_watch").len(), 1);
}

#[test]
fn fresh_once_rerun_uses_dedup_safety_net() {
    // Each `--once` invocation starts with a cold in-memory cursor (offset 0)
    // and re-reads the whole file. The `decision_exists` dedup guard must keep
    // the row count stable across reruns (the cron/--once safety net).
    let tmp = tempfile_dir();
    let db = tmp.join("watch.db");
    let config_dir = tmp.join(".claude");

    let tu = tool_use(
        "call_tc_rerun",
        "TaskCreate",
        json!({ "subject": "stable task", "description": "" }),
    );
    let lines = vec![record("assistant", vec![text_block("x"), tu])];
    write_transcript(&tmp, "sess_watch", &lines);

    let s1 = run_once(&db, &config_dir);
    assert_eq!(s1.decisions_emitted, 1);
    let s2 = run_once(&db, &config_dir);
    assert_eq!(s2.decisions_emitted, 0, "rerun emits nothing: {:?}", s2);
    assert_eq!(
        s2.decisions_skipped, 1,
        "the re-read line is deduped: {:?}",
        s2
    );
    assert_eq!(
        decision_rows(&db, "sess_watch").len(),
        1,
        "row count stable"
    );
}

fn tempfile_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "atheneum-watch-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}
