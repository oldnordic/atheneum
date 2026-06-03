# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] — 2026-06-03

### Added

- **HopGraph Phase 1: Wiki Pages as First-Class Graph Nodes** — Wiki pages are now full participants in the HopGraph traversal, not just side-table records.
  - `EdgeType::Explains` — semantic edge from a wiki page to the code symbol it documents (e.g., `wiki/http-handler.md` → `build_router`).
  - `EdgeType::DerivedFrom` — provenance edge for entities derived from other entities.
  - `EntityType::WikiPage` — canonical entity type enum variant for wiki page nodes.
  - `get_subgraph_filtered(entry_id, depth, allowed_types)` — BFS subgraph extraction that only follows edges matching a whitelist of `EdgeType` values. Empty whitelist returns all edges (delegates to `get_subgraph`).
  - Ontology seed now includes `Explains` (WikiPage→CodeSymbol) and `DerivedFrom` (ANY→ANY) property definitions.
  - `link_wiki_to_symbols(magellan_db_path, agent_name, project_id)` — bridges wiki content to code symbols via magellan. Idempotent.

- **HopGraph Phase 2: Token-Budgeted Retrieval API** — Query the knowledge graph with token budgets instead of unbounded subgraph walks.
  - `estimate_entity_tokens(entity)` — rough token count for a GraphEntity (~4 chars/token).
  - `truncate_subgraph(view, max_tokens)` — trims entities and edges to fit a token budget. Entry entity always kept. Orphan edges dropped.
  - `hopgraph_query(query, k, depth, allowed_types, max_tokens, project_id)` — main retrieval API: semantic search → `get_subgraph_filtered` → `truncate_subgraph`. Shared token budget across all result views.

- **HopGraph Phase 3: Neural Embedding Interface** — Swappable embedding backend for semantic search.
  - `TextEmbedder` trait — `embed(text) -> Vec<f32>` + `dimension() -> usize`. Thread-safe (`Send + Sync`).
  - `HashEmbedder` — existing bag-of-tokens (128-dim), zero dependencies. Now in `graph/embed.rs`.
  - `OllamaEmbedder` — neural embeddings via ollama `/api/embed` endpoint (768-dim `nomic-embed-text`). Feature-gated behind `neural-embed` (requires `ureq`).
  - `AtheneumGraph::set_embedder(Box<dyn TextEmbedder>)` — runtime embedder swap. `AtheneumGraph::embedder_dimension()` — query current dimension.
  - Feature flag `neural-embed` — `cargo build --features neural-embed` enables `OllamaEmbedder`.

- **HopGraph Phase 4: Discovery Consolidation** — Merge duplicate discoveries into Knowledge entities.
  - `consolidate_discoveries(target, project_id)` — merges all Discovery entities for a target into a single Knowledge entity with `DerivedFrom` edges. Idempotent.
  - `consolidation_pass(project_id)` — scans all distinct discovery targets and consolidates each. Returns `(target, knowledge_id)` pairs.

- **`neural-embed` feature flag** in Cargo.toml — gates `ureq` dep + `OllamaEmbedder`. Default build has zero new deps.

### Changed

- **`evidence.rs` modularized** — Split the 1143-line file into 5 focused submodules under `evidence/`:
  - `helpers.rs` (71 LOC) — shared relation/project helpers
  - `session.rs` (228 LOC) — session lifecycle (record, end, progress)
  - `recording.rs` (662 LOC) — 8 `record_evidence_*` methods
  - `events.rs` (207 LOC) — event logging and querying
  - `mod.rs` (4 LOC) — re-exports
  - No logic, signature, or behavior changes. Pure file reorganization.

- **`graph/search.rs`** — now uses `self.embedder.embed()` instead of static `hash_embed()`. `search_config()` takes dimension as parameter.

### Fixed

- Clippy `collapsible_if` lint in `get_subgraph_scoped` — collapsed nested conditionals.

### Tests

- 36 TDD tests in `tests/hopgraph_tests.rs` covering HopGraph P1–P4: enum roundtrips, serde serialization, wiki entity creation, wikilink edges, edge-type filtering, idempotent re-ingestion, token estimation, subgraph truncation, hopgraph queries, embedder trait, ollama embedder, discovery consolidation.
- Previously failing `test_seed_standard_ontology_populates_hopgraph_relations` now passes with `explains` and `derived_from` in the ontology seed.

