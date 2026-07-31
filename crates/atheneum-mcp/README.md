# atheneum-mcp

MCP server exposing the Atheneum agent memory / knowledge graph **and** the
magellan / llmgrep / mirage code-intelligence stack behind one unified tool
API — 32 tools over stdio, with a shared response envelope across every
dispatch path. Version 0.6.0 (`crates/atheneum-mcp/Cargo.toml:3`); the crate
carries its own version field rather than inheriting the workspace root's
(`Cargo.toml:6`), so its release line is independent of the `atheneum`
library's 0.12.x line (`crates/atheneum/Cargo.toml:3`).

The binary speaks the Model Context Protocol over stdio
(`crates/atheneum-mcp/src/main.rs:55`) and registers all 32 tool routes at
startup (`crates/atheneum-mcp/src/tools.rs:18-52`,
`crates/atheneum-mcp/src/lib.rs:25-29`). A unit test asserts the registered
count is exactly 32 (`crates/atheneum-mcp/src/lib.rs:356`).

## Inter-component data flow

`atheneum-mcp` is the single agent-facing MCP front door. Each tool call is
routed by the `Backend` trait (`crates/atheneum-mcp/src/backend.rs:192-262`)
to one of four downstream edges:

```
MCP client
   │  stdio (main.rs:55)
   ▼
atheneum-mcp ── Backend trait (backend.rs:192)
   │
   ├─ knowledge side ─ in-process AtheneumGraph (DirectBackend)
   │     backend.rs:638-649, main.rs:45-46
   │
   ├─ code side (search/navigate fan-out) ─ atheneum CrossRouter,
   │     lazy read-only ATTACH of per-project magellan DBs routed by
   │     magellan's meta.db registry
   │     atheneum/src/cross.rs:46-57, 125-161; atheneum/src/meta.rs:26-43
   │
   ├─ code side (code_query/refresh) ─ subprocess to magellan/llmgrep/
   │     mirage CLI binaries, read-only allowlist, no --db override
   │     backend.rs:1194-1326, 1572-1660; subprocess.rs:129-228
   │
   └─ coordination side (event) ─ HTTP passthrough to envoy
         ENVOY_URL, default http://localhost:9876
         backend.rs:268-305; events.rs:44-70
```

- **Knowledge side — in-process.** The default backend mode is `direct`
  (`ATHENEUM_BACKEND` env, default `"direct"`, `main.rs:20`), which opens
  the atheneum database with `atheneum::AtheneumGraph::open`
  (`main.rs:45`; path from `ATHENEUM_DB`, default
  `~/.hermes/atheneum/atheneum.db`, `main.rs:37-38`) and wraps it in a
  `DirectBackend` (`main.rs:46`, `backend.rs:686-690`). Every knowledge,
  memory, and graph tool then runs against the graph in-process behind a
  mutex, using `tokio::task::block_in_place` to bridge the synchronous
  graph API (e.g. `search` at `backend.rs:737-747`, `navigate` at
  `backend.rs:1055-1087`). An `http` mode (`ATHENEUM_BACKEND=http`,
  `main.rs:22-33`) forwards a subset of calls to the envoy HTTP bridge at
  `ATHENEUM_URL` (default `http://localhost:9876`, `main.rs:25-26`) via
  `HttpBackend` (`backend.rs:392-428`); memory/graph-mutation methods
  return a hard "requires direct backend" error there
  (`backend.rs:472-510`, `backend.rs:1668-1673`).

