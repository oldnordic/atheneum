# Cross-Project Query Guide

**Status:** Implemented in atheneum 0.5.0, envoy 0.1.1, magellan 4.7.3  
**Scope:** Query across magellan-indexed codebases from atheneum/envoy without copying data.

---

## What Problem This Solves

You have multiple codebases indexed by magellan. You want to ask questions like:

- "How is `build_router` implemented across all my Rust projects?"
- "Find error-handling patterns in my Go services."
- "Show me the call graph around `checkpoint` in every project that has it."

Before this feature, you had three bad options:

1. **Copy data** — Import magellan symbols into atheneum. Stale within minutes.
2. **Query each DB by hand** — Open five SQLite shells, run five queries, merge mentally.
3. **Don't do it** — Work in one project at a time and miss cross-project insights.

Now atheneum maintains a small routing registry (`meta.db`) that knows where every project's magellan database lives. When you run a cross-project query, atheneum lazily `ATTACH DATABASE` each magellan DB (read-only) and runs the query across all of them. No copying, no staleness.

---

## Architecture in Three Sentences

1. **`meta.db`** — A separate SQLite database (`~/.local/share/atheneum/meta.db`) that stores one row per project: name, root path, magellan DB path, atheneum DB path, language.
2. **Lazy `ATTACH`** — Cross-project queries look up candidate projects in `meta.db`, then `ATTACH` each magellan DB as a separate schema. The router keeps an LRU cache of attached schemas (default capacity 8) and auto-`DETACH`s old ones.
3. **Language filtering** — Every registered project has an optional `language` tag. Cross-project queries can filter by it, so a search for `build_router` only hits Rust projects.

---

## Prerequisites

1. You have magellan databases for the projects you want to query.
2. You have atheneum ≥ 0.5.0 installed.
3. Optionally, you have envoy ≥ 0.1.1 running if you want HTTP access.

---

## Step-by-Step: Register Projects

Register each project once. Re-registering the same name updates the record.

```bash
# Register a Rust project
atheneum meta-register envoy \
  /path/to/envoy \
  /path/to/envoy/.magellan/magellan.db \
  --language rust

# Register another Rust project
atheneum meta-register magellan \
  /path/to/magellan \
  /path/to/magellan/.magellan/magellan.db \
  --language rust

# Register a Go project
atheneum meta-register my-api \
  /path/to/my-api \
  /path/to/my-api/.magellan/magellan.db \
  --language go

# List registered projects
atheneum meta-list

# List only Rust projects
atheneum meta-list --language rust
```

**What happens:** Atheneum writes to `~/.local/share/atheneum/meta.db` (or the path in your `config.toml`). The meta.db schema is auto-created if missing.

---

## Step-by-Step: Cross-Project Search

Search for a symbol name across all registered projects (or only those matching a language).

```bash
# Search across ALL registered projects
atheneum cross-search "build_router" --k 10

# Search only Rust projects
atheneum cross-search "build_router" --language rust --k 10

# Search with no language filter, get 20 hits
atheneum cross-search "checkpoint" --k 20
```

**Output format:** JSON array of hits. Each hit has `project`, `id`, `kind`, `name`, `file_path`, and `data`.

```json
{
  "query": "build_router",
  "language": "rust",
  "k": 10,
  "count": 2,
  "results": [
    {
      "project": "envoy",
      "id": 42,
      "kind": "Symbol",
      "name": "build_router",
      "file_path": "src/http/router.rs",
      "data": { ... }
    },
    {
      "project": "magellan",
      "id": 91,
      "kind": "Symbol",
      "name": "build_router",
      "file_path": "src/cli/navigate.rs",
      "data": { ... }
    }
  ]
}
```

**What happens under the hood:**
1. Atheneum reads `meta.db` to find enabled projects matching the language filter.
2. For each project, it `ATTACH DATABASE` the magellan DB (or reuses an already-attached one from the LRU cache).
3. It runs `SELECT ... FROM attached_schema.graph_entities WHERE name LIKE '%query%'`.
4. It ranks exact name matches first, truncates to `k`, and returns JSON.
5. Missing or unreadable DBs are logged as warnings but do not abort the query.

---

## Step-by-Step: Cross-Project Navigate

Search for entry points, then BFS-walk each project's graph up to `depth` hops.

```bash
# Find "error handling" in Rust projects, walk 2 hops
atheneum cross-navigate "error handling" --language rust --k 5 --depth 2
```

**Output format:** One JSON subgraph view per entry point.

```json
{
  "query": "error handling",
  "language": "rust",
  "k": 5,
  "depth": 2,
  "count": 1,
  "views": [
    {
      "project": "envoy",
      "entry_id": 128,
      "entities": [ ... ],
      "edges": [ ... ]
    }
  ]
}
```

