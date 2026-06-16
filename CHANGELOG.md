# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
- **`cross_navigate` edge column mismatch** — The `bfs()` function in `cross.rs` queried `kind` from `graph_edges`, but production magellan databases name that column `edge_type`. Changed to `SELECT id, edge_type AS kind, ...` so both schemas work.

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
