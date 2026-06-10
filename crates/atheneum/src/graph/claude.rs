use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::DateTime;
use serde_json::{json, Value};

use super::hashing::sha256_hex;
use super::{
    AtheneumGraph, ClaudeTranscriptImportParams, ClaudeTranscriptImportSummary, FileAccessParams,
    PromptParams, RecordEventParams, SessionParams, SessionProgressParams, ToolCallParams,
};

const MAX_LINE_BYTES: usize = 10 * 1024 * 1024;
const TRANSCRIPT_SOURCE: &str = "claude_transcript";

#[derive(Debug, Clone, Copy)]
struct TranscriptCursor {
    offset: u64,
    prompt_sequence: i64,
    tool_sequence: i64,
    file_access_sequence: i64,
    file_write_sequence: i64,
    inode: u64,
    mtime_ns: u64,
}

#[derive(Debug, Default)]
struct TranscriptSummary {
    model: Option<String>,
    git_branch: Option<String>,
    total_input_tokens: i64,
    total_output_tokens: i64,
    total_cache_read_tokens: i64,
    total_cache_create_tokens: i64,
    prompt_count: i64,
    tool_call_count: i64,
    file_access_count: i64,
    file_write_count: i64,
    compaction_count: i64,
    last_context_tokens: i64,
    prev_cache_read_tokens: i64,
}

#[derive(Debug, Default)]
struct DeltaImport {
    imported_prompts: i64,
    imported_tool_calls: i64,
    imported_file_accesses: i64,
    imported_file_writes: i64,
    offset: u64,
    prompt_sequence: i64,
    tool_sequence: i64,
    file_access_sequence: i64,
    file_write_sequence: i64,
}

