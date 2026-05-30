# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Wiki query APIs** — `AtheneumGraph` now exposes full CRUD-like query methods for wiki content:
  - `get_wiki_page(path)` — query a single wiki page by path
  - `list_wiki_pages(project_id)` — list all wiki pages, optionally filtered by project
  - `find_pages_by_wikilink(target, project_id)` — find pages referencing a wikilink target
  - `query_journal_sections(path)` — query journal sections by file path

- **Wikilink graph edges** — During `ingest_wiki_page`, `[[...]]` syntax is parsed and `RelatedTo` edges are created in the graph database. Missing target pages become stub entities so LLMs can still navigate to them.

- **Graph navigation** — `outgoing_wikilinks(page_id)` and `incoming_wikilinks(page_id)` for LLM traversal of wiki link graphs.

- **Batch sync methods** — `sync_wiki_directory(dir, project_id)` and `sync_journal_directory(dir, project_id)` for ingesting entire directories of `.md` files.

- **CLI commands** — `main.rs` now supports:
  - `sync-wiki <db> <dir> [project]`
  - `sync-journal <db> <dir> [project]`
  - `query-wiki <db> <path>`
  - `query-journal <db> <path>`

- **Library exports** — `src/lib.rs` now re-exports `WikiPage`, `JournalSection`, `KanbanStatus`, `KanbanUpdate`, and wiki parsing utilities (`extract_wikilinks`, `content_hash`, `parse_journal_sections`, `extract_kanban_updates`).

- **Integration tests** — `tests/wiki_query_tests.rs` with 7 tests covering query APIs, graph edge creation, project filtering, and backlink navigation.

### Fixed

- `outgoing_edges` / `incoming_edges` on `AtheneumGraph` now work for in-memory databases (previously used `pool.direct_connection()` which returns `None` for `:memory:`).
- `examples/batch_ingest_wiki.rs` now uses `sync_wiki_directory` instead of the deprecated `ingest_article` API.

### Changed

- `KanbanStatus` and `KanbanUpdate` now derive `serde::Serialize` and `serde::Deserialize` for JSON round-tripping in journal section queries.

## [0.1.0] — 2024-XX-XX

### Added

- Initial release with sqlitegraph-backed agent coordination graph.
- SQL payload layer: agents, tasks, events, reasoning logs, tool calls.
- Planning domain: tasks, requirements, blockers, kanban board.
- Knowledge domain: wiki pages, journal sections, discoveries.
- Evidence domain: audit trail, handoff records.
- Ontology ingestion with class/property extraction.
- Graph navigation: causal chains, task assignment, event provenance.
