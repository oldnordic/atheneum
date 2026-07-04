# SPEC: Atheneum MCP Server — Full Tool Surface + Direct Default

**Date:** 2026-07-03
**Status:** Planning
**Author:** Hermes Agent (per user direction)

## Problem

The atheneum-mcp binary exists but has two problems:

1. **Wrong default backend.** It defaults to `http` (connects to envoy at
   localhost:9876). But memory/wiki/decision tools have NO HTTP endpoints in
   envoy — they error with "requires direct backend." The default should be
   `direct` (links the atheneum crate, opens the DB directly).

2. **Incomplete tool surface.** Only 9 of ~40 CLI commands are exposed as MCP
   tools. Missing: wiki_search, decision_search, thread, memory_list,
   memory_bootstrap, discoveries_recent, query_wiki, session_digest,
   query_knowledge (has it but with wrong params), and more.

The goal: make atheneum-mcp a complete, direct-stdio MCP server exposing the
full atheneum CLI surface, defaulting to direct backend.

## Architecture (existing, no merge)

Separation of concerns is preserved:
- `crates/atheneum` = library (all graph logic, no MCP)
- `crates/atheneum-mcp` = thin MCP adapter (rmcp SDK + tool definitions + backend trait)

The MCP crate links atheneum as a path dep (the `direct` feature). No logic
moves into the MCP crate — it only calls existing public methods on
`AtheneumGraph` and returns JSON.

## Crate

`rmcp = { version = "1.7", features = ["server", "transport-io"] }` — already
the dependency. This is the official Rust MCP SDK
(github.com/modelcontextprotocol/rust-sdk). No other crate needed.

## Current State (verified from source)

- `src/lib.rs`: `AtheneumMcpServer` struct, `ServerHandler` impl, `ToolRouter`.
  Server info, protocol version V_2025_03_26. Works.
- `src/backend.rs`: `Backend` trait (9 methods). HTTP impl (calls envoy).
  Direct impl (links atheneum, uses `tokio::task::block_in_place`).
- `src/tools.rs`: 9 tools registered. Manual JSON Schema (no schemars dep).
  Pattern: `ToolRoute::new_dyn(Tool::new(...), closure)`.
- `src/main.rs`: hardcodes HTTP backend. No DB path arg. No env for direct mode.

## Target Tool Surface (20 tools)

### Group 1: MEMORY (6 tools) — all use direct backend
| Tool | Atheneum method | CLI command |
|------|----------------|-------------|
| `store_memory` | `store_memory(key, content, scope, confidence, project, tags)` | memory-store |
| `search_memory` | `lexical_search(query, k, project, None, None)` | search --kind Memory |
| `list_memory` | `list_memory_page(scope, project, offset, limit)` | memory-list |
| `memory_bootstrap` | `compose_memory_bootstrap(project, tokens, last_sessions)` | memory-bootstrap |
| `query_wiki` | `get_wiki_page(path)` | query-wiki |
| `wiki_search` | `search_wiki_pages(query, project, offset, limit)` | wiki-search |

### Group 2: KNOWLEDGE & DISCOVERIES (5 tools)
| Tool | Atheneum method | CLI command |
|------|----------------|-------------|
| `search` | `lexical_search(query, k, project, kind, max_tokens)` | search |
| `navigate` | `navigate(query, k, depth, project, kind, max_tokens)` | navigate |
| `query_knowledge` | `query_knowledge_in_project(target, project, max_tokens)` | query-knowledge |
| `discoveries_recent` | `recent_discoveries(project, agent, session, dtype, limit)` | discoveries-recent |
| `decision_search` | `search_decisions(query, project, limit)` | decision-search |

### Group 3: DECISION CHAINS & SESSIONS (4 tools)
| Tool | Atheneum method | CLI command |
|------|----------------|-------------|
| `thread` | `thread_query(query, k, depth, project, max_tokens)` | thread |
| `session_digest` | `compose_session_digest(project, last_sessions, tokens)` | session-digest |
| `list_sessions` | `query_sessions(project, limit, offset)` | query-sessions |
| `list_events` | `query_events(session, event_type, limit)` | query-events |

