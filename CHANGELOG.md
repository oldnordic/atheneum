# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1] — 2026-06-07

### Added

- Repo-local Claude wrapper scripts for Atheneum quality checks:
  - `.claude/scripts/quality-gate.sh`
  - `.claude/hooks/verify-rust.fish`
  - `.claude/hooks/pre-commit-rust-standards`
- `MANUAL.md` maintainer checklist covering local gates plus the rule that public API / CLI changes must update the manual and changelog in the same change.
- `AtheneumGraph::runtime_stats()` — exposes process-local cache/query/write counters so callers can inspect hot read paths and invalidation activity.
- Shared graph hashing helper for deterministic SHA-256 and canonical JSON hashing across Atheneum graph modules.
- `content_hash_excluding(value, volatile_keys)` — unified content hashing that strips volatile fields before hashing, replacing three module-local `*_content_hash` functions (discovery, memory, handoff).
- `AtheneumGraph::get_similar(name, top_k, project_id, entity_kind)` — ranked vector-similarity entity lookup via HNSW search index.
- `AtheneumGraph::resolve(name, min_confidence, project_id, entity_kind)` — single-best entity resolution above a confidence threshold, returning `DisambiguationResult` with resolved entity, candidates, and threshold.
- `DisambiguationResult` — struct capturing entity disambiguation outcome: resolved entity (if confidence met), all ranked candidates, and the minimum confidence threshold used.
- `AtheneumGraph::preview_entity_candidates()` — a no-mutation candidate-preview API for fuzzy entity lookup over the existing search index.
- `AtheneumGraph::preview_discovery()`, `AtheneumGraph::preview_memory()`, and `AtheneumGraph::preview_handoff()` — read-only proposal APIs that return normalized payloads, deterministic content hashes, likely existing matches, and vector-based disambiguation analysis before commit.
- `AtheneumGraph::preview_navigate_query()` — staged validation/repair for navigation queries, including normalized query text, canonical entity-kind resolution, and explicit warnings/errors before execution.
- `ProvenanceData` — typed struct for edge provenance metadata (method, actor, created_at, extraction_mode, source_text). Replaces all 20 ad-hoc JSON provenance sites. Backward-compatible deserialization. Builder pattern. Public API export (ATH-19).
- `EntityType::Concept` — new entity type for knowledge graph concepts extracted from prose.
- `AtheneumGraph::upsert_concept(name, data)` — name-deduped Concept entity creation; returns existing ID if one matches.
- `extract_triples(text, config)` — ollama LLM-powered extraction of (subject, predicate, object) triples from prose text (neural-embed feature).
- `ingest_triples(graph, result, project_id)` — upserts Concept entities and RelatedTo edges with ai_triple provenance for each extracted triple.
- `ProvenanceData::with_actor(actor)` — builder method to set the provenance actor field.

### Changed

- Repeated read APIs now use a concurrent in-process query cache with generation-based invalidation and adaptive TTL refresh for hot entries.
  - Cached reads: `query_memory()`, `list_memory()`, `query_sessions()`, `query_events()`, `query_knowledge[_in_project]()`, and `list_wiki_pages()`.
  - Relevant writes now invalidate those domains coarse-grain after successful mutation (`store_memory`, session/event writes, discovery/handoff writes, wiki ingestion).
  - The cache is runtime-only and safe for concurrent callers; no persisted schema changes were required.
- `store_discovery()` and `store_handoff()` now stamp deterministic `content_hash` values derived from canonical JSON, so equivalent payloads keep the same hash regardless of key order.
- Wiki ingestion now resolves high-confidence wikilinks against existing `WikiPage` titles and uses candidate preview as a conservative fuzzy fallback before creating a stub page.
- Discovery, memory, and handoff preview flows now guarantee exact existing matches are surfaced without mutating the graph, which closes the gap between exact lookup and fuzzy candidate preview.
- `navigate` CLI now accepts `--kind` and includes validated query-plan metadata in its JSON output, so repaired kinds and rejected inputs are explicit instead of silent.

### Fixed

- Navigation kind filters no longer fail silently on lowercase or plural inputs such as `memory`, `memories`, `wiki`, or `discoveries`.
  - `preview_navigate_query()` repairs those aliases to canonical `EntityType` labels before traversal.
  - Invalid kinds are rejected before search with a clear error listing the accepted entity kinds.
