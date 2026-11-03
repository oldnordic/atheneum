# TECH: Atheneum Memory Store and Native Router

## Scope

This spec covers:

1. Additive Atheneum schema updates for canonical memory/state records.
2. Safe merge/import design for existing Atheneum, Hermes, and Bookkeeper memory.
3. Native Pi router architecture that uses local CLIs and avoids MCP.
4. Phased implementation suitable for small Orca/agy dispatches.

This spec does not authorize production DB mutation. All migration and merge work begins on disposable copies.

## Host-specific repositories and stores

Do not assume Desktop and Strix have the same files, binaries, or databases.

### Desktop

- Atheneum source: `/home/feanor/Projects/atheneum`
- Installed Atheneum CLI: `/home/feanor/.local/bin/atheneum`
- Candidate canonical Atheneum DB: `/home/feanor/.atheneum/atheneum.db`
- Pi Atheneum native extension source: `/home/feanor/Projects/pi-atheneum`
- Pi Hermes Memory package: `/home/feanor/.pi/agent/git/github.com/chandra447/pi-hermes-memory`
- Hermes storage: `/home/feanor/.pi/agent/pi-hermes-memory/`, `/home/feanor/.pi/agent/projects-memory/`
- Memory Bookkeeper source: `/home/feanor/Projects/memory-bookkeeper`
- Memory Bookkeeper DB: `/home/feanor/.local/share/memory-bookkeeper/memory.sqlite`

### Strix

- Atheneum source used by Orca workers: `/home/feanor/Projects/atheneum`
- Worker worktrees: `/home/feanor/orca/workspaces/atheneum/...`
- Strix Atheneum DB: `/home/feanor/.atheneum/atheneum.db`
- External Pi/Bookkeeper/Hermes paths may be absent and must be probed per host.

The large Desktop DB is the proposed canonical target. Smaller host DBs, including the Strix DB found during adversarial review, are merge sources until Phase 0 proves otherwise.

## Current Atheneum architecture

Atheneum uses a graph layer plus typed SQL payload tables. Migrations are forward-only and transactional (`crates/atheneum/src/db/mod.rs`). Current registered migrations run through version 14:

1. execution domain
2. planning domain
3. knowledge domain
4. evidence domain
5. hook compatibility
6. transcript imports
7. memory domain
8. planning archive fix
9. wiki FTS
10. wiki FTS path
11. discovery session IDs
12. chat columns/FTS
13. discovery context snapshots
14. grounded claims

Important current tables:

- `memory_entries`: key, scope, content, confidence, project_id, created_at, updated_at
- `tasks`, `requirements`, `blockers`
- `discoveries`, `wiki_pages`, `journal_sections`
- `sessions`, `commits`, `test_runs`, `bench_runs`, `releases`, `fix_chains`, `event_log`
- `grounded_claims`

Important existing merge support:

- `export-ledger` and `import-ledger` support discoveries, memories, and tasks.
- Import uses normal store methods, not raw SQL inserts.
- Dedup key is `(kind, agent, target, content_hash)`.
- Import can produce an audit map.

Important current hazards:

- `store_memory` upserts by `(key, scope, project_id)` and overwrites existing content when a colliding logical key arrives with a different hash.
- Existing public IDs returned by graph APIs are `graph_entities.id`; SQL payload IDs are internal `sql_id` values unless explicitly documented.
- Bare SQLite `ALTER TABLE ADD COLUMN` is not idempotent; migrations must check `PRAGMA table_info` before adding columns.

## Identified gaps

The current store lacks first-class fields/tables for:

- canonical memory kind (`decision`, `article`, `receipt`, etc.)
- short title and bounded summary separate from full content
- lifecycle status and validity windows
- source URI and evidence lists
- supersedes/superseded-by relations using graph entity IDs
- token estimate
- sensitivity/redaction labels
- project state history and head
- agent state
- arbitrary artifacts and verification receipts
- external-store identity mapping for Hermes/Bookkeeper imports
- router query audit records
- graph entity variants and edges for the new record types

