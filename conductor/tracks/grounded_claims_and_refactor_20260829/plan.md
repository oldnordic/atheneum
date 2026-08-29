# Track Implementation Plan: Codebase Modularization & Grounded Claims

## Phase 1: Codebase & CLI Modularization
- [x] Task: Create `crates/atheneum/src/cli/` module hierarchy and define shared CLI context.
- [x] Task: Slim down `src/main.rs` to minimal entrypoint (14 lines) and decompose into `src/cli/dispatch.rs`, `src/cli/util.rs`, `src/cli/mod.rs`.
- [x] Task: Remove dead empty directories (`ingest/`, `web/`).
- [x] Task: Verify parity gate: all existing CLI commands and integration tests work identically (`cargo test --workspace` passed 100%).
- [x] Task: Phase 1 Verification & Checkpoint (Clippy clean `-D warnings`, Rustfmt verified).

## Phase 2: Grounded Claims Database Layer
- [x] Task: Write fail-first unit tests for `grounded_claims` table schema migration and CRUD methods (`tests/grounded_claims_tests.rs`).
- [x] Task: Add `grounded_claims` migration v14 in `crates/atheneum/src/db/` and SQLite connection bootstrap.
- [x] Task: Implement `GroundedClaim` data models and database queries (`insert_claim`, `get_claims_for_entity`, `update_claim_status`, `list_stale_claims`).
- [x] Task: Run tests and verify database layer passes (`test_grounded_claims_migration_and_lifecycle` passed).
- [x] Task: Phase 2 Verification & Checkpoint.

## Phase 3: Hash Invalidation & Staleness Auditor Engine
- [x] Task: Write fail-first unit tests for AST / file hashing and claim verification against modified files (`tests/grounded_claims_tests.rs`).
- [x] Task: Implement SHA256 file and symbol-slice hashing logic in `src/graph/hashing.rs` (`compute_file_sha256`, `compute_bytes_sha256`).
- [x] Task: Implement CLI commands `claim-pin`, `claim-verify`, and `audit` in `src/cli/dispatch.rs`.
- [x] Task: Verify `atheneum audit` and `claim-verify` detect modified code and flip claim status from `verified` to `stale`.
- [x] Task: Phase 3 Verification & Checkpoint.

## Phase 4: Stale-Aware Recall & MCP Integration
- [x] Task: Write fail-first tests for `atheneum session-digest` with stale vs verified claim filtering (`test_stale_aware_session_digest_flags_outdated_memory`).
- [x] Task: Update `session-digest` builder to check claim statuses and omit or flag stale facts (`crates/atheneum/src/graph/digest.rs`).
- [x] Task: Expose `pin_grounded_claim` and `audit_claims` tools in `crates/atheneum-mcp` (`tools.rs` and `backend.rs`).
- [x] Task: Phase 4 Verification & Checkpoint.

## Phase 5: Full Verification & Release Gate
- [x] Task: Run `cargo fmt --all -- --check` (Passed cleanly).
- [x] Task: Run `cargo clippy --workspace --all-targets -- -D warnings` (Zero warnings).
- [x] Task: Run `cargo test --workspace` (100% test suite passing across all packages).
- [x] Task: Final Phase Verification & Checkpoint.

