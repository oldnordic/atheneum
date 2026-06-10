# Cross-Project Routing & HPC Optimization Plan

**Status:** Design draft — not yet implemented  
**Scope:** atheneum + envoy + magellan integration  
**Date:** 2026-06-09

---

## 1. Executive Summary

Magellan has spent years building HPC-grade code graph infrastructure: L3-aware batching, parallel parsing, WAL tuning, in-memory indexes, and a `meta.db` cross-project registry. Envoy (the HTTP layer above atheneum) has almost none of these. Atheneum sits in the middle — it stores agent memory and knowledge, but cannot yet "see" into magellan-indexed codebases without a one-way import bridge.

This plan proposes:

1. **A `meta.db` routing layer in atheneum** — a lightweight registry of all projects, their magellan `.db` paths, languages, and last-update timestamps. This becomes the "front door" for cross-project queries.
2. **SQLite `ATTACH`-based federation** — instead of copying magellan data into atheneum, atheneum attaches magellan databases on-demand and queries them in-place. No duplication, no staleness.
3. **Port HPC optimizations from magellan** — connection pooling, prepared-statement caching, batch transactions, WAL checkpointing, and in-memory lookup indexes.
4. **Language-filtered cross-project hopgraph** — navigate from a concept in one codebase to analogous concepts in another, filtered by language.

---

## 2. What Magellan Has That We Lack

### 2.1 HPC / Performance Optimizations

