# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **`extract-decisions` native subcommand** — Rust port of the
  `~/.local/bin/extract-decisions` operator script, behind the `extract`
  Cargo feature (default off). `atheneum extract-decisions <db> [--all |
  <session-id>] [--dry-run] [--force] [--verbose] [--project P] [--agent A]
  [--model M] [--transcripts-dir D] [--max-chars N] [--ollama-url U]` calls a
  local Ollama LLM (default `qwen3.5`) over transcript JSONL in-process
  (`ureq`), parses the strict decision JSON, applies the placeholder
  hallucination guard, recovers the `sequence` of each decision by matching
  its `chosen`/`rationale` back to the source turn, and stores `Decision`
  discoveries via `graph.store_discovery` directly (no temp file / shell-out)
  with `source = "llm-extract"`. Same prompt/schema and dedup semantics as
  the script; resumable via `--all` (skips sessions that already have an
  `llm-extract` Decision). The Python script remains the default fallback
  (no special build needed); enable the subcommand with `--features extract`
  (or `--all-features`).

## [0.9.0] — 2026-06-23

Implements the chat-decision plan (decision capture from Claude Code chat
transcripts): schema v12 adds generated columns + an FTS5 index over chat
content so navigation queries are cheap; the `chat` command navigates that
content; `extract-decisions` + the live `watch-decisions` watcher convert the
structured-choice signals in transcripts (`AskUserQuestion`, `ExitPlanMode`,
`TaskCreate`, `TodoWrite`) into real `Decision` discovery rows. Schema
migration v12 (additive — new generated columns + triggers + FTS) is a minor
bump; no insert-path changes and no breaking API removal.

### Added

- **Schema migration v12 `chat-columns-fts`** — four `VIRTUAL` generated
  columns over `graph_entities` (`session_id`, `sequence`, `role`,
  `content_text`) extracted from the chat-content JSON, two covering indexes
  (`idx_entities_session_seq`, `idx_entities_session_role_seq`), and an
  `entity_fts` FTS5 external-content table over those columns with four
  `AFTER INSERT/UPDATE/DELETE` sync triggers. `--search` no longer full-scans
  + `json_extract`s; it hits the generated columns + FTS5. Foundation for the
  `chat` command.
- **`chat` command — token-budgeted chat navigation.** `atheneum chat <db>
  <session_id> [--tokens T] [--only-decisions] [--json]` walks a session's
  records in `sequence` order, emitting `role` + a content snippet per record
  and bounding output to a token budget. `--only-decisions` narrows the walk
  to the `Decision` discovery rows attached to that session (from any source —
  transcript extract, watcher, or manual `store_discovery`), deduped by
  `session_id`+`sequence`+`target`+`source`.
- **`extract-decisions` operator script — backfill structured decisions from
  transcripts.** A standalone `~/.local/bin/extract-decisions` script (reusing
  the `dream` + `remember-to-atheneum` pattern) runs a local LLM (Ollama
  `qwen3.5` by default) over each Claude Code transcript, extracts
  decision-shaped turns, and stores each as a `Decision` discovery
  (`source = "llm-extract"`, with `chosen` / `alternatives` / `rationale` /
  `sequence`) via `atheneum store-discovery` — so each is linked into the
  session thread. Covers decisions that lack a Tier-1 structured signal.
  Resumable (`--all` skips sessions already having an `llm-extract`
  Decision), `--dry-run` for review, `--force` to re-extract. Hallucination
  guard rejects entries without a real alphabetic `target`/`chosen`/
  `rationale`. Not an `atheneum` subcommand; the Rust port is deferred per
  the plan.
- **`watch-decisions` command — live structured-decision capture.** `atheneum
  watch-decisions <db> [--once] [--interval S=2] [--config-dir D]...
  [--project P] [--agent A] [--dry-run]`. Tails the same transcript files in a
  loop, detecting the same Tier-1 signals and storing `Decision` rows in real
  time. In-memory per-file cursor (offset/inode/mtime) with partial-line
  tolerance (a half-written final line is re-read next scan, never fabricated
  into a decision). `--once` runs a single scan with a cold cursor — safe for
  cron; relies on `decision_exists` dedup as the cross-invocation safety net.
  Detect-only by design; the SessionStop `sync-claude-transcript` hook still
  owns full ingest at session end.
