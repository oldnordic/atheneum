//! Session-digest composer.
//!
//! Produces a bounded, ranked bootstrap packet so a new session can ground on
//! what prior sessions in the same project actually did — decisions made,
//! files touched, open tasks — without re-discovering from scratch.
//!
//! The digest is extractive (no model call): it composes real rows from
//! `sessions`, `event_log`, `graph_entities` (ReasoningLog / Memory) and
//! `discoveries` into a compact plain-text or JSON packet, ranked by recency
//! and truncated to a token budget.
//!
//! Activity counts are computed from `event_log` rather than trusted from the
//! `sessions` columns, because the session ledger records session start but
//! does not always backfill `tool_call_count` / `file_write_count` at session
//! end. Computing from events is accurate regardless of how the session was
//! recorded.

use anyhow::Result;
use rusqlite::OptionalExtension;
use serde_json::Value;

use super::{AtheneumGraph, EntityType};

/// `graph_entities.kind` for reasoning-log nodes. Not in the `EntityType` enum
/// (that enum covers the first-class ontology kinds), so it is referenced by
/// literal here.
const REASONING_LOG_KIND: &str = "ReasoningLog";

/// One file-access entry for a session, ranked by access frequency.
#[derive(Debug, serde::Serialize)]
struct FileAccess {
    path: String,
    count: i64,
}

/// One reasoning-log decision attributed to a session (via `observed_in`).
#[derive(Debug, serde::Serialize)]
struct DigestReasoning {
    entity_id: i64,
    summary: String,
}

/// One discovery attributed to a session (via `discoveries.session_id`).
#[derive(Debug, serde::Serialize)]
struct DigestDiscovery {
    target: String,
    discovery_type: String,
    summary: Option<String>,
}

/// Per-session block in the digest.
#[derive(Debug, serde::Serialize)]
struct DigestSession {
    session_id: String,
    short_id: String,
    started_at: String,
    project: String,
    git_branch: Option<String>,
    tool: String,
    parent_session_id: Option<String>,
    tool_calls: i64,
    file_writes: i64,
    commits: i64,
    top_files: Vec<FileAccess>,
    last_tool: Option<String>,
    last_tool_summary: Option<String>,
    reasoning: Vec<DigestReasoning>,
    discoveries: Vec<DigestDiscovery>,
}

/// One project-scoped memory entry.
#[derive(Debug, serde::Serialize)]
struct DigestMemory {
    key: String,
    content: String,
    confidence: f64,
}

/// One open task for the project.
#[derive(Debug, serde::Serialize)]
struct DigestTask {
    title: String,
    status: String,
}

/// Full structured digest, serialized for `--json` and rendered to text.
#[derive(Debug, serde::Serialize)]
pub struct DigestReport {
    project: Option<String>,
    fell_back_to_all_projects: bool,
    sessions: Vec<DigestSession>,
    project_memory: Vec<DigestMemory>,
    open_tasks: Vec<DigestTask>,
    thread_anchors: Vec<i64>,
}

impl AtheneumGraph {
    /// Compose a bounded plain-text digest for bootstrap grounding.
    ///
    /// `project` filters sessions; if the filter yields nothing the digest
    /// falls back to the most recent sessions across all projects and flags
    /// it. `last_n` bounds the session count; `tokens` bounds output size
    /// (estimated at 4 chars/token).
    pub fn compose_digest(
        &self,
        project: Option<&str>,
        last_n: i64,
        tokens: usize,
    ) -> Result<String> {
        let report = self.collect_digest(project, last_n)?;
        Ok(render_digest_text(&report, tokens))
    }

    /// Compose the structured digest (unbounded) for `--json` output.
    pub fn compose_digest_value(&self, project: Option<&str>, last_n: i64) -> Result<DigestReport> {
        self.collect_digest(project, last_n)
    }

    /// Compose the digest as a JSON value (for `--json` CLI output).
    pub fn compose_digest_json(&self, project: Option<&str>, last_n: i64) -> Result<Value> {
        let report = self.collect_digest(project, last_n)?;
        digest_report_to_json(&report)
    }

