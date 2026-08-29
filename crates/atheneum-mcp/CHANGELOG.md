# Changelog

All notable changes to `atheneum-mcp`.

Format: Keep a Changelog. Versions: `major.minor.patch`.

---

## [0.8.0] — 2026-08-29

### Added

- **Grounded Claims MCP Tools** (`src/tools.rs`, `src/backend.rs`):
  - Added `pin_grounded_claim` tool to pin falsifiable claims (linking memories to file paths, symbol names, and SHA256 AST/content hashes).
  - Added `audit_claims` tool to audit all grounded claims for a project, reporting total, verified, stale, and invalid claims along with stale entity IDs.
  - Implemented across `DirectBackend`, `HttpBackend`, and `MockBackend`.
  - Expanded total MCP tool count to 34 tools.

## [0.7.0] — 2026-08-24

### Added

- **Retrieval Bounding in `navigate` Tool** (`src/tools.rs:492-540`, `src/backend.rs:325-400, 1040-1120`, commit `088e02b`):
  - Added `edge_limit` parameter (default 50) to cap edge count per subgraph view.
  - Excludes `wikilink` edges by default to avoid flooding results; opt-in with `include_wikilinks: true`.
  - Added `budget` parameter (default 8192 bytes) bounding total serialized response size, with `"truncated": true` marker when truncated.
- **UTF-8 Safe Body Pagination in `query_wiki` Tool** (`src/tools.rs:735-765`, `src/backend.rs:1619-1650`, commit `088e02b`):
  - Added `offset` (default 0) and `limit` (default 8192 bytes) parameters to `query_wiki`.
  - Responses include `offset`, `limit`, `total_bytes`, `truncated`, and `has_more` indicators with UTF-8 char boundary safety.
- **Unknown-Project Guidance in MCP Tool Handlers** (`src/backend.rs:880-920`, commit `088e02b`):
  - `session_digest`, `task_list`, and `discoveries_recent` tool handlers return `unknown_project: true`, `known_projects: [...]`, and a guidance `hint` when the requested project name matches no recorded entities in the database.

## [0.6.0] — 2026-07-31

The Unified Tool API release: `atheneum-mcp` becomes the single MCP front
door for both the atheneum knowledge graph and the magellan/llmgrep/mirage
code-intelligence stack, plus an envoy coordination passthrough — 32 tools
total, with a shared response envelope across every dispatch path. The full
inter-component contract is documented in
[`crates/atheneum-mcp/README.md`](./README.md).

### Added

- **`code_query` tool**: deep structural code queries resolved by project
  name. The project is resolved server-side through magellan's `meta.db`
  project registry (`CrossRouter::meta().get_project()`), then the request
  is dispatched as a subprocess to the `magellan`, `llmgrep`, or `mirage`
  CLI binary with the resolved `--db` path. Guardrails: a read-only
  subcommand allowlist per binary (mutating verbs rejected before spawn —
  `refresh` is the one sanctioned mutation path and has its own tool), a
  `--db`/`--db=`/`-d` override ban in caller-supplied `args`, and caller-
  visible errors that never contain raw subprocess stderr/stdout.
- **`event` tool**: envoy multi-agent coordination passthrough. Verbs
  `send` / `claim` / `heartbeat` / `create_dependency` map to envoy's HTTP
  bridge endpoints; the payload is forwarded as-is and the response is
  wrapped in the shared envelope. Base URL from `ENVOY_URL`, default
  `http://localhost:9876`; 5-second timeout.
- **`refresh` tool**: triggers `magellan refresh` for a resolved project —
  the code-side propagation step of the update path. `llmgrep`/`mirage`
  need no separate refresh since they read magellan's own db.
- **Shared response envelope** (`src/envelope.rs`): every unified dispatch
  path returns the same shape — `items[]`, `limit`, `cursor`, `has_more`,
  `code_stale`, `knowledge_stale`, `depth_clamped`, and
  `errors[]` (`{backend, code, message}`). Includes the pagination cursor
  codec (base64-wrapped `{backend, offset}` JSON, `DEFAULT_LIMIT` 20 /
  `MAX_LIMIT` 100, `DEFAULT_DEPTH` 2 / `MAX_DEPTH` 3), the provenance
  tri-tag (`EXTRACTED` / `INFERRED` / `AMBIGUOUS`), and the error-code
  constants `PROJECT_NOT_FOUND`, `BACKEND_UNAVAILABLE`, `PARSE_ERROR`,
  `TIMEOUT`.
- **Subprocess adapter** (`src/subprocess.rs`): `CodeQueryRunner` resolves
  binaries from `GROUNDED_BIN_DIR` (default `~/.local/bin`), spawns with a
  cleared environment, enforces a 10-second timeout, parses stdout as JSON
  with a `{"tool", "output"}` text fallback, and logs raw subprocess output
  server-side only.
- **Two-tier staleness signal**: `search` reports `code_stale` for the
  resolved project by running `magellan refresh --dry-run --output json`
  and checking the `updated`/`deleted`/`added` arrays — `None` means "not
  applicable to this call", not "checked and clean". `knowledge_stale` is a
  reserved envelope field for the knowledge-side signal.
