use atheneum::graph::{AtheneumGraph, ClaudeTranscriptImportParams, EdgeType};
use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::io::Write;
use tempfile::tempdir;

fn entity_id_by_data(graph: &AtheneumGraph, kind: &str, field: &str, expected: &str) -> i64 {
    graph
        .entities_by_kind(kind)
        .expect("entities_by_kind")
        .into_iter()
        .find(|entity| entity.data.get(field).and_then(Value::as_str) == Some(expected))
        .map(|entity| entity.id)
        .unwrap_or_else(|| panic!("missing {kind} entity with {field}={expected}"))
}

fn count_edges(graph: &AtheneumGraph, from_id: i64, to_id: i64, edge_type: EdgeType) -> usize {
    graph
        .outgoing_edges(from_id)
        .expect("outgoing_edges")
        .into_iter()
        .filter(|edge| edge.to_id == to_id && edge.edge_type == edge_type.as_str())
        .count()
}

fn latest_session_id(graph: &AtheneumGraph, project: &str) -> String {
    graph
        .query_sessions(Some(project), 1, None)
        .expect("query_sessions")
        .into_iter()
        .next()
        .expect("session summary")
        .session_id
}

fn session_metrics(graph: &AtheneumGraph, session_id: &str) -> (i64, i64, i64, i64, i64) {
    graph
        .with_raw_connection(|conn| {
            Ok(conn.query_row(
                "SELECT prompt_count, tool_call_count, file_write_count, total_input_tokens, total_output_tokens
                 FROM sessions WHERE session_id = ?1",
                rusqlite::params![session_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )?)
        })
        .expect("session metrics")
}

#[test]
fn sync_claude_transcript_imports_prompt_tool_and_file_access_evidence() {
    let temp = tempdir().expect("tempdir");
    let db_path = temp.path().join("atheneum.db");
    let transcript_dir = temp
        .path()
        .join("projects")
        .join("-home-user-Projects-forge");
    fs::create_dir_all(&transcript_dir).expect("create transcript dir");
    let transcript_path = transcript_dir.join("session-123.jsonl");
    fs::write(
        &transcript_path,
        concat!(
            "{\"type\":\"user\",\"timestamp\":\"2026-06-03T08:00:00Z\",\"gitBranch\":\"main\",",
            "\"message\":{\"role\":\"user\",\"content\":\"Fix the router bug\"}}\n",
            "{\"type\":\"assistant\",\"timestamp\":\"2026-06-03T08:00:01Z\",\"message\":{\"model\":\"claude-opus-4-6\",",
            "\"usage\":{\"input_tokens\":10,\"output_tokens\":20,\"cache_read_input_tokens\":30,\"cache_creation_input_tokens\":0},",
            "\"content\":[{\"type\":\"text\",\"text\":\"I will inspect the router.\"},",
            "{\"type\":\"tool_use\",\"name\":\"Read\",\"input\":{\"file_path\":\"src/router.rs\"}},",
            "{\"type\":\"tool_use\",\"name\":\"Bash\",\"input\":{\"command\":\"cargo test --lib\"}}]}}\n",
            "{\"type\":\"assistant\",\"timestamp\":\"2026-06-03T08:00:02Z\",\"message\":{\"model\":\"claude-opus-4-6\",",
            "\"usage\":{\"input_tokens\":11,\"output_tokens\":7,\"cache_read_input_tokens\":29,\"cache_creation_input_tokens\":0},",
            "\"content\":[{\"type\":\"tool_use\",\"name\":\"Edit\",\"input\":{\"file_path\":\"src/router.rs\"}}]}}\n"
        ),
    )
    .expect("write transcript");

    let graph = AtheneumGraph::open(&db_path).expect("open graph");
    let summary = graph
        .sync_claude_transcript(ClaudeTranscriptImportParams {
            transcript_path: transcript_path.clone(),
            session_id: None,
            project: Some("forge".into()),
            agent_name: "claude".into(),
            tool: "claude-code".into(),
            trigger: "transcript-import".into(),
        })
        .expect("sync transcript");

    assert_eq!(summary.session_id, "session-123");
    assert_eq!(summary.project, "forge");
    assert_eq!(summary.imported_prompts, 2);
    assert_eq!(summary.imported_tool_calls, 3);
    assert_eq!(summary.imported_file_accesses, 2);
    assert_eq!(summary.total_input_tokens, 21);
    assert_eq!(summary.total_output_tokens, 27);
    assert_eq!(summary.total_cache_read_tokens, 59);
    assert_eq!(summary.prompt_count, 2);
    assert_eq!(summary.tool_call_count, 3);
    assert_eq!(summary.file_access_count, 2);
    assert_eq!(summary.file_write_count, 0);

    let session_id = latest_session_id(&graph, "forge");
    assert_eq!(session_id, "session-123");
    let (prompt_count, tool_call_count, file_write_count, total_input, total_output) =
        session_metrics(&graph, &session_id);
    assert_eq!(prompt_count, 2);
    assert_eq!(tool_call_count, 3);
    assert_eq!(file_write_count, 0);
    assert_eq!(total_input, 21);
    assert_eq!(total_output, 27);

    let session_id = entity_id_by_data(&graph, "Session", "session_id", "session-123");
    let file_id = entity_id_by_data(&graph, "File", "path", "src/router.rs");
    assert_eq!(
        count_edges(&graph, session_id, file_id, EdgeType::Accessed),
        2
    );

    let events = graph
        .query_events(Some("session-123"), Some("transcript_sync"), 10)
        .expect("query events");
    assert_eq!(events.len(), 1);
}

#[test]
fn sync_claude_transcript_is_incremental() {
    let temp = tempdir().expect("tempdir");
    let db_path = temp.path().join("atheneum.db");
    let transcript_dir = temp
        .path()
        .join("projects")
        .join("-home-user-Projects-forge");
    fs::create_dir_all(&transcript_dir).expect("create transcript dir");
    let transcript_path = transcript_dir.join("session-456.jsonl");
    fs::write(
        &transcript_path,
        concat!(
            "{\"type\":\"user\",\"timestamp\":\"2026-06-03T08:10:00Z\",",
            "\"message\":{\"role\":\"user\",\"content\":\"Check tests\"}}\n",
            "{\"type\":\"assistant\",\"timestamp\":\"2026-06-03T08:10:01Z\",\"message\":{\"model\":\"claude-sonnet-4\",",
            "\"usage\":{\"input_tokens\":3,\"output_tokens\":5,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":12},",
            "\"content\":[{\"type\":\"text\",\"text\":\"Running tests.\"}]}}\n"
        ),
    )
    .expect("write transcript");

    let graph = AtheneumGraph::open(&db_path).expect("open graph");
    let first = graph
        .sync_claude_transcript(ClaudeTranscriptImportParams {
            transcript_path: transcript_path.clone(),
            session_id: None,
            project: Some("forge".into()),
            agent_name: "claude".into(),
            tool: "claude-code".into(),
            trigger: "transcript-import".into(),
        })
        .expect("first sync");
    assert_eq!(first.imported_prompts, 2);
    assert_eq!(first.imported_tool_calls, 0);

    let second = graph
        .sync_claude_transcript(ClaudeTranscriptImportParams {
            transcript_path: transcript_path.clone(),
            session_id: None,
            project: Some("forge".into()),
            agent_name: "claude".into(),
            tool: "claude-code".into(),
            trigger: "transcript-import".into(),
        })
        .expect("second sync without changes");
    assert_eq!(second.imported_prompts, 0);
    assert_eq!(second.imported_tool_calls, 0);

    let mut transcript = OpenOptions::new()
        .append(true)
        .open(&transcript_path)
        .expect("open for append");
    write!(
        transcript,
        "{}",
        concat!(
            "{\"type\":\"assistant\",\"timestamp\":\"2026-06-03T08:10:02Z\",\"message\":{\"model\":\"claude-sonnet-4\",",
            "\"usage\":{\"input_tokens\":4,\"output_tokens\":6,\"cache_read_input_tokens\":10,\"cache_creation_input_tokens\":0},",
            "\"content\":[{\"type\":\"text\",\"text\":\"Reading the test file.\"},",
            "{\"type\":\"tool_use\",\"name\":\"Read\",\"input\":{\"file_path\":\"tests/router.rs\"}}]}}\n"
        )
    )
    .expect("append transcript");
    transcript.flush().expect("flush append");

    let third = graph
        .sync_claude_transcript(ClaudeTranscriptImportParams {
            transcript_path: transcript_path.clone(),
            session_id: None,
            project: Some("forge".into()),
            agent_name: "claude".into(),
            tool: "claude-code".into(),
            trigger: "transcript-import".into(),
        })
        .expect("third sync with new data");
    assert_eq!(third.imported_prompts, 1);
    assert_eq!(third.imported_tool_calls, 1);
    assert_eq!(third.imported_file_accesses, 1);
    assert_eq!(third.prompt_count, 3);
    assert_eq!(third.tool_call_count, 1);
    assert_eq!(third.file_access_count, 1);

    let session_id = latest_session_id(&graph, "forge");
    let (prompt_count, tool_call_count, file_write_count, total_input, total_output) =
        session_metrics(&graph, &session_id);
    assert_eq!(prompt_count, 3);
    assert_eq!(tool_call_count, 1);
    assert_eq!(file_write_count, 0);
    assert_eq!(total_input, 7);
    assert_eq!(total_output, 11);

    let events = graph
        .query_events(Some("session-456"), Some("transcript_sync"), 10)
        .expect("query transcript_sync events");
    assert_eq!(events.len(), 2);
}

