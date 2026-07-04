# LLM Coding Memory Systems Landscape — Atheneum Comparison (2026-07-03)

## The landscape

The 2026 agent-memory space has crystallized into ~8 serious systems. They split
into two camps: **conversation memory** (what the user said, what the agent
decided) and **codebase memory** (structural knowledge of code). Atheneum is
unique in attempting both, grounded in a real graph database (sqlitegraph).

## Systems compared

### 1. Mem0 (mem0.ai)
- **Architecture**: Hybrid vector + graph. Single-pass ADD-only fact extraction
  from conversation turns. LLM extracts facts → dedup against existing → store as
  vector embedding + optional graph node.
- **Storage**: Qdrant/Pinecone/Chroma (vectors) + Neo4j optional (graph).
  Cloud-hosted or self-hosted. Requires external services.
- **Retrieval**: Vector similarity search (cosine). Semantic recall.
- **Strength**: 21 framework integrations, 20 vector store backends, managed
  cloud. Production deployment story. Active research (benchmarks, state-of-memory report).
- **Weakness**: Conversation-only. No code structure. No temporal versioning.
  Requires external DBs (no embedded storage). Python-centric.
- **License**: Apache 2.0 (open source) + proprietary cloud.
- **MCP**: Yes — official MCP server.

### 2. Zep / Graphiti (getzep.com)
- **Architecture**: Temporal knowledge graph. Every fact carries time metadata.
  Graphiti engine extracts entities + relationships from conversations, builds
  a temporal graph where edges have valid-from/valid-to intervals. When facts
  change, old edges expire — not overwritten.
- **Storage**: Neo4j (graph) + vector embeddings (FalkorDB/Neo4j vectors).
  Requires Neo4j.
- **Retrieval**: Cypher graph traversal + temporal queries + vector hybrid.
- **Strength**: Temporal awareness — knows WHEN facts were true. Open-source
  Graphiti engine (20K+ stars). Research-backed (arXiv paper).
- **Weakness**: Requires Neo4j (heavy external dep). Conversation-only — no
  code structure. Python. No embedded mode.
- **License**: Apache 2.0 (Graphiti) + proprietary cloud (Zep).
- **MCP**: Yes — via Zep cloud or Graphiti self-hosted.

### 3. Letta / MemGPT (letta.com)
- **Architecture**: OS-inspired memory hierarchy. Core memory (in-context,
  self-editable by the LLM), recall memory (conversation history on disk),
  archival memory (long-term vector store). The LLM itself manages memory
  via function calls (insert/update/search).
- **Storage**: PostgreSQL or SQLite (relational) + vector store (pgvector,
  ChromaDB). Self-hosted or cloud.
- **Retrieval**: LLM decides when to page in archival memories. Vector search
  for archival, direct access for core.
- **Strength**: Self-editing memory — the agent manages its own context window.
  Strong benchmark results (LoCoMo 74% with filesystem approach). Mature platform.
- **Weakness**: LLM-driven memory management burns tokens on memory ops.
  Conversation-only. No code structure. Requires external DBs.
- **License**: Apache 2.0 + proprietary cloud.
- **MCP**: Yes.

### 4. Cognee (cognee.ai)
- **Architecture**: ECL pipeline (Extract-Cognify-Load). Raw data → entity
  extraction → knowledge graph construction → vector embeddings. Combines
  relational, vector, AND graph storage. "No single database can handle all
  aspects of memory."
- **Storage**: PostgreSQL/SQLite (relational) + vector store + graph DB (Neo4j/
  NetworkX). Three complementary stores.
- **Retrieval**: Hybrid — vector similarity + graph traversal.
- **Strength**: Ranked #1 for coding agents (opensourceaireview). Handles
  unstructured data → knowledge graph pipeline. ECL is well-designed.
- **Weakness**: Requires 3 storage systems (operational complexity). Python.
  No code-specific parsing (generic NLP extraction). No temporal versioning.
- **License**: Apache 2.0.
- **MCP**: Yes.

