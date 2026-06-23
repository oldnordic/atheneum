//! Phase 4 — live decision watcher.
//!
//! A long-running process that tails active Claude Code transcripts
//! incrementally and captures *decisions* as first-class `Discovery` nodes in
//! real time — within seconds of the event, not at session end. Tier-1
//! detector only: structured tool signals, no LLM, 100% precision. A signal
//! that is not present in the structured tool input is NOT fabricated (an
//! absent plan text never becomes a fake decision).
//!
//! Design (grounded against real `~/.claude/projects/*/*.jsonl` transcripts +
//! abtop's `transcript_cache`/`refresh_config_dirs` pattern):
//!
//! - The watcher does its OWN detect-only incremental parse. It does not
//!   ingest prompts/tool-calls — the Stop hook's `sync-claude-transcript`
//!   already does full ingest at session end. The watcher's job is to surface
//!   decisions early.
//! - Per-file in-memory cursor (`HashMap<PathBuf, FileCursor>`) holds the byte
//!   offset + a per-session `tool_sequence` counter. File identity (inode +
//!   mtime_ns) detects rotation/truncation; on identity change the cursor
//!   resets to 0 and the file is re-scanned (abtop's `identity_changed`).
//! - A partial final line (no trailing `\n`) is left un-advanced so the next
//!   tick re-reads it once Claude Code finishes writing it — a live stream is
//!   mid-line most of the time.
//! - Dedup key = `(session_id, sequence, target, source)` via
//!   [`AtheneumGraph::decision_exists`], checked before every insert. The
//!   offset cache is the primary idempotency mechanism; the dedup query is the
//!   safety net (covers restart-driven re-scans).
//!
//! Tier-1 signals (verified against real transcripts 2026-06-23):
//!
//! - `AskUserQuestion` — tool_use carries `questions:[{question, header,
//!   multiSelect, options:[{label,description}]}]`. The matching `tool_result`
//!   is a string `Your questions have been answered: "<q>"="<label>" ...`.
//!   One Decision per answered question: `target` = header, `chosen` = the
//!   selected label, `alternatives` = all option labels, `rationale` = the
//!   chosen option's description, `source = "askuser"`.
//! - `ExitPlanMode` — current JSONL records `allowedPrompts` (a list of
//!   permitted action categories), NOT a `plan` markdown field (8/10 real
//!   inputs are `{}`). The plan's "plan text" assumption does not hold for the
//!   current format, so a Decision is emitted ONLY when `allowedPrompts` is a
//!   non-empty array: `target = "plan-approval"`, `chosen = "proceed"`,
//!   `alternatives = allowedPrompts`, `rationale = "ExitPlanMode approved plan
//!   for execution"`, `source = "exitplan"`. Empty input → skip (no structured
//!   signal; 100% precision forbids inventing a plan subject).
//! - `TaskCreate` — the real current equivalent of the plan's "TodoWrite"
//!   (this user's transcripts use `TaskCreate`, not `TodoWrite`).
//!   `target = subject`, `chosen = subject`, `rationale = description`,
//!   `source = "taskcreate"`.
//! - `TodoWrite` — kept for spec completeness + older transcripts that still
//!   use it. `input.todos:[{content, status, ...}]`; one Decision per *new*
//!   task (content not seen in a prior same-session TodoWrite call).
//!   `target = content`, `chosen = content`, `source = "todowrite"`.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use regex::Regex;
use serde_json::{json, Value};

use super::AtheneumGraph;

const MAX_LINE_BYTES: usize = 10 * 1024 * 1024;
const DEFAULT_INTERVAL_SECS: u64 = 2;

const SOURCE_ASKUSER: &str = "askuser";
const SOURCE_EXITPLAN: &str = "exitplan";
const SOURCE_TASKCREATE: &str = "taskcreate";
const SOURCE_TODOWRITE: &str = "todowrite";

