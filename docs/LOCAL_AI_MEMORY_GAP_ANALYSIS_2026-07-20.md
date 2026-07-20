# Atheneum vs. Librarian-Pattern Memory — Gap Analysis

Source: transcript at `docs/transcript-IwN-eK1s8og.clean.txt`
YouTuber: Anirban (Kodikes). Architecture: markdown-on-disk + a librarian
sub-agent that traverses links, exposed to clients via 3 MCP tools.
Reference specs: Karpathy's LLM Wiki + Google's OKF.

Method: every "has" claim below is backed by a citation to atheneum source.
"Missing" = no code path found via grep/read; treat as hypothesis until
implementation verifies.

---

## 1. Feature-by-feature audit

| # | Transcript feature | Atheneum status | Evidence |
|---|---|---|---|
| 1 | Library + librarian agent that takes a query, finds the answer, organizes new knowledge | PARTIAL. Library + deterministic retrieval exists. No autonomous "librarian agent" loop — atheneum is a library/tool the *caller* drives. | `crates/atheneum/src/graph/mod.rs:67` `AtheneumGraph`; CLI `navigate` at `crates/atheneum/src/main.rs:374`; MCP `navigate` at `crates/atheneum-mcp/src/tools.rs:350` |
| 2 | Linked-memory traversal ("read one file → another → another") | HAS (graph BFS, not file-links). `navigate` does lexical entry-point selection + multi-depth BFS subgraph. | `crates/atheneum/src/graph/navigation.rs`; MCP `navigate` paginated in `crates/atheneum-mcp/src/backend.rs` (`serialize_paginated_view`) |
| 3 | Three client tools: query / update / add | PARTIAL. Has `query_memory`, `store_memory`, `search_memory`, `list_memory`. **No `update_memory` tool** — clients cannot patch an existing memory in place. | MCP tools registered at `crates/atheneum-mcp/src/tools.rs:18-41`; params at `tools.rs:190` (`store_memory`), `tools.rs:239` (`query_memory`) |
| 4 | Memory stored as plain markdown the user can read | MISSING for episodic memory. Wiki/journal pages are markdown-backed; episodic Memory entities are JSON blobs in SQLite, not user-readable files. | `db/knowledge.rs` (wiki pages), `graph/memory.rs` (Memory entities as JSON) |
| 5 | Index at every level (`index.md`) | PARTIAL. FTS5 index on wiki pages; runtime in-memory entity-ID index. No per-directory concept index file emitted. | `db/wiki_fts.rs`; `graph/mod.rs:173` `build_entity_id_index` |
| 6 | Deterministic rules, LLM only for decisions | HAS for retrieval/insertion. `insert_edge` validates against ontology deterministically. | `graph/mod.rs:428` `insert_edge` (ontology validation); `graph/ontology.rs:126` `seed_standard_ontology` |
| 7 | Seed memory pushed to client at session start (via MCP `instructions` + tool description) | MISSING. No seed-memory generator; MCP tool descriptions are static strings. | `tools.rs` — all `Tool::new(...)` use literal description strings; no `seed_memory`/`session_summary` tool |
| 8 | Seed lists concepts, not file names | N/A (seed memory missing) | — |
| 9 | Enrich-before-create (patch existing concept, don't spawn new file) | MISSING. `store_memory` always inserts; no "find existing concept first" rule. | `graph/memory.rs`; `tools.rs:190` |
| 10 | Link both ways when creating new concepts | MISSING. `insert_edge` creates one directed edge; no automatic reciprocal edge. | `graph/mod.rs:428` |
| 11 | Graph-health lint: flag orphans | HAS (detection only, wiki pages). `wiki_dream_pass` flags pages with no incoming wikilinks. | `graph/dream.rs:41` `DreamPhase::Orphan`; test at `dream.rs:775` |
| 12 | Lint flags broken links | MISSING. No `broken_link`/dead-wikilink detector found. | grep `broken_link\|fix_orphan\|rewire` → 0 hits |
| 13 | Exclude auto-generated index files as link sources (so orphan detection is meaningful) | UNKNOWN — needs verification on `wiki_dream_pass` source filter. | `graph/dream.rs` (orphan detection at L540+) |
| 14 | `maintain` tool that rewires orphans back into related concepts (bidirectional) | MISSING. Dream only *flags* orphans; no auto-rewire. | grep `maintain\|rewire\|wire_orphan` → 0 hits |
| 15 | Contradiction handling: old fact must be gone everywhere (rewrite whole file) | PARTIAL. `dream_pass` *detects* contradictions (same key, diff scope, diff content) but does not auto-resolve or rewrite. | `graph/dream.rs:286` Phase 4 CONTRADICTION; test `dream_contradiction_detection` at `dream.rs:915` |
| 16 | Dream / idle-time consolidation | HAS (manual). `dream`/`wiki-dream` CLI + MCP `dream` tool. Dedup, stale, verbose, orphan, contradiction phases. Modes: DryRun + AutoMerge. **Not scheduled.** | `graph/dream.rs:22` `DreamMode`; CLI `dream` at `main.rs:1353`, `wiki-dream` at `main.rs:1376`; MCP `dream` tool registered `tools.rs:40` |
| 17 | Hot-tier cache of recent memories | MISSING. No recency-weighted cache; `GraphRuntime` caches entity IDs only. | `graph/cache.rs` (`GraphRuntime` / `RuntimeStats`) |
| 18 | Runs fully local, no cloud API | HAS | `Cargo.toml` deps (rusqlite, sqlitegraph, seahash); no network dep in default features |
| 19 | Web UI to chat with librarian and watch memory grow | MISSING (an `axum` web-ui feature exists but is not this). | `Cargo.toml:54` `web-ui` feature; no chat/librarian UI found |
| 20 | Graph visualizer of memories + links + query traces | MISSING | — |
| 21 | Records query traces for debugging | PARTIAL. Events/tool-call records are stored as graph entities when callers log them, but `navigate` does not emit a per-query trace record. | `graph/evidence/events.rs`; `EntityType::ToolCall`/`ReasoningLog` |
| 22 | Shared memory across all AI agents via MCP | HAS | MCP server `crates/atheneum-mcp`; also exposed via envoy HTTP per `crates/atheneum/AGENTS.md` |

---

## 2. Summary

### Atheneum's strengths vs. the transcript design
- **Persistence + scale**: SQLite/sqlitegraph + FTS5, not markdown files. Survives
  concurrent writers, queryable at GB scale, schema-migrated.
- **Multi-agent from day one**: agents, sessions, handoffs, decisions,
  cross-project routing — the transcript has none of this.
- **Decision provenance**: `caused_by`/`led_to` chains (`thread`), token-bounded
  digests, bootstrap packets. Far richer accountability than a wiki.
- **Dream pipeline**: dedup/stale/verbose/orphan/contradiction detection
  already ships (the transcript only describes it as a roadmap item).

### Confirmed gaps (ranked by leverage)
1. **No `update_memory` MCP tool** — clients can't enrich/patch in place. Forces
   the "junk drawer" anti-pattern the transcript explicitly calls out (#9, #3).
2. **No enrich-before-create rule** — `store_memory` always inserts (#9).
3. **No bidirectional auto-linking** — every new concept is an island unless the
   caller manually wires both edges (#10).
4. **Orphans/broken links are flagged but never auto-rewired** — dream stops at
   detection; no `maintain`/rewire pass (#11, #12, #14).
5. **Contradictions detected, not resolved** — no rule that the old fact must be
   purged (#15).
6. **No seed-memory / bootstrap-to-client tool** — every MCP client is "flying
   blind" exactly as the transcript describes (#7, #8).
7. **No hot-tier recency cache** — `navigate`/`query_memory` pay full cost for
   5-minute-old context (#17).
8. **Episodic memory not human-readable** — JSON-in-SQLite, not markdown (#4).
9. **No per-query trace records** — can't debug "how did the agent find this"
   (#21).
10. **No scheduled/idle dream** — must be invoked manually (#16).

### Non-gaps (explicitly out of scope for this plan)
- Web UI + graph visualizer (#19, #20): valuable but a separate workstream;
  the existing `web-ui` feature can host it later.
- Markdown-on-disk for episodic memory (#4): atheneum's SQLite model is a
  deliberate architectural choice; the leverage is in exposing memory as
  readable *views*, not migrating storage.

---

## 3. Source citations index

- `crates/atheneum/src/graph/mod.rs:67` — `AtheneumGraph` struct
- `crates/atheneum/src/graph/mod.rs:173` — `build_entity_id_index`
- `crates/atheneum/src/graph/mod.rs:428` — `insert_edge` (ontology validation)
- `crates/atheneum/src/graph/dream.rs:22` — `DreamMode` (DryRun / AutoMerge)
- `crates/atheneum/src/graph/dream.rs:41` — `DreamPhase` (incl. `Orphan`)
- `crates/atheneum/src/graph/dream.rs:286` — contradiction detection phase
- `crates/atheneum/src/graph/dream.rs:775` — orphan test
- `crates/atheneum/src/graph/dream.rs:915` — contradiction test
- `crates/atheneum/src/graph/ontology.rs:126` — `seed_standard_ontology`
- `crates/atheneum/src/main.rs:1353` — CLI `dream`
- `crates/atheneum/src/main.rs:1376` — CLI `wiki-dream`
- `crates/atheneum-mcp/src/tools.rs:18-41` — MCP tool registration list
- `crates/atheneum-mcp/src/tools.rs:190` — `store_memory` (no update path)
- `crates/atheneum-mcp/src/tools.rs:350` — `navigate`
- `crates/atheneum-mcp/src/backend.rs` — `serialize_paginated_view` (navigate shaping)