### 5. Supermemory (supermemory.ai)
- **Architecture**: "Cubes" — cross-agent memory containers. Each cube is an
  isolated memory namespace. Vector search + structured metadata.
- **Storage**: Cloud-hosted (proprietary). Turso/libSQL for self-hosted.
- **Retrieval**: Vector similarity + metadata filters.
- **Strength**: Cross-agent memory sharing (multiple agents read/write the same
  cube). Clean API. Container-tag scoping.
- **Weakness**: Cloud-first (self-hosting is secondary). Conversation-only.
  No graph structure. No code awareness.
- **License**: MIT (self-hosted) + proprietary cloud.
- **MCP**: Yes — primary interface.

### 6. Codebase-Memory-MCP (DeusData)
- **Architecture**: Tree-sitter-based knowledge graph. Parses 66 languages via
  tree-sitter, builds a persistent code structure graph (symbols, calls, types,
  scopes). Multi-phase pipeline with parallel workers. Call-graph traversal,
  impact analysis, community detection.
- **Storage**: Embedded SQLite (single binary, zero dependencies).
- **Retrieval**: Graph queries (callers, callees, impact, references).
  ~500 tokens vs ~80K for file-by-file exploration.
- **Strength**: CODE-SPECIFIC. Zero-dep single binary. 66 languages. 83% answer
  quality, 10x fewer tokens (benchmarked, arXiv paper). Plug-and-play across 11
  coding agents. Embedded storage (no external services).
- **Weakness**: Code-only — no conversation memory, no decisions, no wiki.
  Static analysis only (no runtime info). No temporal versioning. C/TypeScript.
- **License**: MIT.
- **MCP**: Yes — sole interface.

### 7. Hindsight
- **Architecture**: Conversation replay + summarization. Stores raw transcripts,
  generates summaries on retrieval. Simple filesystem-first approach.
- **Storage**: Filesystem (JSON/markdown files).
- **Retrieval**: LLM-generated summaries of relevant past conversations.
- **Strength**: Dead simple. No external services. Strong on "what did we
  discuss before" queries.
- **Weakness**: No graph, no code structure, no temporal. Limited scale.
- **License**: Open source.
- **MCP**: Yes.

### 8. Atheneum (oldnordic/atheneum)
- **Architecture**: sqlitegraph-backed agent coordination graph database.
  Stores: memories (keyed KV with scope/confidence/project), discoveries
  (decisions, bugs, findings with caused_by/led_to chains), wiki pages (FTS5-
  indexed markdown), journal sections, session transcripts (chat turns),
  graph entities (typed nodes with JSON data), and code structure (from
  magellan indexing — symbols, calls, references, CFG blocks).
- **Storage**: Embedded SQLite via sqlitegraph (single file, no external
  services). WAL mode. HNSW vector index available (semantic-search feature).
- **Retrieval**:
  - Lexical search (bag-of-tokens + optional HNSW seed) across all entity kinds
  - Wiki FTS5 full-text search
  - Decision content search (target/chosen/why substring matching)
  - Graph traversal (caused_by/led_to decision chains, subgraph walks)
  - Memory recall (token-overlap scoring, graph-boosted)
  - Temporal barcode (symbol/SCC lifetime via magellan temporal-sweep)
- **Strength**:
  - BOTH conversation AND codebase memory in one DB
  - Decision chains with causal edges (caused_by/led_to) — no other system has this
  - Wiki page ingestion (sync-wiki from markdown, FTS5 indexed)
  - Embedded SQLite (zero external deps — unlike Mem0/Zep/Cognee which need
    Neo4j/Qdrant/Postgres)
  - Rust (memory-safe, fast, single binary)
  - Temporal versioning (via magellan temporal-sweep — symbol lifetime barcodes)
  - Graph-native code structure (from magellan — symbols, calls, CFG, references)
  - Dream consolidation (reflective memory compaction — automated dedup/merge)
  - Multi-agent coordination (via envoy — handoffs, discoveries, session tracking)