- `insert_edge()` now validates domain/range constraints against the ontology before insertion. Edges that violate domain/range rules are rejected with `AtheneumError::EdgeValidation` containing structured fields (ATH-20).

### Documentation

- **CLI reference brought up to date with actual command surface.** 21 of 30 CLI commands were undocumented in MANUAL.md and README.md. All now listed with usage, flags, and examples.
- **API.md updated with full public surface.** Previously documented ~20 of ~50 public methods. Now covers sessions, evidence, discoveries, memory, dream, handoffs, tasks, wiki, search, HopGraph, navigation, ontology, and all public types.
- Fixed duplicate `### Fixed` section header from an earlier merge.

### Fixed (continued)

- **Search degrades cleanly when the persisted `discoveries` HNSW index is inconsistent.**
  - `graph/search.rs` — `lexical_search()` now tries the persistent HNSW path, attempts one rebuild, and falls back to direct lexical scanning over graph entities instead of aborting the command.
  - This restores CLI search usability against partially damaged databases where HNSW restore or lookup fails.
  - Regression test: `test_semantic_search_falls_back_when_hnsw_index_is_inconsistent`.

- **Memory upsert recreates missing SQL rows for existing `Memory` entities.**
  - `graph/memory.rs` — `store_memory()` now inserts a replacement row into `memory_entries` when the graph entity exists but the SQL read-model row is missing, then re-stamps `sql_id` into entity data.
  - This keeps graph state and the SQL memory domain consistent after partial data loss or manual table repair.
  - Regression test: `test_store_memory_recreates_missing_sql_row_for_existing_entity`.

## [0.3.0] — 2026-06-05

### Added

- **Dreaming module** — reflective memory consolidation pass inspired by AutoDream.
  - `src/graph/dream.rs` — 6-phase pipeline: SCAN → DEDUPLICATE → STALE → CONTRADICTION → VERBOSE → CONSOLIDATED.
  - Trigram Jaccard similarity for near-duplicate detection (configurable threshold, default 0.65).
  - Staleness detection: entries not updated in N days with confidence below threshold.
  - Contradiction detection: same key across different scopes with low content similarity.
  - Verbosity scoring: content length vs unique-word ratio.
  - `DreamMode::DryRun` (report only) and `DreamMode::AutoMerge` (creates SupersededBy edges).
  - `DreamConfig` with tunable knobs for all thresholds.
  - `DreamReport` / `DreamFinding` / `DreamPhase` serializable output types.
  - CLI command: `atheneum dream <db> [--scope S] [--project P] [--dry-run|--auto-merge]`.
- **Wiki dream pass** — `wiki_dream_pass()` applies the same consolidation pipeline to wiki page entities.
  - CLI command: `atheneum wiki-dream <db> [--project P] [--dry-run|--auto-merge]`.
  - 9 unit tests covering Jaccard similarity, dry-run, auto-merge edge creation, contradiction detection, and edge cases.
- `EdgeType::SupersededBy` — marks superseded memories pointing to their replacement.
- `EdgeType::ConsolidatedFrom` — marks entries absorbed by consolidation (reserved for future merge use).
- CLI flags `--dry-run` and `--auto-merge` added to option parser.

## [0.2.3] — 2026-06-04

### Added

- **Memory domain** — stable-fact storage distinct from Knowledge (merged discoveries) and WikiPage (documents).
  - `EntityType::Memory` — new entity kind with `"Memory"` ontology class.
  - `memory_entries` SQL table — `id, key, scope, content, confidence, project_id, created_at`. Scope: `user` | `project` | `agent`.
  - `db/memory.rs` — `migrate_v7_memory()` migration.
  - `graph/memory.rs` — `store_memory(key, content, scope, confidence, project_id)` creates both SQL row and graph entity, auto-indexes in HNSW. `query_memory(key, scope, project_id)` and `list_memory(scope, project_id)` filter via `json_extract` on graph entity data.
  - CLI commands: `memory-store <db> <key> <content> [--scope S] [--confidence N] [--project P]`, `memory-get <db> <key> [--scope S] [--project P]`, `memory-list <db> [--scope S] [--project P]`.
  - Tests: 8 memory CRUD tests in `tests/memory_tests.rs` covering store, query by key, scope/project filtering, list, entity type kind, and lexical search visibility.

