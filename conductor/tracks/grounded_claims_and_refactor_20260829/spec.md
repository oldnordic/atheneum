# Track Specification: Codebase Modularization & Grounded Claims Invalidation

## 1. Overview
This track delivers two critical improvements to the Atheneum platform:
1. **Architectural Codebase Reorganization**: Refactoring the 137KB monolithic `main.rs` in `crates/atheneum` into a clean, structured `src/cli/` module hierarchy with dedicated command submodules, and cleaning up legacy empty directories.
2. **Grounded Claims Invalidation Engine**: Integrating OpenWiki-style falsifiable grounded claims powered by compiler-grade AST / file hashing and GKH receipts, enabling zero-LLM staleness auditing and stale-aware memory recall.

## 2. Functional Requirements

### FR1: CLI Architecture Modularization (`crates/atheneum/src/cli/`)
- Decompose `main.rs` (3,400+ lines) into clean submodules:
  - `src/cli/mod.rs`: Main CLI parser, arguments, and top-level dispatch.
  - `src/cli/memory.rs`: Memory commands (`memory-store`, `memory-get`, `memory-list`).
  - `src/cli/discoveries.rs`: Discovery logging (`store-discovery`, `discoveries-recent`).
  - `src/cli/tasks.rs`: Task tracking (`task-create`, `task-list`, `task-done`, `task-archive`).
  - `src/cli/graph_nav.rs`: Navigation and decision threading (`navigate`, `thread`, `dream`).
  - `src/cli/digest.rs`: Session digest and packet compilation (`session-digest`, `session-trace`).
  - `src/cli/claims.rs`: New grounded claim commands (`claim-pin`, `claim-verify`, `audit`).
- Reduce `src/main.rs` to a thin entrypoint (< 50 lines) that invokes the CLI dispatcher.
- Clean up legacy empty directories (`ingest/`, `web/`).

### FR2: Grounded Claims Database Schema (`crates/atheneum/src/db/`)
- Add `grounded_claims` table schema migration:
  ```sql
  CREATE TABLE IF NOT EXISTS grounded_claims (
      id TEXT PRIMARY KEY,
      entity_id TEXT NOT NULL,
      project TEXT NOT NULL,
      file_path TEXT NOT NULL,
      symbol_name TEXT,
      ast_hash TEXT NOT NULL,
      receipt_hash TEXT,
      status TEXT DEFAULT 'verified', -- 'verified', 'stale', 'invalid'
      created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
      last_verified_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
      FOREIGN KEY(entity_id) REFERENCES entities(id)
  );
  CREATE INDEX IF NOT EXISTS idx_claims_project_status ON grounded_claims(project, status);
  ```
- Implement CRUD operations and models in `src/db/claims.rs`.

### FR3: Hash Invalidation & Staleness Auditor (`atheneum audit / claim-verify`)
- `atheneum claim-pin <db> <entity_id> <file_path> [--symbol <name>] [--receipt-hash <hash>]`:
  Computes SHA256 of target symbol/file and inserts a verified claim pin.
- `atheneum claim-verify <db> [--project <name>] [--fix]`:
  Scans all claims for the project against current disk files. If file/symbol hash diverges, updates status to `stale`.
- `atheneum audit <db> [--project <name>]`:
  Zero-LLM audit report showing valid vs stale claims and affected entities.

### FR4: Stale-Aware Memory Recall (`atheneum session-digest`)
- `atheneum session-digest` updated to filter out `stale` entities or prefix them with `[STALE: CODE DIVERGED]` notice.
- Ensure agents never ingest outdated memories as verified ground truth.

## 3. Non-Functional & Quality Contract
- **Zero Warnings**: `cargo clippy --workspace --all-targets -- -D warnings` must pass cleanly.
- **Fail-First Testing**: Unit tests for modularized CLI parsing, claim pin creation, hash divergence detection, and session-digest filtering.
- **No Stubs / No Fakes**: Complete implementation of all handlers without placeholder code.
