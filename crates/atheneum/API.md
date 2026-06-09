# Atheneum -- Public API Reference

Atheneum exposes its API as a Rust library. HTTP access is via the envoy bridge
(`GET/POST /atheneum/*`). See [envoy's API.md](https://github.com/oldnordic/envoy/blob/master/API.md)
for HTTP endpoints.

---

## `AtheneumGraph`

Main entry point. All methods take `&self` (shared reference with internal Mutex).

```rust
use atheneum::AtheneumGraph;
use std::path::Path;

let graph = AtheneumGraph::open(Path::new("atheneum.db"))?;
let graph = AtheneumGraph::open_in_memory()?;
```

### `open(path: &Path) -> Result<Self>`

Open or create a persistent graph database. Schema is auto-migrated.

### `open_in_memory() -> Result<Self>`

Create an ephemeral in-memory database. Useful for tests.

### `is_healthy() -> bool`

Check database connectivity. Returns `true` if the graph is operational.

### `runtime_stats() -> RuntimeStats`

Inspect process-local cache/query/write counters.

```rust
let stats = graph.runtime_stats();
println!(
    "hits={} misses={} memory_q={} session_q={} wiki_q={}",
    stats.cache_hits, stats.cache_misses,
    stats.memory_queries, stats.session_queries, stats.wiki_queries,
);
```

### `with_raw_connection<F, R>(&self, f: F) -> Result<R>`

Execute a closure with a raw `rusqlite::Connection` reference. For advanced queries not covered by the typed API.

---

## Embeddings

### `set_embedder(&mut self, embedder: Box<dyn TextEmbedder>)`

Swap the embedding backend at runtime.

### `embedder_dimension() -> usize`

Query the current embedder's vector dimension.

### `build_search_index() -> Result<()>`

Rebuild the HNSW search index over all entities. Called automatically on first search, but can be forced after embedder swaps or bulk imports.

---

## Sessions

### `record_session(params: SessionParams) -> Result<()>`

Record a new LLM session start. Idempotent -- duplicate session_id is silently ignored.

```rust
pub struct SessionParams {
    pub session_id: String,
    pub agent_name: String,
    pub project: String,
    pub tool: String,
    pub trigger: String,           // "cli" | "subagent" | "hook"
    pub model: Option<String>,
    pub git_branch: Option<String>,
    pub git_head: Option<String>,
    pub parent_session_id: Option<String>,
}
```

### `end_session(params: EndSessionParams) -> Result<()>`

Patch a session with completion metrics.

```rust
pub struct EndSessionParams {
    pub session_id: String,
    pub exit_status: String,
    pub prompt_count: i64,
    pub tool_call_count: i64,
    pub file_write_count: i64,
    pub commit_count: i64,
    pub test_run_count: i64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_cost_usd: f64,
}
```

### `update_session_progress(params: SessionProgressParams) -> Result<()>`

Update a running session with incremental progress (tool calls, file writes, etc.) without ending it.

### `query_sessions(project: &str, last_n: i64, parent_id: Option<&str>) -> Result<Vec<SessionSummary>>`

Returns up to `last_n` sessions, newest first. Filter by parent_id for child sessions. This is a cached compatibility wrapper over `query_sessions_page`.

### `query_sessions_page(project: Option<&str>, parent_id: Option<&str>, offset: usize, limit: i64) -> Result<Vec<SessionSummary>>`

Primary paginated session query. Uses SQL `LIMIT ? OFFSET ?` and is not cached.

```rust
pub struct SessionSummary {
    pub session_id: String,
    pub project: String,
    pub git_branch: Option<String>,
    pub trigger: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub exit_status: Option<String>,
    pub tool_call_count: i64,
    pub file_write_count: i64,
    pub commit_count: i64,
    pub parent_session_id: Option<String>,
    pub last_tool: Option<String>,
    pub last_tool_summary: Option<String>,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_cost_usd: f64,
}
```

### `record_subagent_handover(session_id, summary, files_changed, outcome) -> Result<()>`

Store handover note as `subagent_handover` event. Readable via `query_events`.

---

## Evidence

### `record_evidence_prompt(params: PromptParams) -> Result<()>`

Record a prompt/completion exchange within a session.

### `record_evidence_tool_call(params: ToolCallParams) -> Result<()>`

```rust
pub struct ToolCallParams {
    pub session_id: String,
    pub tool_name: String,
    pub tool_version: Option<String>,
    pub input_hash: Option<String>,
    pub input_summary: Option<String>,
    pub output_hash: Option<String>,
    pub output_summary: Option<String>,
    pub exit_status: String,
    pub latency_ms: i64,
    pub input_tokens_est: Option<i64>,
    pub tool_category: String,     // "shell" | "file_read" | "file_write" | "agent" | "network" | "other"
}
```

### `record_evidence_file_write(params: FileWriteParams) -> Result<()>`

```rust
pub struct FileWriteParams {
    pub session_id: String,
    pub file_path: String,
    pub file_id: Option<String>,
    pub before_hash: Option<String>,
    pub after_hash: Option<String>,
    pub lines_added: i64,
    pub lines_deleted: i64,
    pub lines_changed: i64,
    pub write_type: String,    // "create" | "edit" | "delete"
}
```

### `record_evidence_file_access(params: FileAccessParams) -> Result<()>`

Record a file read or access event. Creates `accessed` edges linking the session to the file.

### `record_evidence_commit(params: CommitParams) -> Result<()>`

```rust
pub struct CommitParams {
    pub session_id: String,
    pub commit_sha: String,
    pub parent_sha: Option<String>,
    pub message: String,
    pub author: String,
    pub files_changed: i64,
    pub lines_inserted: i64,
    pub lines_deleted: i64,
    pub commit_type: String,   // "feat" | "fix" | "chore" | etc.
    pub feature_tag: Option<String>,
}
```

### `record_evidence_test_run(params: TestRunParams) -> Result<()>`

```rust
pub struct TestRunParams {
    pub session_id: String,
    pub test_name: String,
    pub test_suite: Option<String>,
    pub test_command: Option<String>,
    pub result: String,        // "pass" | "fail" | "skip"
    pub duration_ms: i64,
    pub logs_summary: Option<String>,
    pub commit_sha: Option<String>,
}
```

### `record_evidence_fix_chain(params: FixChainParams) -> Result<()>`

Record a fix chain linking a bug to its resolution commit.

### `record_evidence_bench_run(params: BenchRunParams) -> Result<()>`

Record a benchmark execution result.

### `query_events(session_id: Option<&str>, event_type: Option<&str>, limit: usize) -> Result<Vec<Value>>`

Query the event log. Both filters are optional. Returns JSON values. This is a cached compatibility wrapper over `query_events_page`.

### `query_events_page(session_id: Option<&str>, event_type: Option<&str>, offset: usize, limit: usize) -> Result<Vec<Value>>`

Primary paginated event log query. Uses SQL `LIMIT ? OFFSET ?` and is not cached.

---

## Discoveries

### `store_discovery(agent, discovery_type, target, metadata) -> Result<i64>`

Returns the entity ID.

```rust
// metadata JSON fields consumed by atheneum:
// - "project_id": scopes to a project
// - "why": human-readable reason (shown in context dumps)
// - "file", "line": source location
```

### `store_discovery_in_project(agent, discovery_type, target, project_id, metadata) -> Result<i64>`

Convenience wrapper -- sets `project_id` in metadata.

### `query_discoveries(target) -> Result<Vec<GraphEntity>>`

All discoveries for a target symbol across all projects.

### `query_discoveries_in_project(target, project_id) -> Result<Vec<GraphEntity>>`

Scoped to a project.

### `recent_project_context(project, limit) -> Result<Vec<GraphEntity>>`

Most recent `limit` discoveries for a project, no target filter. Used by hooks to push context into agent startup.

### `preview_discovery(agent, discovery_type, target, metadata, candidate_limit, score_threshold) -> Result<DiscoveryPreview>`

Read-only preview of a discovery payload before commit. Returns normalized payload, deterministic content hash, and any existing matches. Does not mutate the graph.

---

## Knowledge

### `query_knowledge(target) -> Result<Value>`

Aggregated knowledge for a target across all projects.

### `query_knowledge_in_project(target, project_id) -> Result<Value>`

Returns `{ discoveries: [...], handoffs: [...] }` for a target in a project.

---

## Memory

### `store_memory(key, content, scope, confidence, project_id, tags) -> Result<i64>`

Store or update a memory entry. Upsert by composite key (key, scope, project_id). Auto-indexed in HNSW.

```rust
let id = graph.store_memory(
    "timezone",       // key
    "UTC+1",          // content
    "user",           // scope: "user" | "project" | "agent" | "memory"
    0.9,              // confidence: 0.0-1.0
    None,             // project_id
    None,             // tags: Option<&[String]>
)?;
```

### `query_memory(key, scope, project_id) -> Result<Vec<GraphEntity>>`

Retrieve memories by key. Scope and project filters are optional.

### `list_memory(scope, project_id) -> Result<Vec<GraphEntity>>`

List all memories. Filters are optional. This is a cached compatibility wrapper over `list_memory_page`.

### `list_memory_page(scope: Option<&str>, project_id: Option<&str>, offset: usize, limit: usize) -> Result<Vec<GraphEntity>>`

Primary paginated memory list. Uses SQL `LIMIT ? OFFSET ?` and is not cached.

### `preview_memory(key, content, scope, confidence, project_id, tags, candidate_limit, score_threshold) -> Result<MemoryPreview>`

Read-only preview of a memory payload before commit. Returns normalized payload, content hash, and existing matches.

---

## Handoffs

### `store_handoff(from_agent, to_agent, project_id, manifest) -> Result<i64>`

Create a pending task handoff between agents.

### `query_pending_handoffs(agent, project_id) -> Result<Vec<GraphEntity>>`

List unclaimed handoffs for an agent.

### `claim_handoff(handoff_id) -> Result<bool>`

Claim a handoff. Returns `true` if successful.

### `preview_handoff(from_agent, to_agent, project_id, manifest, candidate_limit, score_threshold) -> Result<HandoffPreview>`

Read-only preview of a handoff payload before commit.

---

## Tasks (Planning)

### `create_task(title, description, project_id) -> Result<i64>`

Create a new task. Returns the entity ID.

### `add_requirement(task_id, statement, verification_method) -> Result<()>`

Add an acceptance criterion to a task.

### `add_blocker(task_id, description, blocker_type) -> Result<()>`

```rust
pub enum BlockerType { Dependency, Bug, InfoGap }
```

### `update_task_status(task_id, status) -> Result<()>`

```rust
pub enum KanbanStatus { Todo, InProgress, Done, Blocked, Archived }
```

### `list_tasks(project_id) -> Result<Vec<GraphEntity>>`

List all non-archived tasks, optionally filtered by project.

### `list_tasks_by_status(status, project_id) -> Result<Vec<GraphEntity>>`

List tasks with a specific status.

### `get_task_detail(task_id) -> Result<TaskDetail>`

Get full task details including requirements, blockers, and status history.

---

## Dream

### `dream_pass(mode, scope, project_id, config) -> Result<DreamReport>`

Run the reflective memory consolidation pipeline.

```rust
pub enum DreamMode { DryRun, AutoMerge }

pub struct DreamConfig {
    pub similarity_threshold: f64,   // default: 0.65
    pub stale_days: i64,             // default: 30
    pub min_confidence: f64,         // default: 0.5
    // ... tunable knobs for all phases
}

pub struct DreamReport {
    pub findings: Vec<DreamFinding>,
    pub total_scanned: usize,
    pub duration_ms: u64,
}

pub struct DreamFinding {
    pub phase: DreamPhase,
    pub entity_ids: Vec<i64>,
    pub description: String,
    pub score: f64,
}

pub enum DreamPhase {
    Scan, Deduplicate, Stale, Contradiction, Verbose, Consolidated,
}
```

### `wiki_dream_pass(mode, project_id, config) -> Result<DreamReport>`

Same consolidation pipeline applied to wiki page entities instead of memories.

---

## Wiki

### `ingest_wiki_page(path, content, project_id) -> Result<i64>`

Parses Markdown frontmatter and `[[wikilinks]]`. Creates stub entities for missing targets. Returns the entity ID.

### `ingest_journal_sections(sections, project_id) -> Result<()>`

Batch-ingest pre-parsed journal sections.

### `ingest_journal(path, content, project_id) -> Result<Vec<i64>>`

Parse and ingest a single journal file. Returns entity IDs.

### `parse_journal_sections(content) -> Result<Vec<JournalSection>>`

Parse journal content into structured sections. Does not write to the graph.

### `sync_wiki_directory(dir, project_id) -> Result<Vec<i64>>`

Sync all `.md` files in a directory. Returns entity IDs.

### `get_wiki_page(path) -> Result<Option<WikiPage>>`

```rust
pub struct WikiPage {
    pub id: i64,
    pub path: String,
    pub title: Option<String>,
    pub body: String,
    pub content_hash: Option<String>,
    pub wikilinks: Vec<String>,
    pub project_id: Option<String>,
    pub created_at: String,
    pub updated_at: Option<String>,
}
```

### `list_wiki_pages(project_id) -> Result<Vec<WikiPage>>`

List all wiki pages, optionally filtered by project. This is a cached compatibility wrapper over `list_wiki_pages_page`.

### `list_wiki_pages_page(project_id: Option<&str>, offset: usize, limit: usize) -> Result<Vec<WikiPage>>`

Primary paginated wiki page list. Uses SQL `LIMIT ? OFFSET ?` and is not cached.

### `find_pages_by_wikilink(target) -> Result<Vec<WikiPage>>`

Find all wiki pages that contain a `[[target]]` wikilink.

### `outgoing_wikilinks(page_id) -> Result<Vec<GraphEntity>>`

Get entities that a wiki page links to via `wikilink` edges.

### `incoming_wikilinks(page_id) -> Result<Vec<GraphEntity>>`

Get entities that link to a wiki page via `wikilink` edges.

### `link_wiki_to_symbols(magellan_db_path, agent_name, project_id) -> Result<()>`

Bridge wiki content to code symbols via magellan. For each wiki page's `[[wikilinks]]`, queries the magellan DB for matching code symbols, imports them as Discovery entities, and creates `Explains` edges. Idempotent.

### `extract_wikilinks(content) -> Vec<String>`

Extract `[[wikilink]]` targets from markdown content. Utility function.

### `extract_kanban_updates(content) -> Vec<KanbanUpdate>`

Extract kanban status transitions from journal content.

### `content_hash(content) -> String`

Compute a deterministic SHA-256 content hash.

---

## Claude Transcripts

### `sync_claude_transcript(params: ClaudeTranscriptImportParams) -> Result<ClaudeTranscriptImportSummary>`

Import a Claude Code transcript JSONL into the session graph. Records prompt/chat summaries, tool calls, file accesses, and token totals. Incremental -- re-running on the same append-only transcript imports only new lines.

```rust
pub struct ClaudeTranscriptImportParams {
    pub transcript_path: PathBuf,
    pub session_id: Option<String>,
    pub project: Option<String>,
    pub agent_name: String,
    pub tool: String,
    pub trigger: String,
}
```

---

## Search

### `full_text_search(query) -> Result<Vec<SearchResult>>`

FTS5 over all entities. Fast keyword search.

### `lexical_search(query, k, project_id, entity_kind) -> Result<Vec<SearchResult>>`

HNSW index over hash-projected token vectors. Finds entities sharing tokens with `query`.
**Lexical similarity only** -- no neural model, no synonym awareness. Synonyms with no token
overlap score 0. Fast and dependency-free; good for symbol/identifier search.

```rust
pub struct SearchResult {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub score: f32,
    pub data: Value,
}
```

### `preview_entity_candidates(query, limit, project_id, entity_kind, score_threshold) -> Result<Vec<SearchResult>>`

Fuzzy entity lookup over the search index without mutation. Returns ranked candidates.

---

## HopGraph

### `hopgraph_query(query, k, depth, allowed_types, max_tokens, project_id) -> Result<Vec<SubgraphView>>`

Main retrieval API: lexical search -> filtered BFS subgraph -> token-budgeted truncation.

```rust
let views = graph.hopgraph_query(
    "session accountability",
    3,                                          // k: max entry points
    2,                                          // depth: BFS expansion depth
    Some(&[EdgeType::Explains, EdgeType::Wikilink]),
    2000,                                       // max_tokens budget per view
    None,                                       // project_id
)?;
```

### `navigate(query, k, depth, project_id, entity_kind) -> Result<Vec<SubgraphView>>`

Search + subgraph walk. Like `hopgraph_query` but without token budgeting.

### `preview_navigate_query(query, k, depth, project_id, entity_kind) -> Result<NavigateQueryPlan>`

Validate and repair a navigation query before execution. Trims whitespace, resolves entity-kind aliases (`memory` -> `Memory`, `wiki` -> `WikiPage`), rejects unknown kinds.

### `get_neighbors(entity_id) -> Result<(Vec<GraphEdge>, Vec<GraphEdge>)>`

One-hop edges: returns (outgoing, incoming).

### `get_subgraph(entry_id, depth) -> Result<SubgraphView>`

BFS subgraph extraction.

### `get_subgraph_scoped(entry_id, depth, project_id) -> Result<SubgraphView>`

BFS subgraph filtered by project.

### `get_subgraph_filtered(entry_id, depth, allowed_types) -> Result<SubgraphView>`

BFS subgraph following only allowed edge types. Empty whitelist returns all edges.

### `estimate_entity_tokens(entity) -> usize`

Rough token count (~4 chars/token).

### `truncate_subgraph(view, max_tokens) -> SubgraphView`

Trim a subgraph view to fit a token budget. Entry entity always kept.

### Discovery Consolidation

### `consolidate_discoveries(target, project_id) -> Result<Option<i64>>`

Merge all Discovery entities for a target into a single Knowledge entity with `DerivedFrom` edges. Returns the Knowledge entity ID. Idempotent.

### `consolidation_pass(project_id) -> Result<Vec<(String, i64)>>`

Consolidate all distinct discovery targets. Returns (target, knowledge_id) pairs.

---

## Graph Introspection

### `get_entity(id) -> Result<GraphEntity>`

Retrieve a single entity by ID.

### `get_edge(id) -> Result<GraphEdge>`

Retrieve a single edge by ID.

### `outgoing_edges(entity_id) -> Result<Vec<GraphEdge>>`

All outgoing edges from an entity.

### `incoming_edges(entity_id) -> Result<Vec<GraphEdge>>`

All incoming edges to an entity.

### `all_entities() -> Result<Vec<GraphEntity>>`

Return every entity in the graph.

### `entities_by_kind(kind) -> Result<Vec<GraphEntity>>`

Filter entities by type string (e.g., "Discovery", "WikiPage").

### `count_entities_by_kind() -> Result<Vec<(String, i64)>>`

Entity counts grouped by type.

### `count_edges_by_type() -> Result<Vec<(String, i64)>>`

Edge counts grouped by type.

### `graph_stats() -> Result<GraphStats>`

Summary counts: total entities, total edges, breakdowns by kind and type.

### `insert_agent(name, data) -> Result<i64>`

Create an Agent entity.

### `insert_task(name, data) -> Result<i64>`

Create a Task entity.

### `insert_event(name, data) -> Result<i64>`

Create an Event entity.

### `insert_edge(from_id, to_id, edge_type, data) -> Result<i64>`

Create a typed edge between two entities.

### `events_performed_by(agent_id) -> Result<Vec<GraphEntity>>`

All events attributed to an agent.

### `tasks_assigned_to(agent_id) -> Result<Vec<GraphEntity>>`

All tasks assigned to an agent.

### `causal_chain(event_id) -> Result<Vec<GraphEntity>>`

Trace a causal chain from an event.

---

## Ontology

### `register_ontology_class(name, description) -> Result<i64>`

### `register_ontology_property(name, domain_class, range_class, description) -> Result<i64>`

### `get_ontology_classes() -> Result<Vec<OntologyClassInfo>>`

### `validate_edge(from_kind, edge_type, to_kind) -> Result<bool>`

---

## Graph Entity Types

| Kind | Description |
|------|-------------|
| `Agent` | LLM session identity |
| `Session` | Session record |
| `Task` | Planning task |
| `Event` | Event log entry |
| `ToolCall` | Single tool invocation |
| `Knowledge` | Consolidated knowledge / wiki page |
| `WikiPage` | Wiki page node |
| `JournalSection` | Journal entry |
| `Discovery` | Stored finding |
| `Handoff` | Inter-agent state transfer |
| `Memory` | Keyed memory entry |
| `Commit` | Git commit evidence |
| `TestRun` | Test execution record |

## Edge Types

| Type | Meaning |
|------|---------|
| `PerformedBy` | action -> agent |
| `AssignedTo` | task -> agent |
| `Called` | reasoning -> tool_call |
| `Calls` | caller -> callee |
| `Accessed` | session -> file |
| `Modified` | tool_call -> entity |
| `VerifiedBy` | session -> test_run |
| `CausedBy` | fix -> bug |
| `Created` | agent -> entity |
| `RelatedTo` | general semantic link |
| `Mentions` | entity -> entity (reference) |
| `Wikilink` | wiki page -> wiki page |
| `Implements` | code -> specification |
| `DependsOn` | entity -> dependency |
| `TestedBy` | code -> test |
| `FixedBy` | bug -> fix commit |
| `RegressedBy` | fix -> new bug |
| `ObservedIn` | finding -> context |
| `BelongsToProject` | entity -> project |
| `SimilarFailure` | failure -> failure |
| `RequiresSkill` | task -> skill |
| `HandledByTool` | task -> tool |
| `Explains` | wiki page -> code symbol |
| `DerivedFrom` | consolidated -> source |
| `SupersededBy` | old entry -> replacement |
| `ConsolidatedFrom` | merge target -> source |

---

## Error Types

```rust
pub enum AtheneumError {
    GraphError(sqlitegraph::SqliteGraphError),
    EntityNotFound(i64),
    EdgeNotFound(i64),
    InvalidData(String),
}
```

All public functions return `anyhow::Result<T>` for ergonomic `?` propagation.