- **Code side — CrossRouter fan-out.** `search kind=code|all` and
  `navigate kind=code|all` dispatch through `atheneum::CrossRouter`
  (`backend.rs:782-824` for search, `backend.rs:1138-1180` for navigate).
  `CrossRouter` wraps magellan's `meta.db` project registry
  (`crates/atheneum/src/cross.rs:46-47`) — resolved as `$MAGELLAN_META_DB`,
  else `$HOME/.magellan/meta.db` (`crates/atheneum/src/meta.rs:37-43`) —
  and lazily `ATTACH`es each project's magellan database read-only under a
  generated schema name, evicting least-recently-used attachments past a
  cap of 8 by default (`crates/atheneum/src/cross.rs:13`,
  `crates/atheneum/src/cross.rs:125-161`). `cross_search`
  (`crates/atheneum/src/cross.rs:180-256`) then queries every attached
  project's `graph_entities` and merges the hits; `cross_navigate`
  (`crates/atheneum/src/cross.rs:265-293`) BFS-walks each hit's subgraph.
  Since atheneum 0.12.2 the router also always attaches the central
  atheneum knowledge store as a synthetic `__atheneum_central__` project,
  even though it is not registered in `meta.db`
  (`crates/atheneum/src/cross.rs:89-113`). The `DirectBackend` holds the
  router as an `Option` (`backend.rs:640`), populated by
  `DirectBackend::with_cross_router` (`backend.rs:661-671`); when no router
  is configured — which includes the stock `main.rs` wiring, that uses
  `direct_from_graph` (`main.rs:46`, `backend.rs:686-690`) — every
  code-side call degrades to a `BACKEND_UNAVAILABLE` entry in `errors[]`
  reading "no CrossRouter configured" (`backend.rs:830-835`,
  `backend.rs:1173-1178`, `backend.rs:1196-1202`, `backend.rs:1574-1580`),
  while knowledge-side results still return normally.

- **Code side — subprocess adapter.** `code_query` and `refresh` shell out
  to the `magellan` / `llmgrep` / `mirage` CLI binaries through
  `CodeQueryRunner` (`crates/atheneum-mcp/src/subprocess.rs:129-166`).
  Binaries resolve from `GROUNDED_BIN_DIR`, default `~/.local/bin`
  (`subprocess.rs:134-145`), spawn with a cleared environment
  (`subprocess.rs:162`), and are bounded by a 10-second timeout
  (`subprocess.rs:13`, `subprocess.rs:203-205`). Stdout is parsed as JSON
  with a `{"tool", "output"}` wrap as fallback for non-JSON output
  (`subprocess.rs:222-227`). Only read-only subcommands are reachable —
  per-binary allowlists at `subprocess.rs:54-126` — and caller-supplied
  `args` may not contain `--db`, `--db=…`, or `-d` (the db is resolved
  server-side; `backend.rs:1260-1280`). Raw subprocess stderr/stdout is
  logged server-side only and never forwarded into the caller-visible
  error (`subprocess.rs:207-219`, regression test at
  `subprocess.rs:330-355`).

- **Coordination side — envoy HTTP passthrough.** `event` POSTs the
  caller's payload as-is to envoy's HTTP bridge (`events.rs:57-69`), with
  the verb selecting the endpoint: `send` → `/messages/send`, `claim` →
  `/handoffs/claim`, `heartbeat` → `/agents/heartbeat`,
  `create_dependency` → `/dependencies` (`events.rs:19-28`). The base URL
  comes from `ENVOY_URL`, default `http://localhost:9876`
  (`backend.rs:268-271`), with a 5-second timeout (`events.rs:9`,
  `events.rs:60-62`). The implementation is shared verbatim by both backend
  modes since it is a separate envoy connection independent of which
  atheneum backend is active (`backend.rs:264-267`, `backend.rs:617-619`,
  `backend.rs:1565-1569`).

- **Refresh — index re-scan trigger.** `refresh` resolves the project's
  magellan db from `meta.db` (`backend.rs:1583-1604`) and runs
  `magellan refresh --db <resolved>` (`backend.rs:1618-1626`). It is the
  one sanctioned mutation path for the code index: `llmgrep` and `mirage`
  read magellan's own db, so they pick up changes automatically with no
  separate refresh (`tools.rs:1288`, `backend.rs:179-181`).

## The 32-tool surface

Grouped by function; registration order at `tools.rs:18-52`, count pinned
by the test at `lib.rs:356`. Descriptions paraphrase each tool's
registered description string (cited per row).

### Knowledge & discovery (8)

