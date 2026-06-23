//! Chat navigation surface (Phase 2): a token-budgeted, directional, filterable,
//! paginated reader over the Phase-1 chat schema (generated columns + FTS5).
//!
//! This reads chat turns already ingested by the existing evidence path
//! (`record_evidence_prompt` / `record_evidence_tool_call`) — it captures nothing
//! new. The four generated columns (`session_id`, `sequence`, `role`,
//! `content_text`) and the `entity_fts` virtual table (migration v12) back
//! every query here, so lookups are index-backed and `--search` is sub-ms.
//!
//! Token accounting reuses the navigation tokenizer's constant
//! (`CHARS_PER_TOKEN = 4`, see `graph/navigation.rs`) so budget numbers line up
//! with `thread` / `session-digest`. The `--walk` flag does a bounded BFS along
//! the `caused_by` / `led_to` decision edges (see `graph/discovery.rs`) and
//! attaches a one-line chain snippet per emitted turn.

use std::collections::{HashSet, VecDeque};

use anyhow::{anyhow, bail, Result};
use rusqlite::params_from_iter;
use serde::{Deserialize, Serialize};

use super::AtheneumGraph;

/// Characters per token — matches `graph/navigation.rs` for consistency.
const CHARS_PER_TOKEN: usize = 4;

/// Chat kinds that carry a `content_text` generated column + FTS index entry.
const ALLOWED_KINDS: &[&str] = &["ReasoningLog", "ToolCall"];

/// Default turn page when no `--limit` is given. Large enough that a
/// token-budget walk over a normal session rarely needs a second round, small
/// enough to never load a runaway transcript.
const DEFAULT_TURN_PAGE: i64 = 200;

/// `--walk` BFS bounds. Depth caps the chain length; node count caps the
/// snippet size so a densely-linked session can't blow up the output.
const WALK_MAX_DEPTH: usize = 8;
const WALK_MAX_NODES: usize = 32;

/// Direction of chat traversal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatDirection {
    /// Most-recent first (`sequence DESC`) — the default.
    Recent,
    /// Chronological order (`sequence ASC`).
    Chronological,
}

impl ChatDirection {
    /// Parse the `--direction` value; unknown values are a hard error.
    pub fn parse(raw: &str) -> Result<Self> {
        match raw {
            "recent" => Ok(Self::Recent),
            "chrono" => Ok(Self::Chronological),
            other => Err(anyhow!(
                "invalid --direction `{other}` (expected `recent` or `chrono`)"
            )),
        }
    }

    fn order(self) -> &'static str {
        match self {
            Self::Recent => "DESC",
            Self::Chronological => "ASC",
        }
    }
}

/// Inputs to [`AtheneumGraph::query_chat`]. All user-controlled strings are
/// bound as SQL parameters by the query builder; nothing is interpolated.
#[derive(Debug, Clone)]
pub struct ChatQuery {
    pub session_id: String,
    /// Extractive token budget. Rows are emitted in order until this is reached
    /// (the first row is always emitted even if it alone exceeds the budget).
    pub tokens: usize,
    pub direction: ChatDirection,
    /// Subset of `ReasoningLog` / `ToolCall`. Empty = both.
    pub kinds: Vec<String>,
    /// Filter `ReasoningLog` rows by `role`. Ignored for `ToolCall`.
    pub role: Option<String>,
    /// `entity_fts MATCH` query. `None` = no FTS filter.
    pub search: Option<String>,
    /// When true, read the `discoveries` decision log instead of chat turns.
    pub only_decisions: bool,
    pub offset: i64,
    /// Page cap. `None` = [`DEFAULT_TURN_PAGE`].
    pub limit: Option<i64>,
    /// Attach a `caused_by` / `led_to` chain snippet to each emitted turn.
    pub walk: bool,
}

