//! Native Rust port of the `extract-decisions` Phase 3 backfiller.
//!
//! Runs over Claude Code session transcript JSONLs, extracts decision-shaped
//! turns, and stores each as an atheneum `Decision` discovery *in-process* via
//! [`AtheneumGraph::store_discovery`] (which auto-links the
//! `caused_by` / `led_to` thread edges). This is the native equivalent of the
//! `~/.local/bin/extract-decisions` Python operator script — same prompt, same
//! chunking, same hallucination guard, same dedup semantics — without shelling
//! out to the CLI.
//!
//! ## Backends — user's choice
//!
//! Two extraction backends, picked per run via `--heuristic` / `--mode` or the
//! `ATHENEUM_EXTRACT_MODE` env var (default `llm`):
//!
//! - [`ExtractMode::Llm`] — a local Ollama LLM (`qwen3.5` by default) reads the
//!   transcript prose and emits structured decisions. Higher precision on
//!   prose decisions; requires a running local Ollama.
//! - [`ExtractMode::Heuristic`] — rule-based, no LLM, no network. Catches
//!   decision-shaped sentences with an explicit rationale clause. Lower
//!   recall + some false positives; zero deps. Use when Ollama is unavailable
//!   or you want a deterministic, offline pass.
//!
//! Each backend writes a distinct `source` tag (`llm-extract` vs `heuristic`),
//! so the two are separately resumable and distinguishable in the graph.
//!
//! Gated behind the `extract` feature (the LLM path needs the `ureq` HTTP
//! client; the heuristic path needs nothing extra but shares the feature
//! gate). Off by default; the Python script is the default fallback.
//! Built + tested under `--all-features`.
//!
//! ## Idempotency
//!
//! LLM extraction is non-deterministic (the model phrases `target`/`chosen`
//! differently across runs), so per-decision exact dedup cannot prevent
//! cross-run duplicates. Instead a store-mode run skips any session that
//! already has a Decision from the *same* backend's `source` tag — re-running
//! the same backend is a true no-op and `--all` is resumable. `--force`
//! re-extracts (exact-dedupe safety net only; near-duplicate phrasings may add
//! rows). `--dry-run` always extracts for review. The heuristic backend is
//! deterministic, so its re-runs dedup exactly on `(target, chosen)`.

#![cfg(feature = "extract")]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};

use super::AtheneumGraph;

const DEFAULT_MODEL: &str = "qwen3.5";
const DEFAULT_MAX_CHARS: usize = 20000;
const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434/api/generate";
const MIN_PHRASE: usize = 12;

/// JSON schema enforced by Ollama's `format` field. Thinking-mode models
/// (qwen3.5) emit valid JSON shaped like this under schema-format; without it
/// the model rambles. Mirrors `DECISION_SCHEMA` in the Python script. Built at
/// runtime (not `const`) because `json!` emits heap allocations.
fn decision_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "decisions": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "target": {"type": "string"},
                        "chosen": {"type": "string"},
                        "alternatives": {"type": "array", "items": {"type": "string"}},
                        "rationale": {"type": "string"},
                        "sequence": {"type": "integer"}
                    },
                    "required": ["target", "chosen", "rationale", "sequence"]
                }
            }
        },
        "required": ["decisions"]
    })
}

const EXTRACTION_INSTRUCTION: &str = "Extract the software-engineering decisions from the transcript below. A decision is a point where the agent or user chose one approach over alternatives with a stated rationale; include architecture choices, library/version picks, trade-off resolutions, and strategy decisions. Ignore routine tool calls, file reads, greetings, status updates, and investigation steps that involve no choice. Use real phrases copied verbatim from the transcript — never use placeholder text like '...' or the literal words 'target'/'chosen'. Emit a decision ONLY when a clear chosen option and a non-empty rationale exist. Return ONLY the JSON object {\"decisions\":[{\"target\":str,\"chosen\":str,\"alternatives\":[str],\"rationale\":str,\"sequence\":int}]} where sequence is the [N|role] turn index where the decision appears. If there are no decisions, return {\"decisions\":[]}.";

/// Placeholder words the hallucination guard rejects verbatim.
const PLACEHOLDER_WORDS: &[&str] = &[
    "target",
    "chosen",
    "rationale",
    "alternatives",
    "string",
    "str",
    "integer",
    "int",
    "null",
    "none",
    "...",
    "..",
    ".",
];

/// Extraction backend. The user picks this per run — see `--heuristic` /
/// `ATHENEUM_EXTRACT_MODE` — so the tradeoff (LLM precision vs no-dependency
/// heuristics) is explicit, not buried in code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractMode {
    /// Ollama LLM (`qwen3.5` by default). Higher precision on prose decisions;
    /// requires a running local Ollama.
    Llm,
    /// Rule-based, no LLM, no network. Catches decision-shaped sentences with
    /// an explicit rationale clause. Lower recall + some false positives vs
    /// the LLM; zero deps. Falls back to this when Ollama is unavailable.
    Heuristic,
}