| Tool | Purpose | Definition |
|------|---------|------------|
| `store_discovery` | Store a discovery into the knowledge graph | `tools.rs:88` |
| `query_knowledge` | Query knowledge about a target entity | `tools.rs:136-137` |
| `search` | Search the knowledge graph and/or cross-project code index | `tools.rs:184` |
| `navigate` | Navigate the knowledge graph using natural language | `tools.rs:514-516` |
| `query_wiki` | Fetch a wiki page by path (partial matching supported) | `tools.rs:739-740` |
| `wiki_search` | Full-text search over wiki pages via FTS5 | `tools.rs:769-770` |
| `discoveries_recent` | List recent discoveries, optionally filtered | `tools.rs:807-808` |
| `decision_search` | Search decisions by target/chosen/why substring | `tools.rs:852-853` |

### Memory (9)

| Tool | Purpose | Definition |
|------|---------|------------|
| `store_memory` | Store an episodic memory entry | `tools.rs:226` |
| `update_memory` | Patch an existing memory entry in place | `tools.rs:274-275` |
| `add_memory` | Add a fact to a concept (append-or-create) | `tools.rs:332-333` |
| `query_memory` | Query episodic memory by exact key lookup | `tools.rs:393-394` |
| `search_memory` | Lexical search over Memory-kind entities only | `tools.rs:634-635` |
| `list_memory` | List stored memories (paginated) | `tools.rs:671` |
| `memory_bootstrap` | Compose a bounded bootstrap packet: memories + session digest | `tools.rs:704-705` |
| `dream` | Reflective memory consolidation pass (dedup/stale/verbose detection) | `tools.rs:1015-1016` |
| `dream_semantic` | Consolidate similar redundant concepts via a local model or lexical trigram fallback | `tools.rs:1148-1149` |

### Graph & sessions (7)

| Tool | Purpose | Definition |
|------|---------|------------|
| `graph_stats` | High-level knowledge graph statistics | `tools.rs:601-603` |
| `get_entity` | Fetch a single graph entity by ID | `tools.rs:962` |
| `get_neighbors` | Get outgoing + incoming edges for an entity | `tools.rs:986-987` |
| `thread` | Walk a decision chain (caused_by/led_to edges) | `tools.rs:891-892` |
| `session_digest` | Compose a bounded session digest from recent sessions | `tools.rs:928-929` |
| `list_sessions` | List recorded agent sessions | `tools.rs:444` |
| `list_events` | List recorded events | `tools.rs:473` |

### Code intelligence (2)

| Tool | Purpose | Definition |
|------|---------|------------|
| `code_query` | Deep structural code query via magellan/llmgrep/mirage, resolved by project name | `tools.rs:559-561` |
| `refresh` | Refresh a project's code index (`magellan refresh`) | `tools.rs:1286-1288` |

### Ops & coordination (6)

| Tool | Purpose | Definition |
|------|---------|------------|
| `event` | Envoy multi-agent coordination passthrough (send/claim/heartbeat/create_dependency) | `tools.rs:1249-1251` |
| `maintain` | Database health checks and automatic repairs | `tools.rs:1052-1053` |
| `seed_memory` | Compact seed summary of instructions, active concepts, recent memories | `tools.rs:1089-1090` |
| `list_models` | List all loaded local models from the Ollama or llama.cpp endpoint | `tools.rs:1119-1120` |
| `pin_entity` | Pin a concept or memory against eviction and prioritize it in seeding | `tools.rs:1187-1188` |
| `unpin_entity` | Unpin a previously pinned concept or memory | `tools.rs:1215-1216` |

## The five new/changed tools in detail

### `search` — extended with `kind`, `limit`, `cursor`

Parameters (`tools.rs:163-181`): `query` (required), `k` (1–100, default
10 — candidate count before pagination), `project` (optional scope),
`kind` (`knowledge` | `code` | `all`, default `knowledge`), `limit`
(1–100, default 20 — page size), `cursor` (opaque, from a previous call's
response).

- `kind` is parsed by `SearchKind::from_str_default`
  (`backend.rs:130-138`); the enum is defined at `backend.rs:121-128`.
