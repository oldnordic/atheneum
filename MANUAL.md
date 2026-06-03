# Atheneum Manual

## Installation

### From crates.io

```bash
cargo add atheneum
```

### From source

```bash
git clone https://github.com/oldnordic/atheneum
cd atheneum
cargo build --release
```

---

## Overview

Atheneum is an embedded graph database for AI agent coordination. It stores discoveries, decisions, session histories, task handoffs, and knowledge across agent sessions — replacing ad-hoc file dumps with a queryable, persistent graph.

It is used as a library (embedded in your agent runtime) or accessed via envoy's HTTP bridge (`GET/POST /atheneum/*`).

---

## Opening a Graph

```rust
use atheneum::AtheneumGraph;
use std::path::Path;

// Persistent — creates file if absent
let graph = AtheneumGraph::open(Path::new("atheneum.db"))?;

// In-memory — for tests and ephemeral sessions
let graph = AtheneumGraph::open_in_memory()?;
```

The database schema is auto-migrated on `open()`. No separate migration step required.

---

## Agent Sessions

Sessions track every coding session — who, when, what branch, how many tool calls, cost.

```rust
use atheneum::graph::{AtheneumGraph, SessionParams};

graph.record_session(SessionParams {
    session_id: "abc-123".into(),
    agent_name: "claude-main".into(),
    project: "my-project".into(),
    tool: "claude-code".into(),
    trigger: "cli".into(),           // "cli" | "subagent" | "hook"
    model: Some("claude-sonnet-4".into()),
    git_branch: Some("feat/auth".into()),
    git_head: Some("a1b2c3d".into()),
    parent_session_id: None,          // set for subagents
})?;
```

### Ending a Session

```rust
use atheneum::graph::EndSessionParams;

graph.end_session(EndSessionParams {
    session_id: "abc-123".into(),
    exit_status: "end_turn".into(),
    prompt_count: 12,
    tool_call_count: 47,
    file_write_count: 3,
    commit_count: 1,
    test_run_count: 2,
    total_input_tokens: 50_000,
    total_output_tokens: 8_000,
    total_cost_usd: 0.15,
})?;
```

### Querying Recent Sessions

```rust
// Last 3 sessions for a project (newest first)
let sessions = graph.query_sessions("my-project", 3, None)?;

// Children of a specific session
let children = graph.query_sessions("my-project", 10, Some("parent-session-id"))?;

for s in sessions {
    println!("{} {} {}tc {}fw last:{:?}",
        s.started_at, s.git_branch.unwrap_or_default(),
        s.tool_call_count, s.file_write_count, s.last_tool);
}
```

### Tool Call Evidence

```rust
use atheneum::graph::ToolCallParams;

graph.record_evidence_tool_call(ToolCallParams {
    session_id: "abc-123".into(),
    tool_name: "Edit".into(),
    tool_version: None,
    input_hash: Some("deadbeef".into()),
    input_summary: Some("write src/lib.rs".into()),
    output_hash: None,
    output_summary: Some("ok".into()),
    exit_status: "success".into(),
    latency_ms: 234,
    input_tokens_est: None,
    tool_category: "file_write".into(),
})?;
```

### Subagent Handover

```rust
// Subagent writes this on stop — the parent reads it
graph.record_subagent_handover(
    "sub-session-id",
    "Fixed SQL param ordering in query_sessions. evidence.rs line 547.",
    &["src/graph/evidence.rs".to_string()],
    "end_turn",
)?;
```

---

## Discoveries

Discoveries are non-obvious facts, invariants, and decisions stored so future agents don't re-discover them.

```rust
use serde_json::json;

let id = graph.store_discovery(
    "claude",           // agent name
    "Bug",              // discovery type
    "query_sessions",   // target symbol
    json!({
        "file": "src/graph/evidence.rs",
        "line": 547,
        "why": "anonymous ? params required when project is None and parent_id is Some",
        "project_id": "atheneum"
    }),
)?;
```

