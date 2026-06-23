# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **`graph::extract_decisions` — native LLM decision backfill** (feature
  `extract`, default off): Rust port of the `~/.local/bin/extract-decisions`
  operator script. `run_extract(&ExtractConfig) -> ExtractStats` calls a local
  Ollama LLM (default `qwen3.5`) over transcript JSONL in-process via `ureq`,
  parses the strict decision JSON, applies the placeholder hallucination
  guard, recovers each decision's `sequence` by matching `chosen`/`rationale`
  back to the source turn, and stores `Decision` discoveries through
  `graph.store_discovery` (no temp file / shell-out) with
  `source = "llm-extract"`. Same prompt/schema and dedup semantics as the
  script; resumable via `--all` (skips sessions that already have an
  `llm-extract` Decision).
- **`extract-decisions` CLI subcommand** (feature `extract`): exposes
  `run_extract` as `atheneum extract-decisions <db> [--all | <session-id>]
  [--dry-run] [--force] [--verbose] [--project P] [--agent A] [--model M]
  [--transcripts-dir D] [--max-chars N] [--ollama-url U] [--heuristic |
  --mode llm|heuristic]`. The Python script remains the default fallback;
  enable the subcommand with `--features extract` (or `--all-features`).
- **`graph::extract_decisions::ExtractMode` + heuristic backend** (feature
  `extract`): `ExtractMode::{Llm, Heuristic}` picks the extraction backend per
  run. `Heuristic` is a rule-based extractor (no LLM, no network) that catches
  decision-shaped sentences with an explicit rationale clause, reuses the
  hallucination guard + store/dedup plumbing, and writes `source = "heuristic"`
  (distinct from `llm-extract`, so the two backends are separately resumable).
  Selected via `--heuristic` / `--mode heuristic` / `ATHENEUM_EXTRACT_MODE=heuristic`.
  Lower recall + some false positives vs the LLM; zero deps. Default stays `Llm`.

### Added

- **`sessions-recent --exclude-project <P>` (repeatable).** Hides named
  project buckets from the recent-sessions view without re-attributing the
  rows. Targets the `tmp` / `Projects` buckets that arise honestly when a
  session runs from `/tmp` or a non-repo parent dir (no git worktree, so the
  shared git-toplevel-basename fallback yields the dir basename). The `LIMIT`
  is applied after exclusion so the returned set is never under-filled.
  `query_sessions_recent` gained an `exclude_projects: &[String]` parameter
  (SQL `AND s.project NOT IN (...)`, built with `params_from_iter`).

### Changed

- **`chat --only-decisions` renderer enriched.** Each decision now prints
  `source` and `sequence` inline, plus the `chosen`, `rationale`,
  `alternatives`, and `why` metadata fields as indented sub-lines (truncated
  to readable snippet widths), and a `--walk` chain snippet when `caused_by` /
  `led_to` edges exist on the decision. Previously the renderer emitted only
  `id` / `target` / `created_at` / `why`, so `--only-decisions` read as a bare
  index rather than a rationale-bearing view.
- **`thread` human renderer polished.** The plain-text `thread` output now
  leads with an entry-count / depth / token-budget header, renders each
  entry's decision metadata (`source` / `sequence` / `chosen` / `rationale`
  / `alternatives`) inline when the entry is a `Decision` (same style as
  `chat --only-decisions`), lists the chain edges literally as
  `from ──caused_by/led_to──> to` with named endpoints instead of a bare
  edge count, and drops the redundant snippet when it merely repeats an
  entity's name. `--json` is unchanged. Previously the renderer printed a
  flat id-ordered list with a `_N chain edge(s)_` footer, so the chain
  structure and rationale were invisible.

### Fixed