- `kind=knowledge` runs the atheneum store's lexical search in-process
  (`backend.rs:753-763`); each hit is tagged `provenance: "INFERRED"` and
  `source: "knowledge"` (`backend.rs:766-772`).
- `kind=code` fans out through `CrossRouter::cross_search` across every
  attached per-project magellan db (`backend.rs:782-788`); each hit is
  tagged `provenance: "EXTRACTED"` and `source: "code"`
  (`backend.rs:805-816`). When `project` is passed, code results are
  filtered down to that project after the fan-out, because `cross_search`
  itself has no per-project scoping (`backend.rs:791-804`).
- `kind=all` does both and merges into one `items[]` with per-item
  provenance/source tags (merged-shape regression test at
  `backend.rs:2051-2073`). If one backend fails, its failure lands in
  `errors[]` and the surviving backend's items still return
  (partial-failure test at `backend.rs:2075-2103`) — including the case
  where no `CrossRouter` is configured at all (`backend.rs:825-836`).
- Pagination: `limit` is clamped to 1–100 with default 20
  (`envelope.rs:9-10`, `envelope.rs:65-67`); the merged items are paged
  server-side, `has_more` and a `cursor` for the next page are set when
  more remain (`backend.rs:868-888`).
- **Default shape unchanged**: a call with no `kind`/`limit`/`cursor`
  returns the pre-0.6.0 bare JSON array from `lexical_search` — no
  envelope, no tags (`backend.rs:726-748`; regression test
  `search_without_kind_defaults_to_knowledge_only_unchanged_shape` at
  `backend.rs:2013`).

### `navigate` — extended with `kind` + server-side depth clamp

Parameters (`tools.rs:492-510`): `query` (required), `k` (1–50, default
10 — entry-point count), `depth` (default 2 — BFS depth), `offset` /
`limit` (per-view entity pagination), `trace` (record a QueryTrace
entity), `kind` (`knowledge` | `code` | `all`, default `knowledge`).

- Depth is **always** clamped server-side to `MAX_DEPTH` 3 regardless of
  kind — an unbounded BFS is a real cost even on the knowledge-only path
  (`backend.rs:1041-1046`; `clamp_depth` at `envelope.rs:69-73`,
  constants at `envelope.rs:11-12`). On the enveloped shape the clamp is
  reported as `depth_clamped: true` (`backend.rs:1091-1092`; test at
  `backend.rs:2205`).
- `kind=knowledge` (the default) returns the pre-0.6.0 shape — a bare
  array of paginated subgraph views, or `{subgraphs, trace_id}` when
  `trace=true` — with no envelope and no provenance tags
  (`backend.rs:1048-1088`; regression test at `backend.rs:2111`). The
  default path's traversal is still bounded by the clamped depth (test at
  `backend.rs:2134`).
- `kind=code|all` returns the envelope: knowledge views tagged
  `source: "knowledge"` with `provenance: "INFERRED"` at depth ≤ 1 and
  `"AMBIGUOUS"` beyond (`backend.rs:1118-1123`); code subgraphs from
  `CrossRouter::cross_navigate` tagged `source: "code"` with
  `provenance: "EXTRACTED"` at depth ≤ 1 and `"AMBIGUOUS"` beyond
  (`backend.rs:1143-1158`).
- `code_stale` is intentionally `None` on `navigate`: `NavigateParams`
  carries no `project` field, and `cross_navigate` fans out across every
  attached project, so no single index exists whose staleness one boolean
  could represent (`backend.rs:1182-1188`).

### `code_query` — new: subprocess code-intelligence passthrough

Parameters (`tools.rs:545-556`): `project` (required — project name,
resolved via magellan's project registry), `tool` (required —
`magellan` | `llmgrep` | `mirage`), `subcommand` (required), `args`
(optional extra CLI arguments beyond `--db`).

Dispatch order in `backend.rs:1194-1326`:

1. A `CrossRouter` must be configured, else `BACKEND_UNAVAILABLE`
   (`backend.rs:1196-1202`).