/// One chat turn in a [`ChatReport`].
#[derive(Debug, Clone, Serialize)]
pub struct ChatTurn {
    pub id: i64,
    pub kind: String,
    pub role: Option<String>,
    pub sequence: Option<i64>,
    /// The FTS-indexed derived text (content_summary / content / tool_name +
    /// tool_category coalesce, per migration v12). May be empty for a ToolCall
    /// with no searchable text — the renderer falls back to `kind` + `data`.
    pub content_text: String,
    pub data: serde_json::Value,
    /// Estimated tokens contributed by this turn's `content_text`.
    pub tokens: usize,
    /// `--walk` chain; empty when `walk` is false or no chain edges exist.
    pub chain: Vec<ChatChainNode>,
}

/// A node in a `--walk` chain snippet.
#[derive(Debug, Clone, Serialize)]
pub struct ChatChainNode {
    pub id: i64,
    pub kind: String,
    pub name: String,
    /// The edge type that reached this node (`caused_by` or `led_to`).
    pub via: String,
}

/// A decision row from the `discoveries` table (`--only-decisions`).
#[derive(Debug, Clone, Serialize)]
pub struct ChatDecision {
    pub id: i64,
    pub target: String,
    pub metadata: serde_json::Value,
    pub created_at: String,
    /// `--walk` chain along `caused_by` / `led_to`; empty when `walk` is false.
    pub chain: Vec<ChatChainNode>,
}

/// A bounded chat report.
#[derive(Debug, Clone, Serialize)]
pub struct ChatReport {
    pub session_id: String,
    pub direction: String,
    pub offset: i64,
    pub token_budget: usize,
    pub token_total: usize,
    /// More rows exist beyond the emitted window (budget cutoff or page end).
    pub has_more: bool,
    pub turns: Vec<ChatTurn>,
    /// Populated only when `only_decisions` is true.
    pub decisions: Vec<ChatDecision>,
}

impl AtheneumGraph {
    /// Navigate chat turns (or the decision log) for a session, bounded by the
    /// query's token budget and page window.
    pub fn query_chat(&self, query: ChatQuery) -> Result<ChatReport> {
        let kinds = sanitize_kinds(&query.kinds)?;
        if query.only_decisions {
            return self.query_decisions(&query);
        }

        let direction = query.direction;
        let limit = query.limit.unwrap_or(DEFAULT_TURN_PAGE).max(1);
        // Fetch one extra row to detect `has_more` without a second query.
        let fetch = limit + 1;

        let mut turns: Vec<ChatTurn> = self.with_raw_connection(|conn| {
            // Build the WHERE clause with bare `?` placeholders only (no mixed
            // `?N` numbering — rusqlite counts anonymous placeholders in order).
            // The only text interpolated into the SQL is the `?` count for the
            // IN list and the enum-derived sort direction; all user data is bound.
            let placeholders = kinds.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let mut where_clauses = vec!["session_id = ?".to_string()];
            where_clauses.push(format!("kind IN ({placeholders})"));
            if query.role.is_some() {
                where_clauses.push("role = ?".to_string());
            }
            if query.search.is_some() {
                where_clauses.push(
                    "id IN (SELECT rowid FROM entity_fts WHERE entity_fts MATCH ?)".to_string(),
                );
            }
            where_clauses.push("sequence IS NOT NULL".to_string());
            let sql = format!(
                "SELECT id, kind, role, sequence, content_text, data
                 FROM graph_entities
                 WHERE {}
                 ORDER BY sequence {} LIMIT ? OFFSET ?",
                where_clauses.join(" AND "),
                direction.order()
            );
            // Params in placeholder order: session, kinds..., role?, search?, fetch, offset.
            let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
            params.push(Box::new(query.session_id.clone()));
            for k in &kinds {
                params.push(Box::new(k.clone()));
            }
            if let Some(role) = &query.role {
                params.push(Box::new(role.clone()));
            }
            if let Some(search) = &query.search {
                params.push(Box::new(search.clone()));
            }
            params.push(Box::new(fetch));
            params.push(Box::new(query.offset));

            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params_from_iter(params.iter()), |r| {
                let data_text: String = r.get(5)?;
                let data: serde_json::Value =
                    serde_json::from_str(&data_text).unwrap_or(serde_json::Value::Null);
                Ok(ChatTurn {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    role: r.get(2)?,
                    sequence: r.get(3)?,
                    content_text: r.get(4)?,
                    data,
                    tokens: 0,
                    chain: Vec::new(),
                })
            })?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok::<_, anyhow::Error>(out)
        })?;

