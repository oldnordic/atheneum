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

## Configuration

Atheneum reads `~/.config/atheneum/config.toml` (or `$XDG_CONFIG_HOME/atheneum/config.toml`). A missing file is not an error — sensible defaults are used.

### Default config file

```toml
[atheneum]
db = "~/.local/share/atheneum/atheneum.db"
meta_db = "~/.local/share/atheneum/meta.db"

[llm]
provider = "ollama"
base_url = "http://localhost:11434"
model = "codellama"
api_key = ""

[embeddings]
provider = "hash"
dimension = 128
base_url = "http://localhost:11434"
model = "nomic-embed-text"
api_key = ""

[integrations]
# Cross-tool integration is opt-in. Each tool stays standalone by default.
[integrations.magellan]
enabled = false
config = "~/.config/magellan/config.toml"

[integrations.envoy]
enabled = false
url = "http://localhost:9876"
```

### CLI

```bash
# Create the default config file (idempotent; use --force to overwrite)
atheneum config init

# Print the currently effective configuration as JSON
atheneum config show
```

### Library

```rust
use atheneum::{Config, load_config, save_config};

let cfg = load_config()?;                         // from default location
let path = cfg.db_path();                         // tilde-expanded PathBuf
let meta = cfg.meta_db_path();

save_config(&Config::default())?;                 // write defaults to disk
```

Environment overrides follow the convention `ATHENEUM_<SECTION>_<KEY>` where supported by callers. Paths may contain leading `~`, which is expanded via `$HOME`.

The meta.db routing layer (`MetaRouter::open()`) honors `atheneum.meta_db` from this config and falls back to the XDG default if the config is missing or invalid.

---

## Maintainer Checklist

When changing Atheneum itself, keep the local docs and gates in sync:

```bash
# Repo-local wrappers around the shared project standards
.claude/scripts/quality-gate.sh
printf '{}\n' | env CLAUDE_PROJECT_DIR="$PWD" fish .claude/hooks/verify-rust.fish
bash .claude/hooks/pre-commit-rust-standards
```

Rules:

- Update `CHANGELOG.md` for every user-visible fix, behavior change, or workflow change.
- Update `MANUAL.md` when you add or change a public function, CLI command, flag, or operator workflow.
- Prefer adding the manual/changelog update in the same patch as the code change so the docs cannot drift.
- If a repo-local `.claude/` wrapper exists, run it from the repo root instead of reaching for a shared path manually.

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

### Runtime Cache Stats

Atheneum now keeps a process-local concurrent query cache for the hottest repeated read paths. You can inspect that runtime state directly:

```rust
let stats = graph.runtime_stats();
println!(
    "hits={} misses={} memory_q={} session_q={} wiki_q={}",
    stats.cache_hits,
    stats.cache_misses,
    stats.memory_queries,
    stats.session_queries,
    stats.wiki_queries,
);
```

Current cached reads:

- `query_memory()` / `list_memory()`
- `query_sessions()`
- `query_events()`
- `query_knowledge()` / `query_knowledge_in_project()`
- `list_wiki_pages()`

Writes invalidate the relevant cache domain automatically after successful mutation.

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

### Preview Candidate Matches

For fuzzy identifiers, Atheneum can return ranked existing candidates without mutating the graph:

```rust
let candidates = graph.preview_entity_candidates(
    "HTTP Router",
    5,
    Some("atheneum"),
    Some("WikiPage"),
    0.2,
)?;

for candidate in candidates {
    println!("{} {} {:.3}", candidate.kind, candidate.name, candidate.score);
}
```

This is intended for preview/disambiguation flows where you want to inspect likely matches before storing new memory, discovery, or wiki links.

### Query Validation And Repair

Atheneum can preview a navigation query plan before execution:

```rust
let plan = graph.preview_navigate_query(
    "timezone",
    5,
    2,
    None,
    Some("memories"),
)?;

assert!(plan.executable);
assert_eq!(plan.resolved_kind.as_deref(), Some("Memory"));
assert!(plan.kind_repaired);
```

This plan stage:

- trims accidental whitespace from the query
- resolves common entity-kind aliases such as `memory`, `memories`, `wiki`, and `discoveries`
- rejects unknown kinds before traversal instead of silently returning empty results
- records warnings/errors so repaired execution is explicit to callers

### Preview Before Commit

Atheneum can also preview normalized discovery, memory, and handoff payloads before writing:

```rust
let discovery = graph.preview_discovery(
    "codex",
    "pattern",
    "query_cache",
    serde_json::json!({"summary": "cache repeated reads", "project_id": "atheneum"}),
    5,
    0.2,
)?;

let memory = graph.preview_memory(
    "timezone",
    "UTC+1",
    "user",
    0.9,
    None,
    None,
    5,
    0.2,
)?;

let handoff = graph.preview_handoff(
    "claude1",
    "claude2",
    Some("atheneum"),
    serde_json::json!({"task": "finish review", "files_analyzed": ["src/lib.rs"]}),
    5,
    0.2,
)?;
```

These preview APIs:

- do not insert entities or edges
- return deterministic `content_hash` values
- include exact existing matches plus fuzzy candidate matches, even when the fuzzy score alone would have filtered them out

### CLI Navigate Kind Filters

The CLI `navigate` command now accepts `--kind` and reports the repaired/validated plan in its JSON output:

```bash
atheneum navigate ./atheneum.db timezone --kind memories
```
- are intended for operator review or agent-side "propose first, commit later" flows

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
let results = graph.lexical_search("SQL parameter ordering bug", 5, Some("atheneum"), None, None)?;

// Token-budgeted search — truncate results to fit a context window.
let results = graph.lexical_search("SQL parameter ordering bug", 5, Some("atheneum"), None, Some(500))?;
```

---

## Memory

Memory entries are stable facts stored distinct from Knowledge (merged discoveries) and WikiPage (documents). Each memory has a key, scope, confidence score, and optional project.

Scopes: `user` (preferences), `project` (project facts), `agent` (agent behavior), `memory` (general notes).

Memories are upserted -- storing with the same key, scope, and project_id updates the existing entry instead of creating a duplicate.

```rust
use atheneum::AtheneumGraph;
use std::path::Path;

let graph = AtheneumGraph::open(Path::new("atheneum.db"))?;

// Store a memory
let id = graph.store_memory(
    "timezone",           // key
    "UTC+1",              // content
    "user",               // scope
    0.9,                  // confidence (0.0-1.0)
    None,                 // project_id
    None,                 // tags
)?;

// Retrieve by key
let items = graph.query_memory("timezone", Some("user"), None)?;

// List all memories in a scope
let all = graph.list_memory(Some("user"), None)?;
```

---

## Dream

Dream is atheneum's reflective consolidation pass. It scans memories for problems -- duplicates, stale entries, contradictions, and verbosity -- and either reports them (dry run) or merges them (auto-merge).

What dream does:
1. **SCAN** -- reads all memories in scope
2. **DEDUPLICATE** -- finds near-duplicates using trigram Jaccard similarity (entries that say the same thing differently)
3. **STALE** -- flags entries not updated in N days with low confidence
4. **CONTRADICTION** -- detects same key across different scopes with low content similarity
5. **VERBOSE** -- scores content length vs unique-word ratio
6. **CONSOLIDATED** -- merges findings, creates `SupersededBy` edges pointing old entries to replacements

There are two dream commands:
- `dream` -- runs consolidation over memory entries
- `wiki-dream` -- runs the same pipeline over wiki page entities

```rust
use atheneum::{AtheneumGraph, DreamConfig, DreamMode};
use std::path::Path;

let graph = AtheneumGraph::open(Path::new("atheneum.db"))?;

// Dry run -- report only, no mutations
let report = graph.dream_pass(
    DreamMode::DryRun,
    None,                   // scope filter (None = all)
    Some("my-project"),     // project filter
    &DreamConfig::default(),
)?;
for finding in &report.findings {
    println!("{:?}: {}", finding.phase, finding.description);
}

// Auto-merge -- actually create SupersededBy edges
let report = graph.dream_pass(DreamMode::AutoMerge, None, None, &DreamConfig::default())?;

// Wiki dream -- same pipeline for wiki pages
let wiki_report = graph.wiki_dream_pass(DreamMode::AutoMerge, Some("my-project"), &DreamConfig::default())?;
```

---

## CLI Commands

### Ingest

```bash
# Initialize a new graph database
atheneum init <db-path>

# Sync a wiki directory into the graph
atheneum sync-wiki <db-path> <wiki-dir> [project-id]

# Sync journal files
atheneum sync-journal <db-path> <journal-dir> [project-id]

# Recursively sync a Logseq graph root
atheneum sync-logseq <db-path> <wiki-root> [project-id]

