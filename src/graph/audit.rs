use anyhow::Result;
use chrono::Utc;
use serde_json::{json, Value};
use sqlitegraph::GraphEntity;

use super::{
    ActionRecord, ActionTrace, AtheneumGraph, EdgeType, EntityType, ToolCallRecord, ToolCallTrace,
};

impl AtheneumGraph {
    pub fn insert_reasoning_log(
        &self,
        agent: &str,
        content: &str,
        project_id: Option<&str>,
    ) -> Result<i64> {
        let agent_entity_id = self.ensure_agent(agent)?;

        let agent_name = agent.to_string();
        let content_str = content.to_string();
        let project = project_id.map(|s| s.to_string());
        let timestamp = Utc::now().to_rfc3339();
        let sql_id = self.with_raw_connection(|conn| {
            let agent_sql_id: i64 = conn.query_row(
                "SELECT id FROM agents WHERE name = ?1",
                rusqlite::params![agent_name],
                |row| row.get(0),
            )?;
            conn.execute(
                "INSERT INTO reasoning_logs
                    (agent_id, content, project_id, metadata, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![agent_sql_id, content_str, project, "{}", timestamp],
            )?;
            Ok(conn.last_insert_rowid())
        })?;

        let mut data = json!({
            "sql_id": sql_id,
            "agent": agent,
            "content": content,
            "timestamp": Utc::now().to_rfc3339(),
        });
        if let (Some(pid), Some(obj)) = (project_id, data.as_object_mut()) {
            obj.insert("project_id".to_string(), Value::String(pid.to_string()));
        }

        let entity = GraphEntity {
            id: 0,
            kind: "ReasoningLog".to_string(),
            name: format!(
                "{}: {}",
                agent,
                content.chars().take(48).collect::<String>()
            ),
            file_path: None,
            data,
        };
        let log_entity_id = self
            .inner
            .insert_entity(&entity)
            .map_err(|e| anyhow::anyhow!("Failed to insert ReasoningLog: {}", e))?;

        self.insert_edge(
            agent_entity_id,
            log_entity_id,
            EdgeType::PerformedBy,
            json!({"provenance": {"actor": "atheneum", "method": "insert_reasoning_log"}}),
        )?;

        Ok(log_entity_id)
    }

    pub fn insert_tool_call(
        &self,
        reasoning_log_id: i64,
        tool_name: &str,
        args: Value,
        project_id: Option<&str>,
    ) -> Result<i64> {
        let log_entity = self.get_entity(reasoning_log_id)?;
        let log_sql_id = log_entity
            .data
            .get("sql_id")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "ReasoningLog graph_entity {} has no sql_id pointer; migration may not have run",
                    reasoning_log_id
                )
            })?;

        let tool_name_owned = tool_name.to_string();
        let args_str = serde_json::to_string(&args).unwrap_or_else(|_| "null".into());
        let project = project_id.map(|s| s.to_string());
        let timestamp = Utc::now().to_rfc3339();
        let sql_id = self.with_raw_connection(|conn| {
            conn.execute(
                "INSERT INTO tool_calls
                    (reasoning_log_id, tool_name, args, project_id, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![log_sql_id, tool_name_owned, args_str, project, timestamp],
            )?;
            Ok(conn.last_insert_rowid())
        })?;

        let mut data = json!({
            "sql_id": sql_id,
            "tool_name": tool_name,
            "args": args,
            "timestamp": Utc::now().to_rfc3339(),
        });
        if let (Some(pid), Some(obj)) = (project_id, data.as_object_mut()) {
            obj.insert("project_id".to_string(), Value::String(pid.to_string()));
        }

        let entity = GraphEntity {
            id: 0,
            kind: "ToolCall".to_string(),
            name: tool_name.to_string(),
            file_path: None,
            data,
        };
        let tool_entity_id = self
            .inner
            .insert_entity(&entity)
            .map_err(|e| anyhow::anyhow!("Failed to insert ToolCall: {}", e))?;

        self.insert_edge(
            reasoning_log_id,
            tool_entity_id,
            EdgeType::Called,
            json!({"provenance": {"actor": "atheneum", "method": "insert_tool_call"}}),
        )?;

        Ok(tool_entity_id)
    }

    pub fn record_tool_modifies(&self, tool_call_id: i64, target_id: i64) -> Result<i64> {
        self.insert_edge(
            tool_call_id,
            target_id,
            EdgeType::Modified,
            json!({"provenance": {"actor": "atheneum", "method": "record_tool_modifies"}}),
        )
    }

    pub fn record_agent_action(
        &self,
        agent: &str,
        thought: &str,
        tool_calls: Vec<ToolCallRecord>,
        project_id: Option<&str>,
    ) -> Result<ActionTrace> {
        let log_id = self.insert_reasoning_log(agent, thought, project_id)?;
        let agent_id = self.ensure_agent(agent)?;

        let mut tool_call_ids = Vec::with_capacity(tool_calls.len());
        let mut modified_edge_ids = Vec::new();
        for tc in tool_calls {
            let tool_id = self.insert_tool_call(log_id, &tc.tool_name, tc.args, project_id)?;
            tool_call_ids.push(tool_id);
            for target in tc.modified_targets {
                let edge_id = self.record_tool_modifies(tool_id, target)?;
                modified_edge_ids.push(edge_id);
            }
        }

        Ok(ActionTrace {
            agent_id,
            reasoning_log_id: log_id,
            tool_call_ids,
            modified_edge_ids,
        })
    }

    pub fn get_action_trace(
        &self,
        agent: &str,
        project_id: Option<&str>,
    ) -> Result<Vec<ActionRecord>> {
        let logs: Vec<GraphEntity> = self
            .entities_by_kind("ReasoningLog")?
            .into_iter()
            .filter(|log| log.data.get("agent").and_then(|v| v.as_str()) == Some(agent))
            .filter(|log| match project_id {
                None => true,
                Some(pid) => log.data.get("project_id").and_then(|v| v.as_str()) == Some(pid),
            })
            .collect();

        let mut records = Vec::with_capacity(logs.len());
        for log in logs {
            let log_id = log.id;
            let tool_call_entities: Vec<GraphEntity> = self
                .outgoing_edges(log_id)?
                .into_iter()
                .filter(|e| e.edge_type == EdgeType::Called.as_str())
                .filter_map(|e| self.get_entity(e.to_id).ok())
                .collect();

            let mut tool_calls = Vec::with_capacity(tool_call_entities.len());
            for tc in tool_call_entities {
                let modified: Vec<GraphEntity> = self
                    .outgoing_edges(tc.id)?
                    .into_iter()
                    .filter(|e| e.edge_type == EdgeType::Modified.as_str())
                    .filter_map(|e| self.get_entity(e.to_id).ok())
                    .collect();
                tool_calls.push(ToolCallTrace {
                    tool_call: tc,
                    modified,
                });
            }

            records.push(ActionRecord {
                reasoning_log: log,
                tool_calls,
            });
        }
        Ok(records)
    }

    pub(super) fn ensure_agent(&self, name: &str) -> Result<i64> {
        if let Some(id) = self.find_entity_id_by_kind_and_name(EntityType::Agent.as_str(), name)? {
            return Ok(id);
        }
        self.insert_agent(name, json!({}))
    }
}
