use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Result};
use atheneum::AtheneumGraph;
use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};
use sqlitegraph::GraphEntity;

const DEFAULT_K: usize = 5;
const DEFAULT_MAX_TOKENS: usize = 500;
const EPS: f64 = 1e-8;

const WEIGHT_BM25: f64 = 0.35;
const WEIGHT_TF_IDF: f64 = 0.25;
const WEIGHT_KIND: f64 = 0.15;
const WEIGHT_TRAJECTORY: f64 = 0.25;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HintCandidate {
    handle: u32,
    entity_id: i64,
    kind: String,
    name: String,
    score: f64,
    #[serde(default)]
    score_breakdown: HashMap<String, f64>,
    estimated_tokens: usize,
    skip_prob: f64,
    session_id: Option<String>,
    #[serde(default)]
    prefetch: bool,
    #[serde(default)]
    handle_kind: String,
}

#[derive(Debug, Clone, Serialize)]
struct TrajectoryNode {
    source_token: String,
    next_token: String,
    context_len: usize,
    trajectory: Vec<f32>,
}

impl PartialEq for TrajectoryNode {
    fn eq(&self, other: &Self) -> bool {
        self.source_token == other.source_token
            && self.next_token == other.next_token
            && self.context_len == other.context_len
            && self.trajectory.len() == other.trajectory.len()
    }
}

impl Eq for TrajectoryNode {}

impl PartialOrd for TrajectoryNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TrajectoryNode {
    fn cmp(&self, other: &Self) -> Ordering {
        let la = self.trajectory.len();
        let lb = other.trajectory.len();
        la.cmp(&lb)
            .then_with(|| self.source_token.cmp(&other.source_token))
            .then_with(|| self.next_token.cmp(&other.next_token))
    }
}

#[derive(Debug, Clone, Serialize)]
struct TrajectoryIndex {
    context_len: usize,
    nodes: Vec<TrajectoryNode>,
}

#[derive(Debug, Clone, Serialize)]
struct PrefetchHintResponse {
    query: String,
    candidates: Vec<HintCandidate>,
}

#[derive(Debug, Clone, PartialEq)]
struct RankedCandidate {
    handle: u32,
    entity_id: i64,
    kind: String,
    name: String,
    score: f64,
    estimated_tokens: usize,
    skip_prob: f64,
    session_id: Option<String>,
    score_breakdown: HashMap<String, f64>,
    inferred_next_tokens: usize,
}

fn trajectory_distance(a: &[f32], b: &[f32]) -> f32 {
    let mut d2: f32 = 0.0;
    for (x, y) in a.iter().zip(b.iter()) {
        let diff = x - y;
        d2 += diff * diff;
    }
    d2.sqrt()
}

fn most_common_next_token(nodes: &[TrajectoryNode]) -> Option<String> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for node in nodes {
        *counts.entry(node.next_token.as_str()).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(token, _)| token.to_string())
}

fn query_tokens(query: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_ascii_lowercase())
        .filter(|t| seen.insert(t.clone()))
        .collect()
}

fn build_term_freq(text: &str, tokens: &[String]) -> HashMap<String, usize> {
    let lower = text.to_ascii_lowercase();
    let mut counts = HashMap::new();
    for token in tokens {
        let mut start = 0usize;
        while let Some(pos) = lower[start..].find(token.as_str()) {
            let idx = start + pos;
            counts
                .entry(token.clone())
                .and_modify(|c| *c += 1)
                .or_insert(1);
            start = idx + 1;
        }
    }
    counts
}

fn compute_tf_idf_score(text: &str, tokens: &[String], idf: &HashMap<String, f64>) -> f64 {
    if tokens.is_empty() {
        return 0.0;
    }
    let term_freq = build_term_freq(text, tokens);
    if term_freq.is_empty() {
        return 0.0;
    }

    let max_tf = term_freq.values().copied().max().unwrap_or(0) as f64;
    if max_tf <= 0.0 {
        return 0.0;
    }

    let mut tf_idf_sum = 0.0f64;
    for (token, count) in &term_freq {
        let tf = (*count as f64) / max_tf;
        let idf_val = idf.get(token).copied().unwrap_or(EPS);
        tf_idf_sum += tf * idf_val;
    }

    (tf_idf_sum / tokens.len() as f64).clamp(0.0, 1.0)
}