2. `project` resolves to a magellan db path via
   `cross.meta().get_project()` (`backend.rs:1205-1207`); unknown project
   → `PROJECT_NOT_FOUND` (`backend.rs:1209-1215`), registry error →
   `BACKEND_UNAVAILABLE` (`backend.rs:1217-1223`).
3. `tool` maps to a `CodeTool` (`backend.rs:1228-1242`); unknown tool →
   `PARSE_ERROR`.
4. `subcommand` must be on the binary's read-only allowlist
   (`backend.rs:1244-1246`); anything else → `PARSE_ERROR` with a message
   pointing at the dedicated `refresh` tool as the mutation path
   (`backend.rs:1248-1257`). The allowlists — 32 magellan, 10 llmgrep,
   21 mirage subcommands — live at `subprocess.rs:54-126` with the
   exclusion rationale (mutating verbs, ambiguous verbs) documented at
   `subprocess.rs:31-53`.
5. `args` containing `--db`, `--db=…`, or `-d` are rejected with
   `PARSE_ERROR` — the db path is resolved server-side and prepended as
   `--db <resolved>` (`backend.rs:1260-1287`). The `-d` guard covers
   magellan `score`'s hand-rolled parser (comment at
   `backend.rs:1260-1267`).
6. The subprocess runs with a cleared environment and 10-second timeout
   (`subprocess.rs:160-166`, `subprocess.rs:203-205`); its JSON output is
   tagged `provenance: "EXTRACTED"`, `source: "code"` and pushed into
   `items[]` (`backend.rs:1292-1310`). Failures land in `errors[]` as
   `TIMEOUT` or `BACKEND_UNAVAILABLE` without raw stderr/stdout
   (`backend.rs:1311-1322`).

The response is an envelope with a single item (`backend.rs:1195`).

### `event` — new: envoy coordination passthrough

Parameters (`tools.rs:1237-1246`): `verb` (required — `send` | `claim` |
`heartbeat` | `create_dependency`), `payload` (required — verb-specific
object, forwarded as-is to envoy).

- The verb string parses via `FromStr` into `EnvoyVerb`
  (`events.rs:30-42`); an unknown verb returns the envelope with a
  `PARSE_ERROR` entry (`backend.rs:280-289`).
- The payload is POSTed unchanged to the verb's envoy endpoint
  (`events.rs:19-28`, `events.rs:57-69`); a successful response body is
  pushed into `items[]` (`backend.rs:292-293`).
- Failure mapping: timeout → `TIMEOUT`, anything else (connection
  refused, non-2xx) → `BACKEND_UNAVAILABLE`, always inside `errors[]`
  (`backend.rs:294-303`).
- Works identically under both backend modes (`backend.rs:264-267`,
  `backend.rs:617-619`, `backend.rs:1565-1569`).

### `refresh` — new: code-index re-scan trigger