| Optimization | Magellan | Envoy | Atheneum | Priority |
|-------------|----------|-------|----------|----------|
| **WAL mode + sync NORMAL** | ✓ (`src/graph/mod.rs:477–505`) | ✗ | ✗ | High |
| **64MB SQLite page cache** | ✓ | ✗ | ✗ | High |
| **L3 cache-aware batching** | ✓ (`src/indexer.rs:16–89`) | N/A | N/A | Medium |
| **Parallel file I/O (rayon)** | ✓ (`src/graph/scan.rs`) | N/A | N/A | Low (atheneum doesn't index files) |
| **Parser pool / parse-once** | ✓ (`src/graph/ops.rs:112–122`) | N/A | N/A | Low |
| **Batch SQLite transactions** | ✓ (~27× throughput) | ✗ (per-request connection) | ✗ (single writes) | **Critical** |
| **WAL checkpointing** | ✓ (`src/graph/wal.rs`) | ✗ | ✗ | High |
| **In-memory SymbolLookup** | ✓ (O(1), ~50–100ms rebuild) | ✗ | ✗ | **Critical** |
| **Clustered adjacency storage** | ✓ (`src/graph/algorithms.rs:19–21`) | ✗ | ✗ | Medium |
| **Prepared statement caching** | ✓ (`src/graph/navigator.rs:112`) | ✗ | ✗ | High |
| **Lazy HNSW load** | ✓ (`src/graph/mod.rs:320–419`) | ✗ | ✗ | Medium |
| **Parallel embedding (rayon pool)** | ✓ (`src/graph/mod.rs:738–993`) | ✗ | ✗ | Low |
| **Debounced file watcher** | ✓ (`src/watcher/mod.rs`) | N/A | N/A | Low |

### 2.2 Cross-Project / Meta-Database Features

| Feature | Magellan | Atheneum | Notes |
|---------|----------|----------|-------|
| **Meta-database (`meta.db`)** | ✓ (`src/service/meta_db.rs`) | ✗ | Registry of all projects, embeddings, cross-references |
| **Project registry table** | ✓ (`project_registry`) | ✗ | name, root, db_path, enabled, counts |
| **Cross-project embeddings** | ✓ (`concept_embeddings`) | ✗ | Structural embeddings for analogy search |
| **Pattern cross-references** | ✓ (`pattern_cross_refs`) | ✗ | "Symbol A in project X ≈ Symbol B in project Y" |
| **Multi-DB context** | ✓ (`src/graph/multi_db.rs`) | ✗ | Unified queries across multiple `.db` files |
| **Service daemon** | ✓ (Unix socket JSON-RPC) | ✗ | Keeps databases open, avoids re-open cost |
| **Open graph cache** | ✓ (`HashMap<String, CodeGraph>`) | ✗ | Avoids `open()` per request |

### 2.3 Analytical Features Missing from Envoy

| Feature | Magellan CLI | Envoy HTTP | Impact |
|---------|-------------|------------|--------|
| `--concise` mode | ✓ | ✗ | Compact output for LLM context windows |
| `--budget` (token limit) | ✓ | ✗ | Hard truncation to fit context |
| `hopgraph` (semantic + graph) | ✓ | ✗ | Vector entry + BFS expansion |
| `cfg` / `paths` / `hotspots` | ✓ (mirage) | ✗ | Control flow analysis |
| `impact` / `affected` | ✓ | Partial (`neighbors` only) | Blast radius analysis |
| `doctor` / `refresh` | ✓ | ✗ | Index health & maintenance |

---

## 3. Proposed Architecture: `meta.db` Routing Layer

### 3.1 Core Idea

Instead of importing magellan data into atheneum (current one-way bridge), atheneum maintains a `meta.db` that knows where every project's magellan database lives. When a query asks for cross-project results, atheneum:

1. Looks up candidate projects in `meta.db`
2. `ATTACH DATABASE` the magellan `.db` files (read-only)
3. Runs unified SQL queries across attached schemas
4. `DETACH` when done (or keeps a small LRU cache of attached dbs)

This is the same pattern SQLite uses for multi-tenant analytics and the same pattern magellan's `MultiDbContext` uses.

### 3.2 `meta.db` Schema

```sql
-- Project registry
CREATE TABLE project_registry (
    id          INTEGER PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE,
    root_path   TEXT NOT NULL,
    magellan_db TEXT NOT NULL,   -- path to .magellan/*.db
    atheneum_db TEXT,            -- path to atheneum.db (if exists)
    language    TEXT,            -- 'rust', 'typescript', 'python', ...
    enabled     BOOLEAN DEFAULT 1,
    last_indexed TIMESTAMP,
    file_count   INTEGER DEFAULT 0,
    symbol_count INTEGER DEFAULT 0,
    created_at   TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Cross-project symbol analogies
CREATE TABLE symbol_analogies (
    id              INTEGER PRIMARY KEY,
    from_project    TEXT NOT NULL REFERENCES project_registry(name),
    from_symbol     TEXT NOT NULL,
    to_project      TEXT NOT NULL REFERENCES project_registry(name),
    to_symbol       TEXT NOT NULL,
    similarity_score REAL NOT NULL,
    created_at      TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Routing index: which projects have which symbols
CREATE TABLE symbol_index (
    id          INTEGER PRIMARY KEY,
    project     TEXT NOT NULL REFERENCES project_registry(name),
    symbol_name TEXT NOT NULL,
    kind        TEXT,
    file_path   TEXT,
    line        INTEGER,
    UNIQUE(project, symbol_name, file_path)
);

CREATE INDEX idx_symbol_name ON symbol_index(symbol_name);
CREATE INDEX idx_project_lang ON project_registry(language) WHERE enabled = 1;
```

### 3.3 Query Routing Examples

**Find `build_router` across all Rust projects:**

```sql
-- Step 1: Look up candidate projects in meta.db
SELECT name, magellan_db FROM project_registry
WHERE enabled = 1 AND language = 'rust';

-- Step 2: For each candidate, ATTACH and query
ATTACH DATABASE '/path/to/envoy/.magellan/magellan.db' AS envoy_db;
ATTACH DATABASE '/path/to/magellan/.magellan/magellan.db' AS magellan_db;

-- Step 3: Union query across all attached schemas
SELECT 'envoy' AS project, name, kind, file_path, line
FROM envoy_db.entities WHERE name = 'build_router'
UNION ALL
SELECT 'magellan' AS project, name, kind, file_path, line
FROM magellan_db.entities WHERE name = 'build_router';
```

**Cross-project hopgraph (conceptual):**

```
User asks: "How is routing handled across my Rust projects?"

1. Lexical search meta.db.symbol_index for "router" → hits in envoy, mirage
2. ATTACH envoy.magellan_db, mirage.magellan_db
3. For each hit, BFS within its local graph (1–2 hops)
4. Merge results, filter by language = 'rust'
5. Return unified subgraph views
```

### 3.4 Connection Model

Magellan's `MultiDbContext` opens databases eagerly and keeps them in a `HashMap`. For atheneum, we propose **lazy attachment with LRU eviction**:

- Keep a pool of `rusqlite::Connection` objects
- Each connection can `ATTACH` up to 10 databases (SQLite default)
- LRU cache of attached schemas; evict oldest when limit reached
- Read-only attach for safety (no risk of corrupting magellan dbs)
- Connection reuse across queries (eliminates envoy's per-request open cost)

This gives us the performance of magellan's open-graph-cache without the memory cost of keeping every project open simultaneously.

---

## 4. HPC Optimizations to Port

### Phase 1: SQLite Tuning (1–2 days, high impact)

Apply magellan's PRAGMA tuning to atheneum's `open()` path:

```rust
conn.execute_batch(
    "PRAGMA journal_mode = WAL;
     PRAGMA synchronous = NORMAL;
     PRAGMA cache_size = -64000;
     PRAGMA temp_store = MEMORY;
     PRAGMA mmap_size = 30000000000;"
)?;
```

**Expected impact:** 2–5× write throughput improvement, reduced WAL growth.

### Phase 2: Connection Pooling (2–3 days, critical)

Replace envoy's per-request `AtheneumGraph::open()` with a connection pool:

```rust
pub struct GraphPool {
    pool: HashMap<PathBuf, Arc<AtheneumGraph>>,
}

impl GraphPool {
    pub fn get(&mut self, path: &Path) -> Result<Arc<AtheneumGraph>> {
        if let Some(g) = self.pool.get(path) {
            return Ok(g.clone());
        }
        let g = Arc::new(AtheneumGraph::open(path)?);
        self.pool.insert(path.to_path_buf(), g.clone());
        Ok(g)
    }
}
```

**Expected impact:** Eliminates ~50–100ms per-request open latency.

### Phase 3: Prepared Statement Caching (1–2 days, high impact)

Wrap `rusqlite::Connection` with `prepare_cached()` for common queries:

- `get_entity`
- `get_neighbors`
- `search_by_name`
- `count_entities`

**Expected impact:** 10–50× speedup for repeated query patterns.

### Phase 4: In-Memory Lookup Index (3–5 days, critical)

Port magellan's `SymbolLookup` to atheneum:

```rust
pub struct SymbolLookup {
    by_name: HashMap<String, Vec<i64>>,      // name → entity_ids
    by_kind: HashMap<String, Vec<i64>>,      // kind → entity_ids
    by_project: HashMap<String, Vec<i64>>,   // project → entity_ids
}
```

Rebuild on `open()`, incrementally update on writes. Provides O(1) name/kind/project lookups instead of O(N) table scans.

**Expected impact:** Sub-millisecond entity resolution vs. 10–100ms table scans.

### Phase 5: Batch Write API (2–3 days, high impact)

Add bulk insert methods to atheneum:

```rust
pub fn bulk_store_discoveries(&self, items: &[DiscoveryParams]) -> Result<Vec<i64>>;
pub fn bulk_store_memories(&self, items: &[MemoryParams]) -> Result<Vec<i64>>;
```

Wrap in a single transaction. Envoy can then batch multiple discoveries/memories into one request.

**Expected impact:** 10–50× write throughput for bulk ingest.

### Phase 6: WAL Checkpointing (1 day, high impact)

Add `checkpoint_wal()` call after bulk writes and on graceful shutdown:

```rust
pub fn checkpoint(&self) -> Result<()> {
    self.conn.execute("PRAGMA wal_checkpoint(TRUNCATE)", [])?;
    Ok(())
}
```

**Expected impact:** Prevents unbounded WAL growth, reduces corruption risk.

---

## 5. Cross-Project Navigation API Design

### 5.1 New CLI Commands

```bash
# Register a project in the meta.db
atheneum meta-register ./envoy rust /path/to/envoy/.magellan/magellan.db

# List all registered projects
atheneum meta-list

# Cross-project search
atheneum cross-search "build_router" --language rust --k 10

# Cross-project navigate
atheneum cross-navigate "error handling pattern" --language rust --depth 2

# Find symbol analogies across projects
atheneum cross-analogy "Result" --from-project envoy --language rust
```

### 5.2 New Library API

```rust
impl AtheneumGraph {
    /// Open the meta.db routing layer
    pub fn open_meta(path: &Path) -> Result<MetaRouter>;
}

pub struct MetaRouter {
    conn: rusqlite::Connection,
    attached: LruCache<String, AttachedDb>,
}

impl MetaRouter {
    /// Search for a symbol across all enabled projects
    pub fn cross_search(&self, query: &str, language: Option<&str>, k: usize) -> Result<Vec<CrossResult>>;

    /// Navigate from a symbol in one project to related symbols in others
    pub fn cross_navigate(&self, query: &str, depth: u32, language: Option<&str>) -> Result<Vec<CrossSubgraph>>;

    /// Register a new project
    pub fn register_project(&self, name: &str, root: &str, magellan_db: &str, language: &str) -> Result<()>;
}
```

### 5.3 HTTP Endpoints (Envoy)

```
GET  /atheneum/meta/projects              # list registered projects
POST /atheneum/meta/projects              # register project
GET  /atheneum/cross/search?q=...&lang=   # cross-project search
GET  /atheneum/cross/navigate?q=...&lang= # cross-project navigate
GET  /atheneum/cross/analogy?symbol=...   # find analogies
```

---

## 6. Language Filtering Strategy

The user's concern: "if the hopgraph is cross codebases, it needs to be filtered by coding language."

### Approach: Language as a First-Class Routing Dimension

1. **Registration-time tagging:** Every project in `meta.db` has a `language` column.
2. **Query-time filtering:** Cross-project queries always include `WHERE language = ?` on the project registry lookup.
3. **No cross-language embeddings:** We do NOT mix Rust and TypeScript vectors in the same HNSW index. Each project maintains its own embedding space.
4. **Cross-language analogies are explicit:** `symbol_analogies` table stores explicit human-or-agent-curated mappings (e.g., "Rust `Result` ≈ TypeScript `Result`"). These are opt-in, not inferred from embeddings.

This avoids the "false friends" problem (e.g., `Result` in Rust vs. TypeScript) while still allowing deliberate cross-language exploration.

---

## 7. Implementation Roadmap

### Milestone 1: Foundation (Week 1)
- [ ] Add SQLite PRAGMA tuning to atheneum
- [ ] Add `checkpoint_wal()` and call it after bulk writes
- [ ] Add connection pool to envoy (eliminate per-request open)

### Milestone 2: Performance (Week 2)
- [ ] Add prepared statement caching to atheneum
- [ ] Add in-memory `SymbolLookup` index
- [ ] Add batch write API (`bulk_store_discoveries`, `bulk_store_memories`)

### Milestone 3: Meta.db (Week 3)
- [ ] Create `meta.db` schema and `MetaRouter` struct
- [ ] Implement `meta-register`, `meta-list` CLI commands
- [ ] Implement lazy `ATTACH` with LRU eviction

### Milestone 4: Cross-Project Query (Week 4)
- [ ] Implement `cross-search` (lexical + union across attached dbs)
- [ ] Implement `cross-navigate` (entry point search + per-db BFS)
- [ ] Add language filtering

### Milestone 5: Integration (Week 5)
- [ ] Add HTTP endpoints to envoy
- [ ] Update grounded-coding skill with new CLI commands
- [ ] End-to-end test: register 3 projects, run cross-search, verify results

### Milestone 6: Analytical Features (Week 6–8)
- [ ] Port `concise` mode to atheneum navigate output
- [ ] Add `impact` / `affected` endpoints (blast radius analysis)
- [ ] Add `doctor` endpoint (index health check)

---

## 8. Risks & Mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| SQLite `ATTACH` limit (10 dbs) | High | Blocks large cross-project queries | LRU eviction + pagination; or use `SQLITE_MAX_ATTACHED=125` compile flag |
| Magellan schema changes break queries | Medium | Cross-project queries fail | Versioned schema detection; fallback to import bridge |
| Embedding dimension mismatch across projects | Medium | HNSW search fails | Store dimension in `project_registry`; validate on attach |
| Memory pressure from open dbs | Medium | OOM on large workspaces | LRU cache with configurable size; lazy load |
| Stale meta.db data | Low | Queries hit moved/deleted dbs | Background health check daemon; prune dead entries |

---

## 9. Open Questions

1. **Should atheneum write to magellan dbs?** Proposal: No — read-only attach preserves magellan's ownership of its data. Writes go through magellan's own APIs.
2. **How do we handle magellan's HNSW index in atheneum?** Option A: Query magellan's HNSW via SQL (it stores vectors in SQLite tables). Option B: Skip HNSW for cross-project; use lexical + graph traversal only.
3. **Who maintains `symbol_analogies`?** Option A: Agent-curated (store_discovery with cross-project metadata). Option B: Automated structural similarity (AST shape hashing). Option C: Hybrid — automated suggestions, agent approval.
4. **Should the meta.db live inside atheneum.db or be separate?** Proposal: Separate (`~/.local/share/atheneum/meta.db`) so it can reference multiple atheneum dbs too.

---

## 10. References

- Magellan `meta.db`: `magellan/src/service/meta_db.rs`
- Magellan `MultiDbContext`: `magellan/src/graph/multi_db.rs`
- Magellan HPC tuning: `magellan/src/graph/mod.rs:477–505`
- Magellan batch transactions: `magellan/src/graph/symbols.rs:279–393`
- Magellan SymbolLookup: `magellan/src/graph/symbol_lookup.rs`
- Envoy per-request open: `envoy/src/atheneum_bridge/discovery.rs` (and all other bridge modules)
- SQLite `ATTACH DATABASE`: [Turso blog on read-only attach](https://turso.tech/blog/introducing-read-only-database-attach-in-turso)
- Twitter SQL federation: [CIDR 2022 paper](https://arxiv.org/pdf/2207.04199)