fn compute_bm25_score(
    text: &str,
    tokens: &[String],
    idf: &HashMap<String, f64>,
    avg_len: f64,
) -> f64 {
    if tokens.is_empty() || avg_len <= 0.0 {
        return 0.0;
    }
    let term_freq = build_term_freq(text, tokens);
    if term_freq.is_empty() {
        return 0.0;
    }

    let doc_len = text.chars().count() as f64;
    let mut score = 0.0;
    for (token, count) in &term_freq {
        let idf_val = idf.get(token).copied().unwrap_or(EPS);
        let tf = *count as f64;
        let numerator = tf * (1.2 + 1.0);
        let denominator = tf + 1.2 * (1.0 - 1.2_f64.powi(2) + 1.2 * (doc_len / (avg_len + EPS)));
        score += idf_val * (numerator / denominator);
    }

    score.clamp(0.0, 1.0)
}

fn token_overlap(text: &str, tokens: &[String]) -> f64 {
    if tokens.is_empty() {
        return 0.0;
    }
    let lower = text.to_ascii_lowercase();
    let matched = tokens.iter().filter(|token| lower.contains(*token)).count();
    matched as f64 / tokens.len() as f64
}

fn kind_weight(kind: &str) -> f64 {
    match kind {
        "WikiPage" => 0.18,
        "Discovery" => 0.12,
        "Memory" => 0.10,
        "Planning" => 0.05,
        "ReasoningLog" => -0.08,
        "Event" => -0.10,
        "File" => -0.18,
        _ => 0.0,
    }
}