- **CLI rejects flag-looking positionals.** Subcommand arms historically read
  raw positionals (`PathBuf::from(&args[2])`, `&args[3]`, …) without checking
  whether the value started with `-`, so a bare flag in a positional slot was
  silently accepted as the value — `atheneum init --help` created a real SQLite
  file named `--help` in the cwd. A central `positional` / `optional_positional`
  guard now fails fast with `expected positional <name>, got flag-looking
  argument '<x>'` across every subcommand. Required slots error on a missing or
  flag-looking value; optional slots not followed by the option parser
  (sync-* project-id, sync-claude-transcript project/agent) error on a
  flag-looking value; optional slots followed by the option parser keep the
  existing flag-means-absent behavior. The option-parsing path is unchanged.

## [0.9.0] — 2026-06-23

Implements the chat-decision plan (decision capture from Claude Code chat
transcripts). Schema v12 is additive (new generated columns + triggers + an
FTS5 table), so no insert-path changes and no breaking API removal — a minor
bump.

### Added

- **Schema migration v12 `chat-columns-fts`** (`graph::db::chat`): four
  `VIRTUAL` generated columns over `graph_entities` (`session_id`,
  `sequence`, `role`, `content_text`) extracted from the chat-content JSON,
  two covering indexes (`idx_entities_session_seq`,
  `idx_entities_session_role_seq`), and an `entity_fts` FTS5 external-content
  table over those columns with four `AFTER INSERT/UPDATE/DELETE` sync
  triggers. Registered in `graph::db::MIGRATIONS`.
- **`chat` command — token-budgeted chat navigation.** `atheneum chat <db>
  <session_id> [--tokens T] [--only-decisions] [--json]` walks a session's
  records in `sequence` order, emitting `role` + a content snippet per record
  and bounding output to a token budget. `--only-decisions` narrows the walk
  to the session's `Decision` discovery rows (any source), deduped by
  `session_id`+`sequence`+`target`+`source`.
- **`extract-decisions` operator script — backfill structured decisions
  from transcripts.** A standalone `~/.local/bin/extract-decisions` script
  runs a local LLM (Ollama `qwen3.5` by default) over each Claude Code
  transcript, extracts decision-shaped turns, and stores each as a `Decision`
  discovery (`source = "llm-extract"`, with `chosen` / `alternatives` /
  `rationale` / `sequence`) via `atheneum store-discovery` — so each is
  linked into the session thread. Resumable (`--all` skips sessions already
  having an `llm-extract` Decision), `--dry-run`, `--force`. Hallucination
  guard rejects entries without a real alphabetic `target`/`chosen`/
  `rationale`. Not an `atheneum` subcommand; the Rust port is deferred per
  the plan.
- **`watch-decisions` command + `graph::watch` — live structured-decision
  capture.** `atheneum watch-decisions <db> [--once] [--interval S=2]
  [--config-dir D]... [--project P] [--agent A] [--dry-run]` tails transcript
  files in a loop and stores `Decision` rows in real time. In-memory
  per-file cursor (offset/inode/mtime) with partial-line tolerance (a
  half-written final line is re-read next scan, never fabricated).
  `--once` runs a single cold-cursor scan — safe for cron; `decision_exists`
  dedup is the cross-invocation safety net. Detect-only; the SessionStop
  `sync-claude-transcript` hook owns full ingest at session end.
- **`decision_exists` graph method** (`graph::discovery`): indexed dedup
  lookup on the `discoveries` table so the backfiller and watcher skip
  already-captured decisions without a graph full-scan.
- **`recent_discoveries` `--session` + `--type` filters.**
  `recent_discoveries(project_id, agent, session_id, discovery_type, limit)`
  accepts a session id and a discovery-type filter (e.g. `"Decision"`), both
  applied as `json_extract` predicates over the discovery `data` JSON, and
  `atheneum discoveries-recent` exposes `--session <id>` and `--type <T>`.
  The `discoveries` table also carries `session_id` + `discovery_type`
  columns with indexes (v11 / v3), used by `decision_exists` for indexed
  dedup.
- **`session-digest` surfaces a dedicated `decisions` block.** Each
  session's digest lists its `Decision` discoveries (filtered to
  `discovery_type = 'Decision'`, limited to 5) labeled with the capture
  `source`. The existing `discoveries` block is preserved — the decisions
  block is additive.
