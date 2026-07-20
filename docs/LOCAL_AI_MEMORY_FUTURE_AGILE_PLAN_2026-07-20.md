# Atheneum Librarian Future Roadmap — Agile Plan

Spec: `docs/LOCAL_AI_MEMORY_FUTURE_SPEC_2026-07-20.md`

## Working agreements

- **Definition of Done** for every story:
  1. Code merged to `master` with scope-specific commits.
  2. At least one unit test in the module exercising the real code path.
  3. MCP tools verified via integration tests in `crates/atheneum-mcp/tests/`.
  4. Axum/Dashboard features tested under `web-ui` feature gates.
  5. `cargo test --all-targets` green; clippy green; fmt check passing.
  6. No stubs, placeholders, todos, or warnings.

---

## Epic F — Dynamic Model & Swap Guard

**Goal**: Discover active LLM backends dynamically and protect shared local hardware from GPU memory thrashing or slow swaps.

### Story F1 — `AtheneumGraph::discover_available_models` primitive
- **Spec**: FR-F1.
- **Files**: `crates/atheneum/src/graph/models.rs` (new module); `graph/mod.rs` (re-export).
- **Tasks**:
  1. Define `ModelInfo` and query `http://127.0.0.1:8080/v1/models` using optional `ureq` client.
  2. Return listed models, their loaded statuses, and active broker tags.
  3. Unit test: mock local API model response; assert parsing correctness.

### Story F2 — Swap Guard Fallbacks
- **Spec**: FR-F1 swap guard.
- **Files**: `crates/atheneum/src/graph/dream.rs`, `crates/atheneum/src/graph/memory.rs`.
- **Tasks**:
  1. Guard LLM calls inside `dream` and semantic embedders with model verification.
  2. If preferred model is not loaded, fall back to bag-of-words/Jaccard similarity without blocking.
  3. Unit test: verify fallback trigger.

### Story F3 — CLI & MCP model queries
- **Depends on**: F1.
- **Files**: `main.rs`, `crates/atheneum-mcp/src/tools.rs`, `backend.rs`.
- **Tasks**:
  1. Expose CLI command `models-list`.
  2. Expose MCP tool `list_models`.
  3. Integration test verifying tool routing.

---

## Epic G — Semantic Consolidation & LLM Dream Resolvers

**Goal**: Shift dream-time contradictions and merges from strict syntactic trigram matching to smart semantic LLM consolidation.

### Story G1 — `AtheneumGraph::semantic_consolidation` primitive
- **Spec**: FR-F2.
- **Files**: `crates/atheneum/src/graph/dream.rs` or `crates/atheneum/src/graph/consolidation.rs`.
- **Tasks**:
  1. Identify highly-similar concepts.
  2. Format and send a prompt to local model to merge content, returning unified markdown text.
  3. Write winner, mark loser as superseded, and redirect incoming edges.
  4. Unit test: seed redundant nodes, trigger semantic merge, assert unified result.

### Story G2 — CLI & MCP Semantic Consolidation tools
- **Depends on**: G1.
- **Files**: `main.rs`, `tools.rs`.
- **Tasks**:
  1. Expose CLI subcommand `dream-semantic <db> [--apply]`.
  2. Expose MCP tool `dream_semantic`.
  3. Integration tests verifying end-to-end tool run.

---

## Epic H — Premium Web UI & Interactive Visualizer

**Goal**: Provide a state-of-the-art interactive visualizer dashboard behind the `web-ui` feature flag.

### Story H1 — Axum HTTP Server integration
- **Spec**: FR-F3.
- **Files**: `crates/atheneum/src/web/mod.rs` (new module); `main.rs` (dashboard command).
- **Tasks**:
  1. Implement Axum router under `#[cfg(feature = "web-ui")]`.
  2. Map JSON API routes: `/api/graph`, `/api/traces`, `/api/lint`.
  3. CLI command `dashboard <db> [--port N]`.
  4. Compilation tests under `--features web-ui`.

### Story H2 — Force-Directed Graph UI Assets
- **Depends on**: H1.
- **Files**: `crates/atheneum/src/web/assets/` (HTML/JS SPA bundle).
- **Tasks**:
  1. Develop static SPA page utilizing D3.js or G6 graph visualization framework.
  2. Draw nodes by kind and color-code edges by relation type.
  3. Double-click node to show details and markdown text editor.

### Story H3 — Trace Flowchart & Contradiction Hub
- **Depends on**: H2.
- **Files**: `web/assets/`.
- **Tasks**:
  1. Render `QueryTrace` executions as flowcharts showing search paths.
  2. Build a Contradiction Center: list conflicts and allow user to click "Merge".

---

## Epic I — Pinning & TTL Strategy

**Goal**: Prevent eviction of critical core memories from hot tiers and implement self-expiring temporary facts.

### Story I1 — Pinned memory status
- **Spec**: FR-F4.
- **Files**: `crates/atheneum/src/graph/memory.rs`, `graph/types.rs`.
- **Tasks**:
  1. Add `pinned: bool` to Entity structures.
  2. Ensure pinned nodes are exempt from hot-tier LRU cache eviction and always included in `seed_memory`.
  3. Unit test: Pin concept, assert presence in seed memory under heavy token budget cuts.

### Story I2 — Temporary Memory TTLs
- **Spec**: FR-F4.
- **Files**: `crates/atheneum/src/graph/lint.rs`, `crates/atheneum/src/graph/types.rs`.
- **Tasks**:
  1. Add optional `ttl_hours: Option<u32>` and `expires_at: Option<String>` to entity data schema.
  2. In `lint_graph`/`maintain`, identify expired memories and auto-archive/delete.
  3. Unit tests asserting self-eviction of expired memories.