#[test]
fn sync_claude_transcript_replays_after_truncation() {
    let temp = tempdir().expect("tempdir");
    let db_path = temp.path().join("atheneum.db");
    let transcript_dir = temp
        .path()
        .join("projects")
        .join("-home-user-Projects-forge");
    fs::create_dir_all(&transcript_dir).expect("create transcript dir");
    let transcript_path = transcript_dir.join("session-789.jsonl");
    fs::write(
        &transcript_path,
        concat!(
            "{\"type\":\"user\",\"timestamp\":\"2026-06-03T08:20:00Z\",\"message\":{\"role\":\"user\",\"content\":\"Inspect A\"}}\n",
            "{\"type\":\"assistant\",\"timestamp\":\"2026-06-03T08:20:01Z\",\"message\":{\"model\":\"claude-sonnet-4\",\"usage\":{\"input_tokens\":2,\"output_tokens\":3,\"cache_read_input_tokens\":4,\"cache_creation_input_tokens\":0},\"content\":[{\"type\":\"text\",\"text\":\"Reading A.\"},{\"type\":\"tool_use\",\"name\":\"Read\",\"input\":{\"file_path\":\"src/a.rs\"}}]}}\n"
        ),
    )
    .expect("write initial transcript");

    let graph = AtheneumGraph::open(&db_path).expect("open graph");
    let first = graph
        .sync_claude_transcript(ClaudeTranscriptImportParams {
            transcript_path: transcript_path.clone(),
            session_id: None,
            project: Some("forge".into()),
            agent_name: "claude".into(),
            tool: "claude-code".into(),
            trigger: "transcript-import".into(),
        })
        .expect("first sync");
    assert_eq!(first.imported_prompts, 2);
    assert_eq!(first.imported_file_accesses, 1);

    fs::write(
        &transcript_path,
        concat!(
            "{\"type\":\"user\",\"timestamp\":\"2026-06-03T08:21:00Z\",\"message\":{\"role\":\"user\",\"content\":\"Inspect B\"}}\n",
            "{\"type\":\"assistant\",\"timestamp\":\"2026-06-03T08:21:01Z\",\"message\":{\"model\":\"claude-sonnet-4\",\"usage\":{\"input_tokens\":5,\"output_tokens\":6,\"cache_read_input_tokens\":7,\"cache_creation_input_tokens\":0},\"content\":[{\"type\":\"text\",\"text\":\"Reading B.\"},{\"type\":\"tool_use\",\"name\":\"Read\",\"input\":{\"file_path\":\"src/b.rs\"}}]}}\n"
        ),
    )
    .expect("replace transcript contents");

    let replay = graph
        .sync_claude_transcript(ClaudeTranscriptImportParams {
            transcript_path: transcript_path.clone(),
            session_id: None,
            project: Some("forge".into()),
            agent_name: "claude".into(),
            tool: "claude-code".into(),
            trigger: "transcript-import".into(),
        })
        .expect("replay sync");
    assert_eq!(replay.imported_prompts, 2);
    assert_eq!(replay.imported_tool_calls, 1);
    assert_eq!(replay.imported_file_accesses, 1);
    assert_eq!(replay.total_input_tokens, 5);
    assert_eq!(replay.total_output_tokens, 6);

    let session_id = latest_session_id(&graph, "forge");
    let (prompt_count, tool_call_count, file_write_count, total_input, total_output) =
        session_metrics(&graph, &session_id);
    assert_eq!(prompt_count, 2);
    assert_eq!(tool_call_count, 1);
    assert_eq!(file_write_count, 0);
    assert_eq!(total_input, 5);
    assert_eq!(total_output, 6);

    let session_entity_id = entity_id_by_data(&graph, "Session", "session_id", "session-789");
    let new_file_id = entity_id_by_data(&graph, "File", "path", "src/b.rs");
    assert_eq!(
        count_edges(&graph, session_entity_id, new_file_id, EdgeType::Accessed),
        1
    );

    let old_a_exists = graph
        .with_raw_connection(|conn| {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM event_log
                 WHERE session_id = 'session-789'
                   AND event_type = 'file_access'
                   AND json_extract(payload, '$.file_path') = 'src/a.rs'",
                [],
                |row| row.get::<_, i64>(0),
            )?)
        })
        .expect("query old file access count");
    assert_eq!(old_a_exists, 0);

    let reset_events = graph
        .query_events(Some("session-789"), Some("transcript_reset"), 10)
        .expect("query transcript_reset events");
    assert_eq!(reset_events.len(), 1);
}