    fn collect_digest(&self, project: Option<&str>, last_n: i64) -> Result<DigestReport> {
        let project_owned = project.map(|s| s.to_string());

        // Try the project filter first; fall back to all projects if empty so
        // the digest is still useful when project tagging is sparse (many
        // sessions are tagged "tmp").
        let mut sessions = self.query_sessions(project, last_n, None)?;
        let fell_back = if sessions.is_empty() && project.is_some() {
            sessions = self.query_sessions(None, last_n, None)?;
            !sessions.is_empty()
        } else {
            false
        };

        let mut digest_sessions = Vec::with_capacity(sessions.len());
        for s in &sessions {
            digest_sessions.push(self.collect_session_detail(s)?);
        }

        let project_memory = self.collect_project_memory(project_owned.as_deref(), 5)?;
        let open_tasks = self.collect_open_tasks(project_owned.as_deref())?;
        let thread_anchors = self.collect_thread_anchors(3)?;

        Ok(DigestReport {
            project: project_owned,
            fell_back_to_all_projects: fell_back,
            sessions: digest_sessions,
            project_memory,
            open_tasks,
            thread_anchors,
        })
    }

    /// Gather per-session detail in one connection round-trip.
    fn collect_session_detail(&self, s: &super::SessionSummary) -> Result<DigestSession> {
        let session_id = s.session_id.clone();
        let tool_call_count = s.tool_call_count;

        let detail = self.with_raw_connection(|conn| {
            // Activity from event_log — accurate even when the sessions
            // ledger columns are zero.
            let computed_tool_calls: i64 = if tool_call_count > 0 {
                tool_call_count
            } else {
                conn.query_row(
                    "SELECT COUNT(*) FROM event_log
                     WHERE session_id = ?1 AND event_type = 'tool_call'",
                    rusqlite::params![session_id],
                    |row| row.get(0),
                )
                .optional()?
                .unwrap_or(0)
            };

            let file_writes: i64 = if s.file_write_count > 0 {
                s.file_write_count
            } else {
                // Two ingest paths record writes: `record_evidence_file_write`
                // emits a `file_write` event, while the transcript-sync path
                // emits `file_access` events with `access_type = "write"`.
                // Count both so the activity is accurate regardless of which
                // recorder produced the session's events.
                conn.query_row(
                    "SELECT COUNT(*) FROM event_log
                     WHERE session_id = ?1
                       AND (event_type = 'file_write'
                            OR (event_type = 'file_access'
                                AND json_extract(payload, '$.access_type') = 'write'))",
                    rusqlite::params![session_id],
                    |row| row.get(0),
                )
                .optional()?
                .unwrap_or(0)
            };

            // Top files accessed (distinct paths, by frequency). Both
            // `file_access` and `file_write` events carry the path under
            // `payload.file_path`. We pull more rows than needed and merge by
            // basename in Rust, because distinct files in different
            // directories can share a basename (e.g. several projects'
            // `SKILL.md`) — grouping by full path would list `SKILL.md`
            // three times.
            let mut raw_files = Vec::new();
            let mut stmt = conn.prepare(
                "SELECT json_extract(payload, '$.file_path') AS p, COUNT(*) AS c
                 FROM event_log
                 WHERE session_id = ?1
                   AND event_type IN ('file_access', 'file_write')
                   AND json_extract(payload, '$.file_path') IS NOT NULL
                 GROUP BY p ORDER BY c DESC LIMIT 20",
            )?;
            let rows = stmt.query_map(rusqlite::params![session_id], |row| {
                Ok(FileAccess {
                    path: row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    count: row.get(1)?,
                })
            })?;
            for row in rows {
                raw_files.push(row?);
            }
            drop(stmt);
            let top_files = merge_by_basename(raw_files, 5);

            // ReasoningLog decisions linked to this session via `observed_in`.
            // Session graph entities are named `<tool>:<session_id>` (e.g.
            // `claude-code:07ff7531-...`), so matching `name = session_id`
            // never hits. Join on the Session entity's `data.session_id`
            // instead, which also tolerates multiple Session entities for the
            // same session id.
            let mut reasoning = Vec::new();
            // ReasoningLog entities use either `content_summary` (transcript
            // sync path) or `content` (the `insert_reasoning_log` audit path)
            // for their text — coalesce both so decisions surface regardless
            // of which recorder produced them.
            let mut stmt = conn.prepare(
                "SELECT rl.id,
                        COALESCE(json_extract(rl.data, '$.content_summary'),
                                 json_extract(rl.data, '$.content'))
                 FROM graph_entities rl
                 JOIN graph_edges e
                   ON e.from_id = rl.id AND e.edge_type = 'observed_in'
                 JOIN graph_entities s
                   ON e.to_id = s.id AND s.kind = ?1
                 WHERE rl.kind = ?2
                   AND json_extract(s.data, '$.session_id') = ?3
                 ORDER BY rl.id DESC LIMIT 2",
            )?;
            let rows = stmt.query_map(
                rusqlite::params![EntityType::Session.as_str(), REASONING_LOG_KIND, session_id],
                |row| {
                    Ok(DigestReasoning {
                        entity_id: row.get(0)?,
                        summary: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    })
                },
            )?;
            for row in rows {
                reasoning.push(row?);
            }

            // Discoveries attributed to this session via the session_id column.
            let mut discoveries = Vec::new();
            let mut stmt = conn.prepare(
                "SELECT target, discovery_type, json_extract(metadata, '$.summary')
                 FROM discoveries
                 WHERE session_id = ?1
                 ORDER BY id DESC LIMIT 3",
            )?;
            let rows = stmt.query_map(rusqlite::params![session_id], |row| {
                Ok(DigestDiscovery {
                    target: row.get(0)?,
                    discovery_type: row.get(1)?,
                    summary: row.get(2)?,
                })
            })?;
            for row in rows {
                discoveries.push(row?);
            }

            Ok::<SessionDetail, anyhow::Error>(SessionDetail {
                computed_tool_calls,
                file_writes,
                top_files,
                reasoning,
                discoveries,
            })
        })?;

        Ok(DigestSession {
            session_id: s.session_id.clone(),
            short_id: short_id(&s.session_id),
            started_at: s.started_at.clone(),
            project: s.project.clone(),
            git_branch: s.git_branch.clone(),
            tool: if s.tool.is_empty() {
                s.trigger.clone()
            } else {
                s.tool.clone()
            },
            parent_session_id: s.parent_session_id.clone(),
            tool_calls: detail.computed_tool_calls,
            file_writes: detail.file_writes,
            commits: s.commit_count,
            top_files: detail.top_files,
            last_tool: s.last_tool.clone(),
            last_tool_summary: s.last_tool_summary.clone(),
            reasoning: detail.reasoning,
            discoveries: detail.discoveries,
        })
    }