- **`decision_exists` graph method** — indexed dedup lookup on the
  `discoveries` table (`session_id` + `target` + `discovery_type='Decision'`
  + `source` + `sequence` via `json_extract`), so both the backfiller and the
  watcher skip already-captured decisions without a graph full-scan.
- **`recent_discoveries` `--session` + `--type` filters.**
  `recent_discoveries(project_id, agent, session_id, discovery_type, limit)`
  accepts a session id and a discovery-type filter (e.g. `"Decision"`), both
  applied as `json_extract` predicates over the discovery `data` JSON, and
  `atheneum discoveries-recent` exposes `--session <id>` and `--type <T>` so
  `chat --only-decisions` and the watcher's session scope are observable. The
  `discoveries` table also carries `session_id` + `discovery_type` columns
  with indexes (v11 / v3), used by `decision_exists` for indexed dedup.
- **`session-digest` surfaces a dedicated `decisions` block.** Each
  session's digest now lists its `Decision` discoveries (filtered to
  `discovery_type = 'Decision'`, limited to 5) labeled with the capture
  `source` (`askuser` / `exitplan` / `taskcreate` / `todowrite` /
  `llm-extract`, or `manual`), so decisions from every layer appear
  together. The existing `discoveries` block (recent discoveries of any
  type) is preserved — the decisions block is additive.
- **Phase 5 cooperative-skill capture — `plugin/atheneum-decisions/`.** A
  Claude Code companion plugin that records architectural choices as `Decision`
  rows as the model makes them (`source = "skill"`), the highest-fidelity layer
  on top of the transcript watcher and LLM backfiller. Contains:
  - `skills/record-decision` — auto-triggers on choosing between approaches /
    an architectural tradeoff; writes `metadata.json` and calls
    `atheneum store-discovery ... --session $CLAUDE_CODE_SESSION_ID --dedup`.
  - `commands/decision.md` — `/decision <target> <chosen> [rationale]` manual
    fallback (same store path + `--dedup`).
  - `hooks/decision-gate.fish` — non-blocking Stop hook that warns when a
    session made tool calls but recorded zero Decision rows.
- **`decision_exists_chosen` graph method + `store-discovery --dedup`.** The
  skill / manual layer has no stable transcript `sequence`, so it dedups on
  `(session_id, target, source, chosen)` — what "the same choice was already
  recorded" means for that layer — via the new
  `AtheneumGraph::decision_exists_chosen`. `atheneum store-discovery` gains
  `--dedup` (opt-in; skips a duplicate Decision insert and prints
  `deduped: true`) and `--force` (bypass). The watcher's sequence-keyed
  `decision_exists` is unchanged; cross-layer doubles remain an accepted
  tradeoff.

### Fixed

- **`recent_discoveries` accepts a `session_id` filter.** The query now
  narrows to discoveries attributed to a session (via `json_extract` on the
  `data` JSON), so `chat --only-decisions` and the watcher's session scope
  are observable from the CLI.

## [0.8.0] — 2026-06-22

Consolidates the session-digest plan Phases 1–3 (session-digest composer,
thread decision-chain navigation, SessionStart hook injection). 0.7.1 was
prepared but never published; its contents ship here as 0.8.0 because Phase 2
removed `semantic-search` from default features — a breaking change for
crates.io consumers who relied on HNSW-backed `search`/`navigate` by default.

### Added

- **`thread` command — decision-chain navigation**: `atheneum thread <db>
  <query> [--tokens T=1500] [--depth D=3] [--k N=3] [--project P] [--json]`.
  Lexical match on `ReasoningLog` + `Discovery` entry points, then BFS outward
  along `caused_by`/`led_to` chain edges only, bounded to a token budget. Phase
  2 of the session-digest plan. Plain-text renderer orders the chain by entity
  id (chronological) and shows a content snippet per decision.
