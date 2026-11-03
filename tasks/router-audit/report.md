# Atheneum Router Queries Audit Trail: Zero-Stranded-Table Fix Report

## 1. Executive Summary & Root Cause

**Context**: Database migration v15 created the `router_queries` SQL table (`id`, `graph_entity_id`, `project_id`, `agent_name`, `query_hash`, `kinds_json`, `budget_tokens`, `returned_ids_json`, `omitted_count`, `created_at`) as an audit trail for memory routing. However, `route_memory` in `crates/atheneum/src/graph/memory_router.rs` was never wired to insert into it. After days of live routing, the table had 0 rows, violating the Zero Stranded Code Gate (Quality Gate 1).

**Resolution**:
1. **Audit Logging in `route_memory`**: `route_memory` now records an audit row in `router_queries` for every routed query.
   - `query_hash`: SHA-256 digest over the normalized query string (whitespace-collapsed and lowercased; raw query text is never stored).
   - `kinds_json`: Serialized JSON array of requested entity kinds (or `"[]"` if unspecified).
   - `budget_tokens`: Token budget allocated for the query.
   - `returned_ids_json`: Serialized JSON array of returned graph entity IDs.
   - `omitted_count`: Number of candidate memories omitted due to the token budget limit.
   - `project_id`: Optional project identifier from `MemoryRouteFilters`.
   - `agent_name`: Querying agent/caller name threaded via `MemoryRouteFilters.agent_name`, falling back cleanly to `"unknown"` if omitted or empty.
   - `created_at`: ISO 8601 UTC timestamp.
2. **Architectural Polarity for `expand_memory`**: `expand_memory` deliberately does NOT write to `router_queries`. It is a deterministic key/entity lookup by explicit ID (`memory:<id>`), not a semantic query routing decision. It has no query text, no filters, and no routing context. Logging expansions would skew query budgeting and query-frequency statistics.
3. **Read CLI (`router-queries`)**: Added `atheneum router-queries <db-path> [--project P] [--limit N]` which retrieves recent audit records formatted as JSON.
4. **Agent Threading via CLI & Hooks**:
   - `MemoryRouteFilters` extended with `pub agent_name: Option<String>`.
   - `atheneum memory-route` accepts `--agent <name>` and `agent=<name>`, falling back to `$ATHENEUM_AGENT` if set.
   - `atheneum-hook session-start` detects the agent (`$ATHENEUM_AGENT`, `$PI_SESSION_ID` -> `pi`, `$CLAUDE_CODE_SESSION_ID` -> `claude`, or positional `agent=...`) and forwards `--agent <name>` to `memory-route`.
5. **Quality Gates Enforced**:
   - Immutable test surfaces (`crates/atheneum/tests/` unmodified: `git diff --exit-code crates/atheneum/tests/` is clean).
   - TDD fail-first evidence captured in `tasks/router-audit/fail-first.log`.
   - Standard build, clippy (`-D warnings`), and fmt gates fully satisfied.
   - All 155 library tests passing (152 pre-existing + 3 new audit & bipolar tests).
   - Live DB never touched; work done entirely in the allocated worktree.

---

## 2. Architecture & Design Verification

### A. Dual-Layer & Transient Audit Data Invariant
As defined in `crates/atheneum/ARCHITECTURE.md`, transient operational audit data in `router_queries` does not require searchable GraphEntities unless exposed to graph traversal. `router_queries.graph_entity_id` is kept `NULL`, and rows are written directly inside SQLite transactions via `with_raw_connection`.

### B. Query Normalization & Query Hash Invariant
To ensure hash stability across varying input formats (e.g. trailing whitespace, uppercase characters, multiple internal spaces), queries are normalized prior to computing SHA-256:
```rust
let normalized_query = query
    .split_whitespace()
    .collect::<Vec<_>>()
    .join(" ")
    .to_lowercase();
let query_hash = crate::graph::hashing::sha256_hex(&normalized_query);
```

### C. Agent Resolution Hierarchy
Agent identity is resolved in order of precedence:
1. Explicit `--agent <A>` or `agent=<A>` flag in CLI arguments.
2. Environment variable `$ATHENEUM_AGENT`.
3. If unspecified or empty, defaults to `"unknown"` without raising errors.

### D. Architectural Decision: Why `expand_memory` Does Not Log
`expand_memory(&self, ids: &[String], budget: usize)` retrieves full item envelopes by specific public entity IDs.
- **No Query**: There is no query text, so no `query_hash` can be computed.
- **No Filters**: It takes no filter parameters (`kinds`, `confidence_floor`, `project`).
- **No Skewing**: `route_memory` already audits the query that produced the returned IDs. Logging secondary expansion requests would duplicate audit records and skew query frequency and token budget metrics.

---

## 3. Fail-First Evidence (`tasks/router-audit/fail-first.log`)

Tests were written before inserting into `router_queries`. Running `cargo test -p atheneum --lib test_router_queries` confirmed the failure of both audit logging and default agent handling:

```
   Compiling atheneum v0.15.0 (/home/feanor/orca/workspaces/atheneum/Router-Audit/crates/atheneum)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 4.96s
warning: the following packages contain code that will be rejected by a future version of Rust: binrw v0.13.4
note: to see what the problems were, use the option `--future-incompat-report`, or run `cargo report future-incompatibilities --id 1`
     Running unittests src/lib.rs (target/debug/deps/atheneum-6a80c9fc5d7b2231)

running 3 tests
test graph::memory_router::tests::test_router_queries_audit_logging ... FAILED
test graph::memory_router::tests::test_router_queries_bipolar_expand_does_not_log ... ok
test graph::memory_router::tests::test_router_queries_unknown_agent_default ... FAILED

failures:

---- graph::memory_router::tests::test_router_queries_audit_logging stdout ----

thread 'graph::memory_router::tests::test_router_queries_audit_logging' (839044) panicked at crates/atheneum/src/graph/memory_router.rs:4998:9:
assertion `left == right` failed: Expected exactly 1 audit row in router_queries
  left: 0
 right: 1
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

---- graph::memory_router::tests::test_router_queries_unknown_agent_default stdout ----

thread 'graph::memory_router::tests::test_router_queries_unknown_agent_default' (839047) panicked at crates/atheneum/src/graph/memory_router.rs:5041:9:
assertion `left == right` failed
  left: 0
 right: 1


failures:
    graph::memory_router::tests::test_router_queries_audit_logging
    graph::memory_router::tests::test_router_queries_unknown_agent_default

test result: FAILED. 1 passed; 2 failed; 0 ignored; 0 measured; 152 filtered out; finished in 0.14s

error: test failed, to rerun pass `-p atheneum --lib`
```

---

## 4. Passing Verification Evidence

After implementing audit insertion in `route_memory`:

### Unit Test Suite
```
running 3 tests
test graph::memory_router::tests::test_router_queries_audit_logging ... ok
test graph::memory_router::tests::test_router_queries_bipolar_expand_does_not_log ... ok
test graph::memory_router::tests::test_router_queries_unknown_agent_default ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 152 filtered out; finished in 0.15s
```

### Full Library Regression Suite
```
running 155 tests
...
test result: ok. 155 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 28.04s
```

### Static Analysis & Formatting Gates
- `cargo fmt --all -- --check`: Clean (exit code 0).
- `cargo clippy --all-targets -- -D warnings`: Clean (exit code 0).
- `git diff --exit-code crates/atheneum/tests/`: Clean (0 modifications to tests directory).

---

## 5. Manual Fixture Proof

Conducted manual black-box verification against an isolated fixture database (`fixture.db`):

