use anyhow::Result;
use chrono::Utc;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlitegraph::GraphEntity;

use super::{
    AtheneumGraph, CommitParams, EdgeType, EndSessionParams, EntityType, FileWriteParams,
    FixChainParams, PromptParams, SessionParams, SessionSummary, TestRunParams, ToolCallParams,
};

fn event_row_from_rusqlite(row: &rusqlite::Row<'_>) -> Result<serde_json::Value, rusqlite::Error> {
    Ok(json!({
        "event_id": row.get::<_, i64>(0)?,
        "event_type": row.get::<_, String>(1)?,
        "entity_id": row.get::<_, String>(2)?,
        "session_id": row.get::<_, String>(3)?,
        "payload": serde_json::from_str::<Value>(row.get_ref(4)?.as_str()?)
            .unwrap_or(Value::Null),
        "timestamp": row.get::<_, String>(5)?,
    }))
}

impl AtheneumGraph {
    pub fn record_session(&self, params: SessionParams) -> Result<()> {
        let agent_id = self.ensure_agent(&params.agent_name)?;
        let agent_sql: i64 = self.with_raw_connection(|conn| {
            conn.query_row(
                "SELECT id FROM agents WHERE name = ?1",
                rusqlite::params![params.agent_name],
                |r| r.get(0),
            )
            .map_err(|e| anyhow::anyhow!(e))
        })?;

        let now = Utc::now().to_rfc3339();
        self.with_raw_connection(|conn| {
            conn.execute(
                "INSERT INTO sessions
                    (session_id, agent_id, project, tool, trigger, model, started_at,
                     git_branch, git_head, parent_session_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(session_id) DO NOTHING",
                rusqlite::params![
                    params.session_id,
                    agent_sql,
                    params.project,
                    params.tool,
                    params.trigger,
                    params.model,
                    now,
                    params.git_branch,
                    params.git_head,
                    params.parent_session_id,
                ],
            )?;
            Ok::<(), anyhow::Error>(())
        })?;

        let data = json!({
            "session_id": params.session_id,
            "project": params.project,
            "tool": params.tool,
            "trigger": params.trigger,
            "model": params.model,
            "started_at": now,
            "git_branch": params.git_branch,
            "git_head": params.git_head,
            "parent_session_id": params.parent_session_id,
        });
        let entity = GraphEntity {
            id: 0,
            kind: EntityType::Session.as_str().to_string(),
            name: format!("{}:{}", params.tool, params.session_id),
            file_path: None,
            data,
        };
        let entity_id = self
            .inner
            .insert_entity(&entity)
            .map_err(|e| anyhow::anyhow!("Failed to insert Session entity: {}", e))?;

        self.insert_edge(
            agent_id,
            entity_id,
            EdgeType::PerformedBy,
            json!({"provenance": {"method": "record_session"}}),
        )?;

        self.append_event_log(
            "session_start",
            &params.session_id,
            &params.session_id,
            &json!({"project": params.project, "tool": params.tool}),
        )?;
        Ok(())
    }

    pub fn end_session(&self, params: EndSessionParams) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.with_raw_connection(|conn| {
            conn.execute(
                "UPDATE sessions SET ended_at = ?1, exit_status = ?2,
                    prompt_count = ?3, tool_call_count = ?4, file_write_count = ?5,
                    commit_count = ?6, test_run_count = ?7,
                    total_input_tokens = ?8, total_output_tokens = ?9, total_cost_usd = ?10
                 WHERE session_id = ?11",
                rusqlite::params![
                    now,
                    params.exit_status,
                    params.prompt_count,
                    params.tool_call_count,
                    params.file_write_count,
                    params.commit_count,
                    params.test_run_count,
                    params.total_input_tokens,
                    params.total_output_tokens,
                    params.total_cost_usd,
                    params.session_id
                ],
            )?;
            Ok::<(), anyhow::Error>(())
        })?;

        self.append_event_log(
            "session_end",
            &params.session_id,
            &params.session_id,
            &json!({"exit_status": params.exit_status}),
        )?;
        Ok(())
    }

