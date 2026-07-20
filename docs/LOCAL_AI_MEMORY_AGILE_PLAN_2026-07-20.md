# Atheneum Librarian — Agile Plan

Spec: `docs/LOCAL_AI_MEMORY_SPEC_2026-07-20.md`
Gap analysis: `docs/LOCAL_AI_MEMORY_GAP_ANALYSIS_2026-07-20.md`

## Working agreements

- **Definition of Done** for every story:
  1. Code merged to `master` with a scope-specific commit (conventional commits,
     matching existing style: `feat(atheneum/memory): …`).
  2. At least one unit test in the same module exercising the real code path.
  3. For MCP tools: one integration test under `crates/atheneum-mcp/tests/`.
  4. `cargo test --workspace` green; `cargo build --all-features` green.
  5. MANUAL.md + relevant API.md sections updated, citing new `file:line`.
  6. CHANGELOG.md "Unreleased" entry added.
  7. No `todo!()`, `unimplemented!()`, `#[allow(dead_code)]`, or stub paths
     (Projects/AGENTS.md "Real Results Only").
- **Story size**: ≤1 day of work. Split if larger.
- **Ordering**: priorities below are debt-first — we fix the read paths
  clients depend on before we add new surfaces.

---

## Epic A — Repair the write path  (stops ongoing graph rot)

**Goal**: every memory write enriches instead of duplicating; every concept is
linked both ways. Closes gaps #1, #3, #9, #10.

### Story A1 — `AtheneumGraph::update_memory` primitive
- **Spec**: FR-1.
- **Files**: `crates/atheneum/src/graph/memory.rs` (new fn); new
  `MemoryPatch` struct in `graph/types.rs`; re-export from `lib.rs`.
