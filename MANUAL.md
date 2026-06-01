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

Sessions track every LLM coding session — who, when, what branch, how many tool calls, cost.

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

# Query a wiki page
atheneum query-wiki   <db-path> <page-path>

# Query a journal
atheneum query-journal <db-path> <journal-path>
```

---

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `default` | ✓ | Core graph, wiki, sessions, planning |
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