Parameters (`tools.rs:1274-1283`): `project` (required — resolved via
magellan's project registry), `refresh_code` (default `true`).

- Resolution is identical to `code_query` (`backend.rs:1583-1604`), with
  the same `PROJECT_NOT_FOUND` / `BACKEND_UNAVAILABLE` mapping.
- `refresh_code=false` short-circuits to `{project, refreshed: false}`
  without spawning anything (`backend.rs:1606-1612`).
- Otherwise runs `magellan refresh --db <resolved>` via the subprocess
  adapter (`backend.rs:1614-1626`) and returns its output tagged
  `provenance: "EXTRACTED"`, `source: "code"` (`backend.rs:1627-1643`).
- Run it when `search` reports `code_stale: true` for the project (see
  staleness below). `llmgrep`/`mirage` need no separate refresh since
  they read magellan's own db (`tools.rs:1279`, `backend.rs:179-181`).

## The envelope contract

Every unified dispatch path — atheneum in-process, code-tool subprocess,
envoy HTTP — assembles the same response shape (`envelope.rs:1-3`), the
`Envelope` struct at `envelope.rs:34-44`:

```json
{
  "items": [ { "…": "result payload", "provenance": "EXTRACTED|INFERRED|AMBIGUOUS", "source": "code|knowledge" } ],
  "limit": 20,
  "cursor": "base64… | null",
  "has_more": false,
  "code_stale": null,
  "knowledge_stale": null,
  "depth_clamped": false,
  "errors": [ { "backend": "code", "code": "BACKEND_UNAVAILABLE", "message": "…" } ]
}
```

- **`items[]`** — result payloads. Unified tools tag each item with
  `provenance` and `source` (search: `backend.rs:766-772` and
  `backend.rs:805-816`; navigate: `backend.rs:1118-1123` and
  `backend.rs:1148-1158`; code_query/refresh: `backend.rs:1294-1309` and
  `backend.rs:1627-1643`).
- **`provenance`** — per-item honesty signal, the `Provenance` enum at
  `envelope.rs:19-25`, serialized uppercase:
  - `EXTRACTED` — deterministic extraction from the code backends:
    symbol/AST/call-graph data returned by magellan/llmgrep/mirage
    (`backend.rs:813`, `backend.rs:1297-1305`), and first-hop code
    traversal (`backend.rs:1156`).
  - `INFERRED` — knowledge-side inference: atheneum lexical-search hits
    (`backend.rs:768-769`) and first-hop knowledge traversal
    (`backend.rs:1118-1119`).
  - `AMBIGUOUS` — compounding uncertainty: any traversal beyond the first
    hop, on either side (`backend.rs:1120-1121`, `backend.rs:1156`).
- **`limit` / `cursor` / `has_more`** — pagination. `limit` echoes the
  clamped page size actually used (`DEFAULT_LIMIT` 20, `MAX_LIMIT` 100,
  `envelope.rs:9-10`, `envelope.rs:65-67`). When `has_more` is true,
  `cursor` carries an opaque base64-wrapped `{backend, offset}` JSON
  token (`Cursor` at `envelope.rs:75-79`, codec at `envelope.rs:81-90`);
  pass it back unchanged to resume — `search` decodes it and continues at
  the recorded offset (`backend.rs:868-888`). A garbage cursor decodes to
  `None` and simply restarts at offset 0 (`backend.rs:868-873`,
  `envelope.rs:86-90`, test at `envelope.rs:143-146`).
- **Two-tier staleness** — `code_stale` and `knowledge_stale` are
  separate nullable fields (`envelope.rs:40-41`) because the two backends
  go stale on different timescales via different mechanisms, and the
  cheap fix (`refresh`) shouldn't be conflated with the costly one.
  - `code_stale` is populated by `search` only, and only when a single
    project was resolved via the `project` parameter: the adapter runs
    `magellan refresh --dry-run --output json` and reports stale when any
    of the `updated` / `deleted` / `added` arrays is non-empty
    (`backend.rs:839-865`, `subprocess.rs:168-192`). `null` means "not
    applicable to this call", not "checked and clean"
    (`backend.rs:839-844`). When it comes back `true`, call `refresh`
    with the same `project`.
  - `knowledge_stale` is defined in the envelope (`envelope.rs:41`) but
    no dispatch path currently populates it; it is always `null`.
- **`depth_clamped`** — `true` when a requested `navigate` depth exceeded
  `MAX_DEPTH` 3 and was clamped (`envelope.rs:42`, `envelope.rs:69-73`,
  `backend.rs:1091-1092`). Out-of-range params clamp rather than error.
- **`errors[]`** — see the error contract below.

## Error contract

A caller never sees an exception from a unified tool: every failure
surfaces as an entry in `errors[]` beside whatever `items[]` did come
back — a partial answer beats a total failure (fan-out partial failure at
`backend.rs:774-779` and `backend.rs:818-823`, tested at
`backend.rs:2075-2103`). Each entry is an `EnvelopeError`
(`envelope.rs:27-32`):

| Field | Meaning |
|-------|---------|
| `backend` | Which dispatch edge failed: `"knowledge"`, `"code"`, or `"event"` (`backend.rs:775`, `backend.rs:819`, `backend.rs:295`) |
| `code` | One of the four constants below (`envelope.rs:14-17`) |
| `message` | Human-readable detail; never raw subprocess stderr/stdout (`subprocess.rs:207-219`) |

| Code constant | Value | Meaning | Raised at |
|---------------|-------|---------|-----------|
| `ERR_PROJECT_NOT_FOUND` | `PROJECT_NOT_FOUND` | The `project` name is not in magellan's `meta.db` registry — distinct from "resolved, zero matches" | `envelope.rs:14`; `backend.rs:1210-1214`, `backend.rs:1588-1592` |
| `ERR_BACKEND_UNAVAILABLE` | `BACKEND_UNAVAILABLE` | The backend is down or unconfigured: no CrossRouter, envoy unreachable/non-2xx, code-tool binary missing or crashed, knowledge-store error | `envelope.rs:15`; `backend.rs:774-778`, `backend.rs:1196-1202`, `backend.rs:294-300` |
| `ERR_PARSE_ERROR` | `PARSE_ERROR` | Caller input rejected before dispatch: unknown `tool`, disallowed `subcommand`, `--db` override attempt, unknown event `verb` | `envelope.rs:16`; `backend.rs:1233-1239`, `backend.rs:1248-1256`, `backend.rs:1273-1278`, `backend.rs:281-288` |
| `ERR_TIMEOUT` | `TIMEOUT` | Fixed per-backend budget exceeded: 10 s for code-tool subprocesses (`subprocess.rs:13`, `subprocess.rs:203-205`), 5 s for envoy HTTP (`events.rs:9`, `events.rs:60-62`) | `envelope.rs:17`; `backend.rs:296-299`, `backend.rs:1315-1317`, `backend.rs:1649-1651` |

## Backward compatibility

- `search` with no `kind`/`limit`/`cursor` returns exactly the pre-0.6.0
  bare array — the envelope is opt-in, only engaged when the caller asks
  for something the old shape can't express (`backend.rs:726-748`;
  regression test at `backend.rs:2013`). Errors on that path still
  propagate as tool errors exactly as before (`backend.rs:731-732`).