- **`LedTo` edge type** — the forward thread edge (inverse of `CausedBy`),
  stored explicitly for cheap outward chain walks. Seeded in the standard
  ontology (domain/range `ANY`).
- **`store_discovery` thread auto-linking** — on store, a discovery with
  `session_id` is linked `observed_in → Session` plus `caused_by → prior` and
  `led_to` inverse to the most-recent earlier same-session decision (by entity
  id = insert/chronological order). No prior ⇒ thread root. Best-effort.

### Changed

- **`semantic-search` (HNSW) is now opt-in.** Removed from `default` features.
  `search`/`navigate`/`thread` fall back to a bag-of-tokens lexical scan plus
  BFS graph traversal — graph navigation suffices for the workflow, HNSW + an
  embedder are heavy and unnecessary by default. Enable with `--features
  semantic-search` for vector similarity. **Breaking:** consumers who relied on
  HNSW-backed similarity by default must now enable the feature explicitly.

### Fixed

- **ReasoningLog content is now searchable.** `embed_text_for_entity` now
  includes `content_summary` and `content`, so ReasoningLog entities (whose
  `name` is a `<session_id>:<sequence>` id with no content tokens) match by
  text in `search`/`thread`.
- **`hnsw_counters_track_hits_and_fallbacks` test** is now
  `#[cfg(feature = "semantic-search")]`-gated; it asserts HNSW-only counters.

### Integration (Phase 3 — envoy/hook plumbing)

- **`session-digest` now auto-fires at SessionStart.** The
  `session-bootstrap.fish` Claude Code hook injects
  `atheneum session-digest ~/.magellan/atheneum/atheneum.db --project <repo> --last N --tokens 500`
  into startup context after the `hist:` block, so every new session grounds on
  prior sessions' decisions, files, and open tasks without a manual CLI call.
  Reads the DB directly via the CLI (no envoy dependency — works when envoy is
  down). Subagents (`CLAUDE_PARENT_SESSION_ID` set) get `--last 1` for a lighter
  digest. Project resolves to the git toplevel basename (matches session
  tagging) with a cwd-basename fallback. Gracefully skipped when the atheneum
  binary or DB is absent.
- **Session project tagging fixed.** `session-stop-sync.fish` (the Stop hook
  that runs `atheneum sync-claude-transcript`) and `envoy`'s
  `cmd_session_start` now resolve the project to the git toplevel basename
  instead of `basename $PWD`, so worktree/subdir launches tag sessions with the
  repository name rather than `tmp` or a subdirectory. The `session-digest`
  `--project` filter now matches newly-recorded sessions.

### Added — Phase 1 (session-digest composer, observability CLI)

- **`session-digest` composer** — bounded, ranked plain-text bootstrap packet
  (`atheneum session-digest <db> [--project P] [--last N] [--tokens T] [--json]`)
  so a new session grounds on prior sessions' decisions, files, and open tasks.
  Computes activity from `event_log` (not the zero `sessions` ledger columns),
  falls back across all projects when `--project` is empty, emits thread anchors
  for `navigate` follow-up. `discoveries.session_id` migration (v11) attributes
  findings to sessions; `store-discovery --session <id>` writes it.
- **Operator-facing observability CLI** for sessions, tool usage, discoveries, handoffs, events, and recent sessions:
  - `session-trace <db> --session <id> [--limit N]`
  - `tool-usage <db> --session <id> [--limit N]`
  - `discoveries-recent <db> [--project P] [--agent A] [--limit N]`
  - `handoffs-recent <db> [--project P] [--agent A] [--limit N]`
  - `events-recent <db> [--session ID] [--type T] [--limit N]`
  - `sessions-recent <db> [--project P] [--agent A] [--limit N]`
- **Recent session and handoff query helpers** in the graph layer, so operator workflows no longer need ad hoc SQLite for these common read paths.