```bash
=== 1. Init & Migrate Fixture DB ===
$ atheneum init tasks/router-audit/fixture.db
Initializing Atheneum graph at: tasks/router-audit/fixture.db
Graph initialized successfully
   Health: OK
$ atheneum migrate tasks/router-audit/fixture.db
Database is already up to date at schema version 15.

=== 2. Seed Fixture Memory Item ===
$ atheneum memory-item-upsert tasks/router-audit/fixture.db test_key "Verification content for router queries audit trail" --project test_proj
{
  "content_hash": "cf7a69a33f5e4b51d02fa1786fe01c4a822e4829a4f5d50269d1a7411a3e6bea",
  "graph_entity_id": 1,
  "sql_id": 1,
  "status": "created"
}

=== 3. Query router-queries Before Route (expect empty) ===
$ atheneum router-queries tasks/router-audit/fixture.db --project test_proj
[]

=== 4. Execute memory-route with --agent pi ===
$ atheneum memory-route tasks/router-audit/fixture.db "verification content" --project test_proj --agent pi --depth summary --budget 1000
{
  "budget_tokens": 1000,
  "budget_used": 4,
  "depth": "summary",
  "items": [
    {
      "evidence_state": "evidence_missing",
      "expandable": true,
      "graph_entity_id": 1,
      "id": "memory:1",
      "kind": "memory",
      "score": 1.0,
      "token_cost": 4
    }
  ],
  "omitted": 0,
  "project": "test_proj",
  "query": "verification content",
  "warnings": []
}

=== 5. Query router-queries After First Route ===
$ atheneum router-queries tasks/router-audit/fixture.db --project test_proj
[
  {
    "agent_name": "pi",
    "budget_tokens": 1000,
    "created_at": "2026-11-03T23:46:51.539997656+00:00",
    "id": 1,
    "kinds_json": "[]",
    "omitted_count": 0,
    "project_id": "test_proj",
    "query_hash": "9a100c1145f1d8bfef38af07ab46396b0689a8555c0c0c0f5bf93f08d6c0f00e",
    "returned_ids_json": "[1]"
  }
]

=== 6. Execute memory-route with no agent (expect unknown) ===
$ atheneum memory-route tasks/router-audit/fixture.db "another query" --project test_proj --depth summary --budget 800
{
  "budget_tokens": 800,
  "budget_used": 0,
  "depth": "summary",
  "items": [],
  "omitted": 0,
  "project": "test_proj",
  "query": "another query",
  "warnings": []
}

=== 7. Query router-queries After Second Route ===
$ atheneum router-queries tasks/router-audit/fixture.db --project test_proj
[
  {
    "agent_name": "unknown",
    "budget_tokens": 800,
    "created_at": "2026-11-03T23:46:51.556240335+00:00",
    "id": 2,
    "kinds_json": "[]",
    "omitted_count": 0,
    "project_id": "test_proj",
    "query_hash": "93f059c34b2cba7f079ad56e0ad4b72d44dc4cd1597af6933cafdd6927a39a3b",
    "returned_ids_json": "[]"
  },
  {
    "agent_name": "pi",
    "budget_tokens": 1000,
    "created_at": "2026-11-03T23:46:51.539997656+00:00",
    "id": 1,
    "kinds_json": "[]",
    "omitted_count": 0,
    "project_id": "test_proj",
    "query_hash": "9a100c1145f1d8bfef38af07ab46396b0689a8555c0c0c0f5bf93f08d6c0f00e",
    "returned_ids_json": "[1]"
  }
]

=== 8. Execute memory-expand (bipolar check) ===
$ atheneum memory-expand tasks/router-audit/fixture.db memory:1 --budget 500
{
  "budget_tokens": 500,
  "budget_used": 13,
  "items": [
    {
      "confidence": 1.0,
      "content": "Verification content for router queries audit trail",
      "content_hash": "cf7a69a33f5e4b51d02fa1786fe01c4a822e4829a4f5d50269d1a7411a3e6bea",
      "created_at": "2026-11-03T23:46:51.519952285+00:00",
      "evidence": [],
      "graph_entity_id": 1,
      "id": "memory:1",
      "key": "test_key",
      "kind": "memory",
      "project_id": "test_proj",
      "scope": "user",
      "sensitivity": "normal",
      "sql_id": 1,
      "status": "active",
      "summary": null,
      "title": null,
      "token_estimate": 13,
      "updated_at": "2026-11-03T23:46:51.519952285+00:00"
    }
  ],
  "omitted": 0,
  "warnings": []
}

=== 9. Query router-queries After Expand (expect unchanged count 2) ===
$ atheneum router-queries tasks/router-audit/fixture.db --project test_proj
[
  {
    "agent_name": "unknown",
    "budget_tokens": 800,
    "created_at": "2026-11-03T23:46:51.556240335+00:00",
    "id": 2,
    "kinds_json": "[]",
    "omitted_count": 0,
    "project_id": "test_proj",
    "query_hash": "93f059c34b2cba7f079ad56e0ad4b72d44dc4cd1597af6933cafdd6927a39a3b",
    "returned_ids_json": "[]"
  },
  {
    "agent_name": "pi",
    "budget_tokens": 1000,
    "created_at": "2026-11-03T23:46:51.539997656+00:00",
    "id": 1,
    "kinds_json": "[]",
    "omitted_count": 0,
    "project_id": "test_proj",
    "query_hash": "9a100c1145f1d8bfef38af07ab46396b0689a8555c0c0c0f5bf93f08d6c0f00e",
    "returned_ids_json": "[1]"
  }
]
```

---

## 6. Modified Files Summary

| File | Change |
| :--- | :--- |
| `crates/atheneum/src/graph/memory_router.rs` | Added `agent_name` to `MemoryRouteFilters`, added `RouterQueryAudit` struct with helper accessors, implemented audit insert in `route_memory`, added `recent_router_queries` method to `AtheneumGraph`, documented why `expand_memory` does not log, and added 3 unit tests. |
| `crates/atheneum/src/graph/mod.rs` | Re-exported `RouterQueryAudit`. |
| `crates/atheneum/src/lib.rs` | Re-exported `RouterQueryAudit`. |
| `crates/atheneum/src/cli/dispatch.rs` | Threaded `--agent` into `memory-route`, added `router-queries` subcommand dispatch. |
| `crates/atheneum/src/cli/util.rs` | Added `agent=` argument prefix parsing in `parse_options`, registered `router-queries` CLI usage string. |
| `crates/atheneum/API.md` | Documented `RouterQueryAudit`, `MemoryRouteFilters.agent_name`, `recent_router_queries`, and `expand_memory` non-logging behavior. |
| `pi-memory-router/bin/atheneum-hook` | Added agent detection and `--agent` forwarding to `memory-route` during `session-start`. |
| `tasks/router-audit/fail-first.log` | Recorded pre-implementation failing test log evidence. |
| `tasks/router-audit/report.md` | Deliverable report with implementation details, fail-first evidence, passing test evidence, and manual fixture proof. |