- **Weakness**:
  - Solo project (vs venture-backed Mem0/Zep/Letta)
  - No managed cloud (self-hosted only)
  - Project-ID scoping bug (just fixed — wiki/decisions were unreachable under
    `--project forge` because they carried different project tags)
  - No semantic embedding model bundled (HNSW index exists but requires external
    embedder or the hash-embedder fallback)
  - Limited language support for code indexing (via magellan: Rust, Python, C,
    C++, Java, JS, TS, Go, CUDA — 9 languages vs codebase-memory-mcp's 66)
  - No MCP server for direct agent integration (atheneum-mcp connects via envoy
    HTTP, not stdio MCP — adds a hop)
  - Sparse documentation (no public docs site vs Cognee/Zep/Mem0)
- **License**: GPL-3.0-only.
- **MCP**: Via atheneum-mcp (HTTP to envoy) — not direct stdio MCP.

## Comparison Matrix

| Feature | Atheneum | Mem0 | Zep/Graphiti | Letta/MemGPT | Cognee | Supermemory | Codebase-Memory-MCP |
|---------|----------|------|--------------|--------------|--------|-------------|---------------------|
| **Conversation memory** | Yes (sessions, turns, summaries) | Yes (facts from turns) | Yes (temporal graph) | Yes (core/recall/archival) | Yes (ECL pipeline) | Yes (cubes) | No |
| **Code structure** | Yes (via magellan: symbols/calls/CFG/refs) | No | No | No | No | No | Yes (tree-sitter, 66 langs) |
| **Decision chains** | Yes (caused_by/led_to edges) | No | No (facts expire, no causal) | No | No | No | No |
| **Wiki/knowledge docs** | Yes (sync-wiki, FTS5) | No | No | No | Yes (ingests documents) | No | No |
| **Temporal versioning** | Yes (magellan temporal-sweep) | No | Yes (edge time intervals) | No | No | No | No |
| **Graph traversal** | Yes (sqlitegraph BFS/SCS/cycles) | Optional (Neo4j) | Yes (Neo4j Cypher) | No | Yes (Neo4j/NetworkX) | No | Yes (call graph) |
| **Storage** | Embedded SQLite | External (Qdrant/Pinecone + Neo4j) | External (Neo4j + FalkorDB) | External (Postgres + vector) | External (3 systems) | Cloud/Turso | Embedded SQLite |
| **Language** | Rust | Python | Python | Python | Python | TypeScript | C/TypeScript |
| **Dependencies** | Zero (single binary) | Multiple services | Neo4j required | Postgres required | 3 storage systems | Cloud account | Zero (single binary) |
| **MCP** | Via envoy HTTP | Yes (stdio) | Yes | Yes | Yes | Yes (primary) | Yes (sole interface) |
| **Multi-agent** | Yes (envoy coordination) | No | No | No | No | Yes (cubes) | No |
| **Memory consolidation** | Yes (dream) | No | No | No | No | No | No |
| **License** | GPL-3.0 | Apache 2.0 | Apache 2.0 | Apache 2.0 | Apache 2.0 | MIT | MIT |

## Where Atheneum is unique (the moat)

1. **Decision chains with causal edges** — no other system tracks WHY a decision
   was made and WHAT IT LED TO. Mem0 stores facts flat. Zep stores temporal facts.
   Atheneum stores decisions as graph nodes with caused_by/led_to edges, enabling
   "trace the decision chain from this bug back to the architectural choice that
   introduced it." This is the single most differentiated feature.

2. **Unified conversation + codebase memory** — codebase-memory-mcp does code
   only. Mem0/Zep/Letta do conversation only. Cognee does generic document
   ingestion but no code-specific parsing. Atheneum (via magellan) has BOTH:
   the agent can query "what did we decide about the rocmforge kernel dispatch?"
   AND "show me the call graph for that function" in one DB.

3. **Embedded SQLite, zero external deps** — Mem0, Zep, Cognee, Letta all
   require external database servers (Neo4j, Postgres, Qdrant). Atheneum uses
   sqlitegraph — a single `.db` file. No Docker, no services, no cloud account.
   This is a deployment advantage for solo developers and local-first setups.

4. **Dream consolidation** — automated memory compaction (dedup, merge, stale
   detection). No other system does this. Mem0 has ADD-only extraction. Letta
   has LLM-driven archival. Atheneum has a deterministic consolidation pass
   that merges duplicate memories and prunes stale ones.

5. **Wiki page ingestion with wikilink graph** — sync-wiki ingests markdown
   pages, extracts wikilinks ([[page-name]]), and builds a navigable cross-
   reference graph. No other memory system does this. The wiki is the user's
   personal knowledge base, and atheneum makes it graph-navigable.

## Where Atheneum is behind

1. **MCP integration** — atheneum-mcp connects via envoy HTTP, not direct stdio
   MCP. Every other system has a clean stdio MCP server. This adds latency and
   complexity. A direct stdio MCP server for atheneum would close this gap.

2. **Documentation** — no public docs site. Mem0, Zep, Cognee, Letta all have
   polished documentation. Atheneum has a MANUAL.md and AGENTS.md but nothing
   discoverable. For adoption, this matters.

3. **Embedding model** — the HNSW semantic search requires an external embedder
   (Ollama/OpenAI). Mem0/Cognee bundle embedding pipelines. Atheneum's hash-
   embedder fallback is structural-only (no semantic recall). The user's decision
   to abandon HNSW for LLM use mitigates this — graph-first retrieval is the path.

4. **Language coverage** — 9 languages (via magellan tree-sitter grammars) vs
   codebase-memory-mcp's 66. For polyglot codebases, this is a gap. But
   magellan's grammars cover the languages in the user's actual stack.

5. **Benchmark validation** — codebase-memory-mcp has an arXiv paper with 83%
   answer quality and 10x token reduction benchmarks. Mem0 has a state-of-memory
   report. Atheneum has no published benchmarks. For credibility (especially for
   the AI-systems-lab positioning), this is a gap.

## Strategic positioning

Atheneum sits in an uncontested niche: **agent coordination graph database with
decision chains, code structure, and wiki knowledge — all in embedded SQLite.**

The closest competitor is **Cognee** (hybrid vector+graph, document ingestion),
but Cognee lacks: (a) code-specific parsing, (b) decision chains, (c) embedded
storage, (d) wiki wikilink graph, (e) multi-agent coordination.

The second-closest is **codebase-memory-mcp** (tree-sitter code graph, embedded
SQLite), but it lacks: (a) conversation memory, (b) decisions, (c) wiki, (d)
multi-agent coordination, (e) temporal versioning.

**Atheneum's thesis**: an agent doesn't need vector similarity to find relevant
context — it needs the GRAPH of decisions, code, and knowledge. Graph traversal
+ SQL is more precise and more token-efficient than ANN retrieval. This aligns
with the user's stated position (2026-06-18): "graph metadata to navigate and
query what you need from the SQL is better."

## Recommended next steps (from this comparison)

1. **Direct stdio MCP server** — bypass envoy for single-agent use. The atheneum-mcp
   binary already exists; adding a stdio transport mode would make it plug-and-play
   with any MCP-compatible agent (Claude Code, Cursor, etc.).

2. **Publish benchmarks** — run the codebase-memory-mcp benchmark methodology
   against atheneum's magellan-backed code queries. The token-savings story (500
   tokens via graph query vs 80K via file reads) is the same pitch.

3. **Write a positioning doc** — "Atheneum: Agent Coordination Graph Database" —
   emphasizing decision chains + embedded SQLite + unified conversation/code
   memory. Target: AI-systems labs (the user's career goal).

4. **Close the project_id scoping gap** — the fix we just implemented (wiki-search
   and decision-search without forced project filter) is the right direction. Make
   cross-project knowledge the default, not the exception.

---

*Generated 2026-07-03 by Hermes Agent. Sources: web searches on Mem0, Zep/Graphiti,
Letta/MemGPT, Cognee, Supermemory, Codebase-Memory-MCP, Hindsight. Architecture
details from official docs, arXiv papers, and comparison articles.*