## Design invariants

1. **Dual-layer invariant.** Every user-visible record has a graph entity and a typed SQL payload row. Internal migration bookkeeping may be SQL-only.
2. **Public ID invariant.** Public APIs, router responses, and ledger records use graph entity IDs. SQL IDs are internal and exported only as `sql_id`.
3. **No silent overwrite.** Import/update paths never replace existing content when the incoming logical key collides but content hash differs. They create a new revision or emit a conflict.
4. **Append-only state history.** Project and agent state updates create new versions; the latest head is a query/view over history.
5. **Idempotent migrations.** Every `ALTER TABLE` is guarded by `PRAGMA table_info`; every migration can be re-run safely.
6. **CLI compatibility.** Existing commands such as `memory-get <key>` remain unchanged.
7. **Evidence by reference.** Records cite files, commands, receipts, commits, transcript URIs, or external record IDs.

## Proposed additive migration: v15 `memory-router-domain`

Migration v15 must be additive, transactional, and idempotent. It must also create any missing graph ontology entries and backfill graph entities for new payload rows.

### Extend `memory_entries`

Add guarded columns:

```sql
kind TEXT NOT NULL DEFAULT 'memory';
title TEXT;
summary TEXT;
status TEXT NOT NULL DEFAULT 'active';
source_uri TEXT;
evidence_json TEXT NOT NULL DEFAULT '[]';
content_hash TEXT;
token_estimate INTEGER;
sensitivity TEXT NOT NULL DEFAULT 'normal';
valid_from TEXT;
valid_to TEXT;
ttl_seconds INTEGER;
graph_entity_id INTEGER;
supersedes_graph_entity_id INTEGER;
```

Indexes:

```sql
CREATE INDEX IF NOT EXISTS memory_kind_idx ON memory_entries(kind);
CREATE INDEX IF NOT EXISTS memory_status_idx ON memory_entries(status);
CREATE INDEX IF NOT EXISTS memory_project_kind_idx ON memory_entries(project_id, kind);
CREATE INDEX IF NOT EXISTS memory_content_hash_idx ON memory_entries(content_hash);
CREATE INDEX IF NOT EXISTS memory_graph_entity_idx ON memory_entries(graph_entity_id);
CREATE INDEX IF NOT EXISTS memory_supersedes_graph_idx ON memory_entries(supersedes_graph_entity_id);
```

Backfill rules:

- Existing rows keep `kind = 'memory'` unless an adapter can classify them with evidence.
- Existing `content` remains authoritative.
- `summary` is generated only by an explicit migration utility, never by silent truncation in the open path.
- Existing graph `data.content_hash` is copied into SQL when present.
- `graph_entity_id` is backfilled from the corresponding `Memory` graph entity whose `data.sql_id` matches the SQL row.
- Supersession always references `graph_entities.id`, not `memory_entries.id`.

### New graph entity kinds

Add `EntityType` variants and ontology entries for:

- `ProjectState`
- `AgentState`
- `Artifact`
- `Receipt`
- `Source`
- `ExternalMemoryRef`
- `RouterQuery`

Router queries may also be represented as `Event` entities if the implementation chooses not to expose them to search.

### New table: `project_states`

Append-only history; latest head is selected by max version:

```sql
CREATE TABLE IF NOT EXISTS project_states (
  id INTEGER PRIMARY KEY,
  graph_entity_id INTEGER NOT NULL UNIQUE,
  project_id TEXT NOT NULL,
  version INTEGER NOT NULL,
  is_head INTEGER NOT NULL DEFAULT 0,
  objective TEXT,
  state_json TEXT NOT NULL,
  branch TEXT,
  head_sha TEXT,
  constraints_json TEXT NOT NULL DEFAULT '[]',
  active_task_ids_json TEXT NOT NULL DEFAULT '[]',
  blocker_ids_json TEXT NOT NULL DEFAULT '[]',
  decision_ids_json TEXT NOT NULL DEFAULT '[]',
  receipt_ids_json TEXT NOT NULL DEFAULT '[]',
  changelog TEXT,
  evidence_json TEXT NOT NULL DEFAULT '[]',
  token_estimate INTEGER,
  sensitivity TEXT NOT NULL DEFAULT 'normal',
  created_at TEXT NOT NULL,
  UNIQUE(project_id, version)
);
CREATE INDEX IF NOT EXISTS project_states_project_head_idx ON project_states(project_id, is_head);
CREATE INDEX IF NOT EXISTS project_states_project_created_idx ON project_states(project_id, created_at);
```

### New table: `agent_states`

Append-only agent state history:

```sql
CREATE TABLE IF NOT EXISTS agent_states (
  id INTEGER PRIMARY KEY,
  graph_entity_id INTEGER NOT NULL UNIQUE,
  agent_name TEXT NOT NULL,
  project_id TEXT,
  session_id TEXT,
  dispatch_id TEXT,
  task_id INTEGER REFERENCES tasks(id),
  version INTEGER NOT NULL,
  status TEXT NOT NULL,
  current_objective TEXT,
  state_json TEXT NOT NULL DEFAULT '{}',
  budget_json TEXT NOT NULL DEFAULT '{}',
  last_receipt_graph_entity_id INTEGER,
  sensitivity TEXT NOT NULL DEFAULT 'normal',
  created_at TEXT NOT NULL,
  UNIQUE(agent_name, project_id, dispatch_id, version)
);
CREATE INDEX IF NOT EXISTS agent_states_project_idx ON agent_states(project_id);
CREATE INDEX IF NOT EXISTS agent_states_agent_idx ON agent_states(agent_name);
CREATE INDEX IF NOT EXISTS agent_states_dispatch_idx ON agent_states(dispatch_id);
```

### New table: `artifacts`

```sql
CREATE TABLE IF NOT EXISTS artifacts (
  id INTEGER PRIMARY KEY,
  graph_entity_id INTEGER NOT NULL UNIQUE,
  project_id TEXT,
  task_id INTEGER REFERENCES tasks(id),
  artifact_kind TEXT NOT NULL,
  path TEXT NOT NULL,
  sha256 TEXT NOT NULL,
  bytes INTEGER,
  commit_sha TEXT,
  created_by TEXT,
  sensitivity TEXT NOT NULL DEFAULT 'normal',
  metadata_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL,
  UNIQUE(project_id, path, sha256)
);
```

### New table: `receipts`

```sql
CREATE TABLE IF NOT EXISTS receipts (
  id INTEGER PRIMARY KEY,
  graph_entity_id INTEGER NOT NULL UNIQUE,
  project_id TEXT,
  task_id INTEGER REFERENCES tasks(id),
  artifact_graph_entity_id INTEGER,
  session_id TEXT REFERENCES sessions(session_id),
  verdict TEXT NOT NULL,
  verifier TEXT NOT NULL,
  command TEXT,
  exit_code INTEGER,
  duration_ms INTEGER,
  output_hash TEXT,
  output_sample_redacted TEXT,
  evidence_json TEXT NOT NULL DEFAULT '[]',
  sensitivity TEXT NOT NULL DEFAULT 'normal',
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS receipts_project_idx ON receipts(project_id);
CREATE INDEX IF NOT EXISTS receipts_task_idx ON receipts(task_id);
CREATE INDEX IF NOT EXISTS receipts_artifact_idx ON receipts(artifact_graph_entity_id);
```

### New table: `sources`