### Querying Discoveries

```rust
// By target symbol
let discoveries = graph.query_discoveries("query_sessions")?;

// By project (no target required — for session bootstrap context injection)
let recent = graph.recent_project_context("atheneum", 8)?;
```

---

## Knowledge Graph

```rust
// Store a linked discovery
let id = graph.store_discovery_in_project(
    "claude", "Decision", "auth-middleware",
    Some("my-project"),
    json!({ "why": "legal compliance", "risk": "high" }),
)?;

// Query knowledge for a symbol+project
let knowledge = graph.query_knowledge_in_project("auth-middleware", Some("my-project"))?;
```

---

## Task Planning

```rust
use atheneum::graph::AtheneumGraph;
use serde_json::json;

// Create a task
let task_id = graph.create_task("Implement session handover", Some("my-project"))?;

// Add requirements
graph.add_requirement(task_id, "Writes git diff on stop", None)?;

// Update status
graph.update_task_status(task_id, atheneum::graph::KanbanStatus::InProgress)?;
```

---

## Wiki Ingestion

Atheneum parses Markdown files with frontmatter and `[[wikilinks]]` into the knowledge graph.

```rust
let content = r#"---
title: "Session Accountability"
type: concept
---
# Session Accountability
See also [[envoy]] and [[grounded-coding]].
"#;

let entity_id = graph.ingest_wiki_page("session-accountability.md", content, None)?;
```

### Journal Sections

```rust
// Journals use ## HH:MM | Title headers and Kanban lines
let journal = r#"
## 14:23 | Fixed param bug
Corrected SQL ordering in evidence.rs.

## 15:00 | Deployed
"envoy" -> DONE
"#;
let sections = graph.parse_journal_sections(journal)?;
graph.ingest_journal_sections(&sections, Some("my-project"))?;
```

---

## HopGraph

HopGraph is atheneum's retrieval model: **embeddings find the door, graph walk retrieves the room.** Unlike flat RAG, HopGraph uses vector similarity only to locate entry points, then expands connected knowledge via graph traversal.

### Token-Budgeted Retrieval

```rust
use atheneum::graph::{AtheneumGraph, EdgeType};

let graph = AtheneumGraph::open(Path::new("atheneum.db"))?;

let views = graph.hopgraph_query(
    "session accountability",      // query text
    3,                             // k: max entry-point entities
    2,                             // depth: BFS expansion depth
    Some(&[EdgeType::Explains, EdgeType::Wikilink]),  // allowed edge types
    2000,                          // max_tokens budget per view
    None,                          // project_id filter
)?;

for view in &views {
    println!("entry={} entities={} edges={}",
        view.entry_id, view.entities.len(), view.edges.len());
}
```

`hopgraph_query` performs: lexical search → filtered BFS subgraph → token-budgeted truncation. Orphan edges (pointing to removed entities) are dropped. The entry entity is always kept regardless of budget.

### Filtered Subgraph Walk

```rust
use atheneum::graph::EdgeType;

// Walk only Explains and Wikilink edges from an entity
let view = graph.get_subgraph_filtered(
    entity_id,
    3,      // depth
    Some(&[EdgeType::Explains, EdgeType::Wikilink]),
)?;
```

### Embedding Backends

```rust
// Default: HashEmbedder (128-dim, zero deps, always available)
let dim = graph.embedder_dimension(); // 128

// Switch to neural embeddings (requires --features neural-embed)
#[cfg(feature = "neural-embed")]
{
    use atheneum::graph::OllamaEmbedder;
    graph.set_embedder(Box::new(OllamaEmbedder::nomic_embed_text()));
    graph.build_search_index()?; // rebuild index with new dimension (768)
    assert_eq!(graph.embedder_dimension(), 768);
}
```

| Backend | Dimension | Dependencies | Quality |
|---------|-----------|-------------|---------|
| `HashEmbedder` | 128 | None | Token overlap only ("car" ≠ "automobile") |
| `OllamaEmbedder` | 768 | ollama + nomic-embed-text | Semantic similarity |