impl AtheneumGraph {
    pub fn sync_claude_transcript(
        &self,
        params: ClaudeTranscriptImportParams,
    ) -> Result<ClaudeTranscriptImportSummary> {
        let transcript_path = params.transcript_path;
        let transcript_path = transcript_path.canonicalize().unwrap_or(transcript_path);
        let session_id = params
            .session_id
            .clone()
            .unwrap_or_else(|| transcript_stem(&transcript_path));
        let project = params
            .project
            .clone()
            .unwrap_or_else(|| infer_project_id(&transcript_path));
        let cursor_key = format!("claude:{}:{}", session_id, transcript_path.display());
        let identity = file_identity(&transcript_path)?;
        let metadata = fs::metadata(&transcript_path)
            .map_err(|e| anyhow::anyhow!("stat {} failed: {}", transcript_path.display(), e))?;
        let file_len = metadata.len();
        let cursor = self.load_transcript_cursor(&cursor_key)?;
        let mut reset_reason = None;
        if let Some(existing) = cursor {
            if file_len < existing.offset {
                reset_reason = Some(format!(
                    "shrank from {} to {} bytes",
                    existing.offset, file_len
                ));
            }
            if reset_reason.is_none()
                && existing.mtime_ns != 0
                && existing.mtime_ns != identity.1
                && file_len == existing.offset
            {
                reset_reason = Some("rewritten in place with identical length".to_string());
            }
            if existing.inode != 0 && existing.inode != identity.0 && existing.offset > 0 {
                reset_reason = Some(format!(
                    "inode changed from {} to {}",
                    existing.inode, identity.0
                ));
            }
        }
        let cursor = if let Some(reason) = reset_reason {
            self.reset_claude_transcript_import(&cursor_key, &session_id, &reason)?;
            None
        } else {
            cursor
        };

        let full_summary = scan_transcript_summary(&transcript_path)?;
        self.record_session(SessionParams {
            session_id: session_id.clone(),
            agent_name: params.agent_name.clone(),
            project: project.clone(),
            tool: params.tool.clone(),
            trigger: params.trigger.clone(),
            model: full_summary.model.clone(),
            git_branch: full_summary.git_branch.clone(),
            git_head: None,
            parent_session_id: None,
            relations: vec![],
        })?;

        let delta = import_transcript_delta(
            self,
            &transcript_path,
            &session_id,
            cursor.unwrap_or(TranscriptCursor {
                offset: 0,
                prompt_sequence: 0,
                tool_sequence: 0,
                file_access_sequence: 0,
                file_write_sequence: 0,
                inode: identity.0,
                mtime_ns: identity.1,
            }),
        )?;

        self.update_session_progress(SessionProgressParams {
            session_id: session_id.clone(),
            model: full_summary.model.clone(),
            git_branch: full_summary.git_branch.clone(),
            prompt_count: full_summary.prompt_count,
            tool_call_count: full_summary.tool_call_count,
            file_write_count: full_summary.file_write_count,
            total_input_tokens: full_summary.total_input_tokens,
            total_output_tokens: full_summary.total_output_tokens,
            total_cost_usd: 0.0,
        })?;

        self.store_transcript_cursor(
            &cursor_key,
            &session_id,
            &params.tool,
            &transcript_path,
            delta.offset,
            delta.prompt_sequence,
            delta.tool_sequence,
            delta.file_access_sequence,
            delta.file_write_sequence,
            identity,
        )?;

        if delta.imported_prompts > 0
            || delta.imported_tool_calls > 0
            || delta.imported_file_accesses > 0
            || delta.imported_file_writes > 0
        {
            self.record_event(RecordEventParams {
                event_type: "transcript_sync".to_string(),
                entity_id: cursor_key.clone(),
                session_id: session_id.clone(),
                payload: json!({
                    "tool": params.tool,
                    "source": TRANSCRIPT_SOURCE,
                    "transcript_path": transcript_path,
                    "project": project,
                    "imported_offset": delta.offset,
                    "imported_prompts": delta.imported_prompts,
                    "imported_tool_calls": delta.imported_tool_calls,
                    "imported_file_accesses": delta.imported_file_accesses,
                    "imported_file_writes": delta.imported_file_writes,
                    "total_input_tokens": full_summary.total_input_tokens,
                    "total_output_tokens": full_summary.total_output_tokens,
                    "total_cache_read_tokens": full_summary.total_cache_read_tokens,
                    "total_cache_create_tokens": full_summary.total_cache_create_tokens,
                    "compaction_count": full_summary.compaction_count,
                }),
                relations: vec![],
            })?;
        }

        Ok(ClaudeTranscriptImportSummary {
            session_id,
            project,
            model: full_summary.model,
            git_branch: full_summary.git_branch,
            total_input_tokens: full_summary.total_input_tokens,
            total_output_tokens: full_summary.total_output_tokens,
            total_cache_read_tokens: full_summary.total_cache_read_tokens,
            total_cache_create_tokens: full_summary.total_cache_create_tokens,
            prompt_count: full_summary.prompt_count,
            tool_call_count: full_summary.tool_call_count,
            file_access_count: full_summary.file_access_count,
            file_write_count: full_summary.file_write_count,
            compaction_count: full_summary.compaction_count,
            imported_prompts: delta.imported_prompts,
            imported_tool_calls: delta.imported_tool_calls,
            imported_file_accesses: delta.imported_file_accesses,
            imported_file_writes: delta.imported_file_writes,
            imported_offset: delta.offset,
        })
    }

    fn load_transcript_cursor(&self, source_key: &str) -> Result<Option<TranscriptCursor>> {
        let source_key = source_key.to_string();
        self.with_raw_connection(|conn| {
            Ok(conn
                .query_row(
                    "SELECT offset, prompt_sequence, tool_sequence, file_access_sequence,
                            file_write_sequence, COALESCE(file_inode, 0), COALESCE(file_mtime_ns, 0)
                     FROM transcript_imports WHERE source_key = ?1",
                    rusqlite::params![source_key],
                    |row| {
                        Ok(TranscriptCursor {
                            offset: row.get::<_, i64>(0)? as u64,
                            prompt_sequence: row.get(1)?,
                            tool_sequence: row.get(2)?,
                            file_access_sequence: row.get(3)?,
                            file_write_sequence: row.get(4)?,
                            inode: row.get::<_, i64>(5)? as u64,
                            mtime_ns: row.get::<_, i64>(6)? as u64,
                        })
                    },
                )
                .ok())
        })
    }