fn recency_weight(entity: &GraphEntity) -> f64 {
    let mut best: Option<f64> = None;
    for key in ["updated_at", "created_at", "timestamp"] {
        if let Some(value) = entity.data.get(key).and_then(|v| v.as_str()) {
            let parsed = DateTime::parse_from_rfc3339(value)
                .or_else(|_| {
                    value
                        .parse::<DateTime<FixedOffset>>()
                        .map(|dt| dt.to_utc().fixed_offset())
                })
                .ok();
            if let Some(dt) = parsed {
                let unix = dt.timestamp() as f64;
                best = Some(best.map(|b| b.max(unix)).unwrap_or(unix));
            }
        }
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    let age_hours = best.map(|t| (now - t) / 3600.0).unwrap_or(1e9);
    if age_hours <= 0.0 {
        0.12
    } else if age_hours <= 24.0 {
        0.10
    } else if age_hours <= 72.0 {
        0.05
    } else {
        0.0
    }
}

fn session_continuity_score(entity: &GraphEntity, session_ids: &HashSet<String>) -> f64 {
    let mut best: Option<f64> = None;
    for key in ["updated_at", "created_at", "timestamp"] {
        if let Some(value) = entity.data.get(key).and_then(|v| v.as_str()) {
            let parsed = DateTime::parse_from_rfc3339(value)
                .or_else(|_| {
                    value
                        .parse::<DateTime<FixedOffset>>()
                        .map(|dt| dt.to_utc().fixed_offset())
                })
                .ok();
            if let Some(dt) = parsed {
                let unix = dt.timestamp() as f64;
                best = Some(best.map(|b| b.max(unix)).unwrap_or(unix));
            }
        }
    }
    let unix = best.unwrap_or(0.0);
    let recency = if unix <= 0.0 {
        0.0
    } else {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let age_hours = (now - unix) / 3600.0;
        if age_hours <= 0.0 {
            0.18
        } else if age_hours <= 6.0 {
            0.16
        } else if age_hours <= 24.0 {
            0.10
        } else {
            0.0
        }
    };

    let session_bonus =
        if let Some(session_id) = entity.data.get("session_id").and_then(|v| v.as_str()) {
            if session_ids.contains(session_id) {
                0.12
            } else {
                0.0
            }
        } else {
            0.0
        };

    recency + session_bonus
}

fn candidate_text(entity: &GraphEntity) -> String {
    let mut parts = vec![entity.kind.clone(), entity.name.clone()];
    for key in [
        "summary",
        "title",
        "content",
        "target",
        "description",
        "path",
        "file_path",
        "body",
    ] {
        if let Some(value) = entity.data.get(key).and_then(|v| v.as_str()) {
            parts.push(value.to_string());
        }
    }
    parts.join(" ")
}

fn estimate_tokens(text: &str) -> usize {
    let chars = text.chars().count();
    ((chars as f64) / 3.5).ceil() as usize + 12
}

fn decode_u32_token(raw: u32) -> String {
    match std::char::from_u32(raw) {
        Some(ch) if !ch.is_control() => ch.to_string(),
        _ => format!("{raw}"),
    }
}

fn load_trajectory_index(path: Option<&Path>) -> Result<TrajectoryIndex> {
    let Some(path) = path else {
        return Ok(TrajectoryIndex {
            context_len: 4,
            nodes: Vec::new(),
        });
    };
    let bytes = std::fs::read(path)?;
    anyhow::ensure!(bytes.len() >= 28, "trajectory blob too short");
    if bytes.len() >= 28 && &bytes[..4] == b"PSF2" {
        let context_len = u64::from_le_bytes(bytes[4..12].try_into().unwrap()) as usize;
        let _feat_dim = u64::from_le_bytes(bytes[12..20].try_into().unwrap()) as usize;
        let n = u64::from_le_bytes(bytes[20..28].try_into().unwrap()) as usize;
        anyhow::ensure!(
            context_len <= 256,
            "trajectory context_len too large: {context_len}"
        );
        anyhow::ensure!(
            _feat_dim <= 4096,
            "trajectory feat_dim too large: {_feat_dim}"
        );
        anyhow::ensure!(n <= 65536, "trajectory node count too large: {n}");
        let mut nodes = Vec::with_capacity(n);
        let mut cursor = 28usize;
        for _ in 0..n {
            anyhow::ensure!(cursor + 32 <= bytes.len());
            let _corpus_position =
                u64::from_le_bytes(bytes[cursor..cursor + 8].try_into().unwrap());
            cursor += 8;
            let _valid_len =
                u64::from_le_bytes(bytes[cursor..cursor + 8].try_into().unwrap()) as usize;
            cursor += 8;
            let source_token = u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap());
            cursor += 4;
            let next_token = u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap());
            cursor += 4;
            let _next_token_logit =
                f32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap());
            cursor += 4;
            let dim = context_len * _feat_dim;
            anyhow::ensure!(cursor + dim * 4 <= bytes.len());
            let mut trajectory = Vec::with_capacity(dim);
            for _ in 0..dim {
                trajectory.push(f32::from_le_bytes(
                    bytes[cursor..cursor + 4].try_into().unwrap(),
                ));
                cursor += 4;
            }
            let source_token = decode_u32_token(source_token);
            let next_token = decode_u32_token(next_token);
            nodes.push(TrajectoryNode {
                source_token,
                next_token,
                context_len,
                trajectory,
            });
        }
        return Ok(TrajectoryIndex { context_len, nodes });
    }
    if bytes.len() >= 28 && &bytes[..4] == b"PSF1" {
        let context_len = u64::from_le_bytes(bytes[4..12].try_into().unwrap()) as usize;
        let _feat_dim = u64::from_le_bytes(bytes[12..20].try_into().unwrap()) as usize;
        let n = u64::from_le_bytes(bytes[20..28].try_into().unwrap()) as usize;
        anyhow::ensure!(
            context_len <= 256,
            "trajectory context_len too large: {context_len}"
        );
        anyhow::ensure!(
            _feat_dim <= 4096,
            "trajectory feat_dim too large: {_feat_dim}"
        );
        anyhow::ensure!(n <= 65536, "trajectory node count too large: {n}");
        let mut nodes = Vec::with_capacity(n);
        let mut cursor = 28usize;
        for _ in 0..n {
            anyhow::ensure!(cursor + 24 <= bytes.len());
            let _corpus_position =
                u64::from_le_bytes(bytes[cursor..cursor + 8].try_into().unwrap());
            cursor += 8;
            let _valid_len =
                u64::from_le_bytes(bytes[cursor..cursor + 8].try_into().unwrap()) as usize;
            cursor += 8;
            let source_token = u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap());
            cursor += 4;
            let next_token = u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap());
            cursor += 4;
            let _next_token_logit =
                f32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap());
            cursor += 4;
            let dim = context_len.min(_feat_dim.max(1));
            anyhow::ensure!(cursor + dim * 4 <= bytes.len());
            let mut trajectory = Vec::with_capacity(dim);
            for _ in 0..dim {
                trajectory.push(f32::from_le_bytes(
                    bytes[cursor..cursor + 4].try_into().unwrap(),
                ));
                cursor += 4;
            }
            let source_token = decode_u32_token(source_token);
            let next_token = decode_u32_token(next_token);
            nodes.push(TrajectoryNode {
                source_token,
                next_token,
                context_len,
                trajectory,
            });
        }
        return Ok(TrajectoryIndex { context_len, nodes });
    }
    bail!("unsupported trajectory format");
}