### Discovery Consolidation

Merge duplicate Discovery entities into deduplicated Knowledge entities:

```rust
// Consolidate a single target
let knowledge_id = graph.consolidate_discoveries("query_sessions", Some("my-project"))?;

// Consolidate all targets in a project
let results = graph.consolidation_pass(Some("my-project"))?;
for (target, kid) in &results {
    println!("{} → knowledge {}", target, kid);
}
```

Consolidation creates `DerivedFrom` edges from Knowledge → source Discoveries. Idempotent — re-running returns the existing Knowledge entity.

### Bridge Wiki to Code Symbols

```rust
graph.link_wiki_to_symbols(
    "/path/to/.magellan/magellan/magellan.db",
    "claude",
    Some("my-project"),
)?;
```

For each wiki page's `[[wikilinks]]`, queries the magellan DB for matching code symbols, imports them as Discovery entities, and creates `Explains` edges from wiki page → symbol. Idempotent.

---

## Search

```rust
// Full-text search
let results = graph.full_text_search("query_sessions")?;

// Lexical search via HNSW hash-projected index.
// Matches on shared tokens — not neural/semantic. "car" won't match "automobile".
let results = graph.lexical_search("SQL parameter ordering bug", 5, Some("atheneum"))?;
```

---

## CLI Commands

```bash
# Sync a wiki directory into the graph
atheneum sync-wiki   <db-path> <wiki-dir> [project-id]

# Sync journal files
atheneum sync-journal <db-path> <journal-dir> [project-id]

# Recursively sync a Logseq graph root
atheneum sync-logseq <db-path> <wiki-root> [project-id]

# Import a Claude Code transcript JSONL
atheneum sync-claude-transcript <db-path> <transcript.jsonl> [project-id] [agent-name]

# Query a wiki page
atheneum query-wiki   <db-path> <page-path>

# Query a journal
atheneum query-journal <db-path> <journal-path>

# Query graph topology
atheneum graph-stats <db-path>
atheneum entity <db-path> <entity-id>
atheneum edge <db-path> <edge-id>
atheneum neighbors <db-path> <entity-id> [--depth N]

# Search indexed knowledge, then BFS-walk each hit
atheneum navigate <db-path> "<query>" [--k N] [--depth N] [--project P]
```

`sync-logseq` expects a Logseq-style root with `pages/` and/or `journals/`.
It recursively ingests markdown files under those directories. Wiki page
`[[links]]` are stored as first-class `wikilink` edges, enabling graph traversal through
article and note relationships.

`sync-claude-transcript` expects a Claude Code transcript JSONL, typically under
`~/.claude/projects/<encoded-project>/<session-id>.jsonl`. It imports prompt
summaries, assistant replies, observed tool calls, `accessed` file relations for
`Read`/`Edit`/`Write`, and session token/cache totals. Re-running the command on the
same append-only transcript imports only new lines because Atheneum stores a
transcript cursor in SQL.

---

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `default` | ✓ | Core graph, wiki, sessions, planning |
| `neural-embed` | — | Ollama neural embeddings (requires `ureq`) |
| `web` | — | Web dashboard (axum + askama) |
| `cli` | — | CLI binary |
| `async` | — | Async runtime support |

---

## Error Handling

All functions return `anyhow::Result<T>`. Errors include context about which operation failed.

```rust
match graph.record_session(params) {
    Ok(()) => {},
    Err(e) => eprintln!("Session record failed: {:#}", e),
}
```

---

## Thread Safety

`AtheneumGraph` is not `Send + Sync`. For concurrent access wrap in `Arc<Mutex<AtheneumGraph>>` or use connection pooling per thread.

---

## Requirements

- Rust 1.75+
- SQLite 3.35+ with JSON1 extension (bundled via rusqlite by default)

## License

GPL-3.0-only — see [LICENSE](LICENSE).