## [0.7.0] — 2026-06-19

### Added

- **Read-bridge to magellan's canonical project registry** (`meta.rs`): atheneum no longer maintains its own duplicate copy of the project list. `MetaRouter` now ATTACHes magellan's canonical `~/.magellan/meta.db` (daemon-maintained by `magellan.service`) as a read-only source of project names, roots, and database paths, then overlays its own enrichment data (language, atheneum-db path) on top. This is one source of truth with zero coupling: atheneum works standalone — if magellan is not installed, the bridge simply finds an empty registry and atheneum falls back to overlay-only operation. `meta-list` now reflects everything magellan knows about (e.g. all 25 indexed databases) without any manual registration.

### Changed

- **`project_registry` table renamed to `project_overlay`.** Atheneum's local table is now an *enrichment overlay* on top of magellan's canonical registry, not a competing copy of it. A one-time migration (`ensure_schema`) copies any enrichment rows from the legacy `project_registry` into `project_overlay` and then drops the old table, so no user data is lost.
- **`cross-search` / `cross-navigate` are resilient to partial registries.** Because the bridge can surface projects whose databases are missing or not yet fully indexed (missing `graph_entities` table), the per-project query loop now skips unattachable or schema-incompatible projects with a warning instead of aborting the whole search. Previously a single incompatible project would fail the entire cross-search.

## [0.6.2] — 2026-06-17

### Fixed

- **`atheneum reindex`** no longer crashes with "Execute returned results - did you mean to call query?". `Graph::checkpoint()` now uses `query_row` for `PRAGMA wal_checkpoint(TRUNCATE)`, because that PRAGMA returns a row.
- **`wiki_pages_fts` self-heals on open** when the FTS5 shadow tables are left corrupt by an external SQLite writer. The recovery purges `sqlite_master` directly (bypassing the broken vtable), recreates the table and triggers on fresh connections, then runs a full `delete-all` → repopulate → `rebuild` cycle. This makes `sync-wiki`, `search-wiki`, and `backfill-wiki` robust against "database disk image is malformed" / "vtable constructor failed" corruption.

## [Unreleased] — Pending

(nothing yet)

## [0.6.1] — 2026-06-16

### Added

- **Path-aware wiki search.** The `wiki_pages_fts` FTS5 index now includes the `path` column, so `search-wiki` matches path fragments (e.g. `session` matches `wiki/session-accountability.md`).
- **Prefix wildcard matching.** `search-wiki` automatically treats each query token as a prefix (`rout` matches `Routes`, `Router`, path fragments).
- **Graph-entity fallback search.** If FTS5 returns no hits, `search_wiki_pages` falls back to a substring search over graph entity names/paths and wiki titles, so partial concept queries still find stored pages.
- New tests cover path-fragment search, prefix wildcards, and the graph-name fallback.

### Changed

- `wiki_pages_fts` migration is now parameterised over columns. Migration v10 recreates the FTS5 table with `title, body, path` and updates triggers accordingly.

## [0.6.0] — 2026-06-16

### Added

- **FTS5 full-text search over wiki pages.** New `wiki_pages_fts` index backs `atheneum search-wiki` with paginated, ranked results and excerpts. The full article body is never returned by the search API.
- **`backfill-wiki` CLI command** repairs wiki pages that were written directly to the `wiki_pages` SQL table without a proper `WikiPage` graph entity, restoring wikilink navigation and resolving stub targets.
- **Library API additions:**
  - `AtheneumGraph::search_wiki_pages(query, project_id, offset, limit) -> Result<Vec<WikiSearchResult>>`
  - `AtheneumGraph::backfill_wiki_pages_to_graph(project_id) -> Result<Vec<(i64, String)>>`
  - Re-exported `WikiSearchResult` from the crate root.

### Fixed

