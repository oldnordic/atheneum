# SQL + Graph Separation — Design Spec

**Status:** Approved 2026-05-19. Scope this session: Stage 11a (foundation) + Stage 11b (Execution domain).

## Problem

Atheneum currently stores every entity as a row in sqlitegraph's `graph_entities` table with a JSON `data` blob. After ten stages of feature work, three problems are converging:

1. **Dashboard queries** (aggregations, time-windowed counts, joins) require `json_extract(data, '$.field')` on every row in `graph_entities`. Fine for hundreds of rows, painful past tens of thousands.
2. **Scale**: high-volume types — tool calls, reasoning logs, journal sections — share the same table as everything else. As the audit trail grows, *every* query gets slower, not just queries that touch high-volume kinds.
3. **Data integrity**: foreign-key fields like `requirement.task_id` are JSON strings. Nothing prevents a typo or a dangling reference. Status enums are strings without CHECK constraints. Project ids are JSON values without indexes.

The Python atheneum-py was designed with this split (see `atheneum-py-sketch.md` — "Graph as Metadata, SQL as Data"). The Rust port has been working in graph-only mode through stage 10. This spec ports the split.

## Goals

- Typed SQL tables with proper columns, indexes, and FOREIGN KEY constraints for high-volume entity kinds.
- Keep sqlitegraph for the *relationship* layer (graph_edges) so Cypher traversal queries keep working.
- A schema-versioning system that runs idempotent migrations on `AtheneumGraph::open()`, including a one-shot backfill from old `graph_entities` rows.
- Public method signatures unchanged. The 71 existing atheneum tests + 137 envoy tests stay green through every commit.

## Non-goals (this spec)

- Migrating Planning domain (tasks/requirements/blockers) — that's Stage 11c.
- Migrating Knowledge domain (discoveries/wiki_pages/journal_sections) — that's Stage 11d.
- Dropping `graph_entities` use entirely — it continues to hold OntologyClass / OntologyProperty (small, dynamic, no fixed schema).
- Switching the HNSW index storage. HNSW continues to store vector → metadata, where metadata is `{sql_id}` once 11d lands; for 11b the discovery entity hasn't moved yet so its metadata stays `{entity_id}`.

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│ SQL "Payload" Layer — typed data                                │
├─────────────────────────────────────────────────────────────────┤
│  agents              (Stage 11b)                                │
│  reasoning_logs      (Stage 11b)                                │
│  tool_calls          (Stage 11b)                                │
│  tasks / reqs / blockers          ← Stage 11c                   │
│  discoveries / wiki_pages / …     ← Stage 11d                   │
│  atheneum_schema_version           (Stage 11a)                  │
└─────────────────────────────────────────────────────────────────┘
┌─────────────────────────────────────────────────────────────────┐
│ Graph "Bridge" Layer — sqlitegraph                              │
├─────────────────────────────────────────────────────────────────┤
│  graph_entities — for typed rows, this is a POINTER node:       │
│      { kind: "ReasoningLog", name: …, data: {"sql_id": 42} }    │
│    Plus continues to hold OntologyClass / OntologyProperty.     │
│  graph_edges — relationships between pointer nodes              │
│  Cypher queries traverse over the whole graph                   │
└─────────────────────────────────────────────────────────────────┘
```

**Principle**: data in SQL, relationships in the graph, bridged by `sql_id` pointers.

## Stage 11a — Foundation

### `atheneum_schema_version` table

```sql
CREATE TABLE atheneum_schema_version (
    version INTEGER PRIMARY KEY NOT NULL,
    applied_at TEXT NOT NULL
);
```

Row count == applied versions. Highest version is "current". Empty table = no migrations applied (fresh DB).

### Migration runner

```rust
type Migration = fn(&rusqlite::Transaction) -> Result<()>;

const MIGRATIONS: &[(u32, &str, Migration)] = &[
    (1, "execution-domain", migrate_v1_execution),
    // (2, "planning-domain", migrate_v2_planning),     // Stage 11c
    // (3, "knowledge-domain", migrate_v3_knowledge),   // Stage 11d
];