fn build_trajectory_from_bytes(query_trajectory: &[f32], context_len: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(context_len * query_trajectory.len());
    for value in query_trajectory {
        out.extend(std::iter::repeat_n(*value, context_len.max(1)));
    }
    out
}

fn query_candidates(
    index: &TrajectoryIndex,
    query_source_token: &str,
    query_trajectory: &[f32],
    accessible_count: usize,
) -> Vec<String> {
    if index.nodes.is_empty() || query_trajectory.is_empty() {
        return Vec::new();
    }
    let limit = accessible_count.min(8);
    let mut candidates: Vec<(&TrajectoryNode, f32)> = Vec::new();
    for node in index.nodes.iter() {
        if node.source_token == query_source_token {
            candidates.push((
                node,
                trajectory_distance(query_trajectory, &node.trajectory),
            ));
        }
    }
    candidates.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal));
    let mut seen: HashSet<&str> = HashSet::new();
    for (node, _) in candidates.into_iter().take(limit * 4) {
        if seen.insert(node.next_token.as_str())
            && (node.next_token == "__eos__"
                || node.next_token == "__pick__"
                || node.next_token == "__skip__")
        {
            break;
        }
    }
    let mut out: Vec<String> = seen.into_iter().map(String::from).collect();
    out.truncate(limit);
    out
}

#[derive(Debug, Clone)]
struct TfIdfStats {
    idf: HashMap<String, f64>,
    avg_len: f64,
}

fn compute_tf_idf_stats(entity_texts: &[String], tokens_per_doc: &[Vec<String>]) -> TfIdfStats {
    let n = entity_texts.len() as f64;
    let mut df: HashMap<String, usize> = HashMap::new();
    let mut total_len = 0usize;

    for (text, tokens) in entity_texts.iter().zip(tokens_per_doc.iter()) {
        total_len += text.chars().count();
        let mut seen = HashSet::new();
        for token in tokens {
            if seen.insert(token.clone()) {
                *df.entry(token.clone()).or_default() += 1;
            }
        }
    }

    let mut idf = HashMap::new();
    for (token, doc_freq) in df {
        let val = ((n / (doc_freq as f64 + EPS)) + 1.0).ln();
        idf.insert(token, val);
    }

    let avg_len = if entity_texts.is_empty() {
        0.0
    } else {
        total_len as f64 / entity_texts.len() as f64
    };

    TfIdfStats { idf, avg_len }
}

/// Return type of [`parse_args`]: graph handle, query string, k, max-tokens,
/// trajectory index, trajectory query vector, and session id.
type ParsedArgs = (
    AtheneumGraph,
    String,
    usize,
    usize,
    TrajectoryIndex,
    Vec<f32>,
    String,
);

