# Atheneum — Public API Reference

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

---

## Sessions

### `record_session(params: SessionParams) → Result<()>`

Record a new LLM session start. Idempotent — duplicate session_id is silently ignored.

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

### `end_session(params: EndSessionParams) → Result<()>`

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

### `query_sessions(project: &str, last_n: i64, parent_id: Option<&str>) → Result<Vec<SessionSummary>>`

Returns up to `last_n` sessions, newest first. Filter by parent_id for child sessions.

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

### `record_subagent_handover(session_id, summary, files_changed, outcome) → Result<()>`

Store handover note as `subagent_handover` event. Readable via `query_events`.

---

## Evidence

### `record_evidence_tool_call(params: ToolCallParams) → Result<()>`

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

### `record_evidence_file_write(params: FileWriteParams) → Result<()>`

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

### `record_evidence_commit(params: CommitParams) → Result<()>`

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

### `record_evidence_test_run(params: TestRunParams) → Result<()>`

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

### `query_events(session_id, event_type, limit) → Result<Vec<Value>>`

Query the event log. Both filters are optional.

---

## Discoveries

### `store_discovery(agent, discovery_type, target, metadata) → Result<i64>`

Returns the entity ID.

```rust
// metadata JSON fields consumed by atheneum:
// - "project_id": scopes to a project
// - "why": human-readable reason (shown in context dumps)
// - "file", "line": source location
```

### `store_discovery_in_project(agent, discovery_type, target, project_id, metadata) → Result<i64>`

Convenience wrapper — sets `project_id` in metadata.

### `query_discoveries(target) → Result<Vec<GraphEntity>>`

All discoveries for a target symbol across all projects.

### `query_discoveries_in_project(target, project_id) → Result<Vec<GraphEntity>>`

Scoped to a project.

### `recent_project_context(project, limit) → Result<Vec<GraphEntity>>`

Most recent `limit` discoveries for a project, no target filter. Used by hooks to push context into agent startup.

---

## Knowledge

### `query_knowledge_in_project(target, project_id) → Result<Value>`

Returns `{ discoveries: [...], handoffs: [...] }` for a target in a project.

---

## Handoffs

### `store_handoff(from_agent, to_agent, project_id, manifest) → Result<i64>`

### `query_pending_handoffs(agent, project_id) → Result<Vec<GraphEntity>>`

### `claim_handoff(handoff_id) → Result<bool>`

---

## Tasks (Planning)

### `create_task(title, project_id) → Result<i64>`

### `add_requirement(task_id, statement, verification_method) → Result<()>`

### `add_blocker(task_id, description, blocker_type) → Result<()>`

```rust
pub enum BlockerType { Dependency, Bug, InfoGap }
```

### `update_task_status(task_id, status) → Result<()>`

```rust
pub enum KanbanStatus { Todo, InProgress, Done, Blocked }
```

### `get_task_detail(task_id) → Result<TaskDetail>`

---

## Wiki

### `ingest_wiki_page(path, content, project_id) → Result<i64>`

Parses Markdown frontmatter and `[[wikilinks]]`. Creates stub entities for missing targets.

### `ingest_journal_sections(sections, project_id) → Result<()>`

### `parse_journal_sections(content) → Result<Vec<JournalSection>>`

### `sync_wiki_directory(dir, project_id) → Result<usize>`

Returns count of pages synced.

---

## Search

### `full_text_search(query) → Result<Vec<SearchResult>>`

FTS5 over all entities.

### `semantic_search(query, k, project_id) → Result<Vec<SearchResult>>`

HNSW vector search via sqlitegraph. Requires embeddings to be built.

```rust
pub struct SearchResult {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub score: f32,
    pub data: Value,
}
```

---

## Ontology

### `register_ontology_class(name, description) → Result<i64>`

### `register_ontology_property(name, domain_class, range_class, description) → Result<i64>`

### `get_ontology_classes() → Result<Vec<OntologyClassInfo>>`

### `validate_edge(from_kind, edge_type, to_kind) → Result<bool>`

---

## Graph Entity Types

| Kind | Description |
|------|-------------|
| `Agent` | LLM session identity |
| `Task` | Planning task |
| `Event` | Event log entry |
| `ToolCall` | Single tool invocation |
| `Knowledge` | Wiki page or note |
| `Discovery` | Stored finding |
| `Handoff` | Inter-agent state transfer |
| `Session` | Session record |
| `Commit` | Git commit evidence |
| `TestRun` | Test execution record |

## Edge Types

| Type | Meaning |
|------|---------|
| `PerformedBy` | action → agent |
| `AssignedTo` | task → agent |
| `Called` | reasoning → tool_call |
| `Modified` | tool_call → entity |
| `VerifiedBy` | session → test_run |
| `CausedBy` | fix → bug |
| `Created` | agent → entity |
| `RelatedTo` | general semantic link |

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