- **`decision_exists_chosen` graph method** (`graph::discovery`): dedup guard
  for the cooperative-skill / manual `/decision` capture paths (Phase 5),
  keyed on `(session_id, target, source, chosen)` — the skill layer has no
  stable transcript `sequence`, so a sequence key (as `decision_exists` uses)
  would let a re-fired duplicate through. The two layers use different
  `source` values, so cross-layer doubles are intentionally not collapsed.
- **`store-discovery --dedup` / `--force`** (`main`): opt-in dedup for the
  skill/command store path. When `--dedup` is set and the discovery type is
  `Decision` with `session_id` + `source` + `chosen` in the metadata, a
  matching existing row skips the insert and prints `deduped: true`;
  `--force` bypasses. Only Decisions are deduped; other types insert
  unconditionally.

### Fixed

- **`recent_discoveries` accepts a `session_id` filter.** The query narrows
  to discoveries attributed to a session (via `json_extract` on the `data`
  JSON), so `chat --only-decisions` and the watcher's session scope are
  observable from the CLI.

## [0.8.0] — 2026-06-22

Consolidates the session-digest plan Phases 1–2 (session-digest composer,
thread decision-chain navigation). 0.7.1 was prepared but never published; its
contents ship here as 0.8.0 because Phase 2 removed `semantic-search` from
default features — a breaking change for consumers who relied on HNSW-backed
`search`/`navigate` by default. Phase 3 (SessionStart hook injection) lives in
the envoy repo + user hooks, not this crate; see the root CHANGELOG.

### Added

- **`thread` command + `thread_query`** (`graph::navigation`): decision-chain
  navigation. Lexical match on `ReasoningLog` + `Discovery` entry points, then
  BFS outward along `caused_by`/`led_to` chain edges only, bounded to a token
  budget. CLI: `atheneum thread <db> <query> [--tokens T=1500] [--depth D=3]
  [--k N=3] [--project P] [--json]`. Plain-text renderer orders the chain by
  entity id (chronological — `caused_by` links to a lower id, `led_to` to a
  higher one) and shows a content snippet per decision. This is the Phase 2
  wrapper over the existing scoped BFS (`get_subgraph_filtered`); `navigate`
  is unchanged.
- **`LedTo` edge type**: the forward thread edge — a prior decision led to
  this one. Inverse of `CausedBy`, stored explicitly for cheap outward walks.
  Added to the `EdgeType` enum (`as_str`/`from_label`/`all`) and the standard
  ontology seed (domain `ANY`, range `ANY`).
- **`store_discovery` thread auto-linking**: on store, if `session_id` is
  present in the metadata, the discovery is linked into its session thread —
  `observed_in → Session` (most-recent Session entity with matching
  `data.session_id`), plus `caused_by → prior` and `led_to` inverse to the
  most-recent earlier same-session `Discovery`/`ReasoningLog` (by entity id,
  which is `AUTOINCREMENT` and reflects insert/chronological order;
  `graph_entities` has no `created_at` column). If no prior exists the
  discovery is a thread root. Best-effort: missing Session entity or no prior
  is not an error. Per Open Decision #2, only discoveries emit chain edges —
  ReasoningLogs have no decision-tag field, so they are chain *search targets*
  but never auto-linked from `store_discovery`.

### Changed

- **`semantic-search` (HNSW) is now opt-in.** Removed from `default` features.
  The HNSW vector index + embedder are heavy and unnecessary for
  graph-navigation workflows: `search`, `navigate`, and `thread` fall back to
  a bag-of-tokens lexical scan over `graph_entities` plus BFS over graph
  edges. Enable with `--features semantic-search` when vector similarity is
  required. `build_search_index` and `add_entity_to_search_index` are no-ops
  without the feature. **Breaking:** consumers who relied on HNSW-backed
  similarity by default must now enable the feature explicitly.