### Changed

- `lexical_search()` and `navigate()` — `entity_kind` parameter changed from `Option<&str>` to accept `Option<EntityType>` filter (post-filter on `SearchResult.kind`). CLI `--kind` flag works for all entity kinds including `Memory`.

## [0.2.3] — 2026-06-04

### Fixed

- **SQLite busy_timeout** — set 5000ms on connection open to reduce lock contention under concurrent access (wiki-watcher + CLI + agent sessions).
- **Memory upsert** — `store_memory()` now correctly updates existing entry by composite key (key, scope, project_id) instead of creating duplicates.
- Parse errors in memory content fail fast instead of silently corrupting graph entity data.
- `updated_at` tracking: upserts now set `updated_at` to current timestamp.

## [0.2.2] — 2026-06-04

### Changed

- **HNSW search index now uses persistent storage** — switched from in-memory `hnsw_index` to `hnsw_index_persistent` (sqlitegraph 3.0.8). Vectors survive process restarts. Requires sqlitegraph ≥ 3.0.8.

## [0.2.0] — 2026-06-03

### Fixed

- **`build_search_index()` indexed only 3 entity kinds** — hardcoded `[Discovery, WikiPage, JournalSection]` whitelist excluded Sessions, Agents, ToolCalls, Events, Tasks, Files etc. from the HNSW index. `navigate` and `hopgraph_query` returned empty for any query targeting those entity types. Now indexes ALL entity kinds via new `all_entities()` method.
- **`lexical_search()` fallback had same whitelist** — token-scoring fallback also skipped non-whitelisted entities. Now scans all entities.

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

- **`build_search_index()` now indexes ALL entity kinds** — previously limited to Discovery, WikiPage, JournalSection. Added `all_entities()` method to AtheneumGraph.
- **`reindex` CLI command** — `atheneum reindex <db>` rebuilds HNSW index over all entities for existing databases.
- **`evidence.rs` modularized** — Split the 1143-line file into 5 focused submodules under `evidence/`:
  - `helpers.rs` (71 LOC) — shared relation/project helpers
  - `session.rs` (228 LOC) — session lifecycle (record, end, progress)
  - `recording.rs` (662 LOC) — 8 `record_evidence_*` methods
  - `events.rs` (207 LOC) — event logging and querying
  - `mod.rs` (4 LOC) — re-exports
  - No logic, signature, or behavior changes. Pure file reorganization.

- **`graph/search.rs`** — now uses `self.embedder.embed()` instead of static `hash_embed()`. `search_config()` takes dimension as parameter.

### Fixed

- **HNSW search index auto-builds on first query.** `ensure_search_index()` now populates the HNSW index from all entities on creation. No manual `reindex` needed on fresh process start. `build_search_index()` / `reindex` CLI still available for forced rebuild.
- **HNSW search index now uses persistent storage** (`hnsw_index_persistent`). Upgraded from in-memory `hnsw_index` to sqlitegraph 3.0.8's persistent backend with sequential vector IDs. Vectors survive process restarts without O(N) repopulation. Requires sqlitegraph ≥ 3.0.8 (fixes `InvalidNodeId` after delete+recreate).
- Clippy `collapsible_if` lint in `get_subgraph_scoped` — collapsed nested conditionals.

### Tests

- 39 TDD tests in `tests/hopgraph_tests.rs` covering HopGraph P1–P4, search index coverage, navigate-for-sessions, and HNSW search across DB reopen.
- Previously failing `test_seed_standard_ontology_populates_hopgraph_relations` now passes with `explains` and `derived_from` in the ontology seed.

### Known Limitations

- **HNSW index graph structure rebuilt from vectors on reopen.** Vector data persists across restarts via sqlitegraph's `hnsw_index_persistent`. Graph structure (neighbor lists, entry points, layer assignments) is rebuilt from stored vectors when the index is re-opened. For large indexes this is O(N). A future sqlitegraph update will persist the graph structure directly.
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