#[derive(Debug, Clone)]
pub struct WatchConfig {
    /// Claude config roots to scan (`<dir>/projects/*/*.jsonl`).
    pub config_dirs: Vec<PathBuf>,
    /// Tick interval for the foreground loop. Ignored when `once` is set.
    pub interval: Duration,
    /// Override the `project_id` stamped onto captured decisions. When `None`,
    /// the project is inferred from the transcript's parent directory.
    pub project: Option<String>,
    /// Agent name on the stored Discovery.
    pub agent: String,
    /// Dry run: run the detector + dedup check, print what would be stored,
    /// but do not call `store_discovery`.
    pub dry_run: bool,
    /// Single scan + exit (testing / cron). Default `false` = run forever.
    pub once: bool,
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            config_dirs: discover_config_dirs(),
            interval: Duration::from_secs(DEFAULT_INTERVAL_SECS),
            project: None,
            agent: "claude".to_string(),
            dry_run: false,
            once: false,
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct WatchStats {
    pub files_scanned: usize,
    pub decisions_emitted: usize,
    pub decisions_skipped: usize,
}

#[derive(Debug, Default, Clone)]
struct FileCursor {
    offset: u64,
    inode: u64,
    mtime_ns: u64,
    /// Per-session (per-file) 1-based ordinal of tool_use blocks seen. Stable
    /// across re-scans of identical bytes; reset to 0 on identity change.
    tool_sequence: i64,
}

#[derive(Debug, Clone)]
struct PendingAsk {
    sequence: i64,
    questions: Vec<AskQuestion>,
}

#[derive(Debug, Clone)]
struct AskQuestion {
    header: String,
    question: String,
    option_labels: Vec<String>,
    option_descriptions: Vec<String>,
}

/// Run the decision watcher. With `config.once` = true, performs a single scan
/// and returns; otherwise loops forever, sleeping `config.interval` between
/// scans. Aggregate stats across all scans are returned on exit.
pub fn watch_decisions(graph: &AtheneumGraph, config: &WatchConfig) -> Result<WatchStats> {
    let mut stats = WatchStats::default();
    let mut cursors: HashMap<PathBuf, FileCursor> = HashMap::new();
    let mut pending: HashMap<String, PendingAsk> = HashMap::new();
    let mut seen_todos: HashMap<String, HashSet<String>> = HashMap::new();
    loop {
        let scan = scan_once(graph, config, &mut cursors, &mut pending, &mut seen_todos)?;
        stats.files_scanned += scan.files_scanned;
        stats.decisions_emitted += scan.decisions_emitted;
        stats.decisions_skipped += scan.decisions_skipped;
        if config.once {
            return Ok(stats);
        }
        std::thread::sleep(config.interval);
    }
}

fn scan_once(
    graph: &AtheneumGraph,
    config: &WatchConfig,
    cursors: &mut HashMap<PathBuf, FileCursor>,
    pending: &mut HashMap<String, PendingAsk>,
    seen_todos: &mut HashMap<String, HashSet<String>>,
) -> Result<WatchStats> {
    let files = discover_transcripts(&config.config_dirs);
    let mut stats = WatchStats::default();
    for path in files {
        stats.files_scanned += 1;
        scan_file(
            graph, config, &path, cursors, pending, seen_todos, &mut stats,
        )?;
    }
    Ok(stats)
}

fn scan_file(
    graph: &AtheneumGraph,
    config: &WatchConfig,
    path: &Path,
    cursors: &mut HashMap<PathBuf, FileCursor>,
    pending: &mut HashMap<String, PendingAsk>,
    seen_todos: &mut HashMap<String, HashSet<String>>,
    stats: &mut WatchStats,
) -> Result<()> {
    let session_id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown-session")
        .to_string();
    let project_id = config
        .project
        .clone()
        .unwrap_or_else(|| infer_project_id(path));

    let metadata = match fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return Ok(()), // file disappeared mid-scan — skip silently
    };
    let file_len = metadata.len();
    let identity = file_identity(path).unwrap_or((0, 0));

