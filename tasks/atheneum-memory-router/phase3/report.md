# Phase 3 Verification Report — Atheneum Memory Router

- **Date**: 2026-11-03
- **Worker**: Orca dispatched worker (`task_6568e4ad52ae` / `ctx_dc81eccb743f` / `term_87560e50-1e18-4527-aa6a-9062395c6bf6`)
- **Coordinator**: `orca_session_id:pi_739e1bdc_c74e_43a3_abe6_5a238a799775`
- **Base Commit**: `66f5ac32429d3188a5d7a2c565d27127fbacae6b`
- **Branch**: `atheneum/memory-router-phase3`
- **Worktree**: `/home/feanor/orca/workspaces/atheneum/Atheneum-Memory-Phase-3`

---

## 1. Scope & Invariants

Phase 3 delivers the ledger extension, canonical NDJSON memory-router envelope, and external store import adapters:

1. **Canonical NDJSON Schema (`crates/atheneum/src/graph/ledger_v2.rs`)**:
   - Machine-readable NDJSON envelope with `record_type` discriminator:
     - `memory`: full canonical envelope (key, content, scope, kind, title, summary, confidence, status, project_id, source_uri, evidence, content_hash, token_estimate, sensitivity, valid_from, valid_to, ttl_seconds, tags, graph_entity_id).
     - `source`: project_id, source_type, uri, content_hash, title, summary, author, published_at, retrieved_at, credibility, sensitivity, metadata.
     - `artifact`: project_id, task_id, artifact_kind, path, sha256, bytes, commit_sha, created_by, sensitivity, metadata.
     - `receipt`: project_id, task_id, artifact_graph_entity_id, session_id, verdict, verifier, command, exit_code, duration_ms, output_hash, output_sample_redacted, evidence, sensitivity.
     - `external_ref`: external_store, external_id, atheneum_graph_entity_id, atheneum_kind, content_hash, sync_state, last_synced_at, metadata.
   - Rejects any record with `sensitivity = "secret"` at ingest; excludes secret records during export.

2. **Conflict-Safe Ledger Import (`import_ledger_v2` & `ledger.rs`)**:
   - All memory imports route through `AtheneumGraph::upsert_memory_item`:
     - Same `content_hash`: idempotent skip (`status: "skipped"` in import map).
     - Different `content_hash` on same logical key `(key, scope, project_id)`:
       - Default: records conflict (`status: "conflict"` in import map), leaves existing active record untouched.
       - `--revision-on-conflict`: marks old record `superseded` and creates new revision linked via `supersedes_graph_entity_id`.
     - Completely eliminates the legacy overwriting `store_memory` path.
   - Dry-run by default (`dry_run = true` unless `--apply` passed):
     - Verified zero SQLite mutations (SHA-256 of target DB identical before and after dry run).
   - Generates line-by-line audit import map NDJSON (`status: merged | skipped | conflict | would_merge | would_revision | failed`).

3. **Hermes Markdown Adapter (`adopt_hermes_markdown`)**:
   - Parses Markdown files separated by section delimiter `§`.
   - Extracts metadata comments `<!-- created=..., last=..., project64=..., sensitivity=... -->`.
   - Infers kind (`failure`, `correction`, `tool_quirk`, `insight`, `convention`, `decision`, `gate`, `preference`, `memory`).
   - Infers scope (`USER.md` → `"user"`, `MEMORY.md`/`failures.md` → `"agent"`).
   - Decodes base64 project IDs (`project64=...`) without external dependencies.
   - Preserves `source_uri` (file path), `content_hash` (SHA-256), and references in `evidence`.

4. **Bookkeeper Markdown Adapter (`adopt_bookkeeper_markdown`)**:
   - Parses Bookkeeper markdown export format (`[<kind>] <title>\n<text> <!-- created=..., project64=... -->`).
   - Preserves record ID, kind, title, source URI, evidence, and content hash.
   - References only: does not copy vector embeddings or raw transcript bodies.