    fn reset_claude_transcript_import(
        &self,
        source_key: &str,
        session_id: &str,
        reason: &str,
    ) -> Result<()> {
        let source_key = source_key.to_string();
        let session_id_owned = session_id.to_string();
        let session_entity_id = self.maybe_session_entity_id(session_id)?;
        self.with_raw_connection(|conn| {
            conn.execute(
                "DELETE FROM transcript_imports WHERE source_key = ?1",
                rusqlite::params![source_key],
            )?;
            conn.execute(
                "DELETE FROM event_log
                 WHERE session_id = ?1
                   AND event_type IN ('prompt', 'tool_call', 'file_access', 'transcript_sync', 'transcript_reset')
                   AND json_extract(payload, '$.source') = ?2",
                rusqlite::params![session_id_owned, TRANSCRIPT_SOURCE],
            )?;
            conn.execute(
                "DELETE FROM tool_calls
                 WHERE session_id = ?1
                   AND json_extract(args, '$.source') = ?2",
                rusqlite::params![session_id, TRANSCRIPT_SOURCE],
            )?;
            conn.execute(
                "DELETE FROM reasoning_logs
                 WHERE session_id = ?1
                   AND json_extract(metadata, '$.source') = ?2",
                rusqlite::params![session_id, TRANSCRIPT_SOURCE],
            )?;
            Ok::<(), anyhow::Error>(())
        })?;
        self.delete_transcript_graph_entities(session_entity_id)?;
        self.record_event(RecordEventParams {
            event_type: "transcript_reset".to_string(),
            entity_id: source_key,
            session_id: session_id.to_string(),
            payload: json!({
                "source": TRANSCRIPT_SOURCE,
                "reason": reason,
            }),
            relations: vec![],
        })?;
        Ok(())
    }

    fn delete_transcript_graph_entities(&self, session_entity_id: Option<i64>) -> Result<()> {
        let prompt_ids = self.entity_ids_by_kind_and_source("ReasoningLog", TRANSCRIPT_SOURCE)?;
        let tool_ids = self.entity_ids_by_kind_and_source("ToolCall", TRANSCRIPT_SOURCE)?;
        self.delete_graph_entities(&prompt_ids)?;
        self.delete_graph_entities(&tool_ids)?;
        if let Some(session_entity_id) = session_entity_id {
            self.delete_transcript_edges_for_session(session_entity_id)?;
        }
        Ok(())
    }

