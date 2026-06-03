use anyhow::Result;
use chrono::Utc;
use serde_json::{json, Value};
use sqlitegraph::GraphEntity;

use super::super::{
    AtheneumGraph, EdgeType, EndSessionParams, EntityType, SessionParams, SessionProgressParams,
};

impl AtheneumGraph {
    pub(super) fn session_agent_sql_id(&self, session_id: &str) -> Result<Option<i64>> {
        self.with_raw_connection(|conn| {
            Ok(conn
                .query_row(
                    "SELECT agent_id FROM sessions WHERE session_id = ?1",
                    rusqlite::params![session_id],
                    |row| row.get(0),
                )
                .ok())
        })
    }

    pub(super) fn session_project(&self, session_id: &str) -> Result<Option<String>> {
        Ok(self
            .maybe_session_entity_id(session_id)?
            .and_then(|id| self.get_entity(id).ok())
            .and_then(|entity| {
                entity
                    .data
                    .get("project")
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
            }))
    }

    fn update_session_entity_progress(&self, params: &SessionProgressParams) -> Result<()> {
        let Some(session_entity_id) = self.maybe_session_entity_id(&params.session_id)? else {
            return Ok(());
        };
        let mut entity = self.get_entity(session_entity_id)?;
        if let Some(obj) = entity.data.as_object_mut() {
            if let Some(model) = params.model.as_deref() {
                obj.insert("model".to_string(), Value::String(model.to_string()));
            }
            if let Some(git_branch) = params.git_branch.as_deref() {
                obj.insert(
                    "git_branch".to_string(),
                    Value::String(git_branch.to_string()),
                );
            }
            obj.insert(
                "prompt_count".to_string(),
                Value::Number(params.prompt_count.into()),
            );
            obj.insert(
                "tool_call_count".to_string(),
                Value::Number(params.tool_call_count.into()),
            );
            obj.insert(
                "file_write_count".to_string(),
                Value::Number(params.file_write_count.into()),
            );
            obj.insert(
                "total_input_tokens".to_string(),
                Value::Number(params.total_input_tokens.into()),
            );
            obj.insert(
                "total_output_tokens".to_string(),
                Value::Number(params.total_output_tokens.into()),
            );
            obj.insert("total_cost_usd".to_string(), json!(params.total_cost_usd));
        }
        self.update_entity_data(session_entity_id, &entity.data)
    }

    pub fn update_session_progress(&self, params: SessionProgressParams) -> Result<()> {
        self.with_raw_connection(|conn| {
            conn.execute(
                "UPDATE sessions
                 SET model = COALESCE(?1, model),
                     git_branch = COALESCE(?2, git_branch),
                     prompt_count = ?3,
                     tool_call_count = ?4,
                     file_write_count = ?5,
                     total_input_tokens = ?6,
                     total_output_tokens = ?7,
                     total_cost_usd = ?8
                 WHERE session_id = ?9",
                rusqlite::params![
                    params.model.as_deref(),
                    params.git_branch.as_deref(),
                    params.prompt_count,
                    params.tool_call_count,
                    params.file_write_count,
                    params.total_input_tokens,
                    params.total_output_tokens,
                    params.total_cost_usd,
                    params.session_id.as_str()
                ],
            )?;
            Ok::<(), anyhow::Error>(())
        })?;
        self.update_session_entity_progress(&params)
    }

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
        self.link_entity_to_project(entity_id, Some(&params.project))?;

        if let Some(parent_session_id) = params.parent_session_id.as_deref() {
            if let Some(parent_entity_id) = self.maybe_session_entity_id(parent_session_id)? {
                self.insert_edge(
                    entity_id,
                    parent_entity_id,
                    EdgeType::DependsOn,
                    json!({"provenance": {"method": "record_session"}}),
                )?;
            }
        }

        self.ingest_relation_hints(&params.relations)?;

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
}