    let cursor = cursors.entry(path.to_path_buf()).or_default();
    let mut reset = false;
    if cursor.offset > 0 && file_len < cursor.offset {
        reset = true; // truncated
    } else if cursor.inode != 0 && cursor.inode != identity.0 && cursor.offset > 0 {
        reset = true; // rotated (inode changed)
    } else if cursor.offset > 0
        && cursor.mtime_ns != 0
        && cursor.mtime_ns != identity.1
        && file_len == cursor.offset
    {
        reset = true; // rewritten in place at identical length
    }
    if reset {
        *cursor = FileCursor {
            offset: 0,
            inode: 0,
            mtime_ns: 0,
            tool_sequence: 0,
        };
    }

    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return Ok(()),
    };
    let mut reader = BufReader::new(file);
    if cursor.offset > 0 {
        reader.seek(SeekFrom::Start(cursor.offset)).ok();
    }

    let mut line_buf = String::new();
    loop {
        line_buf.clear();
        let n = reader
            .by_ref()
            .take(MAX_LINE_BYTES as u64 + 1)
            .read_line(&mut line_buf)?;
        if n == 0 {
            break; // EOF
        }
        let has_newline = line_buf.ends_with('\n');
        // Oversized partial line: guaranteed-progress skip so a pathological
        // line can't pin the watcher. A complete oversized line is also skipped.
        if line_buf.len() > MAX_LINE_BYTES && !has_newline {
            cursor.offset += n as u64;
            continue;
        }
        let line = line_buf.trim();
        if line.is_empty() {
            if has_newline {
                cursor.offset += n as u64;
            }
            continue;
        }
        let value = match serde_json::from_str::<Value>(line) {
            Ok(v) => v,
            Err(_) => {
                if has_newline {
                    cursor.offset += n as u64; // corrupt complete line — skip
                    continue;
                } else {
                    break; // partial line at EOF — don't advance, re-read next tick
                }
            }
        };
        cursor.offset += n as u64;

        match value.get("type").and_then(Value::as_str) {
            Some("assistant") => {
                if let Some(content) = value
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(Value::as_array)
                {
                    for block in content {
                        if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                            continue;
                        }
                        cursor.tool_sequence += 1;
                        let seq = cursor.tool_sequence;
                        let tool_use_id = block.get("id").and_then(Value::as_str).unwrap_or("");
                        let name = block.get("name").and_then(Value::as_str).unwrap_or("");
                        let input = block.get("input").cloned().unwrap_or(Value::Null);
                        handle_tool_use(
                            graph,
                            config,
                            &session_id,
                            &project_id,
                            seq,
                            tool_use_id,
                            name,
                            &input,
                            pending,
                            seen_todos,
                            stats,
                        )?;
                    }
                }
            }
            Some("user") => {
                if let Some(content) = value
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(Value::as_array)
                {
                    for block in content {
                        if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                            continue;
                        }
                        let tool_use_id = block
                            .get("tool_use_id")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        if tool_use_id.is_empty() {
                            continue;
                        }
                        let result_text = extract_tool_result_text(block);
                        if let Some(pend) = pending.remove(&tool_use_id) {
                            resolve_ask(
                                graph,
                                config,
                                &session_id,
                                &project_id,
                                pend,
                                &result_text,
                                stats,
                            )?;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    cursor.inode = identity.0;
    cursor.mtime_ns = identity.1;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn handle_tool_use(
    graph: &AtheneumGraph,
    config: &WatchConfig,
    session_id: &str,
    project_id: &str,
    sequence: i64,
    tool_use_id: &str,
    name: &str,
    input: &Value,
    pending: &mut HashMap<String, PendingAsk>,
    seen_todos: &mut HashMap<String, HashSet<String>>,
    stats: &mut WatchStats,
) -> Result<()> {
    match name {
        "AskUserQuestion" => {
            let questions = parse_ask_questions(input);
            if !questions.is_empty() {
                pending.insert(
                    tool_use_id.to_string(),
                    PendingAsk {
                        sequence,
                        questions,
                    },
                );
            }
        }
        "ExitPlanMode" => {
            if let Some(prompts) = input.get("allowedPrompts").and_then(Value::as_array) {
                let alternatives: Vec<String> = prompts
                    .iter()
                    .filter_map(|p| p.as_str().map(str::to_string))
                    .collect();
                if !alternatives.is_empty() {
                    let stored = store_decision(
                        graph,
                        config,
                        session_id,
                        project_id,
                        sequence,
                        "plan-approval",
                        "proceed",
                        alternatives.clone(),
                        format!(
                            "ExitPlanMode approved plan for execution (scope: {})",
                            alternatives.join(", ")
                        ),
                        SOURCE_EXITPLAN,
                    )?;
                    tally(stored, stats);
                }
            }
            // Empty `{}` input carries no structured signal → skip (100% precision).
        }
        "TaskCreate" => {
            let subject = input.get("subject").and_then(Value::as_str).unwrap_or("");
            if subject.is_empty() {
                return Ok(());
            }
            let description = input
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("");
            let stored = store_decision(
                graph,
                config,
                session_id,
                project_id,
                sequence,
                subject,
                subject,
                Vec::new(),
                description.to_string(),
                SOURCE_TASKCREATE,
            )?;
            tally(stored, stats);
        }
        "TodoWrite" => {
            if let Some(todos) = input.get("todos").and_then(Value::as_array) {
                let seen = seen_todos.entry(session_id.to_string()).or_default();
                for todo in todos {
                    let content = todo.get("content").and_then(Value::as_str).unwrap_or("");
                    if content.is_empty() {
                        continue;
                    }
                    // "New tasks only": a content string already seen in a prior
                    // same-session TodoWrite call is a re-write, not a new task.
                    if !seen.insert(content.to_string()) {
                        continue;
                    }
                    let stored = store_decision(
                        graph,
                        config,
                        session_id,
                        project_id,
                        sequence,
                        content,
                        content,
                        Vec::new(),
                        String::new(),
                        SOURCE_TODOWRITE,
                    )?;
                    tally(stored, stats);
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn parse_ask_questions(input: &Value) -> Vec<AskQuestion> {
    let Some(qs) = input.get("questions").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for q in qs {
        let header = q
            .get("header")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let question = q
            .get("question")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let mut labels = Vec::new();
        let mut descriptions = Vec::new();
        if let Some(options) = q.get("options").and_then(Value::as_array) {
            for opt in options {
                labels.push(
                    opt.get("label")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                );
                descriptions.push(
                    opt.get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                );
            }
        }
        out.push(AskQuestion {
            header,
            question,
            option_labels: labels,
            option_descriptions: descriptions,
        });
    }
    out
}

fn resolve_ask(
    graph: &AtheneumGraph,
    config: &WatchConfig,
    session_id: &str,
    project_id: &str,
    pend: PendingAsk,
    result_text: &str,
    stats: &mut WatchStats,
) -> Result<()> {
    let pairs = parse_answer_pairs(result_text);
    for (q_text, answer_label) in pairs {
        // Match the answered question to its input definition by question text
        // (trimmed), falling back to ordinal position.
        let matched = pend
            .questions
            .iter()
            .find(|q| q.question.trim() == q_text.trim())
            .or_else(|| {
                let idx = pend.questions.iter().position(|q| !q.question.is_empty());
                idx.and_then(|i| pend.questions.get(i))
            });
        let Some(q) = matched else { continue };
        let target = if q.header.is_empty() {
            &q.question
        } else {
            &q.header
        };
        // Rationale = the description of the chosen option (if found).
        let rationale = q
            .option_labels
            .iter()
            .position(|l| l == &answer_label)
            .and_then(|i| q.option_descriptions.get(i))
            .cloned()
            .unwrap_or_default();
        let stored = store_decision(
            graph,
            config,
            session_id,
            project_id,
            pend.sequence,
            target,
            &answer_label,
            q.option_labels.clone(),
            rationale,
            SOURCE_ASKUSER,
        )?;
        tally(stored, stats);
    }
    Ok(())
}

/// Parse `Your questions have been answered: "<q>"="<label>"[, "<q2>"="<l2>"]...`
/// into `(question, label)` pairs. Handles the dominant single-select format
/// observed in real transcripts. Multi-select bracketed forms (`"q"=[...]`) are
/// not matched — a miss, never a fabrication (Tier-1 precision).
fn parse_answer_pairs(text: &str) -> Vec<(String, String)> {
    let re = Regex::new(r#""(?P<q>[^"]+)"\s*=\s*"(?P<a>[^"]+)""#).unwrap();
    re.captures_iter(text)
        .filter_map(|c| {
            let q = c.name("q")?.as_str().to_string();
            let a = c.name("a")?.as_str().to_string();
            Some((q, a))
        })
        .collect()
}

fn extract_tool_result_text(block: &Value) -> String {
    match block.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|b| {
                if b.get("type").and_then(Value::as_str) == Some("text") {
                    b.get("text").and_then(Value::as_str).map(str::to_string)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

#[allow(clippy::too_many_arguments)]
fn store_decision(
    graph: &AtheneumGraph,
    config: &WatchConfig,
    session_id: &str,
    project_id: &str,
    sequence: i64,
    target: &str,
    chosen: &str,
    alternatives: Vec<String>,
    rationale: String,
    source: &str,
) -> Result<bool> {
    if graph.decision_exists(session_id, sequence, target, source)? {
        return Ok(false); // already captured — dedup safety net
    }
    if config.dry_run {
        eprintln!(
            "[watch-dry] src={} seq={} target={:?} chosen={:?} alternatives={:?}",
            source, sequence, target, chosen, alternatives
        );
        return Ok(true);
    }
    let mut metadata = json!({
        "session_id": session_id,
        "sequence": sequence,
        "source": source,
        "chosen": chosen,
        "alternatives": alternatives,
        "rationale": rationale,
        "project_id": project_id,
    });
    if let Some(obj) = metadata.as_object_mut() {
        obj.insert("project_id".to_string(), json!(project_id));
    }
    graph.store_discovery(&config.agent, "Decision", target, metadata)?;
    Ok(true)
}

fn tally(stored: bool, stats: &mut WatchStats) {
    if stored {
        stats.decisions_emitted += 1;
    } else {
        stats.decisions_skipped += 1;
    }
}

/// Discover Claude config roots: default `~/.claude`, the `CLAUDE_CONFIG_DIR`
/// env var, and any `CLAUDE_CONFIG_DIR` set in a running process's
/// `/proc/<pid>/environ` (abtop's `refresh_config_dirs` subset). One-shot at
/// config build time — config dirs change rarely.
fn discover_config_dirs() -> Vec<PathBuf> {
    let mut seen: std::collections::BTreeSet<PathBuf> = std::collections::BTreeSet::new();
    if let Ok(home) = std::env::var("HOME") {
        let p = PathBuf::from(home).join(".claude");
        if p.is_dir() {
            seen.insert(p);
        }
    }
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        let p = PathBuf::from(dir);
        if p.is_dir() {
            seen.insert(p);
        }
    }
    // /proc/<pid>/environ is null-separated KEY=VALUE pairs.
    if let Ok(entries) = fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let environ = entry.path().join("environ");
            if let Ok(content) = fs::read_to_string(&environ) {
                for kv in content.split('\0') {
                    if let Some(rest) = kv.strip_prefix("CLAUDE_CONFIG_DIR=") {
                        let p = PathBuf::from(rest);
                        if p.is_dir() {
                            seen.insert(p);
                        }
                        break;
                    }
                }
            }
        }
    }
    seen.into_iter().collect()
}

/// Collect every `<config_dir>/projects/*/*.jsonl` transcript path.
fn discover_transcripts(config_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for d in config_dirs {
        let projects = d.join("projects");
        let Ok(session_dirs) = fs::read_dir(&projects) else {
            continue;
        };
        for entry in session_dirs.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Ok(files) = fs::read_dir(&path) else {
                continue;
            };
            for f in files.flatten() {
                let p = f.path();
                if p.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                    out.push(p);
                }
            }
        }
    }
    out
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
fn file_identity(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::metadata(path).ok()?;
    let mtime_ns = chrono::DateTime::from_timestamp(metadata.mtime(), metadata.mtime_nsec() as u32)
        .and_then(|dt| dt.timestamp_nanos_opt())
        .map(|ns| ns as u64)
        .unwrap_or(0);
    Some((metadata.ino(), mtime_ns))
}

#[cfg(not(unix))]
fn file_identity(path: &Path) -> Option<(u64, u64)> {
    let metadata = fs::metadata(path).ok()?;
    let mtime_ns = metadata
        .modified()
        .ok()
        .and_then(|mtime| mtime.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    Some((metadata.len(), mtime_ns))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_answer_pairs_single_and_multiple() {
        let s =
            r#"Your questions have been answered: "Which approach?"="HNSW". You can now continue."#;
        let pairs = parse_answer_pairs(s);
        assert_eq!(
            pairs,
            vec![("Which approach?".to_string(), "HNSW".to_string())]
        );

        let s2 = r#"answered: "Q1"="A1", "Q2"="A2". continue."#;
        let pairs2 = parse_answer_pairs(s2);
        assert_eq!(
            pairs2,
            vec![
                ("Q1".to_string(), "A1".to_string()),
                ("Q2".to_string(), "A2".to_string()),
            ]
        );
    }

    #[test]
    fn parse_answer_pairs_missing_bracket_is_empty_not_fabricated() {
        // Multi-select bracketed form is NOT matched — empty result, no fake.
        let s = r#"answered: "Q"=["A","B"]. continue."#;
        assert!(parse_answer_pairs(s).is_empty());
    }

    #[test]
    fn decode_project_dir_name_strips_encoded_prefix() {
        assert_eq!(
            decode_project_dir_name("-home-feanor-Projects-atheneum"),
            "atheneum"
        );
        assert_eq!(decode_project_dir_name("plain"), "plain");
    }
}
