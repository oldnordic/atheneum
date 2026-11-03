# TECH: Atheneum Memory Store and Native Router

## Scope

This spec covers:

1. Additive Atheneum schema updates for canonical memory/state records.
2. Safe merge/import design for existing Atheneum, Hermes, and Bookkeeper memory.
3. Native Pi router architecture that uses local CLIs and avoids MCP.
4. Phased implementation suitable for small Orca/agy dispatches.

This spec does not authorize production DB mutation. All migration and merge work begins on disposable copies.

## Canonical repositories and stores

- Atheneum source: `/home/feanor/Projects/atheneum`
- Installed Atheneum CLI: `/home/feanor/.local/bin/atheneum`
- Canonical Atheneum DB: `/home/feanor/.atheneum/atheneum.db`
- Pi Atheneum native extension source: `/home/feanor/Projects/pi-atheneum`
- Pi Hermes Memory package: `/home/feanor/.pi/agent/git/github.com/chandra447/pi-hermes-memory`
- Hermes storage: `/home/feanor/.pi/agent/pi-hermes-memory/`, `/home/feanor/.pi/agent/projects-memory/`
- Memory Bookkeeper source: `/home/feanor/Projects/memory-bookkeeper`
- Memory Bookkeeper DB: `/home/feanor/.local/share/memory-bookkeeper/memory.sqlite`

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

Important caveat:

- Installed `/home/feanor/.local/bin/atheneum` reports `v0.14.0`, but its SHA-256 differs from `/home/feanor/Projects/atheneum/target/release/atheneum`. Before any schema work, build and compare the current source, then install only after tests pass.

## Identified gaps

The current store lacks first-class fields/tables for:

- canonical memory kind (`decision`, `article`, `receipt`, etc.)
- short title and bounded summary separate from full content
- lifecycle status and validity windows
- source URI and evidence lists
- supersedes/superseded-by relations
- token estimate
- sensitivity/redaction labels
- project state head
- agent state
- arbitrary artifacts and verification receipts
- external-store identity mapping for Hermes/Bookkeeper imports
- router query audit records

## Proposed additive migration: v15 `memory-router-domain`

Migration v15 must be additive, transactional, and idempotent.

### Extend `memory_entries`

