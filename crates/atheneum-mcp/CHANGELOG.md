# Changelog

All notable changes to `atheneum-mcp`.

Format: Keep a Changelog. Versions: `major.minor.patch`.

---

## [Unreleased] — Pending

> **Status: NOT PRODUCTION READY.**  
> This MCP server compiles and passes protocol-level tests, but has **never been verified end-to-end against a real Atheneum graph** via the MCP protocol.

### What Works (Verified)

| Item | Evidence | Notes |
|------|----------|-------|
| Compiles with `--features http` | `cargo check --all-targets` | Default feature set |
| Compiles with `--features direct` | `cargo check --all-targets --features direct` | Requires `atheneum` crate |
| MCP protocol handshake | `tests/integration_test.rs` | Mock backend only |
| Tool registration (9 tools) | Unit test `all_nine_tools_registered` | |
| Tool schema validation | Unit test `get_tool_by_name` | |
| Server info response | Unit test `server_info_is_correct` | |

### What Is Implemented But NOT Verified Against Real Data

> These backends have code but **no integration test exercises them with a real `AtheneumGraph`**.

#### HTTP Backend (default)

| Method | Status | Risk |
|--------|--------|------|
| `store_discovery` | Calls envoy `/atheneum/discoveries` | **UNTESTED** — payload structure inferred from envoy source, not verified live |
| `query_knowledge` | Calls envoy `/atheneum/knowledge` | **UNTESTED** — no live envoy in test suite |
| `search` | Calls envoy `/atheneum/search` | **UNTESTED** — no live envoy in test suite |
| `store_memory` | Returns hard error | **By design** — memory does not go through envoy; use direct backend |
| `query_memory` | Returns hard error | **By design** — memory does not go through envoy; use direct backend |
| `list_sessions` | Calls envoy `/atheneum/sessions` | **UNTESTED** — no live envoy in test suite |
| `list_events` | Calls envoy `/atheneum/events` | **UNTESTED** — no live envoy in test suite |
| `navigate` | Calls envoy `/atheneum/graph/navigate` | **UNTESTED** — no live envoy in test suite |
| `graph_stats` | Calls envoy `/atheneum/graph/stats` | **UNTESTED** — no live envoy in test suite |

#### Direct Backend (`--features direct`)

| Method | Status | Risk |
|--------|--------|------|
| `store_discovery` | Calls `graph.store_discovery_in_project()` | **UNTESTED** — implemented but never run against real graph via MCP protocol |
| `query_knowledge` | Calls `graph.query_knowledge()` | **UNTESTED** |
| `search` | Calls `graph.lexical_search()` | **UNTESTED** |
| `store_memory` | Calls `graph.store_memory()` | **UNTESTED** |
| `query_memory` | Calls `graph.query_memory()` | **UNTESTED** |
| `list_sessions` | Calls `graph.query_sessions()` | **UNTESTED** |
| `list_events` | Calls `graph.query_events()` | **UNTESTED** |
| `navigate` | Calls `graph.navigate()` | **UNTESTED** |
| `graph_stats` | Calls `graph.runtime_stats()` | **UNTESTED** |

### Known Issues / TODO

1. **No end-to-end test with real graph**  
   The integration tests use a `MockBackend` that returns hardcoded JSON. There is no test that spins up `AtheneumGraph::open_in_memory()`, connects an MCP client over a duplex stream, calls a tool, and verifies the graph mutated.

2. ~~`store_memory` tags are dropped~~ **FIXED in atheneum v0.3.3**  
   The atheneum `store_memory()` API now accepts `tags: Option<&[String]>`. The MCP direct backend passes tags through. Call sites updated across tests and production code.

3. **No error propagation contract**  
   Backend errors are converted to `CallToolResult::error()` with the message as text. There is no structured error format, and clients cannot distinguish "graph not found" from "invalid params".

4. **Direct backend uses `tokio::task::block_in_place`**  
   This is a temporary bridge because `AtheneumGraph` methods are synchronous. It works but ties up a Tokio worker thread per call. Long-term, the graph should expose async methods or the backend should use a dedicated blocking pool.

5. **Tool schemas are hand-written JSON**  
   No `schemars` derivation means schemas can drift from the actual parameter structs (`StoreDiscoveryParams`, `StoreMemoryParams`) without compile-time detection.

6. **HTTP backend lacks memory endpoints**  
   `store_memory` and `query_memory` always return errors in HTTP mode. The envoy bridge needs memory route handlers, or the MCP server needs to document that memory operations require direct mode.

7. **No graceful shutdown**  
   The server drops the stream to terminate. There is no `exit` signal or lifecycle hook.

8. **No logging / tracing integration**  
   `tracing` is a dependency but nothing is instrumented.

### Blockers Before Release

- [ ] Write an integration test that uses a real `AtheneumGraph` (in-memory) and exercises each tool via the MCP protocol.
- [ ] Verify HTTP backend against a running envoy instance (or document that it requires envoy).
- [ ] Decide on `store_memory` tags: either update atheneum API or remove tags from tool schema.
- [ ] Add structured error responses.
- [ ] Test with at least one real MCP client (Claude Desktop, Cline, etc.).

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
