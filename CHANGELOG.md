# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased] — Pending

### Fixed

- **`atheneum reindex`** no longer crashes with "Execute returned results - did you mean to call query?". `Graph::checkpoint()` now uses `query_row` for `PRAGMA wal_checkpoint(TRUNCATE)`, because that PRAGMA returns a row.
- **`wiki_pages_fts` self-heals on open** when the FTS5 shadow tables are left corrupt by an external SQLite writer. The recovery purges `sqlite_master` directly (bypassing the broken vtable), recreates the table and triggers on a fresh connection, then runs a full `delete-all` → repopulate → `rebuild` cycle on another fresh connection. This makes `sync-wiki`, `search-wiki`, and `backfill-wiki` robust against "database disk image is malformed" / "vtable constructor failed" corruption.

## [0.6.1] — 2026-06-16

### Added

- **Path-aware wiki search.** The `wiki_pages_fts` FTS5 index now includes the `path` column, so `search-wiki` matches path fragments (e.g. `session` matches `wiki/session-accountability.md`).
- **Prefix wildcard matching.** `search-wiki` automatically treats each query token as a prefix (`rout` matches `Routes`, `Router`, path fragments).
- **Graph-entity fallback search.** If FTS5 returns no hits, `search_wiki_pages` falls back to a substring search over graph entity names/paths and wiki titles, so partial concept queries can find stored pages.

## [0.6.0] — 2026-06-16

### Added

- **FTS5 full-text search over wiki pages.** New `wiki_pages_fts` index backs `atheneum search-wiki` with paginated, ranked results and excerpts. The full article body is never returned by the search API.
- **`backfill-wiki` CLI command** repairs wiki pages that were written directly to the `wiki_pages` SQL table without a proper `WikiPage` graph entity, restoring wikilink navigation and resolving stub targets.
- **Library API additions:**
  - `AtheneumGraph::search_wiki_pages(query, project_id, offset, limit) -> Result<Vec<WikiSearchResult>>`
  - `AtheneumGraph::backfill_wiki_pages_to_graph(project_id) -> Result<Vec<(i64, String)>>`
  - Re-exported `WikiSearchResult` from the crate root.

### Fixed

- **SQLite FTS5 version mismatch** — Migration v9 drops and recreates the `wiki_pages_fts` virtual table during open so the index format matches the SQLite version opening the connection. This addressed the original "database disk image is malformed" error when the DB was touched by a newer system `sqlite3`. The root cause was later generalized and hardened by the [Unreleased] `ensure_wiki_fts_healthy` per-open self-heal.
- **Unicode-safe excerpt slicing** in `search_wiki_pages` no longer panics on multi-byte characters such as `→`.

## [0.5.0] — 2026-06-09

### Added

- Cross-project `CrossRouter` with LRU-backed `ATTACH DATABASE` cache
- `cross_search` and `cross_navigate` for querying symbols across registered project databases
- Meta router (`meta.db`) for project registration and routing
- Memory entries with HNSW search (via sqlitegraph)
- Wiki pages with dream consolidation
- Discovery recording with auto-indexing
- Session lifecycle: create, record evidence (prompts, tool calls, file writes, commits, test runs, fix chains, bench runs), end
- Subagent handover notes
- Generic event logging
- MCP server tool registration (9 tools)