- **Real end-to-end test** for `kind=all` search: a fixture meta.db with
  attached project dbs proves resolve → fan-out → merge → provenance
  tagging beyond mocks, alongside regression tests for the partial-failure,
  project-not-found, depth-clamp, and default-shape compatibility paths.

### Changed

- **`search`**: new optional `kind=knowledge|code|all` (default
  `knowledge`), `limit`, and `cursor` parameters. `kind=code` fans out
  across per-project magellan databases via the atheneum crate's
  `CrossRouter` (lazy, read-only `ATTACH DATABASE` against magellan's
  `meta.db` registry). `kind=all` merges knowledge and code results, each
  item tagged `provenance` + `source`; a backend that fails lands in
  `errors[]` while the surviving backend's items still return. The default
  call shape (no `kind`/`limit`/`cursor`) returns the same bare array as
  before — backward compatible.
- **`navigate`**: new optional `kind` parameter and a server-side depth
  clamp (requested depth is clamped to `MAX_DEPTH` 3 and reported via
  `depth_clamped` on the enveloped shape; the knowledge-only default shape
  is unchanged). `kind=code|all` walks per-project subgraphs via
  `CrossRouter::cross_navigate`.
- **Versioning**: `atheneum-mcp` no longer inherits
  `[workspace.package] version` — it now carries its own version field so
  its release line is independent of the workspace root and of the
  `atheneum` library's 0.12.x line.

### Fixed

- `code_query` error sanitization: subprocess failures surface only the
  tool label and exit status in `errors[]`; raw stderr/stdout is logged
  server-side only, never forwarded to the caller.
- `magellan score`'s hand-rolled `-d <path>` db-override shorthand is
  blocked alongside `--db`/`--db=`.
- `event` tests no longer mutate the process-global `ENVOY_URL`: the envoy
  base URL is injected through `event_impl_with_base(base_url, params)`
  (removes the unsafe env manipulation flagged by semgrep).
- `EnvoyVerb` parsing is a real `FromStr` impl instead of an
  `#[allow]`-suppressed ad-hoc match.
- The stock binary now configures `CrossRouter` at startup
  (`main.rs` direct mode) — code-side tools (`code_query`/`refresh`,
  `search`/`navigate kind=code|all`) no longer degrade to
  `BACKEND_UNAVAILABLE` when meta.db is present. If meta.db cannot be
  opened, the server logs a warning and falls back to the previous
  graph-only behavior.

---

## [0.5.0] — 2026-07-21

### Added

- **`update_memory`, `maintain`, `seed_memory` tools**: expose the new
  `atheneum` librarian primitives (patch-in-place memory updates,
  orphan/broken-link/contradiction repair, and the token-bounded
  concept-grouped knowledge-base summary).
- **`seed_memory` auto-injection**: `get_info`/`list_tools` now call
  `seed_memory` and fold the result into the server `instructions` field and
  the `navigate`/`query_memory`/`search` tool descriptions, so a connecting
  client sees what's in the knowledge base without an explicit call.
- **`list_models` tool**: exposes `discover_available_models` for local-model
  discovery ahead of model-dependent operations.
- **`dream_semantic` tool**: exposes `semantic_consolidation` for merging
  closely-related or redundant concepts.
- **`pin_entity` / `unpin_entity` tools**: mark entities as always-included
  in `seed_memory` and immune to cache eviction.

### Changed

- `query_memory` is now documented and schematized as an exact key lookup,
  matching the direct graph API and the existing integration tests.
- `store_memory` now accepts optional `key`, `scope`, and `project` fields
  instead of forcing every caller into a content-derived key with implicit
  `agent` scope.
- `query_memory` now accepts optional `scope` and `project` filters plus a
  deprecated `query` alias for backward compatibility.

### Fixed

- Direct-mode `atheneum-mcp` failed to start entirely
  (`schema error: database schema version N is newer than supported 6`)
  due to a stale `sqlitegraph` lockfile pin in the `atheneum` workspace —
  see the `atheneum` 0.11.0 changelog entry. Any MCP client configured to
  launch this server silently lost it from its tool list on every session.

---

## [0.3.2] — 2025-06-08

### Added
- Direct backend implementations for `store_discovery` and `store_memory` (previously returned "not yet implemented").

---

## [0.3.1] — 2025-06-08

### Fixed
- Tool schemas corrected: removed `offset` from `list_sessions`/`list_events`, added `depth` to `navigate`.

---

## [0.3.0] — 2025-06-08

### Added
- Integration tests using `tokio::io::duplex` for MCP protocol verification.
- 9 tools: `store_discovery`, `query_knowledge`, `search`, `store_memory`, `query_memory`, `list_sessions`, `list_events`, `navigate`, `graph_stats`.

---

## [0.2.0] — 2025-06-07

### Added
- `Backend` trait with HTTP and direct implementations.
- `ToolRouter` using `rmcp` v1.7.0.

---

## [0.1.0] — 2025-06-07

### Added
- Initial scaffold: `atheneum-mcp` crate in workspace.