5. **CLI Commands (`crates/atheneum/src/cli/dispatch.rs`, `crates/atheneum/src/cli/util.rs`)**:
   - `atheneum ledger-export-v2 <db-path> [--kinds K] [--until RFC3339] [--out FILE]`
   - `atheneum ledger-import <db-path> <file.ndjson> [--apply] [--map PATH] [--revision-on-conflict]`
   - `atheneum adopt-hermes <input-dir> --out <ndjson>`
   - `atheneum adopt-bookkeeper <input-md> --out <ndjson>`

---

## 2. Canonical NDJSON Schema Specification

Every line is a valid JSON object with `record_type` discriminator:

### 2.1. Memory Record (`record_type: "memory"`)
```json
{
  "record_type": "memory",
  "key": "cache-config",
  "content": "Use in-memory LRU cache with 1024 capacity.",
  "scope": "agent",
  "kind": "decision",
  "title": "Cache Configuration",
  "summary": "LRU cache capacity set to 1024.",
  "confidence": 1.0,
  "status": "active",
  "project_id": "atheneum",
  "source_uri": "file:///path/to/spec.md",
  "evidence": [{"source": "spec", "ref": "TECH.md#L45"}],
  "content_hash": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
  "token_estimate": 24,
  "sensitivity": "normal",
  "valid_from": "2026-11-01T00:00:00Z",
  "valid_to": null,
  "ttl_seconds": null,
  "graph_entity_id": 142
}
```

### 2.2. Source Record (`record_type: "source"`)
```json
{
  "record_type": "source",
  "uri": "https://docs.rs/sqlitegraph",
  "source_type": "documentation",
  "project_id": "atheneum",
  "content_hash": "a1b2c3d4e5f6",
  "title": "SqliteGraph Documentation",
  "summary": "Graph database layer over SQLite.",
  "author": "Luiz Spies",
  "published_at": "2026-10-01",
  "retrieved_at": "2026-11-01",
  "credibility": 1.0,
  "sensitivity": "normal",
  "metadata": {"tags": ["graph", "sqlite"]}
}
```

### 2.3. Artifact Record (`record_type: "artifact"`)
```json
{
  "record_type": "artifact",
  "path": "crates/atheneum/src/graph/ledger_v2.rs",
  "sha256": "4a7d3b...",
  "artifact_kind": "source_code",
  "project_id": "atheneum",
  "task_id": 42,
  "bytes": 28412,
  "commit_sha": "66f5ac3...",
  "created_by": "worker_task_6568e4ad52ae",
  "sensitivity": "normal",
  "metadata": {}
}
```

### 2.4. Receipt Record (`record_type: "receipt"`)
```json
{
  "record_type": "receipt",
  "verdict": "passed",
  "verifier": "cargo-test",
  "project_id": "atheneum",
  "task_id": 42,
  "artifact_graph_entity_id": 105,
  "session_id": "sess_123",
  "command": "cargo test -p atheneum --lib",
  "exit_code": 0,
  "duration_ms": 1960,
  "output_hash": "f6e5d4...",
  "output_sample_redacted": "test result: ok. 143 passed",
  "evidence": [{"verifier": "cargo-test"}],
  "sensitivity": "normal"
}
```

### 2.5. External Reference Record (`record_type: "external_ref"`)
```json
{
  "record_type": "external_ref",
  "external_store": "hermes",
  "external_id": "failures.md#id=tool_quirk:d7af0c005c98f184",
  "atheneum_graph_entity_id": 204,
  "atheneum_kind": "Memory",
  "content_hash": "d7af0c005c98f184...",
  "sync_state": "synced",
  "last_synced_at": "2026-11-03T20:25:00Z",
  "metadata": {}
}
```

---

## 3. TDD Sequence & Verification Evidence

### Step 1 & 2: Fail-First Proof
Seven comprehensive unit & integration tests covering all required behaviors were defined in `crates/atheneum/src/graph/ledger_v2.rs`:
1. `test_phase3_ndjson_roundtrip`
2. `test_phase3_conflict_safe_import_both_polarities`
3. `test_phase3_dry_run_no_mutation`
4. `test_phase3_idempotent_replay`
5. `test_phase3_secret_exclusion_and_rejection`
6. `test_phase3_hermes_fixture_parse`
7. `test_phase3_bookkeeper_fixture_parse`