    fn entity_ids_by_kind_and_source(&self, kind: &str, source: &str) -> Result<Vec<i64>> {
        let kind = kind.to_string();
        let source = source.to_string();
        self.with_raw_connection(|conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT id FROM graph_entities
                 WHERE kind = ?1 AND json_extract(data, '$.source') = ?2",
            )?;
            let rows = stmt.query_map(rusqlite::params![kind, source], |row| row.get(0))?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row?);
            }
            Ok(ids)
        })
    }

    fn delete_graph_entities(&self, entity_ids: &[i64]) -> Result<()> {
        if entity_ids.is_empty() {
            return Ok(());
        }
        // Gather kind/name for index invalidation before deletion.
        let mut to_remove = Vec::new();
        for &id in entity_ids {
            if let Ok(entity) = self.get_entity(id) {
                to_remove.push((entity.kind, entity.name));
            }
        }
        self.with_raw_connection(|conn| {
            let tx = conn.unchecked_transaction()?;
            for entity_id in entity_ids {
                tx.execute(
                    "DELETE FROM graph_edges WHERE from_id = ?1 OR to_id = ?1",
                    rusqlite::params![entity_id],
                )?;
                tx.execute(
                    "DELETE FROM graph_entities WHERE id = ?1",
                    rusqlite::params![entity_id],
                )?;
            }
            tx.commit()?;
            Ok::<(), anyhow::Error>(())
        })?;
        for (kind, name) in to_remove {
            self.runtime.remove_entity_id(&kind, &name);
        }
        Ok(())
    }

    fn delete_transcript_edges_for_session(&self, session_entity_id: i64) -> Result<()> {
        self.with_raw_connection(|conn| {
            conn.execute(
                "DELETE FROM graph_edges
                 WHERE edge_type IN ('accessed', 'observed_in')
                   AND json_extract(data, '$.source') = ?1
                   AND (from_id = ?2 OR to_id = ?2)",
                rusqlite::params![TRANSCRIPT_SOURCE, session_entity_id],
            )?;
            Ok::<(), anyhow::Error>(())
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn store_transcript_cursor(
        &self,
        source_key: &str,
        session_id: &str,
        tool: &str,
        transcript_path: &Path,
        offset: u64,
        prompt_sequence: i64,
        tool_sequence: i64,
        file_access_sequence: i64,
        file_write_sequence: i64,
        identity: (u64, u64),
    ) -> Result<()> {
        let source_key = source_key.to_string();
        let session_id = session_id.to_string();
        let tool = tool.to_string();
        let transcript_path = transcript_path.display().to_string();
        let imported_at = chrono::Utc::now().to_rfc3339();
        self.with_raw_connection(|conn| {
            conn.execute(
                "INSERT INTO transcript_imports
                    (source_key, session_id, tool, transcript_path, offset, prompt_sequence,
                     tool_sequence, file_access_sequence, file_write_sequence, file_inode,
                     file_mtime_ns, imported_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT(source_key) DO UPDATE SET
                    session_id = excluded.session_id,
                    tool = excluded.tool,
                    transcript_path = excluded.transcript_path,
                    offset = excluded.offset,
                    prompt_sequence = excluded.prompt_sequence,
                    tool_sequence = excluded.tool_sequence,
                    file_access_sequence = excluded.file_access_sequence,
                    file_write_sequence = excluded.file_write_sequence,
                    file_inode = excluded.file_inode,
                    file_mtime_ns = excluded.file_mtime_ns,
                    imported_at = excluded.imported_at",
                rusqlite::params![
                    source_key,
                    session_id,
                    tool,
                    transcript_path,
                    offset as i64,
                    prompt_sequence,
                    tool_sequence,
                    file_access_sequence,
                    file_write_sequence,
                    identity.0 as i64,
                    identity.1 as i64,
                    imported_at,
                ],
            )?;
            Ok::<(), anyhow::Error>(())
        })
    }
}

fn import_transcript_delta(
    graph: &AtheneumGraph,
    path: &Path,
    session_id: &str,
    cursor: TranscriptCursor,
) -> Result<DeltaImport> {
    let file =
        File::open(path).map_err(|e| anyhow::anyhow!("open {} failed: {}", path.display(), e))?;
    let mut reader = BufReader::new(file);
    if cursor.offset > 0 {
        reader
            .seek(SeekFrom::Start(cursor.offset))
            .map_err(|e| anyhow::anyhow!("seek {} failed: {}", path.display(), e))?;
    }

    let mut imported = DeltaImport {
        offset: cursor.offset,
        prompt_sequence: cursor.prompt_sequence,
        tool_sequence: cursor.tool_sequence,
        file_access_sequence: cursor.file_access_sequence,
        file_write_sequence: cursor.file_write_sequence,
        ..DeltaImport::default()
    };

    let mut line_buf = String::new();
    loop {
        line_buf.clear();
        match reader
            .by_ref()
            .take(MAX_LINE_BYTES as u64 + 1)
            .read_line(&mut line_buf)
        {
            Ok(0) => break,
            Ok(n) => {
                if line_buf.len() > MAX_LINE_BYTES && !line_buf.ends_with('\n') {
                    anyhow::bail!("transcript line exceeded {} bytes", MAX_LINE_BYTES);
                }
                let has_newline = line_buf.ends_with('\n');
                let line = line_buf.trim();
                if line.is_empty() {
                    if has_newline {
                        imported.offset += n as u64;
                    }
                    continue;
                }
                let value = match serde_json::from_str::<Value>(line) {
                    Ok(value) => value,
                    Err(err) => {
                        if has_newline {
                            imported.offset += n as u64;
                            continue;
                        }
                        return Err(anyhow::anyhow!(
                            "incomplete JSON line at {}: {}",
                            path.display(),
                            err
                        ));
                    }
                };
                imported.offset += n as u64;
                match value.get("type").and_then(Value::as_str) {
                    Some("user") => {
                        if is_synthetic_user_msg(&value) {
                            continue;
                        }
                        let content = value
                            .get("message")
                            .map(extract_message_text)
                            .unwrap_or_default();
                        if content.is_empty() {
                            continue;
                        }
                        imported.prompt_sequence += 1;
                        graph.record_evidence_prompt(PromptParams {
                            session_id: session_id.to_string(),
                            role: "user".to_string(),
                            sequence: imported.prompt_sequence,
                            content_summary: Some(content.clone()),
                            source: Some(TRANSCRIPT_SOURCE.to_string()),
                            input_hash: sha256_hex(&content),
                            input_tokens: None,
                            output_hash: None,
                            output_tokens: None,
                            latency_ms: None,
                            model: None,
                            cost_usd: None,
                            relations: vec![],
                        })?;
                        imported.imported_prompts += 1;
                    }
                    Some("assistant") => {
                        let message = value.get("message").cloned().unwrap_or(Value::Null);
                        let usage = message.get("usage");
                        let assistant_text = extract_message_text(&message);
                        if !assistant_text.is_empty() {
                            imported.prompt_sequence += 1;
                            graph.record_evidence_prompt(PromptParams {
                                session_id: session_id.to_string(),
                                role: "assistant".to_string(),
                                sequence: imported.prompt_sequence,
                                content_summary: Some(assistant_text.clone()),
                                source: Some(TRANSCRIPT_SOURCE.to_string()),
                                input_hash: sha256_hex(&assistant_text),
                                input_tokens: usage
                                    .and_then(|u| u.get("input_tokens"))
                                    .and_then(Value::as_i64),
                                output_hash: Some(sha256_hex(&assistant_text)),
                                output_tokens: usage
                                    .and_then(|u| u.get("output_tokens"))
                                    .and_then(Value::as_i64),
                                latency_ms: None,
                                model: message
                                    .get("model")
                                    .and_then(Value::as_str)
                                    .map(str::to_string),
                                cost_usd: None,
                                relations: vec![],
                            })?;
                            imported.imported_prompts += 1;
                        }
                        if let Some(content) = message.get("content").and_then(Value::as_array) {
                            for item in content {
                                if item.get("type").and_then(Value::as_str) != Some("tool_use") {
                                    continue;
                                }
                                let tool_name = item
                                    .get("name")
                                    .and_then(Value::as_str)
                                    .unwrap_or("?")
                                    .to_string();
                                let input = item.get("input").cloned().unwrap_or(Value::Null);
                                imported.tool_sequence += 1;
                                graph.record_evidence_tool_call(ToolCallParams {
                                    session_id: session_id.to_string(),
                                    tool_name: tool_name.clone(),
                                    sequence: Some(imported.tool_sequence),
                                    source: Some(TRANSCRIPT_SOURCE.to_string()),
                                    tool_version: None,
                                    input_hash: Some(sha256_hex(&serde_json::to_string(&input)?)),
                                    input_summary: Some(extract_tool_summary(item)),
                                    output_hash: None,
                                    output_summary: None,
                                    exit_status: "observed".to_string(),
                                    latency_ms: 0,
                                    input_tokens_est: None,
                                    tool_category: "claude_transcript".to_string(),
                                    relations: vec![],
                                })?;
                                imported.imported_tool_calls += 1;

                                if let Some(file_path) =
                                    input.get("file_path").and_then(Value::as_str).filter(|_| {
                                        matches!(tool_name.as_str(), "Read" | "Edit" | "Write")
                                    })
                                {
                                    imported.file_access_sequence += 1;
                                    graph.record_evidence_file_access(FileAccessParams {
                                        session_id: session_id.to_string(),
                                        file_path: file_path.to_string(),
                                        sequence: imported.file_access_sequence,
                                        access_type: tool_name.to_lowercase(),
                                        tool_name: Some(tool_name.clone()),
                                        source: Some(TRANSCRIPT_SOURCE.to_string()),
                                        relations: vec![],
                                    })?;
                                    imported.imported_file_accesses += 1;
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            Err(err) => return Err(anyhow::anyhow!("read {} failed: {}", path.display(), err)),
        }
    }

    Ok(imported)
}

fn scan_transcript_summary(path: &Path) -> Result<TranscriptSummary> {
    let file =
        File::open(path).map_err(|e| anyhow::anyhow!("open {} failed: {}", path.display(), e))?;
    let mut reader = BufReader::new(file);
    let mut summary = TranscriptSummary::default();
    let mut line_buf = String::new();

    loop {
        line_buf.clear();
        match reader
            .by_ref()
            .take(MAX_LINE_BYTES as u64 + 1)
            .read_line(&mut line_buf)
        {
            Ok(0) => break,
            Ok(_) => {
                let line = line_buf.trim();
                if line.is_empty() {
                    continue;
                }
                let value = match serde_json::from_str::<Value>(line) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                match value.get("type").and_then(Value::as_str) {
                    Some("user") => {
                        if is_synthetic_user_msg(&value) {
                            continue;
                        }
                        let content = value
                            .get("message")
                            .map(extract_message_text)
                            .unwrap_or_default();
                        if !content.is_empty() {
                            summary.prompt_count += 1;
                        }
                        if let Some(branch) = value.get("gitBranch").and_then(Value::as_str) {
                            summary.git_branch = Some(branch.to_string());
                        }
                    }
                    Some("assistant") => {
                        if let Some(message) = value.get("message") {
                            if let Some(model) = message.get("model").and_then(Value::as_str) {
                                summary.model = Some(model.to_string());
                            }
                            if !extract_message_text(message).is_empty() {
                                summary.prompt_count += 1;
                            }
                            if let Some(usage) = message.get("usage") {
                                let input = usage
                                    .get("input_tokens")
                                    .and_then(Value::as_i64)
                                    .unwrap_or(0);
                                let output = usage
                                    .get("output_tokens")
                                    .and_then(Value::as_i64)
                                    .unwrap_or(0);
                                let cache_read = usage
                                    .get("cache_read_input_tokens")
                                    .and_then(Value::as_i64)
                                    .unwrap_or(0);
                                let cache_create = usage
                                    .get("cache_creation_input_tokens")
                                    .and_then(Value::as_i64)
                                    .unwrap_or(0);
                                summary.total_input_tokens += input;
                                summary.total_output_tokens += output;
                                summary.total_cache_read_tokens += cache_read;
                                summary.total_cache_create_tokens += cache_create;

                                let current_context = if cache_read == 0 && cache_create > 0 {
                                    input + cache_create
                                } else {
                                    input + cache_read
                                };
                                if summary.last_context_tokens > 0
                                    && current_context < summary.last_context_tokens * 7 / 10
                                    && summary.prev_cache_read_tokens > 1000
                                    && cache_read < summary.prev_cache_read_tokens / 5
                                {
                                    summary.compaction_count += 1;
                                }
                                summary.last_context_tokens = current_context;
                                summary.prev_cache_read_tokens = cache_read;
                            }

                            if let Some(content) = message.get("content").and_then(Value::as_array)
                            {
                                for item in content {
                                    if item.get("type").and_then(Value::as_str) != Some("tool_use")
                                    {
                                        continue;
                                    }
                                    summary.tool_call_count += 1;
                                    let tool_name =
                                        item.get("name").and_then(Value::as_str).unwrap_or("");
                                    if matches!(tool_name, "Read" | "Edit" | "Write")
                                        && item
                                            .get("input")
                                            .and_then(|input| input.get("file_path"))
                                            .and_then(Value::as_str)
                                            .is_some()
                                    {
                                        summary.file_access_count += 1;
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            Err(err) => return Err(anyhow::anyhow!("read {} failed: {}", path.display(), err)),
        }
    }

    Ok(summary)
}

fn is_synthetic_user_msg(entry: &Value) -> bool {
    if entry
        .get("isMeta")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return true;
    }
    let Some(message) = entry.get("message") else {
        return false;
    };
    match message.get("content") {
        Some(Value::Array(arr)) => {
            !arr.is_empty()
                && arr
                    .iter()
                    .all(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))
        }
        Some(Value::String(s)) => {
            let trimmed = s.trim_start();
            trimmed.starts_with("<local-command-stdout>")
                || trimmed.starts_with("<local-command-stderr>")
                || trimmed.starts_with("<local-command-caveat>")
                || trimmed.starts_with("<command-name>")
                || trimmed.starts_with("<bash-input>")
                || trimmed.starts_with("<bash-stdout>")
                || trimmed.starts_with("<bash-stderr>")
        }
        _ => false,
    }
}

fn extract_message_text(message: &Value) -> String {
    let raw = match message.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|block| {
                if block.get("type").and_then(Value::as_str) == Some("text") {
                    block
                        .get("text")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    };
    normalize_text(&raw, 500)
}

fn extract_tool_summary(tool_use: &Value) -> String {
    if let Some(input) = tool_use.get("input") {
        if let Some(file_path) = input.get("file_path").and_then(Value::as_str) {
            return shorten_path(file_path);
        }
        if let Some(command) = input.get("command").and_then(Value::as_str) {
            let first_line = command.lines().next().unwrap_or(command);
            return normalize_text(first_line, 120);
        }
        if let Some(pattern) = input.get("pattern").and_then(Value::as_str) {
            return normalize_text(pattern, 120);
        }
    }
    String::new()
}

fn normalize_text(raw: &str, max: usize) -> String {
    let cleaned = raw
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with("```"))
        .collect::<Vec<_>>()
        .join(" ");
    let mut text = cleaned;
    while let Some(start) = text.find("[Image") {
        if let Some(end) = text[start..].find(']') {
            text = format!("{}{}", &text[..start], text[start + end + 1..].trim_start());
        } else {
            break;
        }
    }
    truncate(&text, max)
}

fn truncate(value: &str, max: usize) -> String {
    let mut out = String::new();
    for ch in value.chars().take(max) {
        out.push(ch);
    }
    out.trim().to_string()
}

fn shorten_path(path: &str) -> String {
    let mut parts = path.rsplit('/');
    match (parts.next(), parts.next()) {
        (Some(last), Some(parent)) => format!("{}/{}", parent, last),
        _ => path.to_string(),
    }
}

fn transcript_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("unknown-session")
        .to_string()
}

fn infer_project_id(path: &Path) -> String {
    path.parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .map(decode_project_dir_name)
        .unwrap_or_else(|| "claude".to_string())
}

fn decode_project_dir_name(encoded: &str) -> String {
    if !encoded.starts_with('-') {
        return encoded.to_string();
    }
    let decoded = format!("/{}", encoded.trim_start_matches('-').replace('-', "/"));
    PathBuf::from(decoded)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(encoded)
        .to_string()
}

#[cfg(unix)]
fn file_identity(path: &Path) -> Result<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;

    let metadata =
        fs::metadata(path).map_err(|e| anyhow::anyhow!("stat {} failed: {}", path.display(), e))?;
    let mtime_ns = DateTime::from_timestamp(metadata.mtime(), metadata.mtime_nsec() as u32)
        .map(|dt| dt.timestamp_nanos_opt().unwrap_or_default() as u64)
        .unwrap_or_default();
    Ok((metadata.ino(), mtime_ns))
}

#[cfg(not(unix))]
fn file_identity(path: &Path) -> Result<(u64, u64)> {
    let metadata =
        fs::metadata(path).map_err(|e| anyhow::anyhow!("stat {} failed: {}", path.display(), e))?;
    let mtime_ns = metadata
        .modified()
        .ok()
        .and_then(|mtime| mtime.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or_default();
    Ok((metadata.len(), mtime_ns))
}
