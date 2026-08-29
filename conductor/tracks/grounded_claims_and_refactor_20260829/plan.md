# Track Implementation Plan: Codebase Modularization & Grounded Claims

## Phase 1: Codebase & CLI Modularization
- [ ] Task: Create `crates/atheneum/src/cli/` module hierarchy and define shared CLI context.
- [ ] Task: Extract memory subcommands into `src/cli/memory.rs`.
- [ ] Task: Extract task management subcommands into `src/cli/tasks.rs`.
- [ ] Task: Extract discovery and audit subcommands into `src/cli/discoveries.rs`.
- [ ] Task: Extract graph navigation, dreaming, and threading into `src/cli/graph_nav.rs`.
- [ ] Task: Extract session digest and trace into `src/cli/digest.rs`.
- [ ] Task: Slim down `src/main.rs` to minimal entrypoint and remove dead empty directories (`ingest/`, `web/`).
- [ ] Task: Verify parity gate: all existing CLI commands work identically after modularization (`cargo test --workspace`).
- [ ] Task: Phase 1 Verification & Checkpoint.

## Phase 2: Grounded Claims Database Layer
- [ ] Task: Write fail-first unit tests for `grounded_claims` table schema migration and CRUD methods.
- [ ] Task: Add `grounded_claims` migration in `crates/atheneum/src/db/` and SQLite connection bootstrap.
- [ ] Task: Implement `GroundedClaim` data models and database queries (`insert_claim`, `get_claims_for_entity`, `update_claim_status`, `list_stale_claims`).
- [ ] Task: Run tests and verify database layer passes.
- [ ] Task: Phase 2 Verification & Checkpoint.

## Phase 3: Hash Invalidation & Staleness Auditor Engine
- [ ] Task: Write fail-first unit tests for AST / file hashing and claim verification against modified files.
- [ ] Task: Implement SHA256 file and symbol-slice hashing logic in `src/graph/hashing.rs` or `src/db/claims.rs`.
- [ ] Task: Implement CLI commands `claim-pin`, `claim-verify`, and `audit` in `src/cli/claims.rs`.
- [ ] Task: Verify `atheneum audit` detects modified code and flips claim status from `verified` to `stale`.
- [ ] Task: Phase 3 Verification & Checkpoint.

## Phase 4: Stale-Aware Recall & MCP Integration
- [ ] Task: Write fail-first tests for `atheneum session-digest` with stale vs verified claim filtering.
- [ ] Task: Update `session-digest` builder to check claim statuses and omit or flag stale facts.
- [ ] Task: Expose `store_grounded_claim` and `verify_claims` tools in `crates/atheneum-mcp`.
- [ ] Task: Phase 4 Verification & Checkpoint.

## Phase 5: Full Verification & Release Gate
- [ ] Task: Run `cargo fmt --all -- --check`.
- [ ] Task: Run `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] Task: Run `cargo test --workspace`.
- [ ] Task: Final Phase Verification & Checkpoint.
