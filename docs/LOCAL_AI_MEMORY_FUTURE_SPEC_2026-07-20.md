# Atheneum Librarian Future Spec — v0.2 (Future Roadmap)

Status: proposed. Scope: outline and specify the next phase of the Atheneum Librarian, focusing on dynamic local model discovery, LLM-driven semantic consolidation during dreaming, an interactive force-directed Web UI dashboard, and memory pinning.

Design tenets (in priority order):
1. **Real results only.** Every implemented roadmap story must ship with robust unit and integration tests. No `todo!()`, stubs, or warnings.
2. **Library-centric + Optional Features.** The Axum Web UI, LLM-driven extraction, and external HTTP integrations must reside behind clear cargo features (`web-ui`, `neural-embed`, `extract`) to maintain a lightweight core.
3. **Traceability.** All automated operations (e.g., semantic dreaming) must record event trace entries linking back to the target nodes.

---

## Functional requirements

### FR-F1: Dynamic Model Auto-Discovery and Swap Guard

**Why.** When running multiple local LLMs via `llama.cpp` or a broker like `llama-swap`, executing model-based operations (like semantic embedding generation or LLM-based memory consolidation) can trigger slow model swaps on a shared GPU. Atheneum needs a mechanism to discover what model is currently loaded and guard execution.

**Surface.**
- Rust: 
  - `AtheneumGraph::discover_available_models() -> Result<Vec<ModelInfo>>` where `ModelInfo { name: String, loaded: bool, size_bytes: Option<u64> }`.
  - Extension to `AtheneumGraph::dream_pass` and embedders: accept a model preference constraint.
- CLI: `atheneum models-list <db> [--endpoint URL]`
- MCP: `list_models` tool

**Semantics.**
- Query local `llama.cpp` (default `http://127.0.0.1:8080/v1/models` or standard environment variables) to find loaded models.
- If a semantic embed or LLM task is requested but the requested model is not loaded, the swap guard either:
  1. Falls back to a lightweight local trigram lexical check (default).
  2. Bails with `AtheneumError::ModelSwapBlocked` if swap guard mode is set to strict.
  3. Adapts the prompt to use the currently loaded model on the host.

**Acceptance.**
- Unit test: Mocking local HTTP model listing endpoint returning loaded model configurations; verifying `discover_available_models` parses correctly.
- Unit test: Swap guard blocks execution and triggers fallback when requesting a missing model.

---

### FR-F2: Semantic Dream Consolidation & LLM-Driven Resolvers

**Why.** While FR-4 `maintain` resolves simple contradictions by setting `superseded_by` self-edges, complex contradictions or redundant concepts need semantic merging (e.g., merging "Hyderabad" and "Cyberabad" or unifying scattered facts about a single person).

**Surface.**
- Rust: `AtheneumGraph::semantic_consolidation(config: &ConsolidationConfig) -> Result<ConsolidationReport>`
- CLI: `atheneum dream-semantic <db> [--apply]`
- MCP tool: `dream_semantic`

**Semantics.**
- Triggers only when no writes have occurred within the specified idle threshold (building on `dream_if_idle`).
- Scans closely-related Concepts (Jaccard similarity > 0.4 or close embedding distance).
- Sends a prompt to the active local model (`llama.cpp` or Ollama):
  ```text
  You are the Atheneum Librarian. Merge the following two conflicting/redundant concepts and their memories into a single clean markdown-styled concept body. Preserve all key facts.
  Concept A: [body]
  Concept B: [body]
  ```
- Replaces/Updates the winner node, sets a `superseded_by` self-edge from the loser node pointing to the winner, and rewires all incoming/outgoing edges of the loser node to the winner node.

**Acceptance.**
- Unit test: Seed two redundant profiles ("Luiz S" and "Luiz Spies"), run `dream-semantic`, assert they are merged into a single profile with all tags/facts unified, and the loser is marked as superseded.

---

### FR-F3: Interactive Web UI Dashboard & Force-Directed Graph Visualizer

**Why.** Users need a premium, visual interface to inspect memory growth, review query traces, search concepts, and manually resolve contradictions without reading SQLite tables directly.

**Surface.**
- Build behind the `web-ui` cargo feature.
- CLI: `atheneum dashboard <db> [--port N] [--host IP]`
- Exposes Axum web routes serving static React/Vite assets and JSON endpoints.

**UI Features.**
- **Force-Directed Graph**: A Canvas/SVG network graph rendering active nodes (Concepts, Memories, WikiPages) and edges. Hovering over a node displays summary stats; double-clicking opens a markdown editor.
- **Trace Explorer**: Visualizes `QueryTrace` executions. Shows how a query entered the graph (lexical search hit), traversed BFS paths, and fetched target memory blocks, presented as a flowchart.
- **Contradiction/Orphan Dashboard**: Tabulates flagged orphans and contradictions, providing a "Merge" button to let the user manually review and approve merges.
- **Chat Console**: A web chat interface to chat directly with the Librarian agent, showing the visual search path updating live in the sidebar as the agent searches.

**Acceptance.**
- Compilation test: `cargo build --features web-ui` compiles cleanly.
- Integration test: Axum server starts on a random port, responds to `/api/graph` and `/api/traces` JSON endpoints with correct schema structure.

---

### FR-F4: Memory Pinning & TTL Strategy

**Why.** Hot-tier caching is useful, but critical rules (like system prompt instructions or project-scoped environment configurations) must never be evicted from the hot tier.

**Surface.**
- Rust: 
  - Add `pinned: bool` to `Memory` and `Concept` entity data blobs.
  - `AtheneumGraph::pin_entity(id: i64) -> Result<()>`
  - `AtheneumGraph::unpin_entity(id: i64) -> Result<()>`
- CLI: `atheneum pin <db> --id N` / `atheneum unpin <db> --id N`
- MCP tools: `pin_entity`, `unpin_entity`

**Semantics.**
- Pinned entities are automatically loaded on `seed_memory` bootstrap, regardless of token budgets or recency sorts.
- Cache layer protects pinned entries: they are immune to LRU cache eviction.
- Add configurable TTL per tags: allow temporary memories (e.g., "debugging session temp facts") to have a self-expiring TTL (e.g., 24 hours), after which a background `dream` run automatically archives them.

**Acceptance.**
- Unit test: Seed a pinned memory, assert it is always returned first in `seed_memory` even if newer memories exist.
- Unit test: Seed a memory with a 1-second TTL, run `maintain` after 2 seconds, assert the memory is archived/deleted automatically.
