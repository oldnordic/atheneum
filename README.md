# Atheneum

Embedded graph database for AI agent coordination — episodic memory, knowledge persistence, and session accountability across LLM coding sessions.

Part of the **grounded-coding ecosystem**. Used as the storage layer inside [envoy](https://github.com/oldnordic/envoy).

## Ecosystem

```
grounded-coding ── install + Claude Code plugin (skills, hooks, MCP)
       │
       ├── magellan   ── code graph indexer (symbols, call graph, CFG)
       ├── llmgrep    ── semantic code search over magellan graphs
       ├── mirage     ── CFG analysis (paths, loops, dominance)
       ├── splice     ── span-safe refactoring
       │
       ├── envoy      ── HTTP coordination server (messaging, agent registry)
       │     └── atheneum  ◀── YOU ARE HERE
       │           └── sqlitegraph  ── SQLite graph engine
       │
       └── envoy-hook ── Claude Code hook binary (session + tool call logging)
```

Atheneum is the persistence layer. It stores what agents discovered, what sessions did, what tasks exist, and what knowledge is worth keeping — so the next agent doesn't start from scratch.

## Install

```bash
cargo add atheneum
```

Atheneum is a library. Production use is via [agent-envoy](https://crates.io/crates/agent-envoy) which exposes all endpoints over HTTP (`envoy` binary). Direct embedding is for custom runtimes.

## Quickstart

```rust
use atheneum::AtheneumGraph;
use std::path::Path;

let graph = AtheneumGraph::open(Path::new("atheneum.db"))?;

// Record a session
graph.record_session(atheneum::graph::SessionParams {
    session_id: "abc-123".into(),
    agent_name: "claude-main".into(),
    project: "my-project".into(),
    tool: "claude-code".into(),
    trigger: "cli".into(),
    model: Some("claude-sonnet-4".into()),
    git_branch: Some("feat/auth".into()),
    git_head: None,
    parent_session_id: None,
})?;

// Store a discovery
use serde_json::json;
graph.store_discovery("claude", "Bug", "query_sessions", json!({
    "file": "src/graph/evidence.rs",
    "line": 547,
    "why": "anonymous ? params required when project is None and parent_id is Some",
    "project_id": "my-project"
}))?;

// Query recent sessions for a project
let sessions = graph.query_sessions("my-project", 3, None)?;
for s in &sessions {
    println!("{} {} {}tc", s.started_at, s.git_branch.as_deref().unwrap_or("?"), s.tool_call_count);
}

// Ingest a wiki article
let content = "---\ntitle: My Note\n---\n# My Note\nSee also [[Related Concept]].\n";
graph.ingest_wiki_page("my-note.md", content, None)?;
```

## What It Stores

| Domain | Types |
|--------|-------|
| Sessions | start, end, tool calls, file writes, commits, test runs, cost |
| Discoveries | facts, decisions, bugs, invariants — scoped by project |
| Knowledge | wiki pages, journal sections, wikilinks |
| Planning | tasks, requirements, blockers, kanban state |
| Handoffs | inter-agent state transfers with manifests |
| Ontology | class/property schemas for typed entity reasoning |
| Search index | FTS5 full-text + HNSW lexical index (hash-projected tokens, not neural) |

## HTTP Access

When embedded in envoy, all atheneum operations are accessible over HTTP:

```bash
# Query recent sessions
curl "http://127.0.0.1:9876/atheneum/sessions?project=my-project&last=3"

# Store a discovery
curl -X POST http://127.0.0.1:9876/atheneum/discoveries \
  -H "X-Agent-Id: $GROUNDED_AGENT_ID" \
  -H "content-type: application/json" \
  -d '{"agent":"claude","discovery_type":"Decision","target":"auth","metadata":{"why":"..."}}'

# Get recent project context (for session bootstrap hooks)
curl "http://127.0.0.1:9876/atheneum/context?project=my-project&limit=6"
```

See [envoy's API.md](https://github.com/oldnordic/envoy/blob/master/API.md) for the full HTTP reference.

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `default` | ✓ | Core graph, wiki, sessions, planning, search |
| `web` | — | Web dashboard (axum + askama templates) |
| `cli` | — | `atheneum` CLI binary |
| `async` | — | Async runtime support |

## CLI

```bash
atheneum sync-wiki    <db> <dir> [project]
atheneum sync-journal <db> <dir> [project]
atheneum query-wiki   <db> <path>
atheneum query-journal <db> <path>
```

## Requirements

- Rust 1.75+
- SQLite 3.35+ with JSON1 (bundled via rusqlite)

## Related

- [envoy](https://github.com/oldnordic/envoy) — HTTP server exposing atheneum over REST
- [grounded-coding](https://github.com/oldnordic/grounded-coding) — Claude Code plugin using this ecosystem
- [magellan](https://github.com/oldnordic/magellan) — code graph indexer
- [sqlitegraph](https://crates.io/crates/sqlitegraph) — underlying SQLite graph engine

## License

GPL-3.0-only — see [LICENSE](LICENSE).