# Import a Claude Code transcript JSONL
atheneum sync-claude-transcript <db-path> <transcript.jsonl> [project-id] [agent-name]

# Store a discovery
atheneum store-discovery <db-path> <agent> <type> <target> [metadata.json]

# Create a relation between two entities
atheneum add-edge <db-path> <from-id> <to-id> <edge-type> [data.json|--data 'json']
```

`sync-logseq` expects a Logseq-style root with `pages/` and/or `journals/`. It recursively ingests markdown files under those directories. Wiki page `[[links]]` are stored as first-class `wikilink` edges, enabling graph traversal through article and note relationships.

`sync-claude-transcript` expects a Claude Code transcript JSONL, typically under `~/.claude/projects/<encoded-project>/<session-id>.jsonl`. It imports prompt summaries, assistant replies, observed tool calls, `accessed` file relations for `Read`/`Edit`/`Write`, and session token/cache totals. Re-running on the same append-only transcript imports only new lines because Atheneum stores a transcript cursor in SQL.

`store-discovery` takes an optional JSON file for metadata. The metadata JSON can contain fields like `project_id`, `why`, `file`, `line`.

`add-edge` creates a typed edge between two entities. Valid edge types include: `performed_by`, `assigned_to`, `called`, `accessed`, `modified`, `verified_by`, `caused_by`, `created`, `related_to`, `mentions`, `wikilink`, `implements`, `depends_on`, `tested_by`, `fixed_by`, `regressed_by`, `observed_in`, `belongs_to_project`, `similar_failure`, `requires_skill`, `handled_by_tool`, `explains`, `derived_from`, `superseded_by`, `consolidated_from`.

### Tasks

```bash
# Create a new task
atheneum task-create <db-path> <title> [description] [--project P]

# List tasks (default: non-archived)
atheneum task-list <db-path> [--project P] [--status S]

# List archived tasks explicitly
atheneum task-list <db-path> --status ARCHIVED [--project P]

# Update task status
atheneum task-update <db-path> <task-id> <status>

# Mark task as DONE
atheneum task-done <db-path> <task-id>

# Archive a task
atheneum task-archive <db-path> <task-id>
```

Valid statuses: `TODO`, `IN_PROGRESS`, `DONE`, `BLOCKED`, `ARCHIVED`.

### Memory

```bash
# Store a memory
atheneum memory-store <db-path> <key> <content> [--scope S] [--confidence N] [--project P]

# Retrieve memory by key
atheneum memory-get <db-path> <key> [--scope S] [--project P]

# List memories (paginated; default limit 1000)
atheneum memory-list <db-path> [--scope S] [--project P] [--offset N] [--limit N]
```

Memories are upserted -- storing with the same key + scope + project updates the existing entry. Default scope is `user`, default confidence is `1.0`.

### Dream

```bash
# Run reflective memory consolidation pass
atheneum dream <db-path> [--scope S] [--project P] [--dry-run|--auto-merge]

# Run consolidation over wiki pages
atheneum wiki-dream <db-path> [--project P] [--dry-run|--auto-merge]
```

`--dry-run` (default) reports findings without modifying the graph. `--auto-merge` creates `SupersededBy` edges pointing old entries to their replacements.

Output is a JSON `DreamReport` with findings organized by phase (DEDUPLICATE, STALE, CONTRADICTION, VERBOSE, CONSOLIDATED).

### Query and Navigation

```bash
# HNSW/lexical search over all entities
atheneum search <db-path> <query> [--k N] [--project P] [--max-tokens N]

# Search then BFS-walk graph subgraphs
atheneum navigate <db-path> <query> [--k N] [--depth N] [--project P] [--kind K] [--max-tokens N] [--concise]

# Query a wiki page by path
atheneum query-wiki <db-path> <path>

# Query journal sections by path
atheneum query-journal <db-path> <path>

# Aggregated knowledge for a target
atheneum query-knowledge <db-path> <target> [--project P] [--max-tokens N]

# Session history
atheneum query-sessions <db-path> [--project P] [--offset N] [--limit N]

# Event log
atheneum query-events <db-path> [--session <id>] [--type <type>] [--offset N] [--limit N]

# List wiki pages (default limit 1000)
atheneum list-pages <db-path> [--project P] [--offset N] [--limit N]

# Print a graph entity as JSON
atheneum entity <db-path> <entity-id>

# Print a graph edge as JSON
atheneum edge <db-path> <edge-id>

