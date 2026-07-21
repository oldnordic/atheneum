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

## What's New In 0.11.0

- `seed_memory`: a token-bounded, concept-grouped summary of the knowledge base. `atheneum-mcp` regenerates it on every session connect and folds it into the server instructions and `navigate`/`query_memory`/`search` tool descriptions, so a connecting client already knows what's in memory.
- `update_memory` and `upsert_memory_by_concept` (`add_memory`): patch a memory or its owning concept in place instead of every correction becoming a new row.
- `lint`/`maintain`: deterministic graph-health lint (orphans, broken wikilinks, stale `superseded_by` edges) plus a mutating pass that rewires, stubs, or resolves what it finds.
- `dream-semantic` (`semantic_consolidation`): merges closely-related or redundant concepts via a local language-model prompt with a lexical-similarity fallback.
- Memory pinning (`pin`/`unpin`) and TTL-based archival via `maintain`.
- `models-list` + swap-guard config: discovers what's loaded on a local model server before running model-dependent operations, so they don't force an unwanted swap on a shared GPU.
- Optional `dashboard` web UI (`web-ui` feature, off by default).
- Query tracing (`navigate --trace`, `trace-get`) for replaying past navigation queries.

## What's New In 0.10.0

- Native `extract-decisions` now ships inside the crate behind the `extract` feature, with both Ollama-backed and `--heuristic` backfill modes.
- `sessions-recent` can hide noisy non-repo buckets with repeatable `--exclude-project`.
- Lexical search now reranks mixed corpora toward authoritative wiki/discovery documents instead of changelog and transcript noise.
- The standard release gate now works from the repo root with checked-in `deny`, `gitleaks`, and `semgrep` config.

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
| Search index | FTS5 full-text + bag-of-tokens lexical scan; optional HNSW candidate index for human fuzzy lookup via `semantic-search` |

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

## Operator Queries

The CLI has direct observability commands for the read paths that operators use most:

```bash
# One session: summary + recent events + recent tool calls
atheneum session-trace ./atheneum.db --session sess-1 --limit 10

# Aggregate tool usage for that session
atheneum tool-usage ./atheneum.db --session sess-1 --limit 20

# Recent activity across the project
atheneum discoveries-recent ./atheneum.db --project envoy --limit 10
atheneum handoffs-recent ./atheneum.db --agent claude4 --limit 10
atheneum events-recent ./atheneum.db --type tool_call --limit 10
atheneum sessions-recent ./atheneum.db --project envoy --limit 10
```

All of these return JSON and are designed to replace routine direct SQLite reads.

### Session Digest

`session-digest` emits a bounded, ranked plain-text bootstrap packet (tool
calls, file writes, top files, decisions, open tasks, thread anchors) so a
new session can ground on what prior sessions in the same project did. It
computes activity from `event_log` (the `sessions` ledger columns are usually
zero), falls back across all projects when the `--project` filter is empty,
and truncates to a token budget. `--json` emits the structured report.

```bash
atheneum session-digest ./atheneum.db --project envoy --last 3 --tokens 500
```

### Decision Capture

Claude Code chat transcripts carry the structured-choice signals that are
genuinely *decisions* — `AskUserQuestion` (a human-answered choice),
`ExitPlanMode` (a plan approved for execution), `TaskCreate`, and `TodoWrite`.
Atheneum captures those as first-class `Decision` discovery rows so the
graph holds the decision chain, not just the chat text.

**Model:** each captured decision stores `source` (`askuser` / `exitplan` /
`taskcreate` / `todowrite`), `chosen`, `alternatives`, and `rationale` in the
discovery metadata, scoped to the session that made it. Dedup key is
`session_id` + `sequence` + `target` + `source`, so a decision is captured
once even if the same transcript is scanned repeatedly.

Three layers, one shape:

```bash
# Backfill: a standalone operator script runs a local LLM (Ollama qwen3.5)
# over each transcript and stores decision-shaped turns via `atheneum
# store-discovery`. One session, or --all (resumable).
extract-decisions <session-id>                 # one session, store
extract-decisions --all                       # every transcript (resumable)
extract-decisions --all --dry-run             # preview everything, store none
extract-decisions <session-id> --model qwen3.5 --project atheneum