- **SQLite FTS5 version mismatch** — Migration v9 drops and recreates the `wiki_pages_fts` virtual table during open so the index format matches the SQLite version opening the connection. This addressed the original "database disk image is malformed" error when the DB was touched by a newer system `sqlite3`. The root cause was later generalized and hardened by v0.6.2's per-open `ensure_wiki_fts_healthy` self-heal.
- **Unicode-safe excerpt slicing** in `search_wiki_pages` no longer panics on multi-byte characters such as `→`.

## [0.5.0] — 2026-06-09

#### Cross-Project Queries — Query Magellan-Indexed Codebases Without Copying Data

Atheneum can now search and navigate across multiple magellan-indexed projects in a single command. Instead of importing magellan data (which goes stale immediately), atheneum maintains a lightweight routing registry (`meta.db`) and lazily `ATTACH DATABASE` each project's magellan DB on demand.

**New CLI commands:**

```bash
# Register a project once
atheneum meta-register envoy /home/feanor/Projects/envoy \
  /home/feanor/Projects/envoy/.magellan/magellan.db --language rust

# Search for a symbol across all Rust projects
atheneum cross-search "build_router" --language rust --k 10

# Search + BFS subgraph walk per project
atheneum cross-navigate "error handling" --language rust --k 5 --depth 2
```

**New library API:**

```rust
use atheneum::CrossRouter;

let mut router = CrossRouter::open()?; // opens meta.db, LRU cache of 8
let hits = router.cross_search("build_router", Some("rust"), 10)?;
let views = router.cross_navigate("error handling", Some("rust"), 5, 2)?;
```

**How it works:**
1. `meta.db` (`~/.local/share/atheneum/meta.db`) stores one row per project: name, root path, magellan DB path, language.
2. `CrossRouter` looks up candidate projects, `ATTACH`es each magellan DB (read-only), and queries `graph_entities`/`graph_edges` across all attached schemas.
3. An LRU cache (default capacity 8) keeps hot DBs attached across queries. Missing or unreadable DBs are skipped with a warning — one broken project does not break the query.
4. Language filtering (`--language rust`) limits the search to projects tagged with that language at registration time.

**Limit:** SQLite defaults to 10 max attached databases. The LRU cache defaults to 8 to stay safely under that limit. Increase with `CrossRouter::with_capacity(10)` if needed.

See the full guide: `docs/cross-project-routing-plan.md`.

#### Per-Tool Configuration (`config.toml`)

Atheneum now reads `~/.config/atheneum/config.toml` (XDG Base Directory compliant). A missing file is not an error — sensible defaults are used. Invalid files fail fast with a clear parse error.

**What you can configure:**

```toml
[atheneum]
db = "~/.local/share/atheneum/atheneum.db"      # your main graph DB
meta_db = "~/.local/share/atheneum/meta.db"      # cross-project routing registry

[llm]
provider = "ollama"
base_url = "http://localhost:11434"
model = "codellama"

[embeddings]
provider = "hash"        # "hash" | "ollama" | "openai"
dimension = 128

[integrations]
# Cross-tool integration is opt-in. These document intent for future auto-discovery.
[integrations.magellan]
enabled = false
config = "~/.config/magellan/config.toml"

[integrations.envoy]
enabled = false
url = "http://localhost:9876"
```

**New CLI:**

```bash
atheneum config init          # write defaults to ~/.config/atheneum/config.toml
atheneum config init --force  # overwrite existing
atheneum config show          # print effective config as JSON
```

**Library API:**

```rust
use atheneum::{Config, load_config, save_config, default_config_path};

let cfg = load_config()?;              // missing file → defaults
let path = cfg.db_path();              // ~ expanded to $HOME
let meta = cfg.meta_db_path();
save_config(&Config::default())?;      // write back to disk
```

**Key design principle:** Every tool works standalone by default. `[integrations]` sections are opt-in. There are no hidden dependencies.

#### Concise Mode for `navigate`

The `navigate` CLI now has a `--concise` flag that emits compact Markdown instead of JSON. This is designed for direct paste into a language-model context window.

```bash
atheneum navigate ./atheneum.db "session accountability" --concise --max-tokens 500
```