fn parse_args(args: &[String]) -> Result<ParsedArgs> {
    if args.len() < 3 {
        bail!("Usage: atheneum memory-prefetch-hints <db-path> --query <query> [--k 5] [--max-tokens 500] [--trajectory prefix.bin] [--trajectory-query t1,t2,...] [--session-id SESSION_ID]\nProvided args: {args:?}");
    }
    let db_path = PathBuf::from(&args[1]);
    let mut query = String::new();
    let mut k = DEFAULT_K;
    let mut max_tokens = DEFAULT_MAX_TOKENS;
    let mut trajectory_path: Option<PathBuf> = None;
    let mut trajectory_query_tokens: Vec<f32> = Vec::new();
    let mut session_id = String::new();

    let mut idx = 2usize;
    while idx < args.len() {
        match args[idx].as_str() {
            "--query" => {
                idx += 1;
                query = args
                    .get(idx)
                    .ok_or_else(|| anyhow!("missing value for --query"))?
                    .clone();
            }
            "--k" => {
                idx += 1;
                k = args
                    .get(idx)
                    .ok_or_else(|| anyhow!("missing value for --k"))?
                    .parse()?;
            }
            "--max-tokens" => {
                idx += 1;
                max_tokens = args
                    .get(idx)
                    .ok_or_else(|| anyhow!("missing value for --max-tokens"))?
                    .parse()?;
            }
            "--trajectory" => {
                idx += 1;
                trajectory_path = Some(PathBuf::from(
                    args.get(idx)
                        .ok_or_else(|| anyhow!("missing value for --trajectory"))?,
                ));
            }
            "--trajectory-query" => {
                idx += 1;
                let raw = args
                    .get(idx)
                    .ok_or_else(|| anyhow!("missing value for --trajectory-query"))?;
                trajectory_query_tokens = raw
                    .split(',')
                    .filter_map(|v| v.parse::<f32>().ok())
                    .collect();
            }
            "--session-id" => {
                idx += 1;
                session_id = args
                    .get(idx)
                    .ok_or_else(|| anyhow!("missing value for --session-id"))?
                    .clone();
            }
            other => bail!("Unexpected argument: {other}"),
        }
        idx += 1;
    }
    if query.is_empty() {
        bail!("--query is required");
    }
    let graph = AtheneumGraph::open(&db_path)?;
    let trajectory_index = load_trajectory_index(trajectory_path.as_deref())?;
    Ok((
        graph,
        query,
        k,
        max_tokens,
        trajectory_index,
        trajectory_query_tokens,
        session_id,
    ))
}

fn memory_search(graph: &AtheneumGraph, tokens: &[String], k: usize) -> Result<Vec<GraphEntity>> {
    let sql_entity_kind = "Memory";
    let mut rows: Vec<(f64, GraphEntity)> = Vec::new();
    graph.with_raw_connection(|conn| -> Result<()> {
        let mut stmt = conn.prepare(
            "SELECT id, kind, name, file_path, data, session_id FROM graph_entities WHERE kind = ?1 AND data IS NOT NULL ORDER BY id DESC LIMIT ?2",
        )?;
        let iter = stmt.query_map(rusqlite::params![sql_entity_kind, (k * 4) as i64], |row| {
            let data_str: String = row.get(4)?;
            let mut data: serde_json::Value = serde_json::from_str(&data_str).unwrap_or_default();
            if let Some(session_id) = row.get::<_, Option<String>>(5)? {
                data["session_id"] = serde_json::Value::String(session_id);
            }
            Ok(GraphEntity {
                id: row.get(0)?,
                kind: row.get(1)?,
                name: row.get(2)?,
                file_path: row.get(3)?,
                data,
            })
        })?;
        for entity in iter {
            let entity = entity?;
            let text = candidate_text(&entity);
            let score = token_overlap(&text, tokens)
                + kind_weight(entity.kind.as_str())
                + recency_weight(&entity);
            rows.push((score, entity));
        }
        Ok(())
    })?;
    rows.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal));
    Ok(rows.into_iter().take(k).map(|(_, e)| e).collect())
}