**What happens under the hood:** Same as `cross-search` for the entry-point lookup, then a per-project BFS through `graph_edges` within the attached schema.

---

## Step-by-Step: HTTP Access via Envoy

If you run `envoy` with the atheneum feature enabled:

```bash
# Cross-project search via HTTP
curl "http://localhost:9876/atheneum/cross/search?q=build_router&language=rust&k=10"

# Cross-project navigate via HTTP
curl "http://localhost:9876/atheneum/cross/navigate?q=error+handling&language=rust&k=5&depth=2"
```

Response shapes match the CLI JSON output.

---

## Configuration

Each tool has its own `config.toml` and stays standalone by default. Integration is opt-in.

### Atheneum (`~/.config/atheneum/config.toml`)

```toml
[atheneum]
db = "~/.local/share/atheneum/atheneum.db"
meta_db = "~/.local/share/atheneum/meta.db"

[integrations]
# These do NOT change atheneum's behavior today; they document intent for
# future auto-discovery. Cross-project queries work as long as meta.db is
# populated via `meta-register`.
[integrations.magellan]
enabled = false

[integrations.envoy]
enabled = false
```

### Magellan (`~/.config/magellan/config.toml`)

```toml
[integrations]
# These are documentation-of-intent today. Future magellan releases may
# use them to auto-export discoveries to atheneum or push status to envoy.
[integrations.atheneum]
enabled = false
db = "~/.local/share/atheneum/atheneum.db"
meta_db = "~/.local/share/atheneum/meta.db"

[integrations.envoy]
enabled = false
url = "http://localhost:9876"
```

### Envoy (`~/.config/envoy/config.toml`)

```toml
[atheneum]
enabled = true
db = "~/.local/share/atheneum/atheneum.db"
meta_db = "~/.local/share/atheneum/meta.db"
```

---

## Important Limits

| Limit | Value | Why |
|-------|-------|-----|
| Max attached DBs (SQLite default) | 10 | Hard SQLite limit |
| CrossRouter LRU cache default | 8 | Stays under the SQLite limit |
| CrossRouter LRU max | 125 | Safety clamp |
| Missing DB handling | Skipped with warning | One broken project should not break the query |
| Attach mode | Read-only | Magellan data is never modified by atheneum |

If you need more than 8 projects in a single query, increase the cache size:

```rust
let mut router = CrossRouter::with_capacity(10)?; // at the SQLite limit
```

Or compile SQLite with `SQLITE_MAX_ATTACHED=125`.

---

## Troubleshooting

**"No projects registered"**
→ Run `atheneum meta-list`. If empty, run `atheneum meta-register <name> <root> <magellan-db> [--language LANG]`.

**"Magellan database for project 'X' not found"**
→ The path in `meta.db` is stale. Re-register the project with the correct path.

**"Too many attached databases"**
→ You have >10 projects and SQLite is at its limit. Either increase the LRU cache size (up to 10) or filter by language to reduce the candidate set.

**Empty results when I know the symbol exists**
→ `cross-search` uses `name LIKE '%query%'`, not semantic similarity. Try the exact symbol name. Also verify the project is registered and its language matches your `--language` filter.

---

## Implementation History

This document was originally a design draft (2026-06-09). The following milestones have been completed:

| Milestone | Status | What was delivered |
|-----------|--------|-------------------|
| SQLite PRAGMA tuning + WAL checkpoint | ✅ Done (0.5.0) | `AtheneumGraph::open()` applies production settings; `checkpoint()` API |
| Prepared statement caching | ✅ Done (0.5.0) | Hot paths use `prepare_cached()` |
| In-memory entity ID lookup index | ✅ Done (0.5.0) | O(1) `(kind, name) → id` lookups |
| Batch write API | ✅ Done (0.5.0) | `batch_insert_entities`, `batch_insert_edges` |
| `meta.db` routing layer | ✅ Done (0.5.0) | `MetaRouter`, `meta-register`, `meta-list` CLI |
| Lazy `ATTACH` + LRU cache | ✅ Done (0.5.0) | `CrossRouter` with configurable cache |
| Cross-project search | ✅ Done (0.5.0) | `cross-search` CLI + `CrossRouter::cross_search()` |
| Cross-project navigate | ✅ Done (0.5.0) | `cross-navigate` CLI + `CrossRouter::cross_navigate()` |
| Language filtering | ✅ Done (0.5.0) | `--language` flag on both CLI commands |
| Envoy HTTP endpoints | ✅ Done (0.1.1) | `GET /atheneum/cross/search`, `GET /atheneum/cross/navigate` |
| Concise mode | ✅ Done (0.5.0) | `navigate --concise` for LLM context windows |
| Per-tool `config.toml` | ✅ Done (0.5.0 / 0.1.1 / 4.7.3) | XDG-compliant configs with `[integrations]` opt-in |