```sql
CREATE TABLE IF NOT EXISTS sources (
  id INTEGER PRIMARY KEY,
  graph_entity_id INTEGER NOT NULL UNIQUE,
  project_id TEXT,
  source_type TEXT NOT NULL,
  uri TEXT NOT NULL,
  content_hash TEXT,
  title TEXT,
  summary TEXT,
  author TEXT,
  published_at TEXT,
  retrieved_at TEXT,
  credibility REAL,
  sensitivity TEXT NOT NULL DEFAULT 'normal',
  metadata_json TEXT NOT NULL DEFAULT '{}',
  UNIQUE(project_id, uri)
);
```

### New table: `external_memory_refs`

```sql
CREATE TABLE IF NOT EXISTS external_memory_refs (
  id INTEGER PRIMARY KEY,
  graph_entity_id INTEGER NOT NULL UNIQUE,
  external_store TEXT NOT NULL,
  external_id TEXT NOT NULL,
  atheneum_graph_entity_id INTEGER NOT NULL,
  atheneum_kind TEXT NOT NULL,
  content_hash TEXT NOT NULL,
  sync_state TEXT NOT NULL,
  last_synced_at TEXT NOT NULL,
  metadata_json TEXT NOT NULL DEFAULT '{}',
  UNIQUE(external_store, external_id)
);
```

### New table: `router_queries`

Stores audit metadata, not raw prompts:

```sql
CREATE TABLE IF NOT EXISTS router_queries (
  id INTEGER PRIMARY KEY,
  graph_entity_id INTEGER UNIQUE,
  project_id TEXT,
  agent_name TEXT,
  query_hash TEXT NOT NULL,
  kinds_json TEXT NOT NULL DEFAULT '[]',
  budget_tokens INTEGER NOT NULL,
  returned_ids_json TEXT NOT NULL DEFAULT '[]',
  omitted_count INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL
);
```

## Store API additions

Add public APIs first, then CLI commands:

- `upsert_memory_item(params)`
- `get_memory_item_by_graph_id(id)`
- `route_memory(query, filters, budget, depth)`
- `expand_memory(ids, budget)`
- `append_project_state(project, state)`
- `get_project_state_head(project, budget)`
- `append_agent_state(agent, state)`
- `record_artifact(params)`
- `record_receipt(params)`
- `record_source(params)`
- `link_external_memory(ref)`
- `build_memory_bundle(task, budget)`

CLI names must avoid existing collisions:

- `memory-item-upsert`
- `memory-item-get <graph-entity-id>`
- `memory-route`
- `memory-expand`
- `memory-bundle`
- `project-state-append`
- `project-state-head`
- `agent-state-append`
- `agent-state-head`
- `artifact-record`
- `receipt-record`
- `source-record`
- `external-ref-link`

Existing `memory-store`, `memory-get <key>`, `memory-list`, and `memory-bootstrap` remain unchanged.

All commands emit JSON.

## Merge architecture

Atheneum remains canonical. Existing stores are imported through adapters.

### Atheneum-to-Atheneum merge

Use existing ledger commands only after conflict-safe memory import is implemented:

```bash
atheneum export-ledger source.db --kinds discoveries,memories,tasks > source.ndjson
atheneum import-ledger target.db source.ndjson --dry-run --map source.ndjson.import-map.ndjson
```

Only after dry-run review:

```bash
atheneum import-ledger target.db source.ndjson --map source.ndjson.import-map.ndjson
```

Required change before real import: if a memory record collides on `(key, scope, project_id)` but differs in content hash, import must create a new revision or mark `conflict` in the import map. It must not call the existing overwriting upsert path unchanged.

### Hermes adapter

Phase 3 imports Markdown memory files and fixtures, not `sessions.db` internals.

Desktop input roots, when present:

- `/home/feanor/.pi/agent/pi-hermes-memory/MEMORY.md`
- `/home/feanor/.pi/agent/pi-hermes-memory/USER.md`
- `/home/feanor/.pi/agent/pi-hermes-memory/failures.md`
- `/home/feanor/.pi/agent/projects-memory/*/MEMORY.md`

Worker phases must also accept fixture directories so they do not depend on host-specific paths.