impl ExtractMode {
    /// Discovery `source` tag stored on each Decision. Distinct tags keep the
    /// two backends separately resumable (a heuristic run won't skip a session
    /// that only has `llm-extract` rows, and vice versa).
    fn source_tag(self) -> &'static str {
        match self {
            ExtractMode::Llm => "llm-extract",
            ExtractMode::Heuristic => "heuristic",
        }
    }
}

/// Configuration for one `extract-decisions` run.
#[derive(Debug, Clone)]
pub struct ExtractConfig {
    /// atheneum DB path.
    pub db: PathBuf,
    /// Root of the transcript trees (default `~/.claude/projects`).
    pub transcripts_dir: PathBuf,
    /// Ollama model (default `qwen3.5`).
    pub model: String,
    /// Ollama generate endpoint URL.
    pub ollama_url: String,
    /// Per-chunk cap on transcript text fed to the LLM.
    pub max_chars: usize,
    /// project_id stored on each Decision; `None` → derive from transcript path.
    pub project: Option<String>,
    /// Agent name stored on each Decision (default `claude`).
    pub agent: String,
    /// Process every transcript under `transcripts_dir` (resumable).
    pub all: bool,
    /// Re-extract sessions that already have `llm-extract` decisions.
    pub force: bool,
    /// Print extracted decisions, store nothing.
    pub dry_run: bool,
    /// Per-session / per-chunk progress on stderr.
    pub verbose: bool,
    /// Extraction backend (LLM vs heuristic). Default `Llm`; override with
    /// `--heuristic` / `--mode` / `ATHENEUM_EXTRACT_MODE`.
    pub mode: ExtractMode,
    /// Single session id to process (`all` must be false).
    pub session_id: Option<String>,
}

impl Default for ExtractConfig {
    fn default() -> Self {
        let db = std::env::var("ATHENEUM_DB")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                dirs_or_home()
                    .join(".magellan")
                    .join("atheneum")
                    .join("atheneum.db")
            });
        let transcripts_dir = dirs_or_home().join(".claude").join("projects");
        let mode = match std::env::var("ATHENEUM_EXTRACT_MODE")
            .unwrap_or_default()
            .trim()
            .to_lowercase()
            .as_str()
        {
            "heuristic" | "rules" | "no-llm" => ExtractMode::Heuristic,
            "llm" | "ollama" | "" => ExtractMode::Llm,
            other => {
                eprintln!(
                    "ATHENEUM_EXTRACT_MODE={:?} unrecognized (expected llm|heuristic); defaulting to llm",
                    other
                );
                ExtractMode::Llm
            }
        };
        Self {
            db,
            transcripts_dir,
            model: DEFAULT_MODEL.to_string(),
            ollama_url: DEFAULT_OLLAMA_URL.to_string(),
            max_chars: DEFAULT_MAX_CHARS,
            project: None,
            agent: "claude".to_string(),
            all: false,
            force: false,
            dry_run: false,
            verbose: false,
            mode,
            session_id: None,
        }
    }
}

fn dirs_or_home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/"))
}

/// Aggregate stats for a run (returned to the CLI).
#[derive(Debug, Clone, Default)]
pub struct ExtractStats {
    pub sessions: usize,
    pub extracted: usize,
    pub stored: usize,
    pub skipped: usize,
}

/// One extracted decision before storage.
#[derive(Debug, Clone)]
struct Decision {
    target: String,
    chosen: String,
    alternatives: Vec<String>,
    rationale: String,
    sequence: i64,
}

/// One flattened transcript turn.
#[derive(Debug, Clone)]
struct Turn {
    sequence: i64,
    role: String,
    text: String,
}

// --- public entry point ---------------------------------------------------

/// Run the extract-decisions pass described by `config`. Opens the graph once
/// and stores in-process. Prints per-session headers (verbose/dry-run) and the
/// final summary line to stdout.
pub fn run_extract(config: &ExtractConfig) -> Result<ExtractStats> {
    if config.all && config.session_id.is_some() {
        return Err(anyhow!("pass either <session-id> or --all, not both"));
    }
    let transcripts = resolve_transcripts(config)?;
    if transcripts.is_empty() {
        return Ok(ExtractStats::default());
    }

    // dry-run never touches the DB, so skip opening it.
    let graph = if config.dry_run {
        None
    } else {
        Some(AtheneumGraph::open(&config.db).context("open atheneum db")?)
    };

    let mut stats = ExtractStats::default();
    for path in &transcripts {
        let sid = transcript_session_id(path).unwrap_or_else(|| {
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
        if config.verbose || config.dry_run {
            let parent = path
                .parent()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            println!("== {} ({})", sid, parent);
        }
        let (ex, st, sk) = process_transcript(path, config, graph.as_ref())?;
        stats.sessions += 1;
        stats.extracted += ex;
        stats.stored += st;
        stats.skipped += sk;
    }

    let mode = if config.dry_run { "dry-run" } else { "store" };
    println!(
        "\nextract-decisions [{}]: {} session(s), {} extracted, {} stored, {} skipped (dup)",
        mode, stats.sessions, stats.extracted, stats.stored, stats.skipped
    );
    if config.dry_run {
        println!("(dry run — nothing stored)");
    }
    Ok(stats)
}

// --- transcript discovery -------------------------------------------------

fn list_transcripts(root: &Path) -> Vec<PathBuf> {
    if !root.is_dir() {
        return Vec::new();
    }
    // Match the Python `*/*.jsonl` glob: one level under root.
    let mut out: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            if let Ok(files) = fs::read_dir(&dir) {
                for f in files.flatten() {
                    let p = f.path();
                    if p.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                        out.push(p);
                    }
                }
            }
        }
    }
    out.sort();
    out
}

