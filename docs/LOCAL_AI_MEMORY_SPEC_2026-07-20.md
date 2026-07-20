# Atheneum Librarian Spec — v0.1

Status: proposed. Scope: close the high-leverage gaps identified in
`docs/LOCAL_AI_MEMORY_GAP_ANALYSIS_2026-07-20.md` without changing atheneum's
SQLite-first architecture.

Design tenets (in priority order):
1. **Real results only.** Every new tool/path must ship with a unit + an
   integration test that exercises the actual code path. No `todo!()`, no
   feature-flagged stubs, no `#[allow(dead_code)]` to hide unused paths.
   (Projects/AGENTS.md "Real Results Only".)
2. **Library, not agent.** Atheneum stays a deterministic library/tool surface.
   Any "librarian agent" lives in the caller (envoy/hermes), not in atheneum.
   Atheneum exposes the *primitives* such an agent needs.
3. **Cite to `file:line`.** Every behavioral claim in this spec is verifiable
   against source. New code must extend that property, not break it.
4. **Backward compatible.** New tools are additive; existing MCP tool schemas
   only gain optional fields. No breaking changes to `AtheneumGraph`.

---

## Functional requirements

### FR-1: `update_memory` primitive (closes gap #1, #3)

**Why.** Today `store_memory` always inserts (`crates/atheneum-mcp/src/tools.rs:190`).
Clients cannot patch an existing memory, so every correction becomes a new row —
the exact "junk drawer" the transcript warns about.

**Surface.**
- Rust: `AtheneumGraph::update_memory(id: i64, patch: &MemoryPatch) -> Result<MemoryPreview>`
  where `MemoryPatch { content: Option<String>, importance: Option<i64>,
  tags: Option<Vec<String>>, replace_tags: bool }`.
- CLI: `atheneum memory-update <db> --id N [--content "..."] [--importance N]
  [--tags a,b --replace-tags]`.
- MCP tool: `update_memory` with the same fields plus required `id`.

**Semantics.**
- `content=Some` replaces the body and recomputes the content hash and embedder
  vector (same path as `store_memory`).
- Tags merge by default; `replace_tags=true` overwrites.
- Returns the updated `MemoryPreview` (same shape as `store_memory`).
- Errors: `AtheneumError::EntityNotFound(id)` if `id` is missing or not kind
  `Memory`.

**Acceptance.**
- Unit test: insert → update content → assert new body, new hash, id unchanged.
- Unit test: update with no fields is a no-op and returns the current preview.
- Integration test (MCP): round-trip `store_memory` → `update_memory` →
  `query_memory` shows updated content only.

---

### FR-2: Enrich-before-create helper (closes gap #9)

**Why.** Clients need a single call that says "this fact belongs to concept X;
patch X if it exists, else create it" instead of forcing them to spawn a new
memory every time.

**Surface.**
- Rust: `AtheneumGraph::upsert_memory_by_concept(
    concept_name: &str, body_patch: &str, link_from: Option<i64>,
  ) -> Result<UpsertResult>` where `UpsertResult { memory_id, action: Enriched|Created }`.
