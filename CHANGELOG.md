# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.12.1] - 2026-07-21

### Added — Kimi Code CLI plugin set (`plugin/atheneum-decisions`)

- `kimi.plugin.json` manifest plus `hooks/kimi/` variants of the three
  hooks (session bootstrap digest, per-turn prefetch hints, decision
  gate), sharing the skills/commands with the Claude Code set. One plugin
  root, per-agent manifests and hook script sets. Install in Kimi Code CLI
  with `/plugins install plugin/atheneum-decisions` then `/reload`.

### Fixed

- `memory-prefetch-hints`: newer-toolchain clippy compliance (MSRV-stable
  CI runs `-D warnings`): `iter::repeat_n`, collapsed condition,
  `ParsedArgs` type alias, `vec!` → arrays in tests. No behavior change.
- Semgrep `p/rust` CI: documented `nosemgrep` for the
  `rust.lang.security.args.args` false positive on the binary's own argv
  parse.
- `atheneum-mcp`: default `ATHENEUM_DB` fallback path moved from
  `~/.magellan` to `~/.hermes` to match the live database location
  (already released as part of `a4439bc`; repo-only crate, not on
  crates.io).

## [0.12.0] - 2026-07-21

### Added — `memory-prefetch-hints` binary (`atheneum` crate)

- Standalone `[[bin]]` ranking CLI: scores `Memory` entities by BM25 +
  TF-IDF + kind weight + recency + session continuity + an optional
  trajectory bonus, returning a token-budgeted JSON candidate list.
  `--session-id` scores entities from the live session higher;
  `--trajectory`/`--trajectory-query` adds optional PSF1/PSF2
  trajectory-graph lookup. Consumed by the Hermes `atheneum` plugin.
  See `crates/atheneum/CHANGELOG.md`, `ARCHITECTURE.md`, `API.md`,
  and `MANUAL.md` for the full scoring/format reference.

### Fixed

- `memory_search`'s candidate-pool query had no `ORDER BY` before its
  `LIMIT` — returned the oldest rows in the table regardless of query
  or recency on any database with more than a few dozen `Memory`
  entities. Added `ORDER BY id DESC`.

## [0.11.0] - 2026-07-21

### Added — Librarian primitives (closes the local-memory gap analysis)

- **`update_memory`** (`AtheneumGraph::update_memory`, CLI `memory-update`, MCP
  `update_memory`): patch an existing memory's content/importance/tags in
  place instead of forcing every correction to become a new row.
- **`upsert_memory_by_concept`** + MCP `add_memory`: enrich-before-create —
  finds the concept a fact belongs to and patches it, or creates the concept
  and memory together if it doesn't exist yet.
- **`insert_edge_pair`**: inserts a forward edge plus its reciprocal in one
  call (`attached_to`↔`has_memory`, `related_to`↔`related_to`,
  `verified_by`↔`verifies`, `superseded_by`↔`supersedes`), so new concepts
  are no longer islands unless the caller forgets the second edge.
- **`lint_graph` / `maintain`** (CLI `lint`, `maintain --apply`; MCP
  `maintain`): deterministic graph-health lint flags orphans, broken
  wikilinks, and stale `superseded_by` edges; `maintain` auto-rewires orphans
  to their closest concept, stubs or severs broken links, and resolves
  flagged contradictions by superseding the old fact rather than leaving both
  versions live.
- **`seed_memory`** (CLI `session-digest`-adjacent, MCP `seed_memory`): a
  compact, token-bounded summary of what's in the knowledge base, grouped by
  concept rather than file name. `atheneum-mcp` now regenerates this on every
  session connect and injects it into the server `instructions` field and the
  `navigate`/`query_memory`/`search` tool descriptions, so a connecting
  client knows what's in memory before it asks — closing the "flying blind"
  failure mode where a client never thought to check memory because it had
  no idea anything useful was there.