### Group 4: STORE & GRAPH (5 tools)
| Tool | Atheneum method | CLI command |
|------|----------------|-------------|
| `store_discovery` | `store_discovery_in_project(agent, dtype, target, project, metadata)` | store-discovery |
| `graph_stats` | `count_entities_by_kind()` + `count_edges_by_type()` | graph-stats |
| `get_entity` | `get_entity(id)` | entity |
| `get_neighbors` | `outgoing_edges(id)` + `incoming_edges(id)` | neighbors |
| `dream` | `dream(scope, project, dry_run)` | dream |

Total: 20 tools (up from 9).

## Phased Implementation

### Phase 1: Flip default to direct, fix main.rs entry point
**Scope:** Make `direct` the default feature. main.rs opens the DB directly
(ATHENEUM_DB env or --db arg), no envoy dependency.

**Changes:**
- `Cargo.toml`: `default = ["direct"]` (was `["http"]`)
- `main.rs`: read `ATHENEUM_DB` env (default
  `~/.magellan/atheneum/atheneum.db`), open `AtheneumGraph`, wrap in
  `Arc<tokio::sync::Mutex>`, create `DirectBackend`, serve via stdio.

**Verify:** `cargo build --bin atheneum-mcp` compiles. `atheneum-mcp` starts
and responds to MCP initialize over stdio.

### Phase 2: Expand Backend trait with new methods
**Scope:** Add trait methods for the 11 new operations. Implement in
DirectBackend only (HTTP backend stays as-is for envoy mode, returns
"not supported" for new methods).

**New trait methods:**
```
search_memory(query, k, project) -> Value
list_memory(scope, project, offset, limit) -> Value
memory_bootstrap(project, tokens, last_sessions) -> Value
query_wiki(path) -> Value
wiki_search(query, project, limit) -> Value
discoveries_recent(project, agent, session, dtype, limit) -> Value
decision_search(query, project, limit) -> Value
thread(query, k, depth, project, tokens) -> Value
session_digest(project, last_sessions, tokens) -> Value
get_entity(id) -> Value
get_neighbors(id) -> Value
dream(scope, project, dry_run) -> Value
```

**Verify:** `cargo check -p atheneum-mcp --features direct` compiles.

### Phase 3: Register 11 new tools in tools.rs
**Scope:** Add tool definitions + handler closures for each new method.
Follow the existing pattern (manual JSON Schema, ToolRoute::new_dyn).

**Verify:** Unit test `all_twenty_tools_registered` passes. Each tool has
correct name, description, and schema.

### Phase 4: Integration test — stdio round-trip
**Scope:** Write a test that spawns the MCP server over stdio (tokio
duplex), sends `initialize` → `tools/list` → verifies 20 tools → calls
`graph_stats` → verifies JSON response.

**Verify:** `cargo test -p atheneum-mcp` passes.

### Phase 5: Install + wire to Hermes config + verify live
**Scope:**
- `cargo build --release --features direct -p atheneum-mcp`
- Install to `~/.local/bin/atheneum-mcp`
- Add to Hermes config as a native MCP server (stdio transport)
- Restart Hermes, verify the tools appear and `graph_stats` returns data

**Verify:** The agent can call `wiki_search`, `decision_search`, `thread`
via the MCP server and get real results from the DB.

## Non-goals

- No merging atheneum logic into atheneum-mcp (separation of concerns)
- No HTTP removal (envoy mode stays as `--features http`)
- No new library methods in atheneum crate (all methods already exist)
- No schemars dependency (manual JSON Schema, same as existing tools)
- No web UI, no dashboard (separation of concerns)

## Quality gates

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets -p atheneum-mcp -- -D warnings`
- `cargo test -p atheneum-mcp`
- `cargo build --release -p atheneum-mcp --features direct`
- E2E: server starts, responds to initialize, lists 20 tools, executes a tool
