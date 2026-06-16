# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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

- **SQLite FTS5 version mismatch** — Migration v9 now drops and recreates the `wiki_pages_fts` virtual table during open so the index format always matches the SQLite version that is actually opening the connection. This fixes `database disk image is malformed` errors when a DB was touched by a newer system `sqlite3` than the SQLite bundled with the atheneum binary.
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