- **Tasks**:
  1. Add `MemoryPatch { content, importance, tags, replace_tags }`.
  2. Implement `update_memory(id, &patch)` — load row, apply fields, recompute
     hash + embedder vector (reuse `store_memory`'s path), write back, bump
     `navigation_generation`.
  3. Unit tests: insert→update→assert; no-op patch; not-found error.
- **DoD**: standard DoD + `cargo test -p atheneum memory` green.

### Story A2 — CLI `memory-update`
- **Depends on**: A1.
- **Files**: `crates/atheneum/src/main.rs` (new arm in the match, ~line 1301
  near existing `memory-store`/`memory-get`/`memory-list`).
- **Tasks**: add `memory-update <db> --id N [--content …] [--importance N]
  [--tags a,b --replace-tags] [--json]`; reuse existing `parse_options`.
- **DoD**: smoke-test on a temp DB shows updated row.

### Story A3 — MCP `update_memory` tool
- **Depends on**: A1.
- **Files**: `crates/atheneum-mcp/src/tools.rs` (new `update_memory()` fn,
  register in `register_all` at line 18); `backend.rs` `Backend` trait + both
  impls.
- **Tasks**: mirror `store_memory`'s structure; required `id`, optional fields
  per FR-1.
- **DoD**: integration test in `crates/atheneum-mcp/tests/integration_test.rs`
  round-tripping store→update→query.

### Story A4 — `upsert_memory_by_concept` primitive
- **Spec**: FR-2.
- **Files**: `crates/atheneum/src/graph/memory.rs`; `UpsertResult` enum in
  `types.rs`.
- **Tasks**:
  1. Look up Concept by name via `find_entity_id_by_kind_and_name`.
  2. Enrich-or-create per FR-2 semantics.
  3. Unit tests: enrich existing; create new; append body.
- **DoD**: standard DoD.

### Story A5 — `insert_edge_pair` primitive
- **Spec**: FR-3.
- **Files**: `crates/atheneum/src/graph/mod.rs` (next to `insert_edge` at
  line 428); seed reciprocal pairs in `ontology.rs:126` `seed_standard_ontology`.
- **Tasks**: implement pair insert with ontology validation on both edges;
  unit tests for valid pair + invalid reciprocal rejection.
- **DoD**: standard DoD.

### Story A6 — MCP `add_memory` tool (wraps A4 + A5)
- **Depends on**: A4, A5.
- **Files**: `crates/atheneum-mcp/src/tools.rs`; backend impls.
- **Tasks**: new tool with required `concept`, `body_patch`, optional
  `link_from`, `link_both_ways` (default true). Calls `upsert_memory_by_concept`
  and (when `link_from` set) `insert_edge_pair`.
- **DoD**: integration test: two calls same concept → one memory with both
  bodies; `add_memory` with `link_from` → both directions present.

---

## Epic B — Graph health  (repair accumulated rot)

**Goal**: orphans, broken links, and contradictions are not just flagged but
fixed. Closes gaps #11, #12, #14, #15.

### Story B1 — `LintConfig` + `lint_graph` primitive (read-only)
- **Spec**: FR-4 `lint` branch.
- **Files**: new module `crates/atheneum/src/graph/lint.rs`; export from
  `graph/mod.rs`.
- **Tasks**:
  1. Orphan scan: Concepts/Memories/WikiPages with zero incoming non-metadata
     edges; exclude `*/index.md` and `data.role == "auto_index"` sources.
  2. Broken-wikilink scan using `extract_wikilinks` (`graph/wiki.rs`).
  3. Stale-superseded scan (self-edges older than threshold).
  4. Unit tests: orphan detected, index-source exclusion works, broken link
     detected.
- **DoD**: standard DoD + determinism test (two runs identical output).

### Story B2 — CLI `lint`
- **Depends on**: B1.
- **Files**: `crates/atheneum/src/main.rs`.
- **Tasks**: `atheneum lint <db> [--project P] [--json]`. Human renderer
  grouped by finding type.
- **DoD**: smoke test on the real `~/Projects/atheneum/atheneum.db` shows
  findings or "clean".

### Story B3 — `MaintainConfig` + `maintain` primitive (mutating)
- **Depends on**: B1 (reuse the scans), A5 (for bidirectional rewire).
- **Spec**: FR-4 `maintain` branch.
- **Files**: extend `crates/atheneum/src/graph/lint.rs` or split into
  `graph/maintain.rs`.
- **Tasks**:
  1. Orphan rewire: trigram-Jaccard candidate selection (reuse `dream.rs:108`
     `trigrams`), insert `related_to` pair above threshold.
  2. Broken-link stub-or-sever (default stub).
  3. Contradiction resolve: mark old row `superseded_by` self-edge +
     `data.superseded_at`; exclude superseded from default
     `query_memory`/`search_memory`.
  4. Unit tests per branch.
- **DoD**: standard DoD + a test that lint→maintain→lint returns zero orphans
  on the seeded fixture.

### Story B4 — CLI `maintain` + MCP `maintain` tool
- **Depends on**: B3.
- **Files**: `main.rs`; `atheneum-mcp/src/tools.rs` + backend.
- **Tasks**: CLI `atheneum maintain <db> [--project P] [--apply]` (default is
  dry-run); MCP `maintain` tool with `apply: bool`.
- **DoD**: integration test for MCP maintain applies and is idempotent.

### Story B5 — Exclude superseded rows from default reads
- **Depends on**: B3.
- **Files**: `graph/memory.rs` (`query_memory`, `search_memory`),
  `graph/knowledge.rs` if needed.
- **Tasks**: add `include_superseded: bool` (default false) to the query types;
  filter on `data.superseded_at` absence.
- **DoD**: unit test — after a contradiction resolve, default query returns
  only the winner; opt-in flag returns both.

---

## Epic C — Make clients aware  (kill the "amnesia" failure mode)

**Goal**: every MCP client opens a session already knowing what the library
contains. Closes gaps #7, #8.

### Story C1 — `seed_memory` primitive
- **Spec**: FR-5.
- **Files**: new `crates/atheneum/src/graph/seed.rs`; export from `lib.rs`.
- **Tasks**:
  1. Group entities by kind then top scope; concepts only; noise kinds
     filtered.
  2. Token-budget truncation using `estimate_entity_tokens`.
  3. Unit tests: budget honoured, noise excluded, project filter works.
- **DoD**: standard DoD.

### Story C2 — MCP `seed_memory` tool + dynamic `instructions`
- **Depends on**: C1.
- **Files**: `crates/atheneum-mcp/src/tools.rs`; `lib.rs`/`main.rs` of the
  server (where the tool list + `instructions` field are built).
- **Tasks**:
  1. Register `seed_memory` tool.
  2. On session `initialize`, call `seed_memory(tokens=800)` and inject the
     result into the MCP `instructions` field and into the `navigate`/`query`
     tool descriptions.
- **DoD**: integration test — a freshly connected client receives non-empty
  `instructions` containing at least one concept name from the seeded graph.

### Story C3 — CLI `seed-memory`
- **Depends on**: C1.
- **Files**: `main.rs`.
- **Tasks**: `atheneum seed-memory <db> [--project P] [--tokens N]` for
  operator inspection / debugging.
- **DoD**: smoke test on real DB.

---

## Epic D — Performance + debuggability

**Goal**: recent memory is instant; every query is replayable. Closes gaps
#17, #21, #16.

### Story D1 — Hot-tier LRU on `GraphRuntime`
- **Spec**: FR-6.
- **Files**: `crates/atheneum/src/graph/cache.rs`.
- **Tasks**: add `HotTier` to `GraphRuntime`; consult in `query_memory` +
  `navigate`; invalidate on every mutating op.
- **DoD**: unit test — second identical read is a cache hit
  (`runtime_stats()`); any mutation clears the tier.

### Story D2 — `QueryTrace` entity + `navigate --trace`
- **Spec**: FR-7.
- **Files**: `graph/navigation.rs`; new `EntityType::QueryTrace` variant;
  `graph/types.rs`; CLI/MCP `--trace` / `trace: bool`.
- **Tasks**: insert trace entity + `produced_by` edges when flag set; CLI
  `trace-get`.
- **DoD**: unit test — trace entity + edges exist; `trace-get` round-trips.

### Story D3 — `dream_if_idle` + scheduling doc
- **Spec**: FR-8.
- **Files**: `graph/dream.rs` (new fn); MANUAL.md section with the recommended
  cron line.
- **Tasks**: implement idle check (no writes for `threshold_secs`); document
  `*/30 * * * * atheneum dream …` cron in MANUAL.md.
- **DoD**: unit test — idle pass runs; busy pass is a no-op.

---

## Epic E — Polish + release

### Story E1 — MANUAL.md "Librarian mode" section
- **Depends on**: A6, B4, C2.
- **Tasks**: new section documenting the add/update/maintain/seed workflow with
  copy-paste commands. Every command cited to its `main.rs` arm.

### Story E2 — API.md additions
- **Depends on**: all of Epic A–D.
- **Tasks**: document every new public symbol with a one-line summary + the
  module path.

### Story E3 — CHANGELOG + version bump
- **Tasks**: collect the "Unreleased" entries; bump `atheneum` 0.10.0 → 0.11.0
  (new MCP tools = minor).

### Story E4 — Dogfood on the operator graph
- **Tasks**: run `maintain --apply` against `~/.magellan/atheneum/atheneum.db`;
  report orphan count before/after; confirm no regression in
  `envoy_knowledge` calls.

---

## Suggested ordering (2-week sprint shape)

| Order | Story | Rationale |
|---|---|---|
| 1 | A1, A2 | unblocks every later write-path story |
| 2 | A3 | first new MCP tool; validates the tool-addition workflow |
| 3 | A5 | needed by A6 and B3 |
| 4 | A4, A6 | close the enrich-before-create loop |
| 5 | B1, B2 | first user-visible "health" output |
| 6 | B3, B4, B5 | the actual repair loop |
| 7 | C1, C2, C3 | clients stop flying blind |
| 8 | D1, D2 | perf + debuggability for the heavier post-repair read load |
| 9 | D3 | scheduling closes the dream loop |
| 10 | E1–E4 | docs + release |

Each row is ≈1 day; the whole plan is 10 working days for one engineer,
shorter with parallelization on Epic B vs C (independent).

---

## Risk register

| Risk | Mitigation |
|---|---|
| `maintain --apply` corrupts a real graph on first run | Ship B1+B2 first (dry-run lint); require explicit `--apply`; add an idempotency test; dogfood on a copy before the live `~/.magellan/atheneum` DB. |
| Seed-memory token budget makes `instructions` too large for some MCP clients | Hard cap (default 800 tokens); make injectable behaviour configurable via a server flag; document. |
| Hot-tier serves stale data after a write through a different connection | Tie invalidation to `bump_navigation_generation` (already called on every mutation); add a test that opens two `AtheneumGraph` handles and asserts the cache clears on write through the other. |
| Excluding superseded rows breaks a caller relying on old behaviour | Add `include_superseded: bool` opt-in (B5) rather than hard-removing; document in CHANGELOG as a behaviour change. |
| `add_memory` concept disambiguation picks the wrong existing concept | Return `UpsertResult::Enriched { id }` so the caller can audit; log a Decision via `store_discovery` when enrichment happens; make the similarity threshold configurable. |
