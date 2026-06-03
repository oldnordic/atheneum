use anyhow::Result;
use chrono::Utc;
use serde_json::json;
use sqlitegraph::GraphEntity;

use super::super::json_to_string;
use super::super::{
    AtheneumGraph, CommitParams, EdgeType, EntityType, FileAccessParams, FileWriteParams,
    FixChainParams, PromptParams, RelationEndpoint, TestRunParams, ToolCallParams,
};

impl AtheneumGraph {
    fn ensure_commit_sql_row(
        &self,
        session_id: &str,
        commit_sha: &str,
        commit_type: &str,
    ) -> Result<String> {
        if let Some(existing) = self.with_raw_connection(|conn| {
            Ok(conn
                .query_row(
                    "SELECT commit_id FROM commits WHERE commit_sha = ?1 LIMIT 1",
                    rusqlite::params![commit_sha],
                    |row| row.get(0),
                )
                .ok())
        })? {
            return Ok(existing);
        }

        let commit_id = uuid::Uuid::now_v7().to_string();
        let now = Utc::now().to_rfc3339();
        self.with_raw_connection(|conn| {
            conn.execute(
                "INSERT INTO commits
                    (commit_id, session_id, commit_sha, parent_sha, message, author,
                     timestamp, files_changed, lines_inserted, lines_deleted, commit_type, feature_tag)
                 VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6, 0, 0, 0, ?7, NULL)",
                rusqlite::params![
                    commit_id,
                    session_id,
                    commit_sha,
                    format!("synthetic {} commit for fix chain", commit_type),
                    "atheneum",
                    now,
                    commit_type,
                ],
            )?;
            Ok::<(), anyhow::Error>(())
        })?;
        Ok(commit_id)
    }

    pub fn record_evidence_prompt(&self, params: PromptParams) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let metadata = json!({
            "session_id": params.session_id,
            "role": params.role,
            "sequence": params.sequence,
            "source": params.source,
            "input_hash": params.input_hash,
        });
        let metadata_str = json_to_string(&metadata)?;
        let session_id = params.session_id.clone();
        let session_agent_id = self.session_agent_sql_id(&session_id)?.ok_or_else(|| {
            anyhow::anyhow!("Session {} not found for prompt evidence", session_id)
        })?;
        self.with_raw_connection(|conn| {
            conn.execute(
                "INSERT INTO reasoning_logs (agent_id, content, project_id, metadata, created_at, session_id)
                 VALUES (?1, ?2, NULL, ?3, ?4, ?5)",
                rusqlite::params![
                    session_agent_id,
                    params.role,
                    metadata_str,
                    now,
                    params.session_id
                ],
            )?;
            Ok::<(), anyhow::Error>(())
        })?;

        let prompt_entity = GraphEntity {
            id: 0,
            kind: "ReasoningLog".to_string(),
            name: format!("{}:{}", session_id, params.sequence),
            file_path: None,
            data: json!({
                "session_id": params.session_id,
                "role": params.role,
                "sequence": params.sequence,
                "content_summary": params.content_summary,
                "source": params.source,
                "input_hash": params.input_hash,
                "output_hash": params.output_hash,
                "model": params.model,
            }),
        };
        let prompt_entity_id = self
            .inner
            .insert_entity(&prompt_entity)
            .map_err(|e| anyhow::anyhow!("Failed to insert prompt entity: {}", e))?;

        if let Some(session_entity_id) = self.maybe_session_entity_id(&session_id)? {
            self.insert_edge(
                prompt_entity_id,
                session_entity_id,
                EdgeType::ObservedIn,
                json!({"provenance": {"method": "record_evidence_prompt"}}),
            )?;
        }
        self.link_entity_to_project(
            prompt_entity_id,
            self.session_project(&session_id)?.as_deref(),
        )?;
        self.ingest_relation_hints(&params.relations)?;

        let payload = json!({
            "role": params.role,
            "sequence": params.sequence,
            "content_summary": params.content_summary,
            "source": params.source,
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
            "sequence": params.sequence,
            "source": params.source,
            "tool_name": params.tool_name,
            "tool_category": params.tool_category,
            "tool_version": params.tool_version,
            "input_hash": params.input_hash,
            "input_summary": params.input_summary,
            "output_hash": params.output_hash,
            "output_summary": params.output_summary,
            "input_tokens_est": params.input_tokens_est,
        });
        let metadata_str = json_to_string(&metadata)?;
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

        let tool_entity = GraphEntity {
            id: 0,
            kind: EntityType::ToolCall.as_str().to_string(),
            name: tool_name.clone(),
            file_path: None,
            data: json!({
                "session_id": params.session_id,
                "sequence": params.sequence,
                "source": params.source,
                "tool_name": params.tool_name,
                "tool_version": params.tool_version,
                "tool_category": params.tool_category,
                "input_hash": params.input_hash,
                "output_hash": params.output_hash,
                "exit_status": params.exit_status,
            }),
        };
        let tool_entity_id = self
            .inner
            .insert_entity(&tool_entity)
            .map_err(|e| anyhow::anyhow!("Failed to insert tool call entity: {}", e))?;

        if let Some(session_entity_id) = self.maybe_session_entity_id(&session_id)? {
            self.insert_edge(
                tool_entity_id,
                session_entity_id,
                EdgeType::ObservedIn,
                json!({"provenance": {"method": "record_evidence_tool_call"}}),
            )?;
            self.insert_edge(
                session_entity_id,
                tool_entity_id,
                EdgeType::HandledByTool,
                json!({"provenance": {"method": "record_evidence_tool_call"}}),
            )?;
        }
        self.link_entity_to_project(
            tool_entity_id,
            self.session_project(&session_id)?.as_deref(),
        )?;
        self.ingest_relation_hints(&params.relations)?;

        let payload = json!({
            "sequence": params.sequence,
            "source": params.source,
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
            &match params.sequence {
                Some(sequence) => format!("{}:{}:{}", session_id, tool_name, sequence),
                None => format!("{}:{}", session_id, tool_name),
            },
            &session_id,
            &payload,
        )?;
        Ok(())
    }

    pub fn record_evidence_file_write(&self, params: FileWriteParams) -> Result<()> {
        let session_id = params.session_id.clone();
        let file_path = params.file_path.clone();
        let file_entity_id = self.ensure_relation_endpoint(&RelationEndpoint {
            kind: "File".to_string(),
            name: params.file_path.clone(),
            file_path: Some(params.file_path.clone()),
            data: json!({
                "sequence": params.sequence,
                "file_id": params.file_id,
                "before_hash": params.before_hash,
                "after_hash": params.after_hash,
                "write_type": params.write_type,
            }),
        })?;
        if let Some(session_entity_id) = self.maybe_session_entity_id(&session_id)? {
            self.insert_edge(
                file_entity_id,
                session_entity_id,
                EdgeType::ObservedIn,
                json!({"provenance": {"method": "record_evidence_file_write"}}),
            )?;
        }
        self.link_entity_to_project(
            file_entity_id,
            self.session_project(&session_id)?.as_deref(),
        )?;
        self.ingest_relation_hints(&params.relations)?;

        let payload = json!({
            "sequence": params.sequence,
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
            &match params.sequence {
                Some(sequence) => format!("{}:{}:{}", session_id, file_path, sequence),
                None => format!("{}:{}", session_id, file_path),
            },
            &params.session_id,
            &payload,
        )?;
        Ok(())
    }

    pub fn record_evidence_file_access(&self, params: FileAccessParams) -> Result<()> {
        let session_id = params.session_id.clone();
        let file_path = params.file_path.clone();
        let file_entity_id = self.ensure_relation_endpoint(&RelationEndpoint {
            kind: "File".to_string(),
            name: params.file_path.clone(),
            file_path: Some(params.file_path.clone()),
            data: json!({
                "path": params.file_path,
            }),
        })?;
        if let Some(session_entity_id) = self.maybe_session_entity_id(&session_id)? {
            self.insert_edge(
                session_entity_id,
                file_entity_id,
                EdgeType::Accessed,
                json!({
                    "sequence": params.sequence,
                    "access_type": params.access_type,
                    "tool_name": params.tool_name,
                    "source": params.source,
                    "provenance": {"method": "record_evidence_file_access"},
                }),
            )?;
            self.insert_edge(
                file_entity_id,
                session_entity_id,
                EdgeType::ObservedIn,
                json!({
                    "source": params.source,
                    "provenance": {"method": "record_evidence_file_access"}
                }),
            )?;
        }
        self.link_entity_to_project(
            file_entity_id,
            self.session_project(&session_id)?.as_deref(),
        )?;
        self.ingest_relation_hints(&params.relations)?;
        let payload = json!({
            "sequence": params.sequence,
            "file_path": params.file_path,
            "access_type": params.access_type,
            "tool_name": params.tool_name,
            "source": params.source,
        });
        self.append_event_log(
            "file_access",
            &format!("{}:{}:{}", session_id, file_path, params.sequence),
            &session_id,
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
            self.insert_edge(
                commit_entity_id,
                session_entity_id,
                EdgeType::ObservedIn,
                json!({"provenance": {"method": "record_evidence_commit"}}),
            )?;
        }
        self.link_entity_to_project(
            commit_entity_id,
            self.session_project(&session_id)?.as_deref(),
        )?;
        self.ingest_relation_hints(&params.relations)?;

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
            self.insert_edge(
                test_entity_id,
                session_entity_id,
                EdgeType::ObservedIn,
                json!({"provenance": {"method": "record_evidence_test_run"}}),
            )?;
        }
        if let Some(commit_sha) = params.commit_sha.as_deref() {
            if let Some(commit_entity_id) =
                self.find_entity_id_by_data(EntityType::Commit.as_str(), "commit_sha", commit_sha)?
            {
                self.insert_edge(
                    commit_entity_id,
                    test_entity_id,
                    EdgeType::TestedBy,
                    json!({"provenance": {"method": "record_evidence_test_run"}}),
                )?;
            }
        }
        self.link_entity_to_project(
            test_entity_id,
            self.session_project(&session_id)?.as_deref(),
        )?;
        self.ingest_relation_hints(&params.relations)?;

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
        let bug_commit_id =
            self.ensure_commit_sql_row(&session_id, &params.bug_commit_sha, "bug")?;
        let fix_commit_id =
            self.ensure_commit_sql_row(&session_id, &params.fix_commit_sha, "fix")?;

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

        let bug_entity_id = self.ensure_relation_endpoint(&RelationEndpoint {
            kind: EntityType::Commit.as_str().to_string(),
            name: format!(
                "{}:{}",
                params.bug_commit_sha.chars().take(8).collect::<String>(),
                "bug"
            ),
            file_path: None,
            data: json!({"commit_sha": params.bug_commit_sha}),
        })?;
        let fix_entity_id = self.ensure_relation_endpoint(&RelationEndpoint {
            kind: EntityType::Commit.as_str().to_string(),
            name: format!(
                "{}:{}",
                params.fix_commit_sha.chars().take(8).collect::<String>(),
                "fix"
            ),
            file_path: None,
            data: json!({"commit_sha": params.fix_commit_sha}),
        })?;
        self.insert_edge(
            bug_entity_id,
            fix_entity_id,
            EdgeType::FixedBy,
            json!({
                "fix_type": params.fix_type,
                "severity": params.severity,
                "cycles_to_fix": params.cycles_to_fix,
                "time_to_fix_ms": params.time_to_fix_ms,
            }),
        )?;
        if let Some(session_entity_id) = self.maybe_session_entity_id(&session_id)? {
            self.insert_edge(
                fix_entity_id,
                session_entity_id,
                EdgeType::ObservedIn,
                json!({"provenance": {"method": "record_evidence_fix_chain"}}),
            )?;
        }
        self.link_entity_to_project(fix_entity_id, self.session_project(&session_id)?.as_deref())?;
        self.ingest_relation_hints(&params.relations)?;

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

        if is_regression {
            let regression_id = self.ensure_relation_endpoint(&RelationEndpoint {
                kind: "Failure".to_string(),
                name: format!("bench:{}", bench_name),
                file_path: None,
                data: json!({
                    "bench_name": bench_name,
                    "mean_ns": mean_ns,
                    "median_ns": median_ns,
                    "p95_ns": p95_ns,
                }),
            })?;
            if let Some(session_entity_id) = self.maybe_session_entity_id(&session_id)? {
                self.insert_edge(
                    regression_id,
                    session_entity_id,
                    EdgeType::RegressedBy,
                    json!({"provenance": {"method": "record_evidence_bench_run"}}),
                )?;
                self.insert_edge(
                    regression_id,
                    session_entity_id,
                    EdgeType::ObservedIn,
                    json!({"provenance": {"method": "record_evidence_bench_run"}}),
                )?;
            }
            self.link_entity_to_project(
                regression_id,
                self.session_project(&session_id)?.as_deref(),
            )?;
        }

        let payload = json!({
            "bench_name": bench_name,
            "mean_ns": mean_ns,
            "is_regression": is_regression,
        });
        self.append_event_log("bench", &bench_run_id, &session_id, &payload)?;
        Ok(())
    }
}