- `navigate` with default `kind=knowledge` returns exactly the pre-0.6.0
  shape (bare array, or `{subgraphs, trace_id}` with `trace=true`)
  (`backend.rs:1048-1088`; regression test at `backend.rs:2111`). The
  only behavioral change on the default path is that traversal depth is
  now bounded by the server-side clamp (default 2, max 3) instead of
  trusting the requested depth (`backend.rs:1041-1046`; test at
  `backend.rs:2134`) — and the legacy shape does not report the clamp, by
  design (`backend.rs:1052-1054`).
- The 27 pre-existing tools are untouched: their schemas and dispatch are
  unchanged, and all 32 routes are registered by the same
  `register_all` (`tools.rs:18-52`).

## Configuration

| Env var | Default | Effect | Reference |
|---------|---------|--------|-----------|
| `ATHENEUM_BACKEND` | `direct` | `direct` = in-process graph; `http` = envoy HTTP bridge | `main.rs:20-33` |
| `ATHENEUM_DB` | `~/.hermes/atheneum/atheneum.db` | atheneum database path (direct mode) | `main.rs:37-40` |
| `ATHENEUM_URL` | `http://localhost:9876` | envoy bridge base URL (http mode) | `main.rs:25-26` |
| `ENVOY_URL` | `http://localhost:9876` | envoy base URL for the `event` tool (both modes) | `backend.rs:268-271` |
| `GROUNDED_BIN_DIR` | `~/.local/bin` | directory holding the magellan/llmgrep/mirage binaries | `subprocess.rs:134-145` |
| `MAGELLAN_META_DB` | `$HOME/.magellan/meta.db` | magellan project-registry db used by `CrossRouter` | `crates/atheneum/src/meta.rs:37-43` |

## Build

Default features are `["direct"]` (`crates/atheneum-mcp/Cargo.toml:37-40`:
`default = ["direct"]`, `direct = ["dep:atheneum"]`, `http = []`). Build
with `cargo build -p atheneum-mcp`; run the binary over stdio and point
your MCP client at it (`main.rs:54-56`).
