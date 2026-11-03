# Atheneum Documentation Sync Report — v0.15.0 (`memory-router`)

- **Date**: 2026-11-03
- **Base Commit**: `a2d044288192893f90950f5f9bf407e8d72502f7`
- **Branch**: `atheneum/docs-sync-0.15.0`
- **Worktree**: `/home/feanor/orca/workspaces/atheneum/Atheneum-Docs-Sync`
- **Task ID**: `task_a26916e7320e`
- **Dispatch ID**: `ctx_8f44ec7d42f0`

---

## 1. Summary of Updates

The five feature commits introducing the memory-router domain landed with zero documentation updates:
- `8d0831d` feat: additive migration v15 memory-router-domain
- `9e826d2` feat: memory item API, routing read path, conflict-safe upsert
- `66f5ac3` feat: append-only state, artifact, receipt, source, external-ref APIs
- `caded9b` feat: ledger v2 NDJSON, conflict-safe import, hermes/bookkeeper adapters
- `a2d0442` chore: bump atheneum to v0.15.0

This synchronization comprehensively documents all new data models, SQL tables, graph entities, public APIs, CLI subcommands, security models, and migration adapters across the repository.

### Modified Files

1. `CHANGELOG.md` (root): Added `## [0.15.0] - 2026-11-03` covering migration v15 (14 new columns, 6 indexes, 7 new tables, graph backfill), new `EntityType` variants, memory item APIs/CLI with conflict-safe semantics, state/artifact/receipt/source/external-ref APIs and 8 CLIs, ledger v2 NDJSON + conflict-safe import + adapters, security behavior (secret rejection, sensitive filtering), and version bump. Uses repo-relative paths (`crates/atheneum/src/...`).
2. `crates/atheneum/CHANGELOG.md`: Identical `## [0.15.0] - 2026-11-03` changelog entry formatted with crate-relative paths (`src/...`).
3. `crates/atheneum/ARCHITECTURE.md`: Added schema documentation for `memory_entries` (extended), `project_states`, `agent_states`, `artifacts`, `receipts`, `sources`, `external_memory_refs`, and `router_queries`. Added new `EntityType` variants. Added `## Memory Router Domain` section covering dual-layer graph invariants, public ID invariants, conflict-safe upsert semantics, budgeted routing depth models (`summary`, `bundle`, `expand`), sensitivity models, and updated the `MIGRATIONS` registry snippet to include migrations 14 and 15.
4. `crates/atheneum/API.md`: Documented all new public data structures (`ConflictStrategy`, `MemoryRouteDepth`, `UpsertMemoryItemParams`, `UpsertMemoryOutcome`, `MemoryItemEnvelope`, `MemoryRouteFilters`, `MemoryRouteItem`, `MemoryRouteResponse`, `MemoryExpandResponse`, `ProjectStateParams`, `ProjectStateOutcome`, `ProjectStateEnvelope`, `AgentStateParams`, `AgentStateOutcome`, `AgentStateEnvelope`, `RecordArtifactParams`, `RecordArtifactOutcome`, `RecordReceiptParams`, `RecordReceiptOutcome`, `RecordSourceParams`, `RecordSourceOutcome`, `LinkExternalMemoryParams`, `LinkExternalMemoryOutcome`, `LedgerV2Counts`, `HermesAdoptSummary`, `BookkeeperAdoptSummary`), public `AtheneumGraph` methods, standalone ledger/adapter functions, and exact CLI subcommand syntax/flags.
5. `crates/atheneum/MANUAL.md`: Added `## Memory Router & Coordination Domain (v0.15.0)` detailing the usage of all 16 new CLI commands with real examples, argument descriptions, and real JSON output examples.
6. `README.md` (root): Bumped core crate release version reference from `0.14.0` to `0.15.0`.
7. `crates/atheneum/README.md`: Added `## What's New In 0.15.0` section detailing the memory router domain, state tracking, verification receipts, adapters, and the 16 new CLI subcommands.

---

## 2. Verification Evidence

### 2.1. CLI Commands Verification Against Real Binary (`target/debug/atheneum`)

Every documented command was executed against `target/debug/atheneum` without arguments to verify usage and error messages:

```
$ target/debug/atheneum ledger-export-v2
Usage: atheneum ledger-export-v2 <db-path> [--kinds <kinds>] [--until <rfc3339>] [--out <file>]

$ target/debug/atheneum ledger-import
Usage: atheneum ledger-import <db-path> <file.ndjson> [--apply] [--map <path>] [--revision-on-conflict]

$ target/debug/atheneum adopt-hermes
Usage: atheneum adopt-hermes <input-dir> --out <ndjson>

$ target/debug/atheneum adopt-bookkeeper
Usage: atheneum adopt-bookkeeper <input-md> --out <ndjson>

$ target/debug/atheneum memory-item-upsert
Usage: atheneum memory-item-upsert <db-path> <key> <content> [flags] OR \atheneum memory-item-upsert <db-path> --json '<json>'

$ target/debug/atheneum memory-item-get
Usage: atheneum memory-item-get <db-path> <graph-entity-id>

$ target/debug/atheneum memory-route
Usage: atheneum memory-route <db-path> [query] [--project P] [--kind K] \[--kinds K1,K2] [--status S] [--confidence-floor N] [--evidence-required] \[--budget B] [--depth summary|bundle|expand]

$ target/debug/atheneum memory-expand
Usage: atheneum memory-expand <db-path> <id1,id2,...> [--budget B]

$ target/debug/atheneum project-state-append
Usage: atheneum project-state-append <db-path> [project] [flags] OR \atheneum project-state-append <db-path> --json '<json>'

$ target/debug/atheneum project-state-head
Usage: atheneum project-state-head <db-path> <project> [--budget B] [--include-sensitive]

$ target/debug/atheneum agent-state-append
Usage: atheneum agent-state-append <db-path> [agent] [flags] OR \atheneum agent-state-append <db-path> --json '<json>'

$ target/debug/atheneum agent-state-head
Usage: atheneum agent-state-head <db-path> <agent> [--project P] [--include-sensitive]

$ target/debug/atheneum artifact-record
Usage: atheneum artifact-record <db-path> [path] [sha256] [kind] [flags] OR \atheneum artifact-record <db-path> --json '<json>'

$ target/debug/atheneum receipt-record
Usage: atheneum receipt-record <db-path> [verdict] [verifier] [flags] OR \atheneum receipt-record <db-path> --json '<json>'

$ target/debug/atheneum source-record
Usage: atheneum source-record <db-path> [uri] [source-type] [flags] OR \atheneum source-record <db-path> --json '<json>'

$ target/debug/atheneum external-ref-link
Usage: atheneum external-ref-link <db-path> [external-store] [external-id] [atheneum-id] [flags] OR \atheneum external-ref-link <db-path> --json '<json>'
```

### 2.2. Public API Methods Verification in Source

Every documented public API function was checked against `crates/atheneum/src/graph/`:

| Method | Source File | Line | Verification Output |
|---|---|---|---|
| `upsert_memory_item` | `crates/atheneum/src/graph/memory_router.rs` | 592 | `pub fn upsert_memory_item(&self, params: &UpsertMemoryItemParams) -> Result<UpsertMemoryOutcome>` |
| `get_memory_item_by_graph_id` | `crates/atheneum/src/graph/memory_router.rs` | 959 | `pub fn get_memory_item_by_graph_id(&self, id: i64) -> Result<Option<MemoryItemEnvelope>>` |
| `route_memory` | `crates/atheneum/src/graph/memory_router.rs` | 1163 | `pub fn route_memory(&self, query: &str, filters: &MemoryRouteFilters, budget: usize, depth: MemoryRouteDepth) -> Result<MemoryRouteResponse>` |
| `expand_memory` | `crates/atheneum/src/graph/memory_router.rs` | 1408 | `pub fn expand_memory(&self, ids: &[String], budget: usize) -> Result<MemoryExpandResponse>` |
| `append_project_state` | `crates/atheneum/src/graph/memory_router.rs` | 1459 | `pub fn append_project_state(&self, project: &str, params: &ProjectStateParams) -> Result<ProjectStateOutcome>` |
| `get_project_state_head` | `crates/atheneum/src/graph/memory_router.rs` | 1603 | `pub fn get_project_state_head(&self, project: &str, budget: usize) -> Result<Option<ProjectStateEnvelope>>` |
| `get_project_state_head_with_options` | `crates/atheneum/src/graph/memory_router.rs` | 1612 | `pub fn get_project_state_head_with_options(&self, project: &str, budget: usize, include_sensitive: bool) -> Result<Option<ProjectStateEnvelope>>` |
| `append_agent_state` | `crates/atheneum/src/graph/memory_router.rs` | 1889 | `pub fn append_agent_state(&self, agent_name: &str, params: &AgentStateParams) -> Result<AgentStateOutcome>` |
| `get_agent_state_head` | `crates/atheneum/src/graph/memory_router.rs` | 2015 | `pub fn get_agent_state_head(&self, agent_name: &str, project_id: Option<&str>) -> Result<Option<AgentStateEnvelope>>` |
| `get_agent_state_head_with_options` | `crates/atheneum/src/graph/memory_router.rs` | 2024 | `pub fn get_agent_state_head_with_options(&self, agent_name: &str, project_id: Option<&str>, include_sensitive: bool) -> Result<Option<AgentStateEnvelope>>` |
| `record_artifact` | `crates/atheneum/src/graph/memory_router.rs` | 2114 | `pub fn record_artifact(&self, params: &RecordArtifactParams) -> Result<RecordArtifactOutcome>` |
| `record_receipt` | `crates/atheneum/src/graph/memory_router.rs` | 2231 | `pub fn record_receipt(&self, params: &RecordReceiptParams) -> Result<RecordReceiptOutcome>` |
| `record_source` | `crates/atheneum/src/graph/memory_router.rs` | 2339 | `pub fn record_source(&self, params: &RecordSourceParams) -> Result<RecordSourceOutcome>` |
| `link_external_memory` | `crates/atheneum/src/graph/memory_router.rs` | 2456 | `pub fn link_external_memory(&self, params: &LinkExternalMemoryParams) -> Result<LinkExternalMemoryOutcome>` |
| `export_ledger_v2` | `crates/atheneum/src/graph/ledger_v2.rs` | 316 | `pub fn export_ledger_v2(graph: &AtheneumGraph, kinds: Option<&[&str]>, until: Option<&str>, mut out: impl Write) -> Result<BTreeMap<String, u64>>` |
| `import_ledger_v2` | `crates/atheneum/src/graph/ledger_v2.rs` | 1042 | `pub fn import_ledger_v2(graph: &AtheneumGraph, path: &Path, dry_run: bool, conflict_strategy: ConflictStrategy, mut map_out: impl Write) -> Result<LedgerV2Counts>` |
| `adopt_hermes_markdown` | `crates/atheneum/src/graph/ledger_v2.rs` | 1708 | `pub fn adopt_hermes_markdown(input_dir: &Path, mut out: impl Write) -> Result<HermesAdoptSummary>` |
| `adopt_bookkeeper_markdown` | `crates/atheneum/src/graph/ledger_v2.rs` | 1891 | `pub fn adopt_bookkeeper_markdown(input_file: &Path, mut out: impl Write) -> Result<BookkeeperAdoptSummary>` |

### 2.3. SQL Tables & Columns Verification Against `crates/atheneum/src/db/memory_router.rs`

All 7 newly created tables and all 14 added columns to `memory_entries` were verified against `crates/atheneum/src/db/memory_router.rs`:

- Tables created:
  - `project_states` (line 104)
  - `agent_states` (line 129)
  - `artifacts` (line 151)
  - `receipts` (line 168)
  - `sources` (line 190)
  - `external_memory_refs` (line 208)
  - `router_queries` (line 222)
- Added columns to `memory_entries`:
  - `kind` (line 55)
  - `title` (line 58)
  - `summary` (line 59)
  - `status` (line 61)
  - `source_uri` (line 66)
  - `evidence_json` (line 69)
  - `content_hash` (line 73)
  - `token_estimate` (line 74)
  - `sensitivity` (line 77)
  - `valid_from` (line 81)
  - `valid_to` (line 82)
  - `ttl_seconds` (line 83)
  - `graph_entity_id` (line 84)
  - `supersedes_graph_entity_id` (line 86)
- New indexes on `memory_entries`:
  - `memory_kind_idx`
  - `memory_status_idx`
  - `memory_project_kind_idx`
  - `memory_content_hash_idx`
  - `memory_graph_entity_idx`
  - `memory_supersedes_graph_idx`

### 2.4. Git Diff Hygiene Check

Command:
```bash
git diff --check
```
Output:
Exit code 0 (clean, no trailing whitespace or unresolved markers).

---

## 3. Worktree & Code Integrity Affirmation

- Zero code files were modified (`git status` confirms only markdown documentation files and this report are modified/untracked).
- No commits were created; all changes remain in the working tree for orchestrator review.
- No live or canonical databases (`/home/feanor/.atheneum/atheneum.db`) were accessed or mutated.