    /// Project-scoped durable memory, highest confidence first.
    fn collect_project_memory(
        &self,
        project: Option<&str>,
        limit: usize,
    ) -> Result<Vec<DigestMemory>> {
        let entities = self.list_memory_page(None, project, 0, limit)?;
        let mut out = Vec::new();
        for e in entities {
            let key = e
                .data
                .get("key")
                .and_then(|v| v.as_str())
                .unwrap_or(&e.name)
                .to_string();
            let content = e
                .data
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let confidence = e
                .data
                .get("confidence")
                .and_then(|v| v.as_f64())
                .unwrap_or(1.0);
            out.push(DigestMemory {
                key,
                content,
                confidence,
            });
        }
        // Highest confidence first, then leave insertion order as tiebreaker.
        out.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(out)
    }

    /// Non-archived tasks for the project, limited to open statuses.
    fn collect_open_tasks(&self, project: Option<&str>) -> Result<Vec<DigestTask>> {
        let entities = self.list_tasks(project)?;
        let open = ["TODO", "IN_PROGRESS", "BLOCKED"];
        let mut out = Vec::new();
        for e in entities {
            let status = e
                .data
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("TODO")
                .to_string();
            if !open.contains(&status.as_str()) {
                continue;
            }
            let title = e.name.clone();
            out.push(DigestTask { title, status });
        }
        Ok(out)
    }