fn run(args: Vec<String>) -> Result<()> {
    let (graph, query, k, max_tokens, trajectory_index, trajectory_query_tokens, session_id) =
        parse_args(&args)?;
    let tokens = query_tokens(&query);
    let entities = memory_search(&graph, &tokens, k * 4)?;

    let mut entity_texts = Vec::with_capacity(entities.len());
    let mut tokens_per_doc = Vec::with_capacity(entities.len());
    for entity in &entities {
        let text = candidate_text(entity);
        entity_texts.push(text.clone());
        tokens_per_doc.push(query_tokens(&text));
    }
    let TfIdfStats { idf, avg_len } = compute_tf_idf_stats(&entity_texts, &tokens_per_doc);

    let current_session_ids: HashSet<String> = if session_id.is_empty() {
        entities
            .iter()
            .filter_map(|e| {
                e.data
                    .get("session_id")
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
            })
            .collect()
    } else {
        std::iter::once(session_id).collect()
    };

    let query_vec = if trajectory_query_tokens.is_empty() || trajectory_index.nodes.is_empty() {
        Vec::new()
    } else {
        build_trajectory_from_bytes(
            &trajectory_query_tokens,
            trajectory_index.context_len.max(1),
        )
    };

    let mut candidates: Vec<RankedCandidate> = Vec::new();
    for entity in entities {
        let text = candidate_text(&entity);
        let bm25 = compute_bm25_score(&text, &tokens, &idf, avg_len);
        let tf_idf = compute_tf_idf_score(&text, &tokens, &idf);
        let recency = recency_weight(&entity);
        let session_continuity = session_continuity_score(&entity, &current_session_ids);

        let estimated_tokens = estimate_tokens(&text);
        let mut score = bm25 * WEIGHT_BM25
            + tf_idf * WEIGHT_TF_IDF
            + recency
            + session_continuity
            + kind_weight(entity.kind.as_str()) * WEIGHT_KIND;

        let inferred_next = if query_vec.is_empty() || trajectory_index.nodes.is_empty() {
            Vec::new()
        } else {
            let source = tokens.first().map(|s| s.as_str()).unwrap_or_default();
            query_candidates(&trajectory_index, source, &query_vec, 4)
        };
        if !inferred_next.is_empty() {
            score += WEIGHT_TRAJECTORY;
        }

        let name = if inferred_next.is_empty() {
            entity.name.clone()
        } else {
            let common = most_common_next_token(
                &trajectory_index
                    .nodes
                    .iter()
                    .filter(|node| inferred_next.contains(&node.next_token))
                    .cloned()
                    .collect::<Vec<_>>(),
            )
            .unwrap_or_default();
            if common.is_empty() {
                entity.name.clone()
            } else {
                format!("{} | next={}", entity.name, common)
            }
        };

        let handle = entity.id as u32;

        let mut breakdown = HashMap::new();
        breakdown.insert("bm25".to_string(), bm25);
        breakdown.insert("tf_idf".to_string(), tf_idf);
        breakdown.insert("recency".to_string(), recency);
        breakdown.insert("session_continuity".to_string(), session_continuity);
        breakdown.insert(
            "kind_weight".to_string(),
            kind_weight(entity.kind.as_str()) * 0.15,
        );
        breakdown.insert(
            "trajectory_bonus".to_string(),
            if inferred_next.is_empty() {
                0.0
            } else {
                WEIGHT_TRAJECTORY
            },
        );

        candidates.push(RankedCandidate {
            handle,
            entity_id: entity.id,
            kind: entity.kind,
            name,
            score,
            estimated_tokens,
            skip_prob: 0.0,
            session_id: entity
                .data
                .get("session_id")
                .and_then(|v| v.as_str().map(|s| s.to_string())),
            score_breakdown: breakdown,
            inferred_next_tokens: inferred_next.len(),
        });
    }

    candidates.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));
    let mut used_tokens = 0usize;
    let mut preview: Vec<HintCandidate> = Vec::new();
    for candidate in candidates.into_iter().take(k) {
        if used_tokens + candidate.estimated_tokens > max_tokens {
            break;
        }
        used_tokens += candidate.estimated_tokens;
        preview.push(HintCandidate {
            handle: candidate.handle,
            entity_id: candidate.entity_id,
            kind: candidate.kind,
            name: candidate.name,
            score: candidate.score,
            score_breakdown: candidate.score_breakdown,
            estimated_tokens: candidate.estimated_tokens,
            skip_prob: candidate.skip_prob,
            session_id: candidate.session_id,
            prefetch: candidate.inferred_next_tokens > 0,
            handle_kind: if candidate.inferred_next_tokens > 0 {
                "trajectory"
            } else {
                "entity"
            }
            .to_string(),
        });
    }
    if preview.is_empty() && !tokens.is_empty() {
        preview.push(HintCandidate {
            handle: 0,
            entity_id: 0,
            kind: "empty".to_string(),
            name: "no-prefetch".to_string(),
            score: 0.0,
            score_breakdown: HashMap::new(),
            estimated_tokens: 0,
            skip_prob: 0.0,
            session_id: None,
            prefetch: false,
            handle_kind: "empty".to_string(),
        });
    }
    let response = PrefetchHintResponse {
        query,
        candidates: preview,
    };
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}