fn transcript_session_id(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let rec: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(sid) = rec.get("sessionId").and_then(|v| v.as_str()) {
            if !sid.is_empty() {
                return Some(sid.to_string());
            }
        }
    }
    None
}

fn resolve_transcripts(config: &ExtractConfig) -> Result<Vec<PathBuf>> {
    if config.all {
        return Ok(list_transcripts(&config.transcripts_dir));
    }
    let sid = config
        .session_id
        .as_deref()
        .ok_or_else(|| anyhow!("pass a <session-id> or --all"))?;
    let matches: Vec<PathBuf> = list_transcripts(&config.transcripts_dir)
        .into_iter()
        .filter(|p| transcript_session_id(p).as_deref() == Some(sid))
        .collect();
    if matches.is_empty() {
        return Err(anyhow!(
            "no transcript found for session {:?} under {}",
            sid,
            config.transcripts_dir.display()
        ));
    }
    Ok(matches)
}

fn project_from_path(path: &Path) -> String {
    let name = path
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if name.is_empty() {
        return "unknown".to_string();
    }
    match name.rsplit_once('-') {
        Some((_, last)) if !last.is_empty() => last.to_string(),
        _ => "unknown".to_string(),
    }
}

// --- turn extraction ------------------------------------------------------

fn extract_turns(path: &Path) -> Result<Vec<Turn>> {
    let content =
        fs::read_to_string(path).with_context(|| format!("read transcript {}", path.display()))?;
    let mut turns: Vec<Turn> = Vec::new();
    let mut seq: i64 = 0;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let rec: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let msg = match rec.get("message").and_then(|v| v.as_object()) {
            Some(m) => m,
            None => continue,
        };
        let role = msg
            .get("role")
            .and_then(|v| v.as_str())
            .or_else(|| rec.get("type").and_then(|v| v.as_str()))
            .unwrap_or("?")
            .to_string();
        let content = match msg.get("content").and_then(|v| v.as_array()) {
            Some(a) => a,
            None => continue,
        };
        let mut parts: Vec<String> = Vec::new();
        for block in content {
            let btype = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match btype {
                "text" => {
                    if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                        parts.push(t.to_string());
                    }
                }
                "thinking" => {
                    if let Some(t) = block.get("thinking").and_then(|v| v.as_str()) {
                        parts.push(format!("[thinking] {}", t));
                    }
                }
                "tool_use" => {
                    let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                    parts.push(format!("[tool_use {}]", name));
                }
                _ => {}
            }
        }
        let text = parts
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string();
        if text.is_empty() {
            continue;
        }
        seq += 1;
        turns.push(Turn {
            sequence: seq,
            role,
            text,
        });
    }
    Ok(turns)
}