    /// Most recent ReasoningLog entity ids — anchors for `atheneum navigate`
    /// follow-up to walk a decision thread.
    fn collect_thread_anchors(&self, limit: i64) -> Result<Vec<i64>> {
        self.with_raw_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id FROM graph_entities
                 WHERE kind = ?1
                 ORDER BY id DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(rusqlite::params![REASONING_LOG_KIND, limit], |row| {
                row.get::<_, i64>(0)
            })?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }
}

struct SessionDetail {
    computed_tool_calls: i64,
    file_writes: i64,
    top_files: Vec<FileAccess>,
    reasoning: Vec<DigestReasoning>,
    discoveries: Vec<DigestDiscovery>,
}

fn short_id(session_id: &str) -> String {
    session_id
        .split('-')
        .next()
        .unwrap_or(session_id)
        .to_string()
}

/// Merge file-access rows by basename (last path segment), summing counts.
/// Distinct files in different directories often share a basename (several
/// projects ship a `SKILL.md`); without merging, the top-N list would show
/// the same basename repeatedly. Input is pre-sorted by count desc; output
/// preserves that order after merging, truncated to `limit`.
fn merge_by_basename(rows: Vec<FileAccess>, limit: usize) -> Vec<FileAccess> {
    use std::collections::BTreeMap;
    let mut merged: BTreeMap<String, i64> = BTreeMap::new();
    // BTreeMap iteration order is by basename, not by count — preserve the
    // original count-desc order via a separate ordering pass.
    let mut order: Vec<String> = Vec::new();
    for f in rows {
        let base = f
            .path
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or(&f.path)
            .to_string();
        let entry = merged.entry(base.clone()).or_insert(0);
        if *entry == 0 {
            order.push(base);
        }
        *entry += f.count;
    }
    let mut out: Vec<FileAccess> = order
        .into_iter()
        .map(|name| {
            let count = merged[&name];
            FileAccess { path: name, count }
        })
        .collect();
    out.sort_by_key(|b| std::cmp::Reverse(b.count));
    out.truncate(limit);
    out
}

/// Render the structured report as a compact, LLM-dense plain-text packet,
/// truncated to `tokens` (4 chars/token). Sections are added in priority
/// order; lowest-priority sections are dropped first when over budget so the
/// most recent session block always survives.
fn render_digest_text(report: &DigestReport, tokens: usize) -> String {
    let budget = tokens.saturating_mul(4);
    let mut out = String::new();

    let header = match &report.project {
        Some(p) => format!(
            "== PRIOR SESSIONS (project: {}, last {}) ==\n",
            p,
            report.sessions.len()
        ),
        None => format!("== PRIOR SESSIONS (last {}) ==\n", report.sessions.len()),
    };
    out.push_str(&header);

    if report.fell_back_to_all_projects {
        out.push_str("note: no sessions tagged with the requested project; showing most recent across all projects.\n");
    }

    // Session blocks are highest priority — render them first and keep them
    // even if later sections get dropped.
    for s in &report.sessions {
        out.push_str(&render_session_block(s));
    }

    let project_memory_block = render_memory_block(&report.project_memory);
    let open_tasks_block = render_tasks_block(&report.open_tasks);
    let thread_anchors_block = render_anchors_block(&report.thread_anchors);

    // Append lower-priority sections only while within budget.
    let mut tail = String::new();
    for block in [project_memory_block, open_tasks_block, thread_anchors_block] {
        if block.is_empty() {
            continue;
        }
        if out.len() + tail.len() + block.len() > budget {
            tail.push_str("[truncated]\n");
            break;
        }
        tail.push_str(&block);
    }
    out.push_str(&tail);

    // Hard cap: if session blocks alone blew the budget, cut from the end.
    if out.len() > budget {
        out.truncate(budget.saturating_sub("[truncated]\n".len()));
        out.push_str("[truncated]\n");
    }

    out
}