### Known Limitations

- `OllamaEmbedder` requires ollama running locally with `nomic-embed-text` model pulled. No fallback — panics on connection failure.
- `estimate_entity_tokens` uses ~4 chars/token approximation, not a real tokenizer. Budget enforcement is approximate.
- `link_wiki_to_symbols` requires a magellan database at the given path. No auto-discovery of magellan DB location.
- `consolidate_discoveries` does not merge metadata fields — keeps the first discovery's metadata. Future: field-level merge strategy.

## [0.1.0] — 2026-05-31

### Added

- Initial release with sqlitegraph-backed agent coordination graph.
- SQL payload layer: agents, tasks, events, reasoning logs, tool calls.
- Planning domain: tasks, requirements, blockers, kanban board.
- Knowledge domain: wiki pages, journal sections, discoveries.
- Evidence domain: audit trail, handoff records.
- Ontology ingestion with class/property extraction.
- Graph navigation: causal chains, task assignment, event provenance.
- Wiki query APIs: `get_wiki_page`, `list_wiki_pages`, `find_pages_by_wikilink`, `query_journal_sections`.
- Wikilink graph edges during `ingest_wiki_page`.
- Graph navigation: `outgoing_wikilinks`, `incoming_wikilinks`.
- Batch sync: `sync_wiki_directory`, `sync_journal_directory`.
- CLI: `sync-wiki`, `sync-journal`, `query-wiki`, `query-journal`, `graph-stats`, `entity`, `edge`, `neighbors`, `navigate`, `sync-logseq`, `sync-claude-transcript`.
- Library re-exports: `WikiPage`, `JournalSection`, `KanbanStatus`, `KanbanUpdate`, wiki parsing utilities.
- Session accountability: `SessionSummary` with `total_input_tokens`, `total_output_tokens`, `total_cost_usd`.
- `record_event()` public API for generic agent events.
- Cross-project session queries via `query_sessions()` with `Option<&str>` project parameter.
- `EndSessionParams` token/cost fields.
- Graph navigation primitives in `graph/navigation.rs`.
- Auto-index on discovery write.
- Lazy HNSW index creation.
- Claude transcript ingestion (incremental, append-safe, replay-safe).
- HopGraph relation vocabulary: `mentions`, `wikilink`, `implements`, `depends_on`, `calls`, `tested_by`, `fixed_by`, `regressed_by`, `observed_in`, `belongs_to_project`, `similar_failure`, `requires_skill`, `handled_by_tool`.
- First-class file access relation (`accessed`).
- Standard relation ontology seed.
- Evidence-to-HopGraph ingestion for all evidence types.
- Generic event relation hints via `payload.relations`.
- `tests/wiki_query_tests.rs` with 7 tests.
- `tests/ingest_test.rs` migrated to `ingest_wiki_page()`.

### Changed

- `KanbanStatus` and `KanbanUpdate` now derive `serde::Serialize` and `serde::Deserialize`.
- `query_events()` uses `with_raw_connection` and explicit `rusqlite::params![]` match arms.

### Fixed

- `is_healthy()` works for in-memory databases.
- `test_ingest_real_wiki_article` no longer hardcodes `/home/feanor/` path.
- Frontmatter parsing only accepts leading `---` block.
- Wiki ingestion creates `wikilink` edges instead of overloading `related_to`.
- `record_evidence_prompt()` uses session's real SQL `agent_id`.
- `record_evidence_fix_chain()` synthesizes missing SQL commit rows.
- `EdgeType` JSON serialization uses stable snake_case labels.
- CLI stdout handles closed pipes cleanly.
- Claude transcript sync is incremental and replay-safe.
- `outgoing_edges` / `incoming_edges` work for in-memory databases.
- `examples/batch_ingest_wiki.rs` uses `sync_wiki_directory`.

### Removed

- Deprecated `serde_yaml` dependency — replaced with lightweight inline YAML-like parser.
- Dead `EntityType` variants (`Decision`, `FileChange`, `Verification`, `Benchmark`, `Release`).
- Dead `EdgeType` variant (`Supersedes`).
- Dead `releases` SQL table.
- Deprecated `ingest_article()` method.

### Security

- Fixed null-byte corruption at end of `tests/ingest_test.rs`.
