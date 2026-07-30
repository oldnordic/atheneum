---
title: "Unified Tool API — magellan/llmgrep/mirage/atheneum/envoy"
created: 2026-07-30
status: DESIGN (approved, pending write-up review) — not implemented
---

# Unified Tool API Design

## Why

Three real MCP servers exist today with three incompatible calling conventions
for "search": grounded-mcp (`{db, name}`, caller must know a raw magellan db
path), atheneum-mcp (`{query, k, project}`), envoy-mcp (`{q, k}`). An agent
has to remember which shape goes with which server, and none of them share a
result envelope, pagination, or error format.

Original direction considered a fourth, new thin proxy MCP server. Rejected
("too many mcps") in favor of extending atheneum-mcp in place — it already
has the cleanest calling convention (no raw db paths, resolves `project` by
name) and already has working, tested, CLI-exposed cross-project federation
(`crates/atheneum/src/cross.rs`: lazy `ATTACH DATABASE` against magellan's
`~/.magellan/meta.db` project registry, `cross_search` + `cross_navigate`).

## Non-goals

- **Not control-flow orchestration.** This is a knowledge/code-graph query
  API, not a multi-agent workflow graph (Claude Code's own `agent()` /
  `parallel()` / `pipeline()` primitives, or LangGraph-style node/edge
  routing). Different problem, don't conflate the two meanings of "graph."
- **Not graphify.** Graphify's community/god-node structural report over
  code+docs+images overlaps with the atheneum `WikiPage`/Logseq-sync design
  already in flight — adding it as a third backend here would duplicate that
  effort mid-flight. Left out of scope.
- **Not a training/embedding architecture change.** No new vector index, no
  new ML model. Existing HNSW/FTS5 in atheneum and existing magellan/llmgrep
  indexes are reused as-is.

## Architecture

`atheneum-mcp` (`crates/atheneum-mcp/`) grows a dispatch layer. No new
process. `grounded-mcp` and `envoy-mcp` stay installed (other tools/sessions
may reference them directly) but drop out of the active MCP config —
atheneum-mcp becomes the single agent-facing surface for this workflow.

Exposed verb set stays small and fixed — `search`, `navigate`, `query`,
`update`, `insert`, `delete`, `event`, plus one cheap orient call — rather
than 1:1-wrapping every magellan/llmgrep/mirage CLI subcommand into its own
tool. Branching happens inside the dispatch layer via a `kind` parameter
(`code` | `knowledge` | `event` | `all`), not via tool proliferation. This
mirrors a documented failure mode in comparable tools (code-review-graph
exposes 30 granular tools and pays real schema-token cost for it) and the
user's own explicit pushback on server sprawl, applied one level down to
tool sprawl within a single server.

CRUD split, established prior to this design and unchanged:
- `search` / `query` / `navigate` — all five tools (read).
- `update` — atheneum (mutate) + magellan `refresh` (propagates to
  llmgrep/mirage automatically since they read magellan's DB — no separate
  refresh call needed).
- `insert` / `delete` — atheneum only. magellan/llmgrep/mirage are
  read-only, derived-from-source-code tools.
- `event` — envoy's own verbs (send/claim/heartbeat/create_dependency) pass
  through under the same envelope, not forced into CRUD shape.

## Components

- **Dispatch/resolve layer** (new). Resolves `project` name via magellan's
  existing `meta.db` registry. Always attaches `.atheneum/atheneum.db`
  alongside whatever project matched — this is a required fix, not
  optional: today `cross_navigate` iterates the per-project registry, and
  since "atheneum" would otherwise need to be registered as an ordinary
  project entry (pointing at the small post-migration source-index file) to
  be reachable at all, the central knowledge store would silently not be
  included in results. Fix: one extra always-attached schema in `cross.rs`'s
  loop, not a registry entry.
- **Code-tool adapter** (new, thin). Shells out to magellan/llmgrep/mirage
  CLI binaries with the resolved db path — same subprocess pattern
  grounded-mcp already uses. Results normalized into the shared envelope.
- **Knowledge adapter** (new, thin). Calls atheneum's own store in-process —
  atheneum-mcp already links the atheneum crate, no subprocess needed here.
- **Event adapter** (new, thin passthrough). HTTP calls to envoy's bridge on
  `:9876`, same pattern envoy-mcp already uses. Response wrapped in the same
  envelope; event verbs themselves stay envoy's own shape.
- **Envelope assembly** (new). Uniform response shape regardless of which
  adapter answered (see Data Flow).

## Data Flow

Worked example: `atheneum.query(project="llama-rs", kind="code",
q="expand_kv_to_query_heads", limit=20)`.

1. **Resolve.** `project="llama-rs"` → llama-rs's magellan db path via
   `meta.db`. `.atheneum/atheneum.db` always attached alongside.
2. **Branch by `kind`.**
   - `code` → subprocess to magellan/llmgrep/mirage CLI.
   - `knowledge` → in-process atheneum store call.
   - `event` → HTTP to envoy `:9876`, passthrough verb shape inside the
     shared envelope.
   - `all` (default for `search`/`navigate`) → fan out code db + central
     atheneum db in parallel, merge, each item tagged `source:
     "code"|"knowledge"`.