        // has_more: we fetched `limit + 1`; if more than `limit` came back,
        // rows beyond the page exist.
        let has_more_page = turns.len() as i64 > limit;
        if has_more_page {
            turns.truncate(limit as usize);
        }

        // Extractive token budget: walk in order, accumulate, stop at budget.
        // Always emit at least the first row so the user sees something.
        let mut token_total: usize = 0;
        let mut emitted = 0usize;
        for turn in &turns {
            let row_tokens = estimate_tokens(&turn.content_text);
            if emitted > 0 && token_total + row_tokens >= query.tokens {
                break;
            }
            token_total += row_tokens;
            emitted += 1;
        }
        let budget_cutoff = emitted < turns.len();
        turns.truncate(emitted);

        // Attach --walk chains if requested.
        if query.walk {
            for turn in turns.iter_mut() {
                turn.chain = self.chat_walk_chain(turn.id)?;
            }
        }

        Ok(ChatReport {
            session_id: query.session_id,
            direction: direction.order().to_lowercase(),
            offset: query.offset,
            token_budget: query.tokens,
            token_total,
            has_more: has_more_page || budget_cutoff,
            turns,
            decisions: Vec::new(),
        })
    }

    /// `--only-decisions` path: read the `discoveries` decision log for the
    /// session, paginated. Decisions are not token-budgeted (they're a separate,
    /// sparse log); `has_more` reflects the page boundary.
    fn query_decisions(&self, query: &ChatQuery) -> Result<ChatReport> {
        let limit = query.limit.unwrap_or(50).max(1);
        let fetch = limit + 1;
        let mut decisions: Vec<ChatDecision> = self.with_raw_connection(|conn| {
            // Direction is enum-derived ("DESC"/"ASC") — safe to interpolate,
            // and SQL does not accept a sort direction as a bound parameter.
            let dir = if query.direction == ChatDirection::Recent {
                "DESC"
            } else {
                "ASC"
            };
            let sql = format!(
                "SELECT id, target, metadata, created_at
                 FROM discoveries
                 WHERE discovery_type = 'Decision' AND session_id = ?
                 ORDER BY created_at {dir} LIMIT ? OFFSET ?"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(
                rusqlite::params![query.session_id, fetch, query.offset],
                |r| {
                    let metadata_text: String = r.get(2)?;
                    let metadata: serde_json::Value =
                        serde_json::from_str(&metadata_text).unwrap_or(serde_json::Value::Null);
                    Ok(ChatDecision {
                        id: r.get(0)?,
                        target: r.get(1)?,
                        metadata,
                        created_at: r.get(3)?,
                        chain: Vec::new(),
                    })
                },
            )?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok::<_, anyhow::Error>(out)
        })?;

        let has_more = decisions.len() as i64 > limit;
        if has_more {
            decisions.truncate(limit as usize);
        }

        if query.walk {
            for decision in decisions.iter_mut() {
                decision.chain = self.chat_walk_chain(decision.id)?;
            }
        }

        Ok(ChatReport {
            session_id: query.session_id.clone(),
            direction: query.direction.order().to_lowercase(),
            offset: query.offset,
            token_budget: query.tokens,
            token_total: 0,
            has_more,
            turns: Vec::new(),
            decisions,
        })
    }

    /// Bounded BFS along `caused_by` / `led_to` edges from a chat-turn entity.
    /// Returns the chain (excluding the start node) in BFS order, capped by
    /// [`WALK_MAX_DEPTH`] and [`WALK_MAX_NODES`]. Cycles are skipped via a
    /// visited set. An empty result (no chain edges) is not an error.
    fn chat_walk_chain(&self, start_id: i64) -> Result<Vec<ChatChainNode>> {
        self.with_raw_connection(|conn| {
            let mut visited: HashSet<i64> = HashSet::new();
            visited.insert(start_id);
            let mut queue: VecDeque<(i64, usize, String)> = VecDeque::new();
            queue.push_back((start_id, 0, String::new()));
            let mut chain: Vec<ChatChainNode> = Vec::new();
            while let Some((id, depth, via)) = queue.pop_front() {
                if chain.len() >= WALK_MAX_NODES {
                    break;
                }
                if id != start_id {
                    let (kind, name): (String, String) = conn.query_row(
                        "SELECT kind, name FROM graph_entities WHERE id = ?1",
                        rusqlite::params![id],
                        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
                    )?;
                    chain.push(ChatChainNode {
                        id,
                        kind,
                        name,
                        via,
                    });
                }
                if depth >= WALK_MAX_DEPTH {
                    continue;
                }
                let mut stmt = conn.prepare(
                    "SELECT to_id, edge_type FROM graph_edges
                     WHERE from_id = ?1 AND edge_type IN ('caused_by', 'led_to')",
                )?;
                let edges = stmt.query_map(rusqlite::params![id], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                })?;
                for edge in edges {
                    let (to_id, et) = edge?;
                    if visited.insert(to_id) {
                        queue.push_back((to_id, depth + 1, et));
                    }
                }
            }
            Ok::<_, anyhow::Error>(chain)
        })
    }
}