Add nullable/defaulted columns:

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
supersedes_memory_id INTEGER REFERENCES memory_entries(id);
```

Indexes:

```sql
CREATE INDEX IF NOT EXISTS memory_kind_idx ON memory_entries(kind);
CREATE INDEX IF NOT EXISTS memory_status_idx ON memory_entries(status);
CREATE INDEX IF NOT EXISTS memory_project_kind_idx ON memory_entries(project_id, kind);
CREATE INDEX IF NOT EXISTS memory_content_hash_idx ON memory_entries(content_hash);
CREATE INDEX IF NOT EXISTS memory_supersedes_idx ON memory_entries(supersedes_memory_id);
```

Backfill rules:

- Existing rows keep `kind = 'memory'` unless an adapter can classify them with evidence.
- Existing `content` remains authoritative.
- `summary` is initially generated only by an explicit migration utility, never by silent lossy truncation in the open path.
- Existing graph `data.content_hash` is copied into SQL when present.

### New table: `project_states`

```sql
CREATE TABLE IF NOT EXISTS project_states (
  id INTEGER PRIMARY KEY,
  project_id TEXT NOT NULL UNIQUE,
  objective TEXT,
  state_json TEXT NOT NULL,
  branch TEXT,
  head_sha TEXT,
  constraints_json TEXT NOT NULL DEFAULT '[]',
  active_task_ids_json TEXT NOT NULL DEFAULT '[]',
  blocker_ids_json TEXT NOT NULL DEFAULT '[]',
  evidence_json TEXT NOT NULL DEFAULT '[]',
  token_estimate INTEGER,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
```

### New table: `agent_states`

```sql
CREATE TABLE IF NOT EXISTS agent_states (
  id INTEGER PRIMARY KEY,
  agent_name TEXT NOT NULL,
  project_id TEXT,
  dispatch_id TEXT,
  task_id INTEGER,
  status TEXT NOT NULL,
  current_objective TEXT,
  state_json TEXT NOT NULL DEFAULT '{}',
  budget_json TEXT NOT NULL DEFAULT '{}',
  last_receipt_id INTEGER,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS agent_states_project_idx ON agent_states(project_id);
CREATE INDEX IF NOT EXISTS agent_states_agent_idx ON agent_states(agent_name);
CREATE INDEX IF NOT EXISTS agent_states_dispatch_idx ON agent_states(dispatch_id);
```

### New table: `artifacts`

```sql
CREATE TABLE IF NOT EXISTS artifacts (
  id INTEGER PRIMARY KEY,
  project_id TEXT,
  task_id INTEGER,
  artifact_kind TEXT NOT NULL,
  path TEXT NOT NULL,
  sha256 TEXT NOT NULL,
  bytes INTEGER,
  commit_sha TEXT,
  created_by TEXT,
  metadata_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL,
  UNIQUE(project_id, path, sha256)
);
```

### New table: `receipts`

```sql
CREATE TABLE IF NOT EXISTS receipts (
  id INTEGER PRIMARY KEY,
  project_id TEXT,
  task_id INTEGER,
  artifact_id INTEGER REFERENCES artifacts(id),
  verdict TEXT NOT NULL,
  verifier TEXT NOT NULL,
  command TEXT,
  output_hash TEXT,
  evidence_json TEXT NOT NULL DEFAULT '[]',
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS receipts_project_idx ON receipts(project_id);
CREATE INDEX IF NOT EXISTS receipts_task_idx ON receipts(task_id);
CREATE INDEX IF NOT EXISTS receipts_artifact_idx ON receipts(artifact_id);
```

### New table: `sources`

```sql
CREATE TABLE IF NOT EXISTS sources (
  id INTEGER PRIMARY KEY,
  project_id TEXT,
  source_type TEXT NOT NULL,
  uri TEXT NOT NULL UNIQUE,
  title TEXT,
  author TEXT,
  published_at TEXT,
  retrieved_at TEXT,
  credibility REAL,
  metadata_json TEXT NOT NULL DEFAULT '{}'
);
```

### New table: `external_memory_refs`

```sql
CREATE TABLE IF NOT EXISTS external_memory_refs (
  id INTEGER PRIMARY KEY,
  external_store TEXT NOT NULL,
  external_id TEXT NOT NULL,
  atheneum_kind TEXT NOT NULL,
  atheneum_id INTEGER NOT NULL,
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

Add public APIs, then CLI commands:

- `upsert_memory_item(params)`
- `get_memory_item(id)`
- `route_memory(query, filters, budget, depth)`
- `expand_memory(ids, budget)`
- `upsert_project_state(project, state)`
- `get_project_state(project, budget)`
- `record_artifact(params)`
- `record_receipt(params)`
- `link_external_memory(ref)`

CLI names should be explicit and scriptable:

- `memory-upsert`
- `memory-get`
- `memory-route`
- `memory-expand`
- `project-state-get`
- `project-state-upsert`
- `artifact-record`
- `receipt-record`
- `external-ref-link`

All commands emit JSON.

## Merge architecture

Atheneum remains canonical. Existing stores are imported through adapters.

### Atheneum-to-Atheneum merge

Use existing:

```bash
atheneum export-ledger source.db --kinds discoveries,memories,tasks > source.ndjson
atheneum import-ledger target.db source.ndjson --dry-run --map source.ndjson.import-map.ndjson
```

Only after dry-run review:

```bash
atheneum import-ledger target.db source.ndjson --map source.ndjson.import-map.ndjson
```

### Hermes adapter

Phase 1 imports Markdown memory files, not `sessions.db` internals.

Input roots:

- `/home/feanor/.pi/agent/pi-hermes-memory/MEMORY.md`
- `/home/feanor/.pi/agent/pi-hermes-memory/USER.md`
- `/home/feanor/.pi/agent/pi-hermes-memory/failures.md`
- `/home/feanor/.pi/agent/projects-memory/*/MEMORY.md`

Output: canonical NDJSON with source URI, content hash, kind, scope, and project.

Session DB import is a later phase and must use a reviewed read-only exporter.

### Bookkeeper adapter

Use the Bookkeeper CLI export path where possible:

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

Embeddings remain in Bookkeeper; Atheneum stores references, not vectors, in phase 1.

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

## Token budgeting

Default budgets:

- session bootstrap: 500 tokens
- project state head: 800 tokens
- subagent task bundle: 1200 tokens
- explicit expansion: 2000 tokens max unless user overrides

Ranking order:

1. current project state
2. active blockers/tasks
3. verified decisions/receipts
4. query-relevant memories
5. older low-confidence memories

The router must report `omitted` count and must not silently drop required constraints.

## Security and safety

- Secret scan records before prompt injection.
- `sensitivity=secret` records are never returned to the model.
- `sensitivity=sensitive` records require explicit user approval.
- Imported memory must retain source URI and content hash.
- Memory never overrides fresh tests or repository state.
- No full-history injection.

## Verification plan

For every implementation phase:

- Add fail-first tests before migration/import/router code.
- Test migration on temporary DB copies.
- Test idempotency by running migration/import twice.
- Verify deterministic import maps.
- Verify no duplicate external refs after replay.
- Verify router budget behavior with oversized candidate sets.
- Verify sensitive records are excluded.
- Verify native Pi extension load with a real `pi` process.
- Verify no MCP tools are required for router operation.

## Rollout rule

Do not mutate `/home/feanor/.atheneum/atheneum.db` until:

1. source and installed CLI identity are reconciled,
2. migration passes on disposable copies,
3. dry-run import maps are reviewed,
4. backups and checksums are stored,
5. the user explicitly approves production migration.