Executing `TMPDIR="$PWD/target/tmp" cargo test -p atheneum --lib graph::ledger_v2::tests` before implementation failed with explicit panics:
- Log saved to: `tasks/atheneum-memory-router/phase3/fail-first.log`
- Scratch copy: `/home/feanor/scratch/dispatch/atheneum-memory-router/phase3/fail-first.log`

Excerpt:
```
running 7 tests
test graph::ledger_v2::tests::test_phase3_bookkeeper_fixture_parse ... FAILED
test graph::ledger_v2::tests::test_phase3_hermes_fixture_parse ... FAILED
test graph::ledger_v2::tests::test_phase3_ndjson_roundtrip ... FAILED
test graph::ledger_v2::tests::test_phase3_secret_exclusion_and_rejection ... FAILED
test graph::ledger_v2::tests::test_phase3_conflict_safe_import_both_polarities ... FAILED
test graph::ledger_v2::tests::test_phase3_idempotent_replay ...浦 FAILED
test graph::ledger_v2::tests::test_phase3_dry_run_no_mutation ... FAILED

test result: FAILED. 0 passed; 7 failed; 0 ignored; 0 measured; 136 filtered out; finished in 0.18s
```

### Step 3 & 4: Implementation & Passing Test Suite
After implementing the schema, adapters, and conflict-safe import engine:
- `TMPDIR="$PWD/target/tmp" cargo test -p atheneum --lib`
- **Result**: `test result: ok. 143 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.96s`
- Log saved to: `tasks/atheneum-memory-router/phase3/tests-pass.log`
- Scratch copy: `/home/feanor/scratch/dispatch/atheneum-memory-router/phase3/tests-pass.log`

---

## 4. Standard Quality Gates

1. **Rustfmt Formatting Gate**:
   ```bash
   cargo fmt --all -- --check
   ```
   *Exit code 0. Clean formatting across all crates.*

2. **Clippy Static Analysis Gate**:
   ```bash
   TMPDIR="$PWD/target/tmp" cargo clippy --all-targets -- -D warnings
   ```
   *Exit code 0. Zero warnings across library, tests, binaries, and examples.*

3. **Banned Markers & Anti-Cheat Scan**:
   ```bash
   git diff --name-only | xargs rg -e 'todo!' -e 'unimplemented!' -e 'FIXME' -e 'TODO' -e 'HACK' -e 'for now' -e 'placeholder'
   rg '#\[allow\(' crates/atheneum/src/graph/ledger_v2.rs
   git diff --check
   ```
   *Zero banned markers in new code. Zero `#[allow(...)]` without reason. Clean diff check.*

4. **Immutable Test Surfaces Gate**:
   ```bash
   git diff --exit-code crates/atheneum/tests/
   ```
   *Exit code 0. Integration test surface was completely untouched.*

---

## 5. Live CLI Verification & Artifacts

All live CLI verification exercises were executed using `target/debug/atheneum` under `/home/feanor/scratch/dispatch/atheneum-memory-router/phase3/`. Zero production database paths were touched.

### 5.1. `ledger-export-v2` on Strix Snapshot
Exported memories from disposable Strix DB copy (`/home/feanor/scratch/dispatch/atheneum-memory-router/phase3/strix_copy.db`):
```bash
target/debug/atheneum ledger-export-v2 \
  /home/feanor/scratch/dispatch/atheneum-memory-router/phase3/strix_copy.db \
  --out /home/feanor/scratch/dispatch/atheneum-memory-router/phase3/strix_exported.ndjson
```
Output:
```json
{
  "exported": {
    "artifact": 0,
    "external_ref": 0,
    "memory": 54,
    "receipt": 0,
    "source": 0
  },
  "out": "/home/feanor/scratch/dispatch/atheneum-memory-router/phase3/strix_exported.ndjson",
  "status": "ok"
}
```