# Live: the atheneum watcher tails transcripts in a loop, captures the
# Tier-1 structured signals deterministically (no LLM).
atheneum watch-decisions ./atheneum.db --interval 2 --project atheneum

# Observe: the decisions captured for a session (any source)
atheneum chat ./atheneum.db --session <session_id> --only-decisions
atheneum discoveries-recent ./atheneum.db --session <session_id> --type Decision --limit 50
```

A fourth, highest-fidelity layer ships as the `plugin/atheneum-decisions/`
Claude Code companion plugin: a `record-decision` skill that records a
decision *as the model makes it* (`source = "skill"`), a
`/decision <target> <chosen> [rationale]` manual fallback, and a non-blocking
Stop-gate that reminds you to record decisions. The skill/command layer
dedups on `(session_id, target, source, chosen)` via `store-discovery --dedup`
(it has no stable transcript `sequence`); see MANUAL.md for the install
command.

`extract-decisions` is the one-shot backfiller for decisions that lack a
Tier-1 structured signal. It ships two ways: a `~/.local/bin` operator script
(the default, no special build needed) and a native `atheneum
extract-decisions` subcommand (a Rust port, behind the `extract` feature —
`--features extract` or `--all-features`). The subcommand has two user-picked
backends: the default local Ollama LLM (`qwen3.5`, `source = "llm-extract"`)
and a rule-based `--heuristic` mode (no LLM, no network, `source =
"heuristic"`) that catches decision-shaped sentences with an explicit
rationale clause — lower recall + some false positives, zero deps. Both
backends store each decision as a `Decision` discovery linked into the
session thread (`caused_by` / `led_to`) for free — the script shells out to
`atheneum store-discovery`, the subcommand stores in-process via
`graph.store_discovery`. Pick the backend per run with `--heuristic` /
`--mode llm|heuristic` / `ATHENEUM_EXTRACT_MODE`. `atheneum watch-decisions` is the always-on deterministic detector
(`--once` runs a single cold-cursor scan, safe for cron). The watcher is
detect-only at the Tier-1 layer — the SessionStop `sync-claude-transcript`
hook still owns full transcript ingest at session end. A `Decision` row
from any source (backfill script, watcher, or manual `store_discovery`) is
visible to `chat --only-decisions` and `session-digest`, deduped across
sources by the same key.

## HopGraph

HopGraph is an optional retrieval mode: **embeddings find the door, graph walk retrieves the room.**

For grounded agent workflows, the default path is still direct graph traversal plus typed SQL reads. HopGraph exists for fuzzy human-style lookup when approximate vector entry points are useful enough to justify the extra index cost.

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
| `default` | ✓ | Core graph, wiki, sessions, planning, search, thread — lexical (bag-of-tokens) search + BFS graph navigation |
| `semantic-search` | — | Optional HNSW candidate index for human fuzzy lookup in `search` (opt-in; heavy — index + embedder). Off by default; agent retrieval uses lexical search, graph traversal, and SQL payload queries |
| `neural-embed` | — | Ollama neural embeddings (requires `ureq`, ollama + nomic-embed-text) |
| `extract` | — | Native `atheneum extract-decisions` subcommand (Rust port of the operator script; LLM backend requires `ureq` + ollama, default `qwen3.5`; `--heuristic` backend needs no LLM/network) |
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
  memory-update <db> --id N [--content C] [--importance N] [--tags a,b --replace-tags]  Patch an existing memory in place
  pin <db> --id N                                   Pin an entity (always in seed_memory, cache-eviction immune)
  unpin <db> --id N                                 Unpin an entity

LIBRARIAN:
  lint <db-path> [--stale-days N]                   Graph-health check: orphans, broken wikilinks, stale superseded_by edges
  maintain <db-path> [--apply] [--stale-days N] [--rewire-threshold F] [--broken-link-mode <stub|sever>]  Rewire orphans, stub/sever broken links, resolve contradictions
  models-list <db-path>                             List models loaded on a local model server
  dashboard <db-path> [--port N]                     Web dashboard server (feature: web-ui)

DREAM:
  dream <db> [--scope S] [--project P] [--dry-run|--auto-merge]  Reflective memory consolidation
  dream-semantic <db> [--apply]                     Merge closely-related/redundant concepts (local-model prompt, lexical fallback)

QUERY & NAVIGATION:
  search <db> <query> [--k N] [--project P] [--max-tokens N]         Lexical search (optional HNSW candidate index with --features semantic-search)
  navigate <db> <query> [--k N] [--depth N] [--project P] [--kind K] [--max-tokens N]  Search then walk subgraphs
  thread <db> <query> [--k N] [--depth D=3] [--tokens T=1500] [--project P] [--json]  Walk a decision chain (caused_by/led_to edges)
  chat <db> --session <id> [--tokens T] [--direction recent|chrono] [--kinds K] [--role R] [--search Q] [--only-decisions] [--walk] [--offset N --limit L] [--json]  Token-budgeted walk of a session's chat records (or just its decisions)
  watch-decisions <db> [--once] [--interval S=2] [--config-dir D]... [--project P] [--agent A] [--dry-run]  Live-tail transcripts, capture structured decisions
  extract-decisions <db> [--all|<session-id>] [--dry-run] [--force] [--project P] [--agent A] [--model M] [--transcripts-dir D] [--max-chars N] [--ollama-url U] [--heuristic|--mode llm|heuristic] [--verbose]  Backfill decisions (LLM or --heuristic; feature: extract)
  wiki-search <db> <query> [--project P] [--limit N]  Full-text search over wiki pages (FTS5, falls back to name match)
  decision-search <db> <query> [--project P] [--limit N]  Content search over Decision discoveries (target/chosen/why)
  seed-memory <db> [--project P] [--tokens N]       Token-bounded, concept-grouped knowledge-base summary
  trace-get <db> --id N                             Replay a past navigate --trace query
  query-wiki <db> <path>                            Query a wiki page by path
  query-journal <db> <path>                         Query journal sections by path
  query-knowledge <db> <target> [--project P] [--max-tokens N]       Aggregated knowledge
  query-sessions <db> [--project P] [--offset N] [--limit N]  Session history
  query-events <db> [--session <id>] [--type <t>] [--offset N] [--limit N]  Event log
  session-digest <db> [--project P] [--last N] [--tokens T] [--json]  Bounded bootstrap digest
  session-trace <db> --session <id> [--limit N]     Session summary plus recent events
  tool-usage <db> --session <id> [--limit N]        Tool breakdown for one session
  discoveries-recent <db> [--project P] [--agent A] [--session S] [--type T] [--limit N]  Recent discoveries (filter by session and/or type)
  handoffs-recent <db> [--project P] [--agent A] [--limit N]     Recent handoffs
  events-recent <db> [--session ID] [--type T] [--limit N]       Recent events
  sessions-recent <db> [--project P] [--agent A] [--limit N] [--exclude-project P ...]     Recent sessions (hide non-repo project buckets)
  list-pages <db> [--project P] [--offset N] [--limit N]  List wiki pages (default limit 1000)
  entity <db> <id>                                  Print entity as JSON
  edge <db> <id>                                    Print edge as JSON
  neighbors <db> <id> [--depth N]                   One-hop edges or BFS subgraph
  graph-stats <db>                                  Graph topology counts

MAINTENANCE:
  reindex <db>                                      Rebuild optional HNSW human-search index
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

## Resilience

The `wiki_pages_fts` FTS5 index self-heals on open. If an external SQLite writer (system `sqlite3`, Python, another tool) leaves the FTS5 shadow tables in an inconsistent state, `AtheneumGraph::open()` detects the corruption, recreates the virtual table and triggers on fresh connections, and rebuilds the index from `wiki_pages`. This keeps `sync-wiki`, `search-wiki`, and `backfill-wiki` working without manual repair.

## License

GPL-3.0-only — see [LICENSE](LICENSE).