fn render_session_block(s: &DigestSession) -> String {
    let mut out = String::new();
    let ts = s.started_at.split('T').next().unwrap_or(&s.started_at);
    let ts_time = s
        .started_at
        .split('T')
        .nth(1)
        .map(|t| t.split('+').next().unwrap_or(t))
        .unwrap_or("");
    out.push_str(&format!(
        "\n[{} {}] {} branch={} tool={}\n",
        ts,
        ts_time,
        s.short_id,
        s.git_branch.as_deref().unwrap_or("-"),
        s.tool
    ));
    if let Some(parent) = &s.parent_session_id {
        let pshort = short_id(parent);
        out.push_str(&format!("  parent: {}\n", pshort));
    }
    // Activity line: always show tool calls; show file writes and commits
    // only when non-zero (the session ledger leaves these zero when the
    // recorder does not backfill them, and there is no commit event type,
    // so they are usually 0 — printing them adds noise).
    let mut activity = format!("{} tool calls", s.tool_calls);
    if s.file_writes > 0 {
        activity.push_str(&format!(", {} file writes", s.file_writes));
    }
    if s.commits > 0 {
        activity.push_str(&format!(", {} commits", s.commits));
    }
    out.push_str(&format!("  activity: {}\n", activity));
    if !s.top_files.is_empty() {
        let files: Vec<String> = s
            .top_files
            .iter()
            .map(|f| {
                let name = f.path.rsplit('/').next().unwrap_or(&f.path);
                format!("{} (x{})", name, f.count)
            })
            .collect();
        out.push_str(&format!("  files: {}\n", files.join(", ")));
    }
    if let Some(lt) = &s.last_tool {
        let summary = s
            .last_tool_summary
            .as_deref()
            .map(|s| truncate_summary(s, 80))
            .unwrap_or_default();
        out.push_str(&format!("  last: {} {}\n", lt, summary));
    }
    for r in &s.reasoning {
        if !r.summary.is_empty() {
            out.push_str(&format!(
                "  decision: {} (#{})\n",
                truncate_summary(&r.summary, 120),
                r.entity_id
            ));
        }
    }
    for d in &s.discoveries {
        let summary = d
            .summary
            .as_deref()
            .map(|s| format!(" — {}", truncate_summary(s, 100)))
            .unwrap_or_default();
        out.push_str(&format!(
            "  discovery: {} [{}]{}\n",
            d.target, d.discovery_type, summary
        ));
    }
    out
}

fn render_memory_block(mem: &[DigestMemory]) -> String {
    if mem.is_empty() {
        return String::new();
    }
    let mut out = String::from("\n== PROJECT MEMORY ==\n");
    for m in mem {
        out.push_str(&format!(
            "- [{}] (conf {:.2}): {}\n",
            m.key,
            m.confidence,
            truncate_summary(&m.content, 160)
        ));
    }
    out
}

fn render_tasks_block(tasks: &[DigestTask]) -> String {
    if tasks.is_empty() {
        return String::new();
    }
    let mut out = String::from("\n== OPEN TASKS ==\n");
    for t in tasks {
        out.push_str(&format!("- [{}] {}\n", t.status, t.title));
    }
    out
}

fn render_anchors_block(anchors: &[i64]) -> String {
    if anchors.is_empty() {
        return String::new();
    }
    let ids: Vec<String> = anchors.iter().map(|a| format!("#{}", a)).collect();
    format!(
        "\n== THREAD ANCHORS (traverse with `atheneum navigate`) ==\n{}\n",
        ids.join(", ")
    )
}

fn truncate_summary(s: &str, max_chars: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    format!("{}…", truncated.trim_end())
}

