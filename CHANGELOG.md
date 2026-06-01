# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- README.md with quickstart, features, CLI usage, requirements.
- `Cargo.toml` metadata: `repository`, `keywords`, `categories`.

### Fixed

- `is_healthy()` now works for in-memory databases (uses `pool.is_in_memory()` instead of failing `pool.get()`).
- `test_ingest_real_wiki_article` no longer hardcodes `/home/feanor/` path; uses inline Markdown content.
- `CHANGELOG` placeholder date fixed.

### Removed

- **Removed deprecated `serde_yaml` dependency** — frontmatter parser in `graph/mod.rs` now uses a lightweight inline YAML-like parser instead of the deprecated `serde_yaml` crate. Handles strings, booleans, integers, floats, and arrays.
- **Removed dead `EntityType` variants** — Pruned 5 unused variants (`Decision`, `FileChange`, `Verification`, `Benchmark`, `Release`) that were defined in `types.rs` and the ontology seed but never actually instantiated by any graph code. Synced `seed_standard_ontology()` and all affected tests.
- **Removed dead `EdgeType` variants** — Pruned `DependsOn` and `Supersedes` which were never used outside `types.rs`. Kept `Modified` and `VerifiedBy` since they have actual edge insertions in `audit.rs` and `evidence.rs`.
- **Removed dead `releases` SQL table** — The `releases` table in `db/evidence.rs` schema migration was never written to by any code. Removed from v4 migration to keep the schema lean.
- **Removed deprecated `ingest_article()`** — Deleted the deprecated method from `graph/mod.rs`. All callers (tests and examples) were already migrated to `ingest_wiki_page()`.

### Internal

- Cleaned up unused `json` import in `graph/mod.rs` after removing `ingest_article`.

## [0.2.0] — 2026-05-31

### Changed

- **`tests/ingest_test.rs`** — migrated all tests from deprecated `ingest_article()` to modern `ingest_wiki_page()`. Removed `#![allow(deprecated)]` from test file.
- **`graph/evidence.rs`** — `query_events()` now uses `with_raw_connection` instead of inline connection boilerplate, and uses a `String` builder with explicit `rusqlite::params![]` match arms instead of dynamic SQL string concatenation with `Box<dyn ToSql>`.

### Security / Robustness

- Fixed null-byte corruption at end of `tests/ingest_test.rs` that was preventing compilation.

## [0.1.0] — 2025-05-31

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

## [0.1.0] — 2025-05-31

### Added

- Initial release with sqlitegraph-backed agent coordination graph.
- SQL payload layer: agents, tasks, events, reasoning logs, tool calls.
- Planning domain: tasks, requirements, blockers, kanban board.
- Knowledge domain: wiki pages, journal sections, discoveries.
- Evidence domain: audit trail, handoff records.
- Ontology ingestion with class/property extraction.
- Graph navigation: causal chains, task assignment, event provenance.