/// Estimate a row's token cost from its `content_text`, matching the
/// navigation tokenizer (`chars / CHARS_PER_TOKEN`). A non-empty row costs at
/// least one token so empty-but-present rows still count against the budget.
fn estimate_tokens(text: &str) -> usize {
    let chars = text.chars().count();
    if chars == 0 {
        0
    } else {
        (chars / CHARS_PER_TOKEN).max(1)
    }
}

/// Validate + normalize the `--kinds` list. Empty input → both chat kinds.
/// Unknown kinds are a hard error (no silent fallback to a wrong filter).
fn sanitize_kinds(raw: &[String]) -> Result<Vec<String>> {
    if raw.is_empty() {
        return Ok(ALLOWED_KINDS.iter().map(|s| s.to_string()).collect());
    }
    let mut out = Vec::with_capacity(raw.len());
    for k in raw {
        let trimmed = k.trim();
        if !ALLOWED_KINDS.contains(&trimmed) {
            bail!(
                "invalid kind `{trimmed}`; allowed: {}",
                ALLOWED_KINDS.join(", ")
            );
        }
        if !out.iter().any(|e: &String| e == trimmed) {
            out.push(trimmed.to_string());
        }
    }
    if out.is_empty() {
        Ok(ALLOWED_KINDS.iter().map(|s| s.to_string()).collect())
    } else {
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direction_parses() {
        assert_eq!(
            ChatDirection::parse("recent").unwrap(),
            ChatDirection::Recent
        );
        assert_eq!(
            ChatDirection::parse("chrono").unwrap(),
            ChatDirection::Chronological
        );
        assert!(ChatDirection::parse("backwards").is_err());
    }

    #[test]
    fn direction_order_strings() {
        assert_eq!(ChatDirection::Recent.order(), "DESC");
        assert_eq!(ChatDirection::Chronological.order(), "ASC");
    }

    #[test]
    fn sanitize_kinds_defaults_to_both() {
        let got = sanitize_kinds(&[]).unwrap();
        assert_eq!(
            got,
            vec!["ReasoningLog".to_string(), "ToolCall".to_string()]
        );
    }

    #[test]
    fn sanitize_kinds_rejects_unknown() {
        assert!(sanitize_kinds(&["Call".to_string()]).is_err());
        assert!(sanitize_kinds(&["ReasoningLog".to_string()]).is_ok());
    }

    #[test]
    fn sanitize_kinds_dedups() {
        let got =
            sanitize_kinds(&["ReasoningLog".to_string(), "ReasoningLog".to_string()]).unwrap();
        assert_eq!(got, vec!["ReasoningLog".to_string()]);
    }

    #[test]
    fn estimate_tokens_uses_chars_per_token() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abc"), 1); // min 1 for non-empty
        assert_eq!(estimate_tokens(&"a".repeat(40)), 10);
    }
}