Output: canonical NDJSON with source URI, content hash, kind, scope, project, and sensitivity.

Session DB import is a later phase and must use a reviewed read-only exporter.

### Bookkeeper adapter

Use the Bookkeeper CLI export path where available:

```bash
memory-bookkeeper export-markdown --db /home/feanor/.local/share/memory-bookkeeper/memory.sqlite --output bookkeeper.md
```

The adapter converts exported records to canonical NDJSON and preserves:

- record ID
- kind
- title
- source URI
- evidence
- content hash
- project/language/framework scope

Embeddings remain in Bookkeeper; Atheneum stores references, not vectors, in the initial migration.

## Native Pi router architecture

Create `/home/feanor/Projects/pi-memory-router` as a native Pi package.

Runtime rules:

- No MCP.
- No raw SQLite access.
- Call `atheneum` CLI for Atheneum operations.
- Call `memory-bookkeeper` CLI only for Bookkeeper-specific expansion until records are imported.
- Read Hermes Markdown only through the migration adapter, not as the normal runtime path.

Pi tools:

- `memory_route(query, project?, kinds?, budget_tokens?, depth?, evidence_required?)`
- `memory_expand(ids, budget_tokens?)`
- `project_state(project?, budget_tokens?)`
- `memory_record(record)`
- `receipt_record(receipt)`

Pi commands:

- `/memory-route <query>`
- `/project-state [project]`
- `/memory-expand <id...>`

Router response shape:

```json
{
  "query": "harness report",
  "project": "Projects",
  "budget_tokens": 800,
  "budget_used": 512,
  "depth": "summary",
  "items": [
    {
      "id": "memory:123",
      "kind": "decision",
      "title": "Harness owns gates",
      "summary": "SkillOpt learns; harness evaluates and issues verdicts.",
      "score": 0.91,
      "evidence_state": "present",
      "token_cost": 48,
      "expandable": true
    }
  ],
  "omitted": 9,
  "warnings": []
}
```

Depth behavior:

- `summary`: metadata and bounded summaries only.
- `bundle`: task-scoped collection of summaries and short excerpts; no full bodies.
- `expand`: full records for explicit IDs, bounded by budget.

## Token budgeting

Default budgets:

- session bootstrap: 500 tokens
- project state head: 800 tokens
- subagent task bundle: 1200 tokens
- explicit expansion: 2000 tokens max unless user overrides

Ranking order:

1. current project state head
2. active blockers/tasks
3. verified decisions/receipts
4. query-relevant memories
5. older low-confidence memories

The router must report `omitted` count and must not silently drop required constraints.

## Security and safety

- Secret scan and classification happen at ingest time.
- `sensitivity=secret` records are rejected or stored only as redacted metadata with a content hash.
- `sensitivity=secret` is never prompt-injected and never exported to plain NDJSON.
- `sensitivity=sensitive` requires explicit operator approval before prompt injection or export.
- Commands, source URIs, artifacts, and receipts also carry sensitivity labels because they can contain credentials.
- Memory never overrides fresh tests or repository state.
- No full-history injection.

## Verification plan

For every implementation phase:

- Add fail-first tests before migration/import/router code.
- Test migration on temporary DB copies.
- Test idempotency by running migration/import twice.
- Verify deterministic import maps.
- Verify no duplicate external refs after replay.
- Verify memory hash collisions produce conflict/revision outcomes, not overwrites.
- Verify router budget behavior with oversized candidate sets.
- Verify sensitive records are excluded from routing and export.
- Verify new record types appear as graph entities and remain searchable/navigable.
- Verify native Pi extension load with a real `pi` process.
- Verify no MCP tools are required for router operation.

## Rollout rule

Do not mutate the selected canonical DB until:

1. every host-specific source/binary/DB identity is reconciled,
2. migration passes on disposable copies,
3. dry-run import maps are reviewed,
4. backups and checksums are stored,
5. the user explicitly approves production migration.