    pub fn record_evidence_prompt(&self, params: PromptParams) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let metadata = json!({
            "session_id": params.session_id,
            "role": params.role,
            "sequence": params.sequence,
            "input_hash": params.input_hash,
        });
        let metadata_str = super::json_to_string(&metadata)?;
        let session_id = params.session_id.clone();
        self.with_raw_connection(|conn| {
            conn.execute(
                "INSERT INTO reasoning_logs (agent_id, content, project_id, metadata, created_at, session_id)
                 VALUES (0, ?1, NULL, ?2, ?3, ?4)",
                rusqlite::params![params.role, metadata_str, now, params.session_id],
            )?;
            Ok::<(), anyhow::Error>(())
        })?;

        let payload = json!({
            "role": params.role,
            "sequence": params.sequence,
            "input_hash": params.input_hash,
            "input_tokens": params.input_tokens,
            "output_hash": params.output_hash,
            "output_tokens": params.output_tokens,
            "latency_ms": params.latency_ms,
            "model": params.model,
            "cost_usd": params.cost_usd,
        });
        self.append_event_log(
            "prompt",
            &format!("{}:{}", session_id, params.sequence),
            &session_id,
            &payload,
        )?;
        Ok(())
    }

    pub fn record_evidence_tool_call(&self, params: ToolCallParams) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let metadata = json!({
            "session_id": params.session_id,
            "tool_name": params.tool_name,
            "tool_category": params.tool_category,
            "tool_version": params.tool_version,
            "input_hash": params.input_hash,
            "input_summary": params.input_summary,
            "output_hash": params.output_hash,
            "output_summary": params.output_summary,
            "input_tokens_est": params.input_tokens_est,
        });
        let metadata_str = super::json_to_string(&metadata)?;
        let session_id = params.session_id.clone();
        let tool_name = params.tool_name.clone();
        self.with_raw_connection(|conn| {
            conn.execute(
                "INSERT INTO tool_calls (reasoning_log_id, tool_name, args, project_id, created_at, session_id)
                 VALUES (NULL, ?1, ?2, NULL, ?3, ?4)",
                rusqlite::params![params.tool_name, metadata_str, now, params.session_id],
            )?;
            Ok::<(), anyhow::Error>(())
        })?;

        let payload = json!({
            "tool_name": params.tool_name,
            "tool_version": params.tool_version,
            "input_hash": params.input_hash,
            "input_summary": params.input_summary,
            "output_hash": params.output_hash,
            "output_summary": params.output_summary,
            "exit_status": params.exit_status,
            "latency_ms": params.latency_ms,
            "input_tokens_est": params.input_tokens_est,
            "tool_category": params.tool_category,
        });
        self.append_event_log(
            "tool_call",
            &format!("{}:{}", session_id, tool_name),
            &session_id,
            &payload,
        )?;
        Ok(())
    }

    pub fn record_evidence_file_write(&self, params: FileWriteParams) -> Result<()> {
        let session_id = params.session_id.clone();
        let file_path = params.file_path.clone();
        let payload = json!({
            "file_path": params.file_path,
            "file_id": params.file_id,
            "before_hash": params.before_hash,
            "after_hash": params.after_hash,
            "lines_added": params.lines_added,
            "lines_deleted": params.lines_deleted,
            "lines_changed": params.lines_changed,
            "write_type": params.write_type,
        });
        self.append_event_log(
            "file_write",
            &format!("{}:{}", session_id, file_path),
            &params.session_id,
            &payload,
        )?;
        Ok(())
    }

    pub fn record_evidence_commit(&self, params: CommitParams) -> Result<()> {
        let commit_id = uuid::Uuid::now_v7().to_string();
        let now = Utc::now().to_rfc3339();
        let session_id = params.session_id.clone();

        self.with_raw_connection(|conn| {
            conn.execute(
                "INSERT INTO commits
                    (commit_id, session_id, commit_sha, parent_sha, message, author,
                     timestamp, files_changed, lines_inserted, lines_deleted, commit_type, feature_tag)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                rusqlite::params![
                    commit_id,
                    params.session_id,
                    params.commit_sha,
                    params.parent_sha,
                    params.message,
                    params.author,
                    now,
                    params.files_changed,
                    params.lines_inserted,
                    params.lines_deleted,
                    params.commit_type,
                    params.feature_tag
                ],
            )?;
            Ok::<(), anyhow::Error>(())
        })?;

        let data = json!({
            "commit_id": commit_id,
            "commit_sha": params.commit_sha,
            "commit_type": params.commit_type,
            "feature_tag": params.feature_tag,
        });
        let entity = GraphEntity {
            id: 0,
            kind: EntityType::Commit.as_str().to_string(),
            name: format!(
                "{}:{}",
                params.commit_sha.chars().take(8).collect::<String>(),
                params.commit_type
            ),
            file_path: None,
            data,
        };
        let commit_entity_id = self
            .inner
            .insert_entity(&entity)
            .map_err(|e| anyhow::anyhow!("Failed to insert Commit entity: {}", e))?;

        if let Some(session_entity_id) =
            self.find_entity_id_by_data("Session", "session_id", &session_id)?
        {
            self.insert_edge(
                session_entity_id,
                commit_entity_id,
                EdgeType::Created,
                json!({"provenance": {"method": "record_evidence_commit"}}),
            )?;
        }

        let payload = json!({
            "commit_sha": params.commit_sha,
            "commit_type": params.commit_type,
            "files_changed": params.files_changed,
            "feature_tag": params.feature_tag,
        });
        self.append_event_log("commit", &commit_id, &session_id, &payload)?;
        Ok(())
    }

    pub fn record_evidence_test_run(&self, params: TestRunParams) -> Result<()> {
        let test_run_id = uuid::Uuid::now_v7().to_string();
        let now = Utc::now().to_rfc3339();
        let session_id = params.session_id.clone();

        self.with_raw_connection(|conn| {
            conn.execute(
                "INSERT INTO test_runs
                    (test_run_id, session_id, commit_sha, test_command, test_suite,
                     test_name, result, duration_ms, logs_summary, timestamp)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    test_run_id,
                    params.session_id,
                    params.commit_sha,
                    params.test_command,
                    params.test_suite,
                    params.test_name,
                    params.result,
                    params.duration_ms,
                    params.logs_summary,
                    now
                ],
            )?;
            Ok::<(), anyhow::Error>(())
        })?;

        let data = json!({
            "test_run_id": test_run_id,
            "test_name": params.test_name,
            "result": params.result,
            "duration_ms": params.duration_ms,
        });
        let entity = GraphEntity {
            id: 0,
            kind: EntityType::TestRun.as_str().to_string(),
            name: format!("{}:{}", params.test_name, params.result),
            file_path: None,
            data,
        };
        let test_entity_id = self
            .inner
            .insert_entity(&entity)
            .map_err(|e| anyhow::anyhow!("Failed to insert TestRun entity: {}", e))?;

        if let Some(session_entity_id) =
            self.find_entity_id_by_data("Session", "session_id", &session_id)?
        {
            self.insert_edge(
                session_entity_id,
                test_entity_id,
                EdgeType::VerifiedBy,
                json!({"provenance": {"method": "record_evidence_test_run"}}),
            )?;
        }

        let payload = json!({
            "test_name": params.test_name,
            "result": params.result,
            "duration_ms": params.duration_ms,
        });
        self.append_event_log("test", &test_run_id, &session_id, &payload)?;
        Ok(())
    }

    pub fn record_evidence_fix_chain(&self, params: FixChainParams) -> Result<()> {
        let fix_chain_id = uuid::Uuid::now_v7().to_string();
        let now = Utc::now().to_rfc3339();
        let session_id = params.session_id.clone();

        let bug_commit_id: Option<String> = self.with_raw_connection(|conn| {
            Ok(conn
                .query_row(
                    "SELECT commit_id FROM commits WHERE commit_sha = ?1 LIMIT 1",
                    rusqlite::params![params.bug_commit_sha],
                    |r| r.get(0),
                )
                .ok())
        })?;
        let fix_commit_id: Option<String> = self.with_raw_connection(|conn| {
            Ok(conn
                .query_row(
                    "SELECT commit_id FROM commits WHERE commit_sha = ?1 LIMIT 1",
                    rusqlite::params![params.fix_commit_sha],
                    |r| r.get(0),
                )
                .ok())
        })?;

        self.with_raw_connection(|conn| {
            conn.execute(
                "INSERT INTO fix_chains
                    (fix_chain_id, bug_commit_id, fix_commit_id, fix_session_id,
                     fix_type, severity, cycles_to_fix, bug_timestamp, fix_timestamp, time_to_fix_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    fix_chain_id,
                    bug_commit_id,
                    fix_commit_id,
                    params.session_id,
                    params.fix_type,
                    params.severity,
                    params.cycles_to_fix,
                    now,
                    now,
                    params.time_to_fix_ms
                ],
            )?;
            Ok::<(), anyhow::Error>(())
        })?;

        let payload = json!({
            "bug_commit_sha": params.bug_commit_sha,
            "fix_commit_sha": params.fix_commit_sha,
            "fix_type": params.fix_type,
            "severity": params.severity,
            "cycles_to_fix": params.cycles_to_fix,
        });
        self.append_event_log("fix_linked", &fix_chain_id, &session_id, &payload)?;
        Ok(())
    }

    pub fn record_evidence_bench_run(
        &self,
        session_id: String,
        bench_name: String,
        mean_ns: Option<i64>,
        median_ns: Option<i64>,
        p95_ns: Option<i64>,
        is_regression: bool,
    ) -> Result<()> {
        let bench_run_id = uuid::Uuid::now_v7().to_string();
        let now = Utc::now().to_rfc3339();

        self.with_raw_connection(|conn| {
            conn.execute(
                "INSERT INTO bench_runs
                    (bench_run_id, session_id, bench_name, mean_ns, median_ns, p95_ns, is_regression, timestamp)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![
                    bench_run_id, session_id, bench_name, mean_ns, median_ns, p95_ns,
                    is_regression as i64, now
                ],
            )?;
            Ok::<(), anyhow::Error>(())
        })?;

        let payload = json!({
            "bench_name": bench_name,
            "mean_ns": mean_ns,
            "is_regression": is_regression,
        });
        self.append_event_log("bench", &bench_run_id, &session_id, &payload)?;
        Ok(())
    }

    pub fn query_events(
        &self,
        session_id: Option<&str>,
        event_type: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Value>> {
        let sid = session_id.map(|s| s.to_string());
        let et = event_type.map(|s| s.to_string());
        let lim = limit as i64;

        self.with_raw_connection(move |conn| {
            let mut sql = String::from(
                "SELECT event_id, event_type, entity_id, session_id, payload, timestamp
                 FROM event_log
                 WHERE 1=1",
            );
            if sid.is_some() {
                sql.push_str(" AND session_id = ?");
            }
            if et.is_some() {
                sql.push_str(" AND event_type = ?");
            }
            sql.push_str(" ORDER BY event_id DESC LIMIT ?");

            let mut stmt = conn.prepare_cached(&sql)?;
            let rows = match (sid, et) {
                (Some(s), Some(e)) => {
                    stmt.query_map(rusqlite::params![s, e, lim], event_row_from_rusqlite)?
                }
                (Some(s), None) => {
                    stmt.query_map(rusqlite::params![s, lim], event_row_from_rusqlite)?
                }
                (None, Some(e)) => {
                    stmt.query_map(rusqlite::params![e, lim], event_row_from_rusqlite)?
                }
                (None, None) => stmt.query_map(rusqlite::params![lim], event_row_from_rusqlite)?,
            };

            let mut events = Vec::new();
            for row in rows {
                events.push(row?);
            }
            Ok(events)
        })
    }

    /// Query recent sessions. If `project` is Some, filter to that project.
    pub fn query_sessions(
        &self,
        project: Option<&str>,
        last_n: i64,
        parent_id: Option<&str>,
    ) -> Result<Vec<SessionSummary>> {
        let pid = parent_id.map(|s| s.to_string());
        let project = project.map(|s| s.to_string());

        self.with_raw_connection(move |conn| {
            let mut sql = String::from(
                "SELECT s.session_id, s.project, s.git_branch, s.trigger,
                        s.started_at, s.ended_at, s.exit_status,
                        COALESCE(s.tool_call_count, 0),
                        COALESCE(s.file_write_count, 0),
                        COALESCE(s.commit_count, 0),
                        s.parent_session_id,
                        (SELECT json_extract(el.payload, '$.tool_name')
                         FROM event_log el
                         WHERE el.session_id = s.session_id AND el.event_type = 'tool_call'
                         ORDER BY el.event_id DESC LIMIT 1),
                        (SELECT json_extract(el.payload, '$.input_summary')
                         FROM event_log el
                         WHERE el.session_id = s.session_id AND el.event_type = 'tool_call'
                         ORDER BY el.event_id DESC LIMIT 1),
                        COALESCE(s.total_input_tokens, 0),
                        COALESCE(s.total_output_tokens, 0),
                        COALESCE(s.total_cost_usd, 0.0)
                 FROM sessions s
                 WHERE 1=1",
            );
            if project.is_some() {
                sql.push_str(" AND s.project = ?");
            }
            if pid.is_some() {
                sql.push_str(" AND s.parent_session_id = ?");
            }
            sql.push_str(" ORDER BY s.started_at DESC LIMIT ?");

            let mut stmt = conn.prepare_cached(&sql)?;
            let row_fn = |row: &rusqlite::Row<'_>| {
                Ok(SessionSummary {
                    session_id: row.get(0)?,
                    project: row.get(1)?,
                    git_branch: row.get(2)?,
                    trigger: row
                        .get::<_, Option<String>>(3)?
                        .unwrap_or_else(|| "cli".into()),
                    started_at: row.get(4)?,
                    ended_at: row.get(5)?,
                    exit_status: row.get(6)?,
                    tool_call_count: row.get(7)?,
                    file_write_count: row.get(8)?,
                    commit_count: row.get(9)?,
                    parent_session_id: row.get(10)?,
                    last_tool: row.get(11)?,
                    last_tool_summary: row.get(12)?,
                    total_input_tokens: row.get(13)?,
                    total_output_tokens: row.get(14)?,
                    total_cost_usd: row.get(15)?,
                })
            };

            let rows = match (&project, &pid) {
                (Some(p), Some(parent)) => {
                    stmt.query_map(rusqlite::params![p, parent, last_n], row_fn)?
                }
                (Some(p), None) => stmt.query_map(rusqlite::params![p, last_n], row_fn)?,
                (None, Some(parent)) => {
                    stmt.query_map(rusqlite::params![parent, last_n], row_fn)?
                }
                (None, None) => stmt.query_map(rusqlite::params![last_n], row_fn)?,
            };
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }

    /// Record a generic event into the event_log.
    pub fn record_event(&self, params: super::RecordEventParams) -> Result<()> {
        self.append_event_log(
            &params.event_type,
            &params.entity_id,
            &params.session_id,
            &params.payload,
        )
    }

    /// Store a subagent handover note on session stop.
    pub fn record_subagent_handover(
        &self,
        session_id: &str,
        summary: &str,
        files_changed: &[String],
        outcome: &str,
    ) -> Result<()> {
        let payload = json!({
            "summary": summary,
            "files_changed": files_changed,
            "outcome": outcome,
        });
        self.append_event_log("subagent_handover", session_id, session_id, &payload)
    }

    fn append_event_log(
        &self,
        event_type: &str,
        entity_id: &str,
        session_id: &str,
        payload: &Value,
    ) -> Result<()> {
        let payload_str = super::json_to_string(payload)?;
        let payload_hash = {
            let mut hasher = Sha256::new();
            hasher.update(payload_str.as_bytes());
            format!("{:x}", hasher.finalize())
        };
        let now = Utc::now().to_rfc3339();
        let event_type = event_type.to_string();
        let entity_id = entity_id.to_string();
        let session_id = session_id.to_string();

        self.with_raw_connection(|conn| {
            conn.execute(
                "INSERT INTO event_log (event_type, entity_id, session_id, payload_hash, payload, timestamp)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![event_type, entity_id, session_id, payload_hash, payload_str, now],
            )?;
            Ok::<(), anyhow::Error>(())
        })?;
        Ok(())
    }
}
