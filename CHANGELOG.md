# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- **`cross_navigate` edge column mismatch** — The `bfs()` function in `cross.rs` queried `kind` from `graph_edges`, but production magellan databases name that column `edge_type`. Changed to `SELECT id, edge_type AS kind, ...` so both schemas work. Test fixture `make_magellan_like_db` updated to use `edge_type` column name.

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