3. **Assemble envelope**, uniform across all backends:
   ```
   {
     items: [ { ..., provenance: "EXTRACTED"|"INFERRED"|"AMBIGUOUS",
                source: "code"|"knowledge" } ],
     limit, cursor, has_more,
     code_stale: bool, knowledge_stale: bool,
     depth_clamped: bool,
     errors: [ { backend, code, message } ]
   }
   ```
   - **Pagination**: small default `limit` (20). Cursor encodes enough
     state (backend + offset) that a follow-up call with the same cursor
     resumes transparently, regardless of which backend originally
     answered. Prevents both silent truncation and unbounded-payload
     blowup (observed failure mode in comparable tools: unfiltered
     blast-radius queries hitting 500+KB responses).
   - **Provenance tri-tag** (adapted from graphify's EXTRACTED/
     INFERRED/AMBIGUOUS convention): code-backend results = `EXTRACTED`
     (deterministic AST/call-graph). atheneum semantic/HNSW results =
     `INFERRED`. `cross_navigate` hops beyond the first = `AMBIGUOUS`
     (compounding uncertainty — see depth cap below). Replaces a flat
     "likely affected"-style caveat string with a per-item honesty signal.
   - **Two-tier staleness**: `code_stale` (true when `magellan status`
     reports dirty/untracked files pending `refresh` for the resolved
     project db) and `knowledge_stale` (true when atheneum's latest write
     timestamp for the queried scope is newer than its latest embedding
     pass timestamp) are separate fields, not one flag — the two backends
     go stale on different timescales via different mechanisms, and cheap
     (`refresh`) vs costly (re-embed) fixes shouldn't be conflated.
4. **Depth cap.** `navigate`/`cross_navigate` default depth 2, hard max 3
   server-side regardless of requested depth. Grounded in two independent
   lines of evidence: multi-hop knowledge-graph traversal reliability
   compounds (~85%/hop → ~44% at 5 hops), and GNN message-passing depth
   hits over-smoothing/over-squashing past a few hops for unrelated
   structural reasons. Two different mechanisms, same practical ceiling.
   Requesting more than 3 doesn't error — it clamps to 3, and
   `depth_clamped: true` says so.
5. **Orient-first call.** One cheap entry-point call (analogous to
   comparable tools' ~100-token "minimal context" call) resolves
   project/kind and suggests which verb to call next, so the caller isn't
   guessing which underlying backend to target for a given question.

## Error Handling

Every failure surfaces inside the envelope's `errors` array — never a raw
subprocess stderr or HTTP error body thrown at the caller.

- **Fan-out partial failure** (`kind=all`, one backend dies): return
  whatever succeeded, tag the dead backend in `errors`. Never fail the
  whole call for one backend's outage.
- **Project not found**: distinct `PROJECT_NOT_FOUND` code — never
  indistinguishable from "resolved, zero matches."
- **Backend unavailable** (envoy HTTP down, magellan binary missing/crashes,
  atheneum db locked): `BACKEND_UNAVAILABLE` in that backend's `errors`
  slot; rest of a fan-out proceeds independently.
- **Timeout**: fixed per-backend budget — code-tool subprocess ~10s,
  atheneum in-process ~2s, envoy HTTP ~5s. Timeout produces an `errors`
  entry, never a hang.
- **Out-of-range params clamp, never reject**: `depth`/`limit` beyond the
  server cap are clamped, not errored; the envelope states the clamp
  (`depth_clamped`, actual `limit` used) rather than silently substituting.
- **Malformed backend output** (CLI schema drift on a magellan/llmgrep
  upgrade): `PARSE_ERROR`, raw output logged server-side, never forwarded
  raw to the caller.

Rule tying it together: a caller never sees an exception — only a populated
`errors` array beside whatever `items` did come back. Partial answer beats
total failure.

## Testing

Scope is the new dispatch/envelope code only — magellan/llmgrep/mirage/
atheneum/envoy each already have their own test suites; not re-testing
those.

- **Resolve + central-store reachability** (highest-risk new logic).
  Fixture: two small project dbs + a fixture `.atheneum/atheneum.db`.
  Assert known project resolves to the right code-db path, unknown project
  returns `PROJECT_NOT_FOUND`, and the atheneum db is attached on every
  resolve regardless of which project matched.
- **Pagination round-trip**: cursor encode→decode→resume lands at the
  right offset; `has_more` correct at the exact-`limit` boundary.
- **Fan-out partial failure**: mock one backend failing mid-`kind=all`
  call; assert the other backend's items still return and the failure
  lands in `errors`, no exception propagates.
- **Depth clamp**: request `depth=10`, assert depth-3 results and
  `depth_clamped: true`.
- **Provenance tagging**: code-backend hit → `EXTRACTED`; atheneum
  semantic hit → `INFERRED`; `cross_navigate` hop ≥2 → `AMBIGUOUS`.
- **Timeout**: fake slow backend (test double, not a real subprocess)
  surfaces as an `errors` entry within the fixed timeout window.
- **One real end-to-end test**: tiny fixture project, full
  resolve→dispatch→envelope path with one real magellan subprocess call
  and one real atheneum in-process call — proves the wiring beyond mocks.

## Related / Deferred

- Antigravity Logseq-Atheneum plugin's raw-SQL-bypass and missing-embedding
  gap — separate, unfixed, not in scope here.
- Splitting `.magellan/atheneum/atheneum.db` back to pure source-index-only
  — blocked on understanding `csr_shards`, not in scope here.
- Orca (parallel-worktree multi-agent dispatch) — complementary layer
  (dispatch, not coordination/knowledge), noted for later, not part of this
  design.