/// Serialize a digest report to a JSON value (for `--json` output).
pub fn digest_report_to_json(report: &DigestReport) -> Result<Value> {
    serde_json::to_value(report).map_err(|e| anyhow::anyhow!("digest serialization failed: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn truncate_summary_short_unchanged() {
        assert_eq!(truncate_summary("hello", 10), "hello");
    }

    #[test]
    fn truncate_summary_long_clamped() {
        let s = "x".repeat(200);
        let out = truncate_summary(&s, 10);
        assert!(out.ends_with('…'));
        assert!(out.chars().count() <= 10);
    }

    #[test]
    fn render_empty_report_has_header() {
        let report = DigestReport {
            project: Some("rocmforge".into()),
            fell_back_to_all_projects: false,
            sessions: vec![],
            project_memory: vec![],
            open_tasks: vec![],
            thread_anchors: vec![],
        };
        let text = render_digest_text(&report, 500);
        assert!(text.contains("PRIOR SESSIONS"));
        assert!(text.contains("rocmforge"));
    }

    #[test]
    fn render_drops_low_priority_when_over_budget() {
        let session = DigestSession {
            session_id: "abcdef-1234".into(),
            short_id: "abcdef".into(),
            started_at: "2026-06-22T08:03:53+00:00".into(),
            project: "p".into(),
            git_branch: Some("HEAD".into()),
            tool: "claude-code".into(),
            parent_session_id: None,
            tool_calls: 230,
            file_writes: 0,
            commits: 0,
            top_files: vec![FileAccess {
                path: "src/main.rs".into(),
                count: 5,
            }],
            last_tool: Some("Bash".into()),
            last_tool_summary: Some("cargo build".into()),
            reasoning: vec![DigestReasoning {
                entity_id: 42,
                summary: "decided X".into(),
            }],
            discoveries: vec![],
        };
        let report = DigestReport {
            project: Some("p".into()),
            fell_back_to_all_projects: false,
            sessions: vec![session],
            project_memory: vec![DigestMemory {
                key: "k".into(),
                content: "m".into(),
                confidence: 1.0,
            }],
            open_tasks: vec![DigestTask {
                title: "t".into(),
                status: "TODO".into(),
            }],
            thread_anchors: vec![7],
        };
        // Tiny budget: session block survives, tail is truncated.
        let text = render_digest_text(&report, 30);
        assert!(text.contains("abcdef"));
        assert!(text.contains("[truncated]"));
    }

    #[test]
    fn digest_report_serializes_to_json() {
        let report = DigestReport {
            project: None,
            fell_back_to_all_projects: false,
            sessions: vec![],
            project_memory: vec![],
            open_tasks: vec![],
            thread_anchors: vec![],
        };
        let v = digest_report_to_json(&report).unwrap();
        assert_eq!(v["sessions"], json!([]));
    }

    #[test]
    fn short_id_takes_first_segment() {
        assert_eq!(short_id("c663d1ff-d1e1-4525-bd70-84af021679bc"), "c663d1ff");
        assert_eq!(short_id("plain"), "plain");
    }

    #[test]
    fn merge_by_basename_sums_shared_names() {
        let rows = vec![
            FileAccess {
                path: "/a/SKILL.md".into(),
                count: 26,
            },
            FileAccess {
                path: "/b/SKILL.md".into(),
                count: 16,
            },
            FileAccess {
                path: "src/layer.rs".into(),
                count: 20,
            },
            FileAccess {
                path: "/c/SKILL.md".into(),
                count: 7,
            },
        ];
        let merged = merge_by_basename(rows, 5);
        // SKILL.md summed to 49 (first), layer.rs second.
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].path, "SKILL.md");
        assert_eq!(merged[0].count, 49);
        assert_eq!(merged[1].path, "layer.rs");
        assert_eq!(merged[1].count, 20);
    }

    #[test]
    fn merge_by_basename_truncates_to_limit() {
        let rows: Vec<FileAccess> = (0..10)
            .map(|i| FileAccess {
                path: format!("src/f{}.rs", i),
                count: 10 - i,
            })
            .collect();
        let merged = merge_by_basename(rows, 3);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].path, "f0.rs");
    }
}