### Fixed

- **ReasoningLog content is now searchable.** `embed_text_for_entity` now
  includes `content_summary` (transcript-sync schema) and `content`
  (`insert_reasoning_log` audit schema) in the searchable text. Previously the
  fixed key set excluded both, so ReasoningLog entities — whose `name` is a
  `<session_id>:<sequence>` identifier with no content tokens — were
  effectively unsearchable by their text. This makes `thread` and `search`
  surface reasoning turns by content.
- **`hnsw_counters_track_hits_and_fallbacks` test** is now
  `#[cfg(feature = "semantic-search")]`-gated. It asserts HNSW-only runtime
  counters (`hnsw_hits`/`hnsw_fallbacks`) and failed without the feature,
  where those counters never increment.

### Added — Phase 1 (session-digest composer, v11 attribution)

- **`session-digest` composer** (`graph::digest`): bounded, ranked bootstrap
  packet so a new session can ground on what prior sessions in the same
  project actually did — decisions made, files touched, open tasks — without
  re-discovering from scratch. CLI: `atheneum session-digest <db>
  [--project P] [--last N=3] [--tokens T=500] [--json]`. Plain-text default
  (LLM-dense, extractive — no model call); `--json` emits the structured
  `DigestReport`. Activity (tool calls, file writes, top files) is computed
  from `event_log` rather than trusted from the `sessions` ledger columns,
  which the session recorder leaves at zero. Falls back to the most recent
  sessions across all projects (with a notice line) when the `--project`
  filter matches nothing, so the digest stays useful while project tagging is
  sparse. Emits thread-anchor ReasoningLog entity ids for `atheneum navigate`
  follow-up.
- **`discoveries.session_id` attribution** (migration v11,
  `discoveries-session-id`): new nullable `session_id` column on `discoveries`
  so findings can be attributed to the session that produced them. Indexed via
  `discoveries_session_idx`. Backfilled from `metadata.session_id` for legacy
  rows. `store-discovery` accepts `--session <id>` (and `--project <id>`) on
  the CLI; `store_discovery` reads `session_id` from the metadata JSON.
- **`SessionSummary::tool`**: the session's driving tool (e.g. `claude-code`),
  populated from `sessions.tool`, distinct from `last_tool` (the most recent
  tool *call*). Surfaced in the digest header.

### Fixed — Phase 1

- **Digest file-write count** now counts both ingest paths: `file_write`
  events (from `record_evidence_file_write`) and `file_access` events with
  `access_type = "write"` (the transcript-sync path). Previously it queried a
  `file_write` event type that the transcript path never emits, so writes were
  always reported as 0 on real databases.
- **Digest top-files** now read `payload.file_path` (the field both event
  types carry) instead of the non-existent `payload.path`, and merge rows by
  basename so distinct files sharing a basename (several projects' `SKILL.md`)
  are summed instead of listed repeatedly.
- **Digest ReasoningLog join** now matches the Session entity by
  `data.session_id` instead of `name = session_id`. Session entities are named
  `<tool>:<session_id>`, so the old match never hit and decisions never
  surfaced. The reasoning text coalesces `content_summary` (transcript-sync)
  and `content` (`insert_reasoning_log`) so decisions from either recorder
  appear.
- **Digest activity line** omits zero file-write / commit counts instead of
  printing `0 file writes, 0 commits` noise.

- **CLI Observability Commands**:
  - `session-trace` returns a specific session's summary plus its associated events and tool calls.
  - `tool-usage` aggregates tool call counts for a specific session.
  - `discoveries-recent` returns a list of recent discoveries, with optional project and agent filtering.
  - `handoffs-recent` returns a list of recent handoffs, with optional project and agent filtering.
  - `events-recent` retrieves recent events, allowing filtering by session ID and event type.
  - `sessions-recent` retrieves recent sessions, with optional project and agent filtering.
- **Graph Queries**:
  - Added the `query_sessions_recent` method to graph events to support recent session retrieval by project/agent.

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
