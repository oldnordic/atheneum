use anyhow::Result;
use chrono::Utc;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::super::json_to_string;
use super::super::{AtheneumGraph, RecordEventParams, SessionSummary};

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
    pub fn record_event(&self, params: RecordEventParams) -> Result<()> {
        let payload_relations = self.relation_hints_from_payload(&params.payload)?;
        self.ingest_relation_hints(&params.relations)?;
        self.ingest_relation_hints(&payload_relations)?;
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

    pub(super) fn append_event_log(
        &self,
        event_type: &str,
        entity_id: &str,
        session_id: &str,
        payload: &Value,
    ) -> Result<()> {
        let payload_str = json_to_string(payload)?;
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