fn main() {
    // argv is only parsed as CLI flags for this offline query tool; it is
    // never relied upon for any security decision (the rule's documented
    // concern), so the security-args finding does not apply here.
    // nosemgrep: rust.lang.security.args.args
    if let Err(err) = run(std::env::args().collect()) {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod preview_tests {
    use super::*;

    #[test]
    fn tf_idf_scoring_prefers_exact_term_match() {
        let text = "activation memory activation test store this as memory".to_string();
        let tokens = query_tokens(&text);
        let docs = vec![text.clone(), "unrelated bank job planning".to_string()];
        let tokens_per_doc = vec![tokens.clone(), query_tokens(&docs[1])];
        let stats = compute_tf_idf_stats(&docs, &tokens_per_doc);
        let score = compute_tf_idf_score(&text, &tokens, &stats.idf);
        assert!(score > 0.2, "expected meaningful tf-idf score, got {score}");
    }

    #[test]
    fn bm25_scoring_handles_rare_terms() {
        let text_a = "memory activation test rare precise exact match rare";
        let text_b = "lorem ipsum dolor sit amet repeated repeated repeated common common common";
        let tokens_a = query_tokens(text_a);
        let tokens_b = query_tokens(text_b);
        let docs = vec![text_a.to_string(), text_b.to_string()];
        let tokens_per_doc = vec![tokens_a.clone(), tokens_b.clone()];
        let stats = compute_tf_idf_stats(&docs, &tokens_per_doc);
        let score_a = compute_bm25_score(text_a, &tokens_a, &stats.idf, stats.avg_len);
        let score_b = compute_bm25_score(text_b, &tokens_b, &stats.idf, stats.avg_len);
        assert!(
            score_a >= score_b,
            "expected BM25 to prefer rarer-term doc: {score_a} < {score_b}"
        );
    }

    #[test]
    fn session_continuity_score_favors_recent_same_session() {
        let mut matched = GraphEntity {
            id: 1,
            kind: "Memory".to_string(),
            name: "test".to_string(),
            file_path: None,
            data: serde_json::json!({"session_id": "session-1", "updated_at": "2026-07-21T12:00:00+00:00"}),
        };
        let mut sessions = HashSet::new();
        sessions.insert("session-1".to_string());
        let matched_score = session_continuity_score(&matched, &sessions);
        assert!(
            matched_score > 0.15,
            "expected bonus for same session, got {matched_score}"
        );

        matched.data = serde_json::json!({"updated_at": "2025-01-01T00:00:00+00:00"});
        let old_score = session_continuity_score(&matched, &sessions);
        assert!(
            old_score < matched_score,
            "expected older non-session fallback to score lower"
        );

        let no_session = GraphEntity {
            id: 2,
            kind: "Memory".to_string(),
            name: "no-session".to_string(),
            file_path: None,
            data: serde_json::json!({"updated_at": "2026-07-21T12:00:00+00:00"}),
        };
        let baseline = session_continuity_score(&no_session, &sessions);
        let delta = matched_score - baseline;
        assert!(
            (delta - 0.12).abs() < 1e-12,
            "expected exact session bonus 0.12, got delta {delta}"
        );
    }

    #[test]
    fn trajectory_distance_same_vector_is_zero() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![1.0, 2.0, 3.0];
        assert!(trajectory_distance(&a, &b) < 1e-6);
    }

    #[test]
    fn skip_gate_zero_weights_is_one_half() {
        let features = [0.0_f64; 4];
        let weights = [0.0_f64; 4];
        let mean = [0.0_f64; 4];
        let std = [1.0_f64; 4];
        let mut z = 0.0_f64;
        for ((w, x), (m, s)) in weights
            .iter()
            .zip(features.iter())
            .zip(mean.iter().zip(std.iter()))
        {
            let std_x = if *s == 0.0 { 0.0 } else { (*x - m) / s };
            z += w * std_x;
        }
        let p = 1.0 / (1.0 + (-z).exp());
        assert!((p - 0.5).abs() < 1e-6);
    }
}