fn chunk_turns(turns: &[Turn], max_chars: usize) -> Vec<Vec<Turn>> {
    if turns.is_empty() {
        return Vec::new();
    }
    let mut chunks: Vec<Vec<Turn>> = Vec::new();
    let mut current: Vec<Turn> = Vec::new();
    let mut used = 0usize;
    for t in turns {
        // render cost: "[seq|role]\ntext" + 2 newlines separator ≈ digits + role + text + 4
        let cost = t.sequence.to_string().len() + t.role.len() + t.text.len() + 4;
        if !current.is_empty() && used + cost > max_chars {
            chunks.push(std::mem::take(&mut current));
            used = 0;
        }
        current.push(t.clone());
        used += cost;
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn render_chunk(chunk: &[Turn]) -> String {
    chunk
        .iter()
        .map(|t| format!("[{}|{}]\n{}", t.sequence, t.role, t.text))
        .collect::<Vec<_>>()
        .join("\n\n")
}

// --- LLM extraction -------------------------------------------------------

/// Call Ollama `/api/generate`. Returns `(response, thinking)` — thinking-mode
/// models (qwen3.5) emit the schema-forced JSON in `thinking` and leave
/// `response` empty; non-thinking models put it in `response`. The caller
/// parses both and unions.
fn call_ollama(model: &str, url: &str, prompt: &str) -> Result<(String, String)> {
    let resp: Value = ureq::post(url)
        .timeout(Duration::from_secs(600))
        .send_json(ureq::json!({
            "model": model,
            "prompt": prompt,
            "stream": false,
            "format": decision_schema(),
            "options": {"temperature": 0.1},
        }))
        .map_err(|e| anyhow!("ollama request failed ({}): {}", url, e))?
        .into_json()
        .context("ollama returned non-JSON envelope")?;
    let response = resp
        .get("response")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let thinking = resp
        .get("thinking")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if response.trim().is_empty() && thinking.trim().is_empty() {
        return Err(anyhow!(
            "ollama returned empty `response` and `thinking` fields"
        ));
    }
    Ok((response, thinking))
}

fn is_real_content(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() {
        return false;
    }
    if PLACEHOLDER_WORDS.contains(&s.to_lowercase().as_str()) {
        return false;
    }
    s.chars().any(|c| c.is_alphabetic())
}

#[derive(Deserialize)]
struct RawDecision {
    target: Option<String>,
    chosen: Option<String>,
    #[serde(default)]
    alternatives: Vec<String>,
    rationale: Option<String>,
    sequence: Option<Value>,
}

/// Parse the LLM's JSON into validated decisions. Accepts `{"decisions":[...]}`
/// or a bare `[...]`; strips markdown fences and prose wrappers. Drops any entry
/// failing the hallucination guard (target/chosen/rationale must each contain a
/// real alphabetic token).
fn parse_decision_json(raw: &str) -> Vec<Decision> {
    let text = raw.trim();
    let text = strip_fence(text);
    // Try direct parse, then fall back to the first {...} or [...] span.
    let parsed: Option<Value> = serde_json::from_str(&text)
        .ok()
        .or_else(|| first_json_span(&text).and_then(|span| serde_json::from_str(&span).ok()));
    let Some(obj) = parsed else {
        return Vec::new();
    };
    let arr = match &obj {
        Value::Object(m) => m.get("decisions").cloned().unwrap_or(Value::Array(vec![])),
        Value::Array(_) => obj,
        _ => return Vec::new(),
    };
    let Value::Array(items) = arr else {
        return Vec::new();
    };
    let mut out: Vec<Decision> = Vec::new();
    for d in items {
        let Ok(rd) = serde_json::from_value::<RawDecision>(d.clone()) else {
            continue;
        };
        let target = rd.target.unwrap_or_default().trim().to_string();
        let chosen = rd.chosen.unwrap_or_default().trim().to_string();
        let rationale = rd.rationale.unwrap_or_default().trim().to_string();
        if !(is_real_content(&target) && is_real_content(&chosen) && is_real_content(&rationale)) {
            continue;
        }
        let alternatives = rd
            .alternatives
            .into_iter()
            .map(|a| a.trim().to_string())
            .filter(|a| !a.is_empty())
            .collect();
        let sequence = match rd.sequence {
            Some(Value::Number(n)) => n.as_i64().unwrap_or(0),
            Some(Value::String(s)) => s.parse::<i64>().unwrap_or(0),
            _ => 0,
        };
        out.push(Decision {
            target,
            chosen,
            alternatives,
            rationale,
            sequence,
        });
    }
    out
}

fn strip_fence(text: &str) -> String {
    // ```json ... ``` or ``` ... ```
    let t = text.trim_start();
    if let Some(rest) = t.strip_prefix("```") {
        // optional language tag up to newline
        let rest = rest.trim_start_matches(|c: char| c.is_alphanumeric());
        let rest = rest.trim_start_matches('\n');
        if let Some(end) = rest.rfind("```") {
            return rest[..end].trim().to_string();
        }
        return rest.trim().to_string();
    }
    text.to_string()
}

fn first_json_span(text: &str) -> Option<String> {
    let t = text.trim();
    let start_obj = t.find('{');
    let start_arr = t.find('[');
    let start = match (start_obj, start_arr) {
        (Some(a), Some(b)) => a.min(b),
        (Some(a), None) | (None, Some(a)) => a,
        (None, None) => return None,
    };
    let open = t.as_bytes()[start];
    let close = if open == b'{' { b'}' } else { b']' };
    // find matching close (last occurrence is fine — JSON span)
    let bytes = t.as_bytes();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    for i in start..bytes.len() {
        let c = bytes[i];
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
            continue;
        }
        match c {
            b'"' => in_str = true,
            b'{' | b'[' => depth += 1,
            b'}' | b']' => {
                depth -= 1;
                if depth == 0 && c == close {
                    return Some(t[start..=i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

fn recover_sequence(decision: &Decision, chunk: &[Turn], default: i64) -> i64 {
    let needles = [
        decision.chosen.to_lowercase(),
        decision.rationale.to_lowercase(),
        decision.target.to_lowercase(),
    ];
    for turn in chunk {
        let hay = turn.text.to_lowercase();
        for needle in &needles {
            if needle.len() >= MIN_PHRASE {
                if hay.contains(&needle[..MIN_PHRASE.min(needle.len())]) {
                    return turn.sequence;
                }
                let tail_from = needle.len().saturating_sub(MIN_PHRASE);
                if hay.contains(&needle[tail_from..]) {
                    return turn.sequence;
                }
            }
        }
    }
    default
}

// --- heuristic extraction (no LLM, no network) ---------------------------

/// Common English stopwords used to derive a short target label from a chosen
/// clause. Deliberately tiny — this builds a slug, not a parse tree.
const STOPWORDS: &[&str] = &[
    "the", "a", "an", "and", "or", "but", "for", "to", "of", "in", "on", "with", "we", "i", "us",
    "let", "let's", "is", "are", "be", "will", "should", "would", "can", "could", "this", "that",
    "these", "those", "it", "its", "as", "at", "by", "from", "our", "their", "your", "my", "than",
    "then", "so", "if",
];

/// Phrases that mark a decision-in-progress. Matched case-insensitively as
/// substrings; the chosen clause follows the trigger up to the next clause
/// boundary or rationale marker. Order matters only for earliest-match; the
/// list is searched and the first hit in the sentence wins.
const DECISION_TRIGGERS: &[&str] = &[
    "i recommend",
    "i suggest we",
    "i suggest",
    "let's go with",
    "let us go with",
    "let's use",
    "let us use",
    "let's choose",
    "let us choose",
    "let's adopt",
    "let us adopt",
    "we should use",
    "we should go with",
    "we should adopt",
    "we'll go with",
    "we will go with",
    "we'll use",
    "we will use",
    "we'll adopt",
    "we will adopt",
    "we adopt",
    "i'll go with",
    "i will go with",
    "i'll use",
    "i will use",
    "i'll adopt",
    "i will adopt",
    "going with",
    "decided to",
    "decision is to",
    "the decision is",
    "chosen approach",
    "i'll choose",
    "we choose",
    "i choose",
];

/// Markers that introduce the rationale clause. The text after the first
/// matching marker is the rationale. A trigger sentence with NO rationale
/// marker is dropped — this is the precision filter that pays for the
/// heuristic's lower recall. Kept strict on purpose ("because"/"since"/
/// "so that"); looser markers like bare "to" would match almost every
/// sentence and drown the graph in false positives.
const RATIONALE_MARKERS: &[&str] = &[" because ", " since ", " so that "];

/// Split turn text into sentence-ish fragments on sentence + clause
/// terminators. Transcript prose is often loosely punctuated, so we also split
/// on newlines and semicolons. Fragments shorter than `MIN_PHRASE` are dropped
/// (they cannot carry a real decision).
fn sentences(text: &str) -> Vec<String> {
    text.split(['.', '!', '?', ';', '\n'])
        .map(|s| s.trim().to_string())
        .filter(|s| s.len() >= MIN_PHRASE)
        .collect()
}

/// First `n` non-stopword tokens of `s`, with surrounding punctuation stripped.
/// Used to build a short target label from a chosen clause when no explicit
/// "for <topic>" prefix is present.
fn first_content_words(s: &str, n: usize) -> String {
    s.split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_'))
        .filter(|w| !w.is_empty() && !STOPWORDS.contains(&w.to_lowercase().as_str()))
        .take(n)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Detect a "<prefix> <topic>," lead-in before the trigger (`for <topic>,`,
/// `on <topic>,`, `about <topic>,`, `regarding <topic>,`). Returns the topic
/// phrase when present, so the decision target is the subject under decision
/// rather than a slug derived from the chosen clause.
fn extract_target(sentence: &str, trigger: &str) -> Option<String> {
    let lower = sentence.to_lowercase();
    let trigger_idx = lower.find(trigger)?;
    let before = &sentence[..trigger_idx];
    let before_lower = before.to_lowercase();
    for prefix in ["for ", "on ", "about ", "regarding "] {
        if let Some(idx) = before_lower.rfind(prefix) {
            let tail = &before[idx + prefix.len()..];
            let phrase = tail.split([',', ':', '\n']).next().unwrap_or("").trim();
            if phrase.len() >= 3 && is_real_content(phrase) {
                let slug = first_content_words(phrase, 4);
                return Some(if slug.is_empty() {
                    phrase.to_string()
                } else {
                    slug
                });
            }
        }
    }
    None
}

/// Best-effort alternatives capture: an `instead of <X>` clause anywhere after
/// the trigger records X as the rejected option. Returns 0 or 1 entries — the
/// heuristic does not attempt to enumerate full option sets.
fn extract_alternatives(after_trigger: &str) -> Vec<String> {
    let lower = after_trigger.to_lowercase();
    let mut alts = Vec::new();
    if let Some(idx) = lower.find("instead of ") {
        let tail = &after_trigger[idx + "instead of ".len()..];
        let lower_tail = tail.to_lowercase();
        // Cut the rejected-option phrase at the first rationale marker or clause
        // boundary, so `instead of btree because …` yields `btree` not the rest.
        let mut end = tail.len();
        for marker in [
            " because ",
            " since ",
            " so that ",
            ",",
            ".",
            ";",
            ":",
            "\n",
        ] {
            if let Some(m) = lower_tail.find(marker) {
                if m < end {
                    end = m;
                }
            }
        }
        let phrase = tail[..end].trim();
        if is_real_content(phrase) {
            alts.push(phrase.to_string());
        }
    }
    alts
}

/// Pull a `Decision` out of one sentence using trigger + rationale-marker
/// rules. Returns `None` when no trigger fires, or when the chosen/rationale
/// clauses fail the hallucination guard (the same guard the LLM path uses).
fn extract_decision_from_sentence(sentence: &str, seq: i64) -> Option<Decision> {
    let lower = sentence.to_lowercase();
    // Earliest trigger present in the sentence.
    let mut trigger_hit: Option<(&str, usize)> = None;
    for t in DECISION_TRIGGERS {
        if let Some(idx) = lower.find(t) {
            match trigger_hit {
                Some((_, best)) if idx >= best => {}
                _ => trigger_hit = Some((t, idx)),
            }
        }
    }
    let (trigger, _) = trigger_hit?;
    let trigger_start = lower.find(trigger)?;
    let after = &sentence[trigger_start + trigger.len()..];
    let lower_after = after.to_lowercase();

    // chosen clause = from after the trigger up to the first rationale marker
    // or clause boundary, whichever comes first.
    let mut chosen_end = after.len();
    let mut rationale_from: Option<usize> = None;
    for marker in RATIONALE_MARKERS {
        if let Some(idx) = lower_after.find(marker) {
            if idx < chosen_end {
                chosen_end = idx;
                rationale_from = Some(idx + marker.len());
            }
        }
    }
    for sep in [", ", ": ", " — ", " – ", " - ", " instead of "] {
        if let Some(idx) = lower_after.find(sep) {
            if idx < chosen_end {
                chosen_end = idx;
            }
        }
    }
    let chosen = after[..chosen_end]
        .trim()
        .trim_end_matches(',')
        .trim()
        .to_string();
    let rationale = match rationale_from {
        Some(from) => after[from..]
            .trim()
            .trim_end_matches('.')
            .trim()
            .to_string(),
        None => String::new(),
    };
    if !is_real_content(&chosen) || !is_real_content(&rationale) {
        return None;
    }
    let target = extract_target(sentence, trigger).unwrap_or_else(|| {
        // No "for <topic>," prefix → derive a short target label. Prefer the
        // chosen clause's head content words, but fall back to the rationale's
        // head nouns when the chosen slug is degenerate (≤ its own words, i.e.
        // would equal `chosen` and carry no extra topic signal). This keeps
        // `target` distinct + informative for prefix-less decision sentences
        // like "I suggest we adopt HNSW because approximate nearest neighbor
        // is the access pattern" (target → "approximate nearest neighbor",
        // not a copy of `chosen`).
        let slug = first_content_words(&chosen, 4);
        if is_real_content(&slug) && slug.to_lowercase() != chosen.to_lowercase() {
            slug
        } else {
            let rslug = first_content_words(&rationale, 3);
            if is_real_content(&rslug) {
                rslug
            } else {
                chosen.clone()
            }
        }
    });
    let alternatives = extract_alternatives(after);
    Some(Decision {
        target,
        chosen,
        alternatives,
        rationale,
        sequence: seq,
    })
}

/// Rule-based decision extraction over a transcript's turns. No LLM, no network.
/// Decisions are emitted with their turn's real `sequence` (no recovery pass
/// needed). Within-session dedup on `(target, chosen)` mirrors the LLM path.
fn heuristic_extract(turns: &[Turn]) -> Vec<Decision> {
    let mut out: Vec<Decision> = Vec::new();
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    for turn in turns {
        for sent in sentences(&turn.text) {
            if let Some(d) = extract_decision_from_sentence(&sent, turn.sequence) {
                let key = (d.target.clone(), d.chosen.clone());
                if seen.insert(key) {
                    out.push(d);
                }
            }
        }
    }
    out
}

// --- store / pre-scan -----------------------------------------------------

/// Decisions already stored for `session_id` with their (source, target, chosen)
/// for dedup — read in-process via `recent_discoveries`.
fn existing_decisions(graph: &AtheneumGraph, session_id: &str) -> Vec<(String, String, String)> {
    let rows = graph
        .recent_discoveries(None, None, Some(session_id), Some("Decision"), 10000)
        .unwrap_or_default();
    rows.into_iter()
        .map(|e| {
            let d = &e.data;
            (
                d.get("source")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                d.get("target")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                d.get("chosen")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            )
        })
        .collect()
}

fn has_extract_decisions(existing: &[(String, String, String)], tag: &str) -> bool {
    existing.iter().any(|(src, _, _)| src == tag)
}

fn already_present(
    existing: &[(String, String, String)],
    target: &str,
    chosen: &str,
    tag: &str,
) -> bool {
    existing
        .iter()
        .any(|(src, t, c)| src == tag && t == target && c == chosen)
}

fn store_decision(
    graph: &AtheneumGraph,
    agent: &str,
    project: Option<&str>,
    session_id: &str,
    source_tag: &str,
    d: &Decision,
) -> Result<i64> {
    let mut metadata = json!({
        "chosen": d.chosen,
        "alternatives": d.alternatives,
        "rationale": d.rationale,
        "target": d.target,
        "session_id": session_id,
        "sequence": d.sequence,
        "source": source_tag,
        "file": null,
        "line": null,
    });
    if let Some(pid) = project {
        metadata["project_id"] = json!(pid);
    }
    let id = graph
        .store_discovery(agent, "Decision", &d.target, metadata)
        .context("store_discovery failed")?;
    Ok(id)
}

// --- driver ---------------------------------------------------------------

/// Process one transcript. Returns `(extracted, stored, skipped)`.
fn process_transcript(
    path: &Path,
    config: &ExtractConfig,
    graph: Option<&AtheneumGraph>,
) -> Result<(usize, usize, usize)> {
    let session_id = match transcript_session_id(path) {
        Some(s) => s,
        None => {
            if config.verbose {
                eprintln!("  skip {}: no sessionId", path.display());
            }
            return Ok((0, 0, 0));
        }
    };
    let project = config
        .project
        .clone()
        .unwrap_or_else(|| project_from_path(path));

    let existing = match graph {
        Some(g) => existing_decisions(g, &session_id),
        None => Vec::new(),
    };

    let tag = config.mode.source_tag();

    // Session-level idempotency: skip if the session already has a Decision
    // from this same backend (store mode, no --force). Distinct backends stay
    // separately resumable — a heuristic run will still process a session that
    // only has `llm-extract` rows, and vice versa.
    if !config.dry_run && !config.force && has_extract_decisions(&existing, tag) {
        if config.verbose {
            eprintln!(
                "  skip {}: already has {} decisions (--force to re-extract)",
                session_id, tag
            );
        }
        return Ok((0, 0, 0));
    }

    let turns = extract_turns(path)?;
    if turns.is_empty() {
        if config.verbose {
            eprintln!("  skip {}: no turns", session_id);
        }
        return Ok((0, 0, 0));
    }

    let extracted: Vec<Decision> = match config.mode {
        ExtractMode::Heuristic => {
            if config.verbose {
                eprintln!(
                    "  heuristic mode: no LLM — lower recall, some false positives; \
                     review with --dry-run first"
                );
            }
            heuristic_extract(&turns)
        }
        ExtractMode::Llm => {
            let chunks = chunk_turns(&turns, config.max_chars);
            let mut out: Vec<Decision> = Vec::new();
            for (ci, chunk) in chunks.iter().enumerate() {
                let body = render_chunk(chunk);
                let prompt = format!(
                    "{}\n\nSession id: {}\n\nTranscript turns (sequence | role | text):\n{}",
                    EXTRACTION_INSTRUCTION, session_id, body
                );
                if config.verbose {
                    eprintln!(
                        "  chunk {}/{} ({} turns)…",
                        ci + 1,
                        chunks.len(),
                        chunk.len()
                    );
                }
                let (response, thinking) = call_ollama(&config.model, &config.ollama_url, &prompt)?;
                let default_seq = chunk[0].sequence;
                for raw in [response.as_str(), thinking.as_str()] {
                    if raw.trim().is_empty() {
                        continue;
                    }
                    for mut d in parse_decision_json(raw) {
                        d.sequence = recover_sequence(&d, chunk, default_seq);
                        out.push(d);
                    }
                }
            }
            out
        }
    };

    // Within-run dedup on (target, chosen).
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    let mut unique: Vec<Decision> = Vec::new();
    for d in extracted {
        let key = (d.target.clone(), d.chosen.clone());
        if seen.contains(&key) {
            continue;
        }
        seen.insert(key);
        unique.push(d);
    }

    let mut stored = 0usize;
    let mut skipped = 0usize;
    for d in &unique {
        if !config.dry_run && already_present(&existing, &d.target, &d.chosen, tag) {
            skipped += 1;
            continue;
        }
        if config.dry_run {
            let out = json!({
                "target": d.target,
                "chosen": d.chosen,
                "alternatives": d.alternatives,
                "rationale": d.rationale,
                "sequence": d.sequence,
                "session_id": session_id,
                "source": tag,
            });
            println!("{}", serde_json::to_string(&out).unwrap_or_default());
            continue;
        }
        if let Some(g) = graph {
            let did = store_decision(g, &config.agent, Some(&project), &session_id, tag, d)?;
            stored += 1;
            if config.verbose {
                eprintln!(
                    "  stored decision #{}: {} (seq {})",
                    did, d.target, d.sequence
                );
            }
        }
    }
    Ok((unique.len(), stored, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_decision_json_accepts_wrapped_and_bare() {
        let wrapped = r#"{"decisions":[{"target":"storage engine","chosen":"CSR adjacency","alternatives":["btree"],"rationale":"scan-heavy","sequence":3}]}"#;
        let bare = r#"[{"target":"auth","chosen":"jwt","alternatives":[],"rationale":"stateless","sequence":1}]"#;
        let w = parse_decision_json(wrapped);
        let b = parse_decision_json(bare);
        assert_eq!(w.len(), 1);
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].target, "auth");
        assert_eq!(b[0].chosen, "jwt");
    }

    #[test]
    fn hallucination_guard_rejects_placeholders() {
        let raw = r#"{"decisions":[
            {"target":"...","chosen":"x","rationale":"y","sequence":1},
            {"target":"target","chosen":"chosen","rationale":"rationale","sequence":2},
            {"target":"real target","chosen":"real chosen","rationale":"real rationale","sequence":3}
        ]}"#;
        let out = parse_decision_json(raw);
        // Only the third entry survives: "..." has no alpha, "target"/"chosen"/
        // "rationale" are placeholder words.
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].target, "real target");
    }

    #[test]
    fn strip_fence_removes_markdown_json_fence() {
        let fenced = "```json\n{\"decisions\":[]}\n```";
        assert_eq!(strip_fence(fenced), "{\"decisions\":[]}");
    }

    #[test]
    fn first_json_span_finds_balanced_object() {
        let prose = "here is the answer: {\"decisions\":[{\"target\":\"a\"}]} trailing";
        let span = first_json_span(prose).unwrap();
        assert!(span.starts_with('{'));
        assert!(span.ends_with('}'));
        let v: Value = serde_json::from_str(&span).unwrap();
        assert!(v.get("decisions").unwrap().is_array());
    }

    #[test]
    fn recover_sequence_matches_chosen_phrase_back_to_turn() {
        let chunk = vec![
            Turn {
                sequence: 5,
                role: "assistant".into(),
                text: "irrelevant".into(),
            },
            Turn {
                sequence: 6,
                role: "assistant".into(),
                text: "We will adopt CSR adjacency for the read path".into(),
            },
        ];
        let d = Decision {
            target: "storage-engine".into(),
            chosen: "CSR adjacency".into(),
            alternatives: vec![],
            rationale: "scan-heavy".into(),
            sequence: 0,
        };
        // "CSR adjacency" is < MIN_PHRASE chars; rationale "scan-heavy" also short;
        // target "storage-engine" >= 12 chars and present in turn 6.
        assert_eq!(recover_sequence(&d, &chunk, 99), 6);
    }

    #[test]
    fn chunk_turns_respects_max_chars_but_keeps_oversize_turns() {
        let turns = vec![
            Turn {
                sequence: 1,
                role: "assistant".into(),
                text: "a".repeat(50),
            },
            Turn {
                sequence: 2,
                role: "assistant".into(),
                text: "b".repeat(50),
            },
            Turn {
                sequence: 3,
                role: "assistant".into(),
                text: "c".repeat(50),
            },
        ];
        let chunks = chunk_turns(&turns, 70);
        // Each turn costs ~50 + small; cap 70 → roughly one turn per chunk.
        assert!(chunks.len() >= 2);
        // No turn is dropped.
        let total: usize = chunks.iter().map(|c| c.len()).sum();
        assert_eq!(total, 3);
        // A single oversize turn becomes its own chunk rather than being split.
        let big = vec![Turn {
            sequence: 1,
            role: "assistant".into(),
            text: "x".repeat(1000),
        }];
        let one = chunk_turns(&big, 100);
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].len(), 1);
    }

    #[test]
    fn heuristic_extracts_decision_with_rationale_clause() {
        let turns = vec![Turn {
            sequence: 4,
            role: "assistant".into(),
            text: "For the storage engine, I recommend CSR adjacency because the read path is scan-heavy and btree point lookups dominate latency."
                .to_string(),
        }];
        let out = heuristic_extract(&turns);
        assert_eq!(out.len(), 1);
        let d = &out[0];
        assert_eq!(d.sequence, 4);
        assert_eq!(d.target, "storage engine");
        assert_eq!(d.chosen, "CSR adjacency");
        assert!(d.rationale.contains("scan-heavy"));
    }

    #[test]
    fn heuristic_drops_trigger_without_rationale() {
        // Trigger present but no because/since/so that → dropped (precision filter).
        let turns = vec![Turn {
            sequence: 1,
            role: "assistant".into(),
            text: "I recommend JWT for auth.".to_string(),
        }];
        assert!(heuristic_extract(&turns).is_empty());
    }

    #[test]
    fn heuristic_dedups_within_session() {
        let turns = vec![
            Turn {
                sequence: 1,
                role: "assistant".into(),
                text: "We should use CSR adjacency because scan-heavy.".to_string(),
            },
            Turn {
                sequence: 2,
                role: "assistant".into(),
                text: "We should use CSR adjacency because scan-heavy.".to_string(),
            },
        ];
        assert_eq!(heuristic_extract(&turns).len(), 1);
    }

    #[test]
    fn heuristic_source_tag_is_distinct_from_llm() {
        assert_eq!(ExtractMode::Llm.source_tag(), "llm-extract");
        assert_eq!(ExtractMode::Heuristic.source_tag(), "heuristic");
    }

    #[test]
    fn heuristic_captures_alternative_from_instead_of() {
        let sent =
            "Let's go with HNSW instead of btree because vector similarity is the access pattern.";
        let d = extract_decision_from_sentence(sent, 7).unwrap();
        assert_eq!(d.sequence, 7);
        assert_eq!(d.chosen, "HNSW");
        assert_eq!(d.alternatives, vec!["btree".to_string()]);
    }
}