// Invoked from AtheneumGraph::open() / open_in_memory().
// Wraps each migration in a transaction. Commits + records version on success.
// Skips already-applied versions. No rollback API — migrations are forward-only.
pub(crate) fn run_migrations(conn: &mut rusqlite::Connection) -> Result<()> { … }
```

Migrations are forward-only and idempotent (each uses `CREATE TABLE IF NOT EXISTS` etc., and the version table prevents re-running the *body*).

### Location & module structure

New module `src/db/` with:
- `src/db/mod.rs` — `run_migrations`, `MIGRATIONS`, version helpers.
- `src/db/execution.rs` — Stage 11b migration + table helpers.
- (Later: `src/db/planning.rs`, `src/db/knowledge.rs`.)

## Stage 11b — Execution domain

### Schema

```sql
CREATE TABLE agents (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    project_id TEXT,
    metadata TEXT,                          -- JSON, anything else
    created_at TEXT NOT NULL
);
CREATE INDEX agents_project_idx ON agents(project_id);

CREATE TABLE reasoning_logs (
    id INTEGER PRIMARY KEY,
    agent_id INTEGER NOT NULL REFERENCES agents(id),
    content TEXT NOT NULL,
    project_id TEXT,
    metadata TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX reasoning_logs_agent_idx ON reasoning_logs(agent_id);
CREATE INDEX reasoning_logs_project_idx ON reasoning_logs(project_id);
CREATE INDEX reasoning_logs_created_at_idx ON reasoning_logs(created_at);

CREATE TABLE tool_calls (
    id INTEGER PRIMARY KEY,
    reasoning_log_id INTEGER NOT NULL REFERENCES reasoning_logs(id),
    tool_name TEXT NOT NULL,
    args TEXT NOT NULL,                     -- JSON
    project_id TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX tool_calls_log_idx ON tool_calls(reasoning_log_id);
CREATE INDEX tool_calls_tool_name_idx ON tool_calls(tool_name);
CREATE INDEX tool_calls_project_idx ON tool_calls(project_id);
```

### Bridge: pointer nodes in `graph_entities`

For every SQL row we write, we also write a `graph_entities` pointer node:

```
{ kind: "Agent",         name: <agent.name>,    data: { sql_id: <agents.id>,         project_id: …} }
{ kind: "ReasoningLog",  name: <truncated>,     data: { sql_id: <reasoning_logs.id>, agent: …, content: …, project_id: …, … } }
{ kind: "ToolCall",      name: <tool_name>,     data: { sql_id: <tool_calls.id>,     tool_name: …, args: …, project_id: …, … } }
```

The pointer node's `data` is a **mirror** of the SQL row's relevant fields so that:
- Existing `entities_by_kind` callers keep working (they read the JSON).
- Edge queries can pick up entity info without an extra SQL fetch.
- Tests that compare entity.data["content"] etc. keep passing unchanged.

Edges still go in `graph_edges` between pointer node ids — `(Agent)-[PerformedBy]->(ReasoningLog)-[Called]->(ToolCall)-[Modified]->(target)` unchanged.

### Method refactor

```rust
// insert_agent (internal/existing helper, used by ensure_agent)
INSERT INTO agents (name, project_id, metadata, created_at) VALUES (?,?,?,?) RETURNING id  → sql_id
INSERT graph_entity { kind:"Agent", name, data: { sql_id, ... } }                          → entity_id
return entity_id

// insert_reasoning_log
sql_agent_id  := lookup agents.id for `agent` (already-resolved by ensure_agent)
sql_log_id    := INSERT INTO reasoning_logs (agent_id, content, project_id, …) RETURNING id
entity_id     := INSERT graph_entity { kind:"ReasoningLog", name:…, data:{sql_id:sql_log_id, content:…, agent:…, project_id:…, …} }
                 INSERT graph_edge (agent_entity_id → entity_id, "performed_by")
return entity_id

// insert_tool_call
sql_tool_id   := INSERT INTO tool_calls (reasoning_log_id, tool_name, args, project_id, …) RETURNING id
entity_id     := INSERT graph_entity { kind:"ToolCall", name:tool_name, data:{sql_id:sql_tool_id, tool_name, args, project_id, …} }
                 INSERT graph_edge (log_entity_id → entity_id, "called")
return entity_id

// record_tool_modifies, record_agent_action — unchanged (compose the above)

// get_action_trace
// Still walks the graph (graph_entities + graph_edges) — the data mirror in the pointer
// node's `data` is enough to satisfy callers. SQL-side queries (count, group-by) are now
// possible directly against agents/reasoning_logs/tool_calls.
```

The audit_trail_tests look at `entity.data["content"]`, `entity.data["tool_name"]` etc. — all present in the pointer node's mirrored `data`. So those tests keep passing.

### Backfill (migration body)

For existing `graph_entities` of kind in `{Agent, ReasoningLog, ToolCall}`:

```text
1. Build name→agent_id map: for each Agent graph_entity, INSERT INTO agents and UPDATE
   graph_entities.data WITH sql_id added.
2. For ReasoningLog graph_entities: resolve agent_id via the map (entity.data.agent), INSERT
   INTO reasoning_logs, UPDATE entity.data.sql_id.
3. For ToolCall graph_entities: resolve reasoning_log_id by reading the Called edge from a
   ReasoningLog pointer to this tool_call entity, then INSERT INTO tool_calls.
4. Done. graph_edges untouched.
```

Failure mode: if backfill cannot resolve a FK (e.g., dangling ReasoningLog without an Agent), insert into a synthetic "_legacy" agent and log a warning so data isn't lost. Recorded in `metadata`.

### Tests

#### Already passing → must stay green

- `tests/audit_trail_tests.rs` (6 tests).
- All other atheneum tests (65) — they don't touch Execution methods so they're unaffected, but they share the same `AtheneumGraph::open_in_memory()` which now runs migrations.

#### New for Stage 11

- `tests/sql_separation_tests.rs`:
  - `test_schema_version_recorded_after_open` — open in-memory DB → version 1 row present.
  - `test_open_is_idempotent` — open twice, version 1 has exactly one row.
  - `test_agents_row_exists_after_insert` — insert_reasoning_log → SELECT FROM agents finds the row.
  - `test_reasoning_logs_row_exists_after_insert` — same.
  - `test_tool_calls_row_exists_after_insert` — same.
  - `test_sql_id_pointer_in_graph_entity` — entity.data["sql_id"] matches the SQL row id.
  - `test_backfill_ports_legacy_graph_entities` — insert raw via sqlitegraph (simulating pre-11 data), close, reopen → migrations port it into SQL tables.

## Risk & mitigation

| Risk | Mitigation |
|---|---|
| The 71 existing atheneum tests + 137 envoy tests have to stay green | Mirror SQL row contents into the pointer node's `data` — public API unchanged. Run full suites after each commit. |
| Migration fails on a real `~/.envoy/atheneum.db` and corrupts it | Run inside a transaction. If migration fails, rollback + bail with a clear error, atheneum doesn't open. User can restore from backup. |
| Double-write overhead | Both writes are in the same transaction. SQLite is fast; the cost is small versus the query speed-up. Re-evaluate if profiling shows it matters at scale. |
| Future stages need new columns | Add via a new migration version (no ALTER on existing rows; add columns with NULL defaults). |

## Verification report shape (for the implementation commits)

Standard grounded-coding template — `cargo test`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo audit`, `cargo deny check`, `gitleaks`, `semgrep`, then `gh run watch + gh run view --json conclusion`. CI green via explicit JSON check, not the watch exit code.

## After this spec lands

- Stage 11c (Planning): same pattern for tasks/requirements/blockers.
- Stage 11d (Knowledge): same pattern for discoveries/wiki_pages/journal_sections. HNSW metadata switches from `entity_id` to `sql_id`.
- Stage 11e (optional cleanup): consider whether `graph_entities` should stop holding mirrored data for typed entities, leaving truly bridge-only pointers. Profile-driven; deferred.