- **Query tracing** (`trace_query`, `QueryTrace` entities, `navigate --trace`,
  CLI `trace-get`): records the plan and result ids of a navigation query so
  a past query can be replayed and inspected.
- **`dream_if_idle`**: runs a consolidation pass only if no writes have
  occurred within a given idle threshold, so dream can be safely wired into
  a periodic scheduler without racing live writers.
- **`semantic_consolidation`** (CLI `dream-semantic`, MCP `dream_semantic`):
  merges closely-related or redundant concepts (e.g. two profiles for the
  same person under different names) using a local language-model prompt
  when one is reachable, with a lexical-similarity fallback when it isn't.
  The superseded concept's edges are rewired onto the surviving one rather
  than dropped.
- **Memory pinning + TTL** (`pin_entity`/`unpin_entity`, CLI `pin`/`unpin`,
  MCP `pin_entity`/`unpin_entity`): pinned entities are always included in
  `seed_memory` regardless of token budget or recency, and are immune to
  cache eviction. `maintain` archives memories past their configured TTL.
- **Local-model discovery + swap guard** (`discover_available_models`, CLI
  `models-list`, MCP `list_models`, `SwapGuardMode` config): queries a local
  model server for what's currently loaded so model-dependent operations
  (like semantic consolidation) don't force an unwanted model swap on a
  shared GPU. `SwapGuardMode` picks the behavior when the preferred model
  isn't loaded: fall back to a lexical check (default), adapt to whatever is
  loaded, or fail closed in `strict` mode.
- **Dashboard web UI** (CLI `dashboard`, `web-ui` Cargo feature, off by
  default): an Axum server exposing the graph, query traces, and flagged
  orphans/contradictions over HTTP for inspection outside the CLI.
- **`wiki-search` CLI command** (`crates/atheneum/src/main.rs`,
  `crates/atheneum/src/graph/wiki.rs`): Full-text search over wiki pages using
  the existing `wiki_pages_fts` FTS5 index. Previously, 661 wiki pages were
  completely unsearchable from the CLI — `query-wiki` required the exact full
  filesystem path. The new `wiki-search <db> <query> [--project P] [--limit N]`
  command queries the FTS5 index and falls back to name-based matching when FTS
  returns no hits.
- **`decision-search` CLI command** (`crates/atheneum/src/main.rs`,
  `crates/atheneum/src/graph/discovery.rs`): Content search over Decision
  discoveries by `target`, `chosen`, and `why` text fields. Previously, 381
  decisions were only reachable via `discoveries-recent --type Decision`
  (chronological list, not searchable by content).

### Security

- **`sqlitegraph` bumped to 3.9.0** (from a lockfile stuck on an old
  resolution, see below) transitively adds `rio` on Linux, which carries an
  unfixed critical advisory (`RUSTSEC-2020-0021`, use-after-free on a leaked
  future). `rio` is only exercised by sqlitegraph's non-default `native-v3`
  backend feature; this workspace uses only the default `sqlite-backend`, so
  the vulnerable code path is compiled but unreachable. CI's `cargo audit`
  step now explicitly ignores this advisory with that justification
  (`.github/workflows/ci.yml`) rather than silently passing or blocking the
  release. Revisit once sqlitegraph makes `rio` optional.

### Fixed

- **`seed_memory` noise filtering**: `Agent` and `Call` entities (agent
  registration records, raw call-graph edges) were appearing in the seed
  summary as bare `- name: ` lines with no content — structural
  graph-plumbing, not knowledge. Both kinds are now excluded, along with any
  entity whose summary/content/body is empty.
- **`seed_memory` token-budget starvation**: concepts render before recent
  memories and, with enough of them, could consume the entire token budget
  before memories ever got a chance to render. `seed_memory` now reserves up
  to a third of the remaining budget (minimum 60 tokens) for Recent Memories
  before concepts are allowed to spend it.