### 5.2. Dry-Run Import (Zero Mutation)
```bash
target/debug/atheneum ledger-import \
  /home/feanor/scratch/dispatch/atheneum-memory-router/phase3/fresh_target.db \
  /home/feanor/scratch/dispatch/atheneum-memory-router/phase3/strix_exported.ndjson \
  --map /home/feanor/scratch/dispatch/atheneum-memory-router/phase3/dry_run.import-map.ndjson
```
Output:
```json
{
  "conflicts": 0,
  "dry_run": true,
  "failed": 0,
  "map_file": "/home/feanor/scratch/dispatch/atheneum-memory-router/phase3/dry_run.import-map.ndjson",
  "merged": 54,
  "skipped": 0
}
```
Import map sample (`dry_run.import-map.ndjson`):
```json
{"content_hash":"f102339c4b7b4291450cec1c9c7a768ef15300ea51d6f749e67b45210006e784","line":1,"new_id":null,"record_type":"memory","status":"would_merge","target":"pi-dispatch-pattern-strix"}
{"content_hash":"9db96a95f0efe5abe3e0957098678b2cdf2b01a5dd321b392057f7f17c582a52","line":2,"new_id":null,"record_type":"memory","status":"would_merge","target":"skill_solve_task_llm_function"}
```

### 5.3. Applied Import
```bash
target/debug/atheneum ledger-import \
  /home/feanor/scratch/dispatch/atheneum-memory-router/phase3/fresh_target.db \
  /home/feanor/scratch/dispatch/atheneum-memory-router/phase3/strix_exported.ndjson \
  --apply \
  --map /home/feanor/scratch/dispatch/atheneum-memory-router/phase3/applied.import-map.ndjson
```
Output:
```json
{
  "conflicts": 0,
  "dry_run": false,
  "failed": 0,
  "map_file": "/home/feanor/scratch/dispatch/atheneum-memory-router/phase3/applied.import-map.ndjson",
  "merged": 54,
  "skipped": 0
}
```

### 5.4. Replay Idempotency Verification (Zero Duplicates)
Re-running the exact same import against `fresh_target.db`:
```bash
target/debug/atheneum ledger-import \
  /home/feanor/scratch/dispatch/atheneum-memory-router/phase3/fresh_target.db \
  /home/feanor/scratch/dispatch/atheneum-memory-router/phase3/strix_exported.ndjson \
  --apply \
  --map /home/feanor/scratch/dispatch/atheneum-memory-router/phase3/replayed.import-map.ndjson
```
Output:
```json
{
  "conflicts": 0,
  "dry_run": false,
  "failed": 0,
  "map_file": "/home/feanor/scratch/dispatch/atheneum-memory-router/phase3/replayed.import-map.ndjson",
  "merged": 0,
  "skipped": 54
}
```
Import map sample (`replayed.import-map.ndjson`):
```json
{"content_hash":"f102339c4b7b4291450cec1c9c7a768ef15300ea51d6f749e67b45210006e784","existing_id":1,"line":1,"record_type":"memory","status":"skipped","target":"pi-dispatch-pattern-strix"}
{"content_hash":"9db96a95f0efe5abe3e0957098678b2cdf2b01a5dd321b392057f7f17c582a52","existing_id":2,"line":2,"record_type":"memory","status":"skipped","target":"skill_solve_task_llm_function"}
```

### 5.5. Bipolar Conflict & Revision Probes
Feeding a conflicting record with key `pi-dispatch-pattern-strix`, project `gkh`, but altered content hash `altered_hash_xyz`:

1. **Default Strategy (Conflict recorded, original preserved)**:
   ```bash
   target/debug/atheneum ledger-import fresh_target.db conflict_test.ndjson --apply --map conflict.import-map.ndjson
   ```
   Output:
   ```json
   {
     "conflicts": 1,
     "dry_run": false,
     "failed": 0,
     "map_file": ".../conflict.import-map.ndjson",
     "merged": 0,
     "skipped": 0
   }
   ```
   Import map entry:
   ```json
   {"content_hash":"altered_hash_xyz","existing_content_hash":"f102339c4b7b4291450cec1c9c7a768ef15300ea51d6f749e67b45210006e784","existing_id":1,"line":1,"reason":"Memory item with key 'pi-dispatch-pattern-strix' already exists with different content hash","record_type":"memory","status":"conflict","target":"pi-dispatch-pattern-strix"}
   ```