Output example:

```markdown
# navigate: session accountability

## Memory `session_cache` (128)
 — `src/graph/memory.rs`

**outgoing**
- related_to:
  → `GraphRuntime` (7)
  → `cache_hit` (129)

**incoming**
- performed_by:
  ← `claude-main` (42)

_2 additional subgraphs omitted._
```

`--max-tokens` truncates the output to an approximate token budget (~4 chars/token). `--concise` respects it.

#### Performance Improvements

- **SQLite PRAGMA tuning on open.** `AtheneumGraph::open()` now applies production-hardened settings automatically:
  - `PRAGMA journal_mode = WAL` — concurrent readers + writers
  - `PRAGMA synchronous = NORMAL` — durability/speed balance
  - `PRAGMA cache_size = -64000` — 64 MB page cache
  - `PRAGMA temp_store = MEMORY` — temp tables in RAM
- **`AtheneumGraph::checkpoint()`** — public API for forced WAL checkpoint. Called by `reindex` after rebuilding the HNSW index to reclaim disk space.
- **Prepared statement caching** — Hot paths (memory CRUD, concept upsert) now use `conn.prepare_cached()` instead of `conn.prepare()`. Rusqlite's per-connection LRU cache (default 16 entries) eliminates recompilation overhead for repeated queries.
- **In-memory entity ID lookup index** — `GraphRuntime` maintains a `HashMap<(kind, name), id>` for O(1) entity-by-name lookups. Rebuilt on open and after migrations. Falls back to SQL on cache miss.
- **Batch write API** — Single-transaction bulk inserts:
  - `AtheneumGraph::batch_insert_entities()` — updates the in-memory index automatically
  - `AtheneumGraph::batch_insert_edges()` — no ontology validation; caller must ensure domain/range constraints
  - `consolidate_discoveries` now uses batch insert for `DerivedFrom` edges

### Fixed

- **MetaRouter path now respects config.** `MetaRouter::open()` reads `atheneum.meta_db` from `~/.config/atheneum/config.toml` when present, falling back to the XDG default (`~/.local/share/atheneum/meta.db`) if the config is missing or invalid.

## [0.4.0] — 2026-06-09

### Added

- **Token budgets on retrieval APIs.** All major query paths now accept an optional `max_tokens` parameter to prevent context bloat when feeding results to language models:
  - `lexical_search(..., max_tokens)` — truncates result list greedily
  - `navigate(..., max_tokens)` — passes each subgraph through `truncate_subgraph`
  - `query_knowledge(..., max_tokens)` and `query_knowledge_in_project(..., max_tokens)` — post-hoc truncation with `"truncated": true` flag
  - CLI `--max-tokens N` added to `search`, `navigate`, and `query-knowledge`

- `store_memory()` and `preview_memory()` now accept optional `tags: Option<&[String]>`.

### Fixed

- **`task-archive` CHECK constraint bug.** The migration v2 `tasks` table omitted `'ARCHIVED'` from its CHECK constraint. Migration v8 recreates the tables for existing databases, preserving all data. Regression test: `test_archive_task_status_transition`.

### Changed

- **Breaking API:** `store_memory` expanded from 5 to 6 parameters (added `tags`). `preview_memory` expanded from 7 to 8 parameters.

## [0.3.2] — 2026-06-09

### Added

- Paged query APIs for large read surfaces (original `Vec`-returning methods remain as backward-compatible caching wrappers):
  - `query_events_page()`, `query_sessions_page()`, `list_memory_page()`, `list_wiki_pages_page()`
  - CLI flags `--offset N` and `--limit N` for `query-sessions`, `query-events`, `memory-list`, `list-pages`

### Changed

- Paged variants use SQL `LIMIT ? OFFSET ?` directly and are intentionally uncached. Original methods remain cached with `offset=0`.
- `memory-list` and `list-pages` CLI default to `--limit 1000` (was unbounded).

## [0.3.1] — 2026-06-07

### Added