- **Stale `sqlitegraph` lockfile pin**: `Cargo.lock` had `sqlitegraph`
  pinned to an old resolution with only 6 of the crate's now-10 schema
  migrations, well behind what's actually published on crates.io (3.9.0).
  Any database written by a newer `sqlitegraph` (including this workspace's
  own CLI) could no longer be opened by binaries still linked against the
  stale pin — `atheneum-mcp` in particular failed to start at all
  (`schema error: database schema version N is newer than supported 6`),
  silently dropping it from any MCP client's server list. `cargo update -p
  sqlitegraph` repins to the current registry release; no dependency
  requirement changed.
- **`query-wiki` now supports partial path matching** (`crates/atheneum/src/
  graph/wiki.rs`): Previously required the exact full filesystem path. Now falls
  back to a `LIKE '%<path>%'` contains-match when exact match returns no results.
- **`memory-bootstrap` excludes session-scoped noise** (`crates/atheneum/src/
  graph/memory.rs`): Previously fetched ALL entries including low-confidence
  session-scoped chat logs (scope LIKE `session:%`) that crowded out durable
  memories in the token budget. Now filters `scope NOT LIKE 'session:%'`.
- **`journal_sections` table populated**: Was empty (0 rows) because
  `sync-journal` had never been run. Ran sync — 17 journal sections ingested.

### fix(wiki): wiki_search returns empty when project filter given

Root cause: all 661 wiki pages have `project_id = NULL` (unscoped). The FTS
and name-search SQL used `wp.project_id = ?` which matches zero rows when
project_id is NULL (SQL NULL comparison semantics: `NULL = 'Projects'` is
NULL/false).

Fix: changed both `search_wiki_pages_fts` and `search_wiki_pages_by_name`
SQL to `(wp.project_id = ?2 OR wp.project_id IS NULL OR wp.project_id = '')`.
Unscoped wiki pages now match any project filter.

Verified:
- `wiki_search(query="sqlitegraph native-v3", project="Projects")`: 3 results (was 0)
- `wiki_search(query="sqlitegraph native-v3")`: 3 results (unchanged)

### atheneum-mcp — memory contract fix

- **query_memory is now explicitly an exact key lookup** (`atheneum-mcp/backend.rs`):
  matches the direct graph API. Was documented as semantic retrieval but
  implemented as key lookup — now the contract is honest.
- **store_memory accepts optional key, scope, project** (`atheneum-mcp/backend.rs`):
  instead of forcing an implicit content-derived key and implicit agent scope.
  Callers can now partition memory by scope/project.
- **query_memory accepts optional scope and project filters** plus deprecated
  `query` as a backward-compatible alias for `key`.

## [0.10.0] - 2026-06-27

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
- **`extract-decisions` heuristic backend** — `--heuristic` (or `--mode
  heuristic`, or `ATHENEUM_EXTRACT_MODE=heuristic`) switches the subcommand to a
  rule-based extractor with no LLM and no network. It catches decision-shaped
  sentences that carry an explicit rationale clause (`because`/`since`/`so
  that`), reuses the same hallucination guard + store/dedup plumbing as the LLM
  path, and writes `source = "heuristic"` so the two backends are separately
  resumable and distinguishable in the graph. Lower recall + some false
  positives vs the LLM; zero deps. The default backend remains the Ollama LLM
  (`--mode llm`).
- **`sessions-recent --exclude-project <P>` (repeatable)** — hides named
  project buckets from the recent-sessions view without re-attributing rows.
  Targets the `tmp` / `Projects` buckets that arise honestly when a session
  runs from `/tmp` or a non-repo parent dir (no git worktree, so the shared
  git-toplevel-basename fallback yields the dir basename). `LIMIT` applies
  after exclusion. `query_sessions_recent` gained an `exclude_projects`
  parameter (SQL `AND s.project NOT IN (...)`).
- **Ranking benchmark manifest** — `docs/ranking-benchmark-manifest.json`
  records mixed-corpus retrieval queries, expected authoritative hits, and
  classes of results that should be demoted (`File`, `ReasoningLog`,
  `CHANGELOG`, low-signal untitled pages). It exists so ranking work is tied
  to explicit query expectations instead of “results feel better”.

### Changed

- **`chat --only-decisions` renderer enriched** — each decision now prints
  `source` + `sequence` inline and the `chosen` / `rationale` / `alternatives`
  / `why` metadata as indented sub-lines (snippet-truncated), plus a `--walk`
  chain snippet when `caused_by` / `led_to` edges exist. Previously emitted
  only `id` / `target` / `created_at` / `why`, so the mode read as a bare
  index rather than a rationale-bearing view.
- **`thread` human renderer polished** — plain-text `thread` output now leads
  with an entry-count / depth / token-budget header, renders each entry's
  decision metadata (`source` / `sequence` / `chosen` / `rationale` /
  `alternatives`) inline when the entry is a `Decision` (same style as
  `chat --only-decisions`), lists chain edges literally as
  `from ──caused_by/led_to──> to` with named endpoints instead of a bare
  edge count, and drops the redundant snippet when it repeats an entity's
  name. `--json` unchanged. Previously a flat id-ordered list with a
  `_N chain edge(s)_` footer, so chain structure and rationale were invisible.
- **Repo no longer publishes machine-specific `.claude/` shims.** The three
  repo-local wrapper scripts (`.claude/hooks/verify-rust.fish`,
  `.claude/hooks/pre-commit-rust-standards`, `.claude/scripts/quality-gate.sh`)
  were personal delegates into `/home/<user>/Projects/.claude/...` with no
  portable target. They are untracked and `.claude/` is gitignored, so the
  public repo no longer exposes a developer's home paths. The maintainer
  checklist in `MANUAL.md` now points at the published `cargo fmt / clippy /
  test` gate instead. The `plugin/atheneum-decisions/` companion plugin was
  already portable (`${CLAUDE_PLUGIN_ROOT}`, `$ATHENEUM_DB`, `$HOME`,
  `$CLAUDE_CODE_SESSION_ID`) and ships unchanged.
- **Lexical ranking now applies a first-pass provenance-aware reranker.**
  Results are still seeded by token overlap, but mixed-kind ranking now boosts
  `WikiPage`, `Discovery`, and canonical project-doc patterns
  (`*-architecture.md`, `*-capabilities.md`, `*-cli-reference.md`) while
  demoting weaker support entities such as `File`, `Event`, and
  architecture-irrelevant `ReasoningLog` hits. Architecture/capabilities-style
  queries also penalize `CHANGELOG`, `Kanban`, and low-signal untitled pages
  so broad navigation favors authoritative project docs over operational or
  transcript exhaust.
- **Workspace verification now runs from the repo root without path drift.**
  Root `deny.toml`, `.gitleaks.toml`, and `.semgrep/rules/` files were added so
  `cargo deny check`, `gitleaks detect --config .gitleaks.toml`, and
  `semgrep ci --config .semgrep/rules/` all execute against the workspace from
  the same directory as `Cargo.toml`, instead of depending on crate-local paths
  or missing config files.
- **`semantic-search` docs now describe the real boundary.** HNSW is now
  documented as an opt-in human fuzzy-search candidate index, while the
  default grounded retrieval path is graph traversal plus typed SQL payload
  queries with no vector index required.

### Fixed

- **CLI rejects flag-looking positionals** — subcommand arms historically read
  raw positionals (`PathBuf::from(&args[2])`, `&args[3]`, …) without checking
  whether the value started with `-`, so a bare flag in a positional slot was
  silently accepted as the value. `atheneum init --help` created a real SQLite
  file named `--help` in the cwd. A central `positional` / `optional_positional`
  guard now fails fast with `expected positional <name>, got flag-looking
  argument '<x>'` across every subcommand (init, sync-*, query-*, entity/edge/
  neighbors, navigate/thread/search(-wiki), store-discovery, add-edge,
  task-create/update/done/archive, query-knowledge, memory-store/get,
  meta-register, cross-search/navigate, extract-decisions). Required slots
  error on a missing or flag-looking value; optional slots not followed by the
  option parser (sync-* project-id, sync-claude-transcript project/agent) error
  on a flag-looking value; optional slots followed by the option parser keep
  the existing flag-means-absent behavior. `--json` and the option-parsing path
  are unchanged.
- **Search regression coverage for mixed wiki corpora.** `semantic_search_tests`
  now includes benchmark-style ranking cases that model the real Atheneum wiki
  mix: canonical `WikiPage` beats transcript/file-style shadows, architecture
  pages outrank changelog-style pages for architecture queries, and a small
  hand-built mixed corpus exercises the demotion rules for low-signal untitled
  pages. This locks the new reranker against the exact navigation failures that
  showed up in the live 649-page Atheneum wiki.
- **`compose_memory_bootstrap` is clippy-clean again.** The graph-aware memory
  bootstrap path now uses iterator flattening for `rusqlite` row iteration,
  factors the scored memory tuple into local type aliases, and replaces manual
  token math with `div_ceil()`. This removes the repo’s `cargo clippy
  --all-targets -D warnings` failures without changing bootstrap behavior.
  A table-driven benchmark asserts expected top hits for
  `rocmforge capabilities`, `atheneum architecture`, `constraint design`, and
  `sovereign llm platform`.

## [0.9.2] - 2026-06-24

### Added
- `compose_memory_bootstrap`: BFS graph traversal (depth-2, led_to/caused_by/related_to edges) boosts graph-connected memories by +2.0 in ranking
- `compose_memory_bootstrap`: `graph_connected` count field in JSON output; each memory entry includes `graph_connected: bool`

## [0.9.1] - 2026-06-24

### Added
- `compose_memory_bootstrap`: graph-aware relevance scoring via recent discovery targets
- `compose_memory_bootstrap`: `relevance_context` field in JSON output (top 10 focus terms)
- `discoveries` table: `context_snapshot TEXT` column (migration v13) for decision-with-context capture
- `store_discovery`: accepts optional `context_snapshot` field in metadata

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
atheneum meta-register envoy /path/to/envoy \
  /path/to/envoy/.magellan/magellan.db --language rust

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

### fix(mcp): navigate noise + memory_bootstrap field rename

navigate: filter metadata edge types (belongs_to_project, accessed, modified,
observed_in, created_in_session) from output. Shape entities to kind+name
only (was full data blob). Edge dump went from 82 noisy edges to 0 (all
were metadata). Readable entity summaries replace raw dumps.

memory_bootstrap: renamed field `graph_connected` to
`memories_graph_connected`. The old name was ambiguous — it looked like a
connectivity health check but actually counts how many returned memories
are connected to Decision/Discovery entities via causal/related edges.

Verified:
- navigate(sqlitegraph native-v3, k=3, d=1): 18 entities, 0 noise edges
- memory_bootstrap(Projects, 500): memories_graph_connected=0 (clear semantics)
- Tests: 76 lib + 3 integration, all pass

### fix(mcp): navigate noise filter + query_knowledge negative token savings

navigate: added NOISE_ENTITY_KINDS filter (ToolCall, ReasoningLog, TestRun)
and handled_by_tool to metadata edge types. ToolCall entities are 84% of
the graph (121,590 of 144k) and flood BFS traversal. At depth=2, 355,343
noise entities filtered (96%). Signal entities now Session/File/Project/
Agent only. Output includes noise_filtered count.

query_knowledge: token_savings.saved was -15000 when no discoveries match.
Root cause: agent_count=0 → without_sharing=0, with_sharing=15000 (default
file token estimate), saved = 0 - 15000 = -15000. Fix: guard the entire
calculation — when agent_count=0 or discoveries empty, report all zeros.
Fixed in both query_knowledge and query_knowledge_in_project.

Verified: saved=0 (was -15000). 76 lib tests pass.