2. **Explicit Strategy (`--revision-on-conflict`)**:
   ```bash
   target/debug/atheneum ledger-import fresh_target.db conflict_test.ndjson --apply --revision-on-conflict --map revision.import-map.ndjson
   ```
   Output:
   ```json
   {
     "conflicts": 0,
     "dry_run": false,
     "failed": 0,
     "map_file": ".../revision.import-map.ndjson",
     "merged": 1,
     "skipped": 0
   }
   ```
   Import map entry:
   ```json
   {"content_hash":"altered_hash_xyz","line":1,"new_id":56,"record_type":"memory","status":"revision_created","supersedes_id":1,"target":"pi-dispatch-pattern-strix"}
   ```

### 5.6. `adopt-hermes` on Fixtures and Live Desktop Memory
1. **Fixture Run**:
   ```bash
   target/debug/atheneum adopt-hermes tasks/atheneum-memory-router/phase3/fixtures/hermes --out .../hermes_fixture.ndjson
   ```
   Output:
   ```json
   {
     "files_scanned": 3,
     "out": "/home/feanor/scratch/dispatch/atheneum-memory-router/phase3/hermes_fixture.ndjson",
     "records_extracted": 5,
     "status": "ok"
   }
   ```

2. **Real Desktop Path Run (Read-Only)**:
   Executed against `/home/feanor/.pi/agent/pi-hermes-memory`:
   ```bash
   target/debug/atheneum adopt-hermes /home/feanor/.pi/agent/pi-hermes-memory --out .../hermes_live.ndjson
   ```
   Output:
   ```json
   {
     "files_scanned": 4,
     "out": "/home/feanor/scratch/dispatch/atheneum-memory-router/phase3/hermes_live.ndjson",
     "records_extracted": 193,
     "status": "ok"
   }
   ```
   Importing the extracted 193 live Hermes memories into a disposable database succeeded with 193 merged, and replaying produced zero duplicates (193 skipped).

### 5.7. `adopt-bookkeeper` on Fixture Export
```bash
target/debug/atheneum adopt-bookkeeper tasks/atheneum-memory-router/phase3/fixtures/bookkeeper/export.md --out .../bookkeeper_fixture.ndjson
```
Output:
```json
{
  "out": "/home/feanor/scratch/dispatch/atheneum-memory-router/phase3/bookkeeper_fixture.ndjson",
  "records_extracted": 2,
  "source_file": "tasks/atheneum-memory-router/phase3/fixtures/bookkeeper/export.md",
  "status": "ok"
}
```

---

## 6. Summary of Changed Files

- `crates/atheneum/src/graph/ledger_v2.rs`: Created. Implements `CanonicalMemoryRecord`, `export_ledger_v2`, `import_ledger_v2`, `adopt_hermes_markdown`, `adopt_bookkeeper_markdown`, and 7 Phase 3 verification tests.
- `crates/atheneum/src/graph/ledger.rs`: Modified. Routes legacy memory import through conflict-safe `upsert_memory_item`, eliminating overwriting `store_memory`.
- `crates/atheneum/src/graph/mod.rs`: Registered and exported `ledger_v2` module and types.
- `crates/atheneum/src/cli/dispatch.rs`: Added CLI command handlers for `ledger-export-v2`, `ledger-import`, `adopt-hermes`, `adopt-bookkeeper`.
- `crates/atheneum/src/cli/util.rs`: Added command descriptions to CLI usage help.
- `tasks/atheneum-memory-router/phase3/fixtures/`: Created Hermes and Bookkeeper fixture files.
- `tasks/atheneum-memory-router/phase3/fail-first.log`: Captured pre-implementation test failure evidence.
- `tasks/atheneum-memory-router/phase3/tests-pass.log`: Captured passing test suite evidence.