- MCP tool: `add_memory` (the transcript's name) that wraps
  `upsert_memory_by_concept`. Required: `concept`, `body_patch`. Optional:
  `link_from` (entity id to bidirectionally link, see FR-3).

**Semantics.**
- Find the Concept entity by name (`find_entity_id_by_kind_and_name` already
  exists at `graph/mod.rs`). If found and it has exactly one attached Memory,
  append `body_patch` to that memory's content (with a newline separator) and
  bump `updated_at`. If found but no attached memory, create one linked to the
  concept. If not found, create the Concept + a Memory attached to it.
- Returns the action taken so the caller can decide whether to log a decision.

**Acceptance.**
- Unit test: two calls with the same concept and different patches result in
  exactly one Memory entity whose body contains both patches.
- Unit test: a new concept name creates exactly one Concept + one Memory and
  one `attached_to` edge.

---

### FR-3: Bidirectional auto-linking (closes gap #10)

**Why.** `insert_edge` (`graph/mod.rs:428`) creates one directed edge and the
caller must remember to create the reciprocal. That is why new concepts become
islands.

**Surface.**
- Rust: `AtheneumGraph::insert_edge_pair(from, to, edge_type, data,
  reciprocal_type, reciprocal_data) -> Result<(i64, i64)>`.
- A flag on the new `add_memory` MCP tool: `link_both_ways: bool` (default
  true).

**Semantics.**
- Inserts the forward edge, then the reciprocal edge with the supplied type.
  Both go through the existing ontology validation.
- Reciprocal pairs we will seed in the standard ontology (`ontology.rs:126`):
  `attached_to` ↔ `has_memory`, `related_to` ↔ `related_to`,
  `verified_by` ↔ `verifies`, `superseded_by` ↔ `supersedes`.

**Acceptance.**
- Unit test: insert pair → `outgoing_edges(a)` and `incoming_edges(a)` both
  non-empty; ontology validation fires for an invalid reciprocal.

---

### FR-4: Graph-health lint + rewire (closes gap #11, #12, #14, #15)

**Why.** Dream currently only flags orphans (`graph/dream.rs:41`, test at
`dream.rs:775`) and contradictions (`dream.rs:286`, test at `dream.rs:915`).
It never *repairs*. The transcript's explicit lesson: detection without
rewiring leaves the graph degrading.

**Surface.**
- Rust: `AtheneumGraph::lint_graph(LintConfig) -> Result<LintReport>` and
  `AtheneumGraph::maintain(MaintainConfig) -> Result<MaintainReport>`.
- CLI: `atheneum lint <db> [--project P] [--json]` and
  `atheneum maintain <db> [--project P] [--apply]`.
- MCP tool: `maintain` (read+edit, gated behind the same direct/http feature
  split as the other mutating tools).

**`lint` semantics (deterministic, read-only).**
- Orphans: entities of kind `Concept`/`Memory`/`WikiPage` with zero incoming
  non-metadata edges. Auto-generated index files (wikilink source path matches
  `*/index.md` or entity `data.role == "auto_index"`) are excluded as link
  sources, so they cannot mask orphans.
- Broken links: wikilinks (`extract_wikilinks` at `graph/wiki.rs`) whose target
  path resolves to no `WikiPage`.
- Stale superseded: entities with a self-edge `superseded_by` older than
  `LintConfig::stale_superseded_days`.

**`maintain` semantics (mutating, `--apply` required).**
- For each orphan: find the candidate concept with the highest lexical
  similarity (reuse the trigram-Jaccard from `dream.rs:108`) above
  `MaintainConfig::rewire_threshold` (default 0.3); insert a bidirectional
  `related_to` pair (FR-3). Below threshold, leave orphan and add to the report.
- For each broken link: either create a stub `WikiPage` at the target path or
  sever the link (configurable; default: stub).
- For each contradiction flagged by `dream_pass`: append the new fact and mark
  the old content with a `superseded_by` self-edge + `reason: contradiction`.
  Does *not* delete — the transcript's "old fact must be gone everywhere" is
  honoured by setting `data.superseded_at` and excluding superseded rows from
  `query_memory`/`search_memory` by default.

**Acceptance.**
- Unit test: seed an orphan concept, run `maintain --apply`, assert a
  bidirectional `related_to` edge to the most-similar concept and that the
  orphan is no longer in the next `lint` report.
- Unit test: seed a broken wikilink, run `maintain --apply` with stub mode,
  assert the target page now exists.
- Unit test: seed a contradiction, run `maintain --apply`, assert old row is
  excluded from `query_memory` and the new row is returned.
- Integration test (MCP): full lint → maintain → lint cycle returns zero
  orphans.

---

### FR-5: Seed-memory / bootstrap-to-client tool (closes gap #7, #8)

**Why.** Today MCP clients see only static tool descriptions and have no idea
what's in the KB — the transcript's "amnesia" failure mode.

**Surface.**
- Rust: `AtheneumGraph::seed_memory(project: Option<&str>, token_budget: usize)
  -> Result<SeedMemory>` returning a compact summary listing *concepts* (not
  file paths) grouped by kind, plus top-N most-recent memories.
- MCP tool: `seed_memory` with optional `project` and `tokens` (default 800).
- The MCP server's `instructions` field and the `query`/`navigate` tool
  descriptions are regenerated at session connect from `seed_memory`, so the
  client model opens every session already aware of what's in the library.

**Semantics.**
- Group entities by kind, then by the most common `data.scope` value within
  each kind. For each group emit one line: concept name + one-line summary
  (first 80 chars of `data.summary` if present, else `data.content`).
- Hard cap at `token_budget` (estimate via the existing `estimate_entity_tokens`
  at `graph/navigation.rs`); truncate with a `... (N more)` footer.
- Concepts only — never raw ToolCall/ReasoningLog/TestRun entities (already
  filtered in `backend.rs::serialize_paginated_view`'s `NOISE_ENTITY_KINDS`).

**Acceptance.**
- Unit test: seed a known set of concepts and memories, assert the output
  contains the concept names and stays under the token budget.
- Unit test: output never contains entities of kind `ToolCall`/`ReasoningLog`.

---

### FR-6: Hot-tier recency cache (closes gap #17)

**Why.** Every `navigate`/`query_memory` pays full BFS/FTS cost even for a
memory written 30 seconds ago.

**Surface.**
- Rust: extend `GraphRuntime` (`graph/cache.rs`) with an LRU keyed by
  `(project, query-normalized)` capped at `HotTierConfig::capacity` (default
  256 entries, 5-minute TTL).
- CLI/MCP: no new surface — `query_memory` and `navigate` consult the hot tier
  first; `store_memory`/`update_memory`/`add_memory`/`maintain` invalidate it.

**Semantics.**
- Cache only *reads*. Any mutating op bumps the existing
  `runtime.bump_navigation_generation()` (already in `insert_edge` at
  `graph/mod.rs:469`) and clears the hot tier.
- TTL is configurable; default 5 min matches the transcript's "5 minutes ago"
  intuition.

**Acceptance.**
- Unit test: two identical `query_memory` calls return the same result; the
  second is served from cache (assert via `runtime_stats()` hit counter).
- Unit test: after `update_memory`, the cache is empty and the next read
  reflects the new content.

---

### FR-7: Per-query trace records (closes gap #21)

**Why.** Without a trace you cannot debug "why did the agent return this answer".
The transcript treats visualisability as a first-class feature.

**Surface.**
- Rust: `AtheneumGraph::trace_query(plan: &NavigateQueryPlan, result_ids: &[i64])
  -> Result<i64>` that inserts a `QueryTrace` entity with `data.plan`,
  `data.result_ids`, `data.started_at`, `data.finished_at`, and a `produced_by`
  edge from the trace to each result entity.
- `navigate` (CLI + MCP) gains `--trace` / `trace: bool`. When set, the trace
  entity id is returned in the response under `trace_id`.
- CLI: `atheneum trace-get <db> --id N` to replay a past query.

**Acceptance.**
- Unit test: run `navigate --trace`, assert exactly one `QueryTrace` entity
  exists with `produced_by` edges to every returned entity id.
- Unit test: `trace-get` returns the original plan and result ids.

---

### FR-8: Scheduled dream (closes gap #16)

**Why.** Dream exists but must be invoked manually; the transcript's "dreaming"
feature is explicitly idle-time.

**Surface.**
- CLI: `atheneum dream --schedule <cron> [--mode auto-merge] [--project P]`
  writes a cron entry via the existing hermes-cron surface (this is an operator
  concern, not an atheneum library concern — atheneum only documents the
  recommended schedule and provides a `--daemonize` flag that loops with a
  sleep).
- Library: `AtheneumGraph::dream_if_idle(threshold_secs: u64) -> Result<Option<DreamReport>>`
  — only runs a pass if no writes have occurred in `threshold_secs`.

**Acceptance.**
- Unit test: with a recent write, `dream_if_idle` returns `Ok(None)` and makes
  no mutations.
- Unit test: with no writes for longer than the threshold, `dream_if_idle`
  returns `Ok(Some(report))` and the report is non-empty when seeded with a
  known duplicate.

---

## Non-functional requirements

### NFR-1: Performance
- `seed_memory(tokens=800)` must return in <50ms p95 on a graph of 10k entities.
- Hot-tier hit path must be <1ms.
- `maintain` must process ≥1000 orphans/sec on a warm cache.

### NFR-2: Backward compatibility
- All existing MCP tools keep their schemas; new fields are optional.
- `AtheneumGraph::open()` of an existing DB must continue to work without a
  manual migration step — new schema additions go through the existing
  `run_startup_migrations` path at `graph/mod.rs:143`.

### NFR-3: Testability
- Every new public function has at least one unit test in the same module
  (matching the pattern in `graph/dream.rs:667+`).
- Every new MCP tool has an integration test under
  `crates/atheneum-mcp/tests/` (matching `integration_test.rs`).

### NFR-4: Determinism
- `lint`, `seed_memory`, and `dream_pass(DryRun, …)` must be deterministic and
  side-effect-free. Verified by running twice in a row and asserting identical
  output.

---

## Out of scope (v0.1)

- Web UI / graph visualizer (transcript items #19, #20) — separate workstream.
- Migrating episodic memory to markdown-on-disk (#4) — SQLite remains the
  store; a markdown *export* may arrive in v0.2.
- A built-in "librarian agent" loop — atheneum exposes primitives; the agent
  loop belongs in envoy/hermes.