- `runtime_stats()` — exposes process-local cache/query/write counters.
- Preview APIs (no-mutation candidate lookup before commit):
  - `preview_entity_candidates()`, `preview_discovery()`, `preview_memory()`, `preview_handoff()`
  - `preview_navigate_query()` — staged validation/repair with intent classification and kind alias resolution
- `QueryIntent`, `ResolvedEntity`, `DisambiguationResult`
- `ProvenanceData` — typed struct for edge provenance metadata (replaces 20 ad-hoc JSON sites)
- `content_hash_excluding()` — deterministic SHA-256 hashing that strips volatile fields
- `get_similar()`, `resolve()`, `preview_entity_candidates()`

### Changed

- Repeated read APIs now use a concurrent in-process query cache with generation-based invalidation.
- Cached reads: `query_memory`, `list_memory`, `query_sessions`, `query_events`, `query_knowledge`, `list_wiki_pages`, `lexical_search`, `navigate`, `hopgraph_query`.
- Relevant writes invalidate the cache domain automatically after mutation.

### Fixed

- Navigation kind filters no longer fail silently on lowercase or plural inputs (`memory`, `memories`, `wiki`, `discoveries`).
- `insert_edge()` now validates domain/range constraints against the ontology before insertion.
- Search degrades cleanly when the persisted HNSW index is inconsistent: tries persistent path, attempts one rebuild, falls back to direct lexical scanning.
- Memory upsert recreates missing SQL rows for existing `Memory` entities.

## [0.3.0] — 2026-06-05

### Added

- **Dreaming module** — reflective memory consolidation pass.
  - 6-phase pipeline: SCAN → DEDUPLICATE → STALE → CONTRADICTION → VERBOSE → CONSOLIDATED
  - Trigram Jaccard similarity for near-duplicate detection
  - `DreamMode::DryRun` and `DreamMode::AutoMerge`
- **Wiki dream pass** — same pipeline applied to wiki page entities
- `EdgeType::SupersededBy` and `EdgeType::ConsolidatedFrom`

## [0.2.3] — 2026-06-04

### Added

- **Memory domain** — stable-fact storage distinct from Knowledge and WikiPage.
  - `EntityType::Memory`, `memory_entries` SQL table
  - `store_memory()`, `query_memory()`, `list_memory()`
  - CLI: `memory-store`, `memory-get`, `memory-list`

### Fixed

- SQLite `busy_timeout` set to 5000ms on connection open.
- Memory upsert now correctly updates by composite key `(key, scope, project_id)`.

## [0.2.2] — 2026-06-04

### Changed

- HNSW search index now uses persistent storage (`hnsw_index_persistent`). Vectors survive process restarts.

## [0.2.0] — 2026-06-03

### Added

- **HopGraph** — vector entry + BFS graph traversal retrieval model.
  - `hopgraph_query()` — search → filtered BFS → token-budgeted truncation
  - `get_subgraph_filtered()` — BFS with edge-type whitelist
  - `TextEmbedder` trait with `HashEmbedder` (128-dim, zero deps) and `OllamaEmbedder` (768-dim, feature-gated)
  - `consolidate_discoveries()` and `consolidation_pass()` — merge duplicate discoveries into Knowledge entities
- `EdgeType::Explains`, `EdgeType::DerivedFrom`
- `link_wiki_to_symbols()` — bridge wiki content to code symbols via magellan

### Changed

- `build_search_index()` now indexes ALL entity kinds (was hardcoded to 3).

## [0.1.0] — 2026-05-31

### Added

- Initial release with sqlitegraph-backed agent coordination graph.
- SQL payload layer: agents, tasks, events, reasoning logs, tool calls.
- Planning domain: tasks, requirements, blockers, kanban board.
- Knowledge domain: wiki pages, journal sections, discoveries.
- Evidence domain: audit trail, handoff records.
- Wiki sync, journal sync, Logseq sync, Claude transcript import.
- Graph navigation: causal chains, task assignment, event provenance.
