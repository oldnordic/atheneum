# Atheneum

Embedded graph database for agent coordination — episodic memory, knowledge persistence, and session accountability across coding sessions.

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
| Memory | keyed memories with scope, confidence, project scoping |
| Dream | reflective consolidation pass over memories (merge, deduplicate, promote) |
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

## HopGraph

HopGraph is atheneum's retrieval model: **embeddings find the door, graph walk retrieves the room.**

1. A text query is embedded and matched against the lexical index to find entry-point entities.
2. From each hit, a BFS walk expands the subgraph — following only allowed edge types (e.g., `Explains`, `Wikilink`).
3. The result is truncated to a token budget so it fits in a context window.

```rust
use atheneum::graph::{AtheneumGraph, hopgraph_query, EdgeType};
use std::path::Path;

let graph = AtheneumGraph::open(Path::new("atheneum.db"))?;

// Token-budgeted HopGraph query — walk only Explains + Wikilink edges
let views = graph.hopgraph_query(
    "session accountability",
    3,        // k: max entry points
    2,        // depth: BFS depth
    Some(&[EdgeType::Explains, EdgeType::Wikilink]),
    2000,     // max_tokens per view
    None,     // project_id
)?;

for view in &views {
    println!("entry: {} ({} entities, {} edges)",
        view.entry_id, view.entities.len(), view.edges.len());
}

// Switch to neural embeddings (requires ollama + nomic-embed-text)
#[cfg(feature = "neural-embed")]
{
    use atheneum::graph::OllamaEmbedder;
    graph.set_embedder(Box::new(OllamaEmbedder::nomic_embed_text()));
    graph.build_search_index()?; // rebuild with new dimension
}
```

### Discovery Consolidation

```rust
// Merge all Discovery entities for a target into a single Knowledge entity
let knowledge_id = graph.consolidate_discoveries("query_sessions", Some("my-project"))?;

// Consolidate all targets at once
let results = graph.consolidation_pass(Some("my-project"))?;
for (target, kid) in &results {
    println!("{} → knowledge {}", target, kid);
}
```

### Bridge Wiki to Code Symbols

```rust
// Link wiki [[wikilinks]] to magellan-indexed code symbols
graph.link_wiki_to_symbols(
    "/path/to/magellan.db",
    "claude",
    Some("my-project"),
)?;
```

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `default` | ✓ | Core graph, wiki, sessions, planning, search |
| `neural-embed` | — | Ollama neural embeddings (requires `ureq`, ollama + nomic-embed-text) |
| `web` | — | Web dashboard (axum + askama templates) |
| `cli` | — | `atheneum` CLI binary |
| `async` | — | Async runtime support |

## CLI

```
INGEST:
  init <db>                               Initialize a new graph database
  sync-wiki <db> <dir> [project]          Ingest .md files as wiki pages
  sync-journal <db> <dir> [project]       Ingest .md files as journal sections
  sync-logseq <db> <root> [project]       Recursively ingest Logseq pages/ and journals/
  sync-claude-transcript <db> <jsonl> [project] [agent]  Import Claude transcript
  store-discovery <db> <agent> <type> <target> [meta.json]  Store a discovery
  add-edge <db> <from> <to> <edge-type> [data.json]        Create a relation

TASKS:
  task-create <db> <title> [desc] [--project P]    Create a new task
  task-list <db> [--project P] [--status S]        List tasks (default: non-archived)
  task-update <db> <task-id> <status>              Update task status
  task-done <db> <task-id>                         Mark task as DONE
  task-archive <db> <task-id>                      Archive a task

MEMORY:
  memory-store <db> <key> <content> [--scope S] [--confidence N] [--project P]  Store a memory
  memory-get <db> <key> [--scope S] [--project P]      Retrieve memory by key
  memory-list <db> [--scope S] [--project P] [--offset N] [--limit N]  List memories (paginated, default limit 1000)

DREAM:
  dream <db> [--scope S] [--project P] [--dry-run|--auto-merge]  Reflective memory consolidation

QUERY & NAVIGATION:
  search <db> <query> [--k N] [--project P] [--max-tokens N]         HNSW/lexical search
  navigate <db> <query> [--k N] [--depth N] [--project P] [--kind K] [--max-tokens N]  Search then walk subgraphs
  query-wiki <db> <path>                            Query a wiki page by path
  query-journal <db> <path>                         Query journal sections by path
  query-knowledge <db> <target> [--project P] [--max-tokens N]       Aggregated knowledge
  query-sessions <db> [--project P] [--offset N] [--limit N]  Session history
  query-events <db> [--session <id>] [--type <t>] [--offset N] [--limit N]  Event log
  list-pages <db> [--project P] [--offset N] [--limit N]  List wiki pages (default limit 1000)
  entity <db> <id>                                  Print entity as JSON
  edge <db> <id>                                    Print edge as JSON
  neighbors <db> <id> [--depth N]                   One-hop edges or BFS subgraph
  graph-stats <db>                                  Graph topology counts

MAINTENANCE:
  reindex <db>                                      Rebuild HNSW search index
  consolidate <db> [target] [--project P]           Merge discoveries into Knowledge
```

`sync-logseq` recursively ingests `<wiki-root>/pages/**/*.md` as wiki pages and
`<wiki-root>/journals/**/*.md` as journal sections. Wiki `[[links]]` become
first-class `wikilink` graph edges so `navigate` and `neighbors` can traverse article
relationships.

`sync-claude-transcript` imports a local Claude Code transcript JSONL into Atheneum's
session graph. It records prompt/chat summaries, observed tool calls, `accessed`
file edges for `Read`/`Edit`/`Write`, and session token/cache totals. Re-running the
command on the same append-only transcript imports only new lines.

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