# One-hop edges or BFS subgraph
atheneum neighbors <db-path> <entity-id> [--depth N]

# Graph topology counts
atheneum graph-stats <db-path>
```

`search` uses the HNSW lexical index. It matches on shared tokens -- not semantic similarity. "car" will not match "automobile". Good for symbol and identifier search. Use `--max-tokens` to truncate the result list before it reaches your LLM context window.

`navigate` performs a search, then expands each hit into a subgraph using BFS. The `--kind` flag filters by entity type (accepts aliases like `memory`, `memories`, `wiki`, `discoveries`). The output includes the validated query plan plus subgraph views. Use `--max-tokens` to truncate each subgraph view to a token budget (the entry entity is always kept; neighbors are dropped until the budget fits). Use `--concise` to emit compact Markdown instead of JSON — designed for pasting into a language-model context window.

`query-knowledge` aggregates discoveries and handoffs for a target. Use `--max-tokens` to limit the total response size; discoveries are dropped first, then handoffs, and `"truncated": true` is set when truncation occurs.

### Cross-Project Registry (Meta)

```bash
# Register a project in the meta.db routing layer
atheneum meta-register envoy /home/feanor/Projects/envoy \
  /home/feanor/Projects/envoy/.magellan/magellan.db \
  --atheneum-db /home/feanor/Projects/envoy/atheneum.db \
  --language rust

# List all registered projects
atheneum meta-list

# List projects filtered by language
atheneum meta-list --language rust
```

`meta-register` upserts a project into `~/.local/share/atheneum/meta.db` (or `$XDG_DATA_HOME/atheneum/meta.db`). Re-registering the same name updates all fields and re-enables the project.

`meta-list` queries enabled projects from the registry. Use `--language` to filter by programming language.

### Cross-Project Queries

Atheneum can query across magellan-indexed codebases without importing their data. It uses `meta.db` as a routing registry and lazily `ATTACH DATABASE` each project's magellan DB on demand.

```bash
# Search for a symbol across all Rust projects
atheneum cross-search "build_router" --language rust --k 10

# Search across all registered projects (no language filter)
atheneum cross-search "checkpoint" --k 20

# Navigate: search + BFS subgraph walk per project
atheneum cross-navigate "error handling" --language rust --k 5 --depth 2
```

Output is JSON. `cross-search` returns ranked symbol hits with project, name, kind, and file path. `cross-navigate` returns one subgraph view per entry point, including entities and edges from each attached magellan database.

The router keeps an LRU cache of attached databases (default capacity 8). Missing or unreadable databases are skipped with a warning rather than aborting the whole query.

### Config

```bash
# Create the default config file at ~/.config/atheneum/config.toml
atheneum config init

# Overwrite an existing config file
atheneum config init --force

# Print the effective configuration as JSON
atheneum config show
```

`config init` writes the default TOML (XDG paths, local Ollama defaults, disabled cross-tool integrations). `config show` reads the file (or defaults if missing) and prints JSON, which is useful for debugging path expansion and integration flags.

### Maintenance

```bash
# Rebuild HNSW search index
atheneum reindex <db-path>

# Merge discoveries into Knowledge entities
atheneum consolidate <db-path> [target] [--project P]

# Print version
atheneum --version

# Print help
atheneum help
```

`reindex` rebuilds the HNSW index over all entities and then runs a WAL checkpoint to reclaim disk space. Useful after bulk imports or if search results seem incomplete.

`consolidate` merges all Discovery entities for a target (or all targets) into deduplicated Knowledge entities with `DerivedFrom` edges. Idempotent -- re-running returns the existing Knowledge entity.

---

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `default` | yes | Core graph, wiki, sessions, planning, search |
| `neural-embed` | no | Ollama neural embeddings (requires `ureq`, ollama + nomic-embed-text) |
| `web` | no | Web dashboard (axum + askama templates) |
| `cli` | no | `atheneum` CLI binary |
| `async` | no | Async runtime support |

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

`AtheneumGraph` uses internal `Mutex` locking. The `pub` methods take `&self` (shared reference) and handle synchronization internally. For concurrent access from multiple threads, wrap in `Arc<AtheneumGraph>` or use connection pooling per thread.

---

## Requirements

- Rust 1.75+
- SQLite 3.35+ with JSON1 extension (bundled via rusqlite by default)

## License

GPL-3.0-only -- see [LICENSE](LICENSE).
