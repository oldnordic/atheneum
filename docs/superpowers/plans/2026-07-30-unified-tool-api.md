# Unified Tool API Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend `atheneum-mcp` with a dispatch layer that exposes magellan/llmgrep/mirage (code) and envoy (events) through the same small, paginated, provenance-tagged envelope atheneum's own tools already use — without adding a new MCP server and without breaking any of the 29 tools already registered.

**Architecture:** `crates/atheneum-mcp/src/envelope.rs` (new) defines the shared response shape. `crates/atheneum/src/cross.rs`'s existing `CrossRouter` gets a central-knowledge-store reachability fix and becomes the code-search fan-out mechanism for `search`/`navigate`. A new thin `subprocess.rs` module (mirroring grounded-mcp's proven `Command`-dispatch pattern) handles deep structural code queries. A new thin `events.rs` module (mirroring envoy-mcp's proven `reqwest` pattern) handles event passthrough. `search` and `navigate` are extended in place with a `kind` parameter, default value chosen so **existing callers see byte-identical behavior**.

**Tech Stack:** Rust, `rmcp` 1.7 (MCP server framework, `ToolRoute::new_dyn` manual registration — no `schemars`), `tokio`, `rusqlite` (via `atheneum`/`sqlitegraph`), `reqwest`, `async-trait`.

## Global Constraints

- Spec: `docs/superpowers/specs/2026-07-30-unified-tool-api-design.md` (committed 64dbbe6).
- No new MCP server/process. Everything lands inside `crates/atheneum-mcp` and `crates/atheneum` (both already in this workspace).
- `search` and `navigate` tool schemas may only gain *optional* fields. Any call with today's arguments (no `kind`, no `cursor`) must return exactly what it returns today — verified per-task, not just at the end.
- `navigate`/`cross_navigate` depth: default 2, hard max 3, server-clamped, never rejected.
- Every backend failure surfaces inside `Envelope.errors[]`; no raw subprocess stderr or HTTP error body ever reaches a caller.
- Pagination default `limit` 20, hard max 100 (matches the existing `search` tool schema's current max — reused for consistency, not invented).
- Timeouts: code-tool subprocess ~10s, atheneum in-process ~2s, envoy HTTP ~5s.

---

### Task 1: Central-knowledge-store reachability fix in `CrossRouter`

**Files:**
- Modify: `crates/atheneum/src/cross.rs`
- Test: `crates/atheneum/src/cross.rs` (inline `#[cfg(test)] mod tests`, same file — matches existing convention in this file)

**Interfaces:**
- Consumes: existing `CrossRouter` (`meta: MetaRouter`, `attached`, `lru`, `max_attached`, `schema_counter`), existing `ProjectInfo { name, root_path, magellan_db, atheneum_db, language, enabled, last_indexed, file_count }`, existing `ensure_attached(&mut self, project: &ProjectInfo) -> Result<String>`.
- Produces: `CrossRouter::with_central_knowledge_db(self, path: PathBuf) -> Self` (builder method), used by Task 3/4. `cross_search`/`cross_navigate` behavior: results from the central store are always included, tagged with `project: "__atheneum_central__"`.

Today, `cross_search`/`cross_navigate` iterate only `self.meta.list_projects()` (or `list_projects_by_language`). The central `.atheneum/atheneum.db` is not in that registry as a normal project — registering it there would make it either language-filtered out or indistinguishable from a real per-project code index. This task adds a second, always-included source.

- [ ] **Step 1: Write the failing test**

```rust
// in crates/atheneum/src/cross.rs, inside mod tests, after test_cross_search_attaches_and_finds_symbols

#[test]
fn test_cross_search_always_includes_central_knowledge_db_even_when_unregistered() {
    let tmp_dir = tempfile::tempdir().unwrap();
    let meta_path = tmp_dir.path().join("meta.db");
    let magellan_a = tmp_dir.path().join("a.db");
    let central = tmp_dir.path().join("central_atheneum.db");

    {
        let ca = make_magellan_like_db(&magellan_a);
        ca.execute(
            "INSERT INTO graph_entities (id, kind, name, file_path, data) VALUES
             (1, 'Symbol', 'unique_code_symbol', 'src/lib.rs', '{}')",
            [],
        )
        .unwrap();
    }
    {
        let cc = make_magellan_like_db(&central);
        cc.execute(
            "INSERT INTO graph_entities (id, kind, name, file_path, data) VALUES
             (1, 'Memory', 'unique_code_symbol_note', NULL, '{}')",
            [],
        )
        .unwrap();
    }

    let mut meta = MetaRouter::open_at(&meta_path).unwrap();
    // Note: "atheneum" / the central store is deliberately NEVER registered
    // via meta.register_project — that is exactly the gap being fixed.
    meta.register_project("alpha", "/alpha", magellan_a.to_str().unwrap(), None, Some("rust"))
        .unwrap();

    let mut cross = CrossRouter::from_meta(meta, 4).with_central_knowledge_db(central.clone());

    let hits = cross.cross_search("unique_code_symbol", None, 10).unwrap();
    let projects: Vec<_> = hits.iter().map(|h| h.project.as_str()).collect();

    assert!(projects.contains(&"alpha"), "expected alpha hit, got {projects:?}");
    assert!(
        projects.contains(&"__atheneum_central__"),
        "central knowledge db must always be searched even when unregistered as a project, got {projects:?}"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p atheneum --lib cross::tests::test_cross_search_always_includes_central_knowledge_db_even_when_unregistered`
Expected: FAIL — compile error (`with_central_knowledge_db` doesn't exist yet), or if stubbed to compile, assertion failure (only `alpha` present).

- [ ] **Step 3: Add the builder field and method**

```rust
// in crates/atheneum/src/cross.rs, add to the CrossRouter struct definition:
pub struct CrossRouter {
    meta: MetaRouter,
    attached: HashMap<String, String>,
    lru: VecDeque<String>,
    max_attached: usize,
    schema_counter: usize,
    central_knowledge_db: Option<std::path::PathBuf>, // NEW
}
```

Update every existing constructor (`open`, `with_capacity`, `from_meta`) to initialize `central_knowledge_db: None`, then add:

```rust
impl CrossRouter {
    /// Always include this knowledge-store db in every cross_search/cross_navigate
    /// call, even though it is not registered in magellan's meta.db project
    /// registry. Fixes the gap where the central atheneum knowledge store would
    /// otherwise be unreachable from cross-tool graph navigation.
    pub fn with_central_knowledge_db(mut self, path: std::path::PathBuf) -> Self {
        self.central_knowledge_db = Some(path);
        self
    }

    const CENTRAL_KNOWLEDGE_PROJECT_NAME: &'static str = "__atheneum_central__";

    fn central_project_info(&self) -> Option<ProjectInfo> {
        self.central_knowledge_db.as_ref().map(|path| ProjectInfo {
            name: Self::CENTRAL_KNOWLEDGE_PROJECT_NAME.to_string(),
            root_path: String::new(),
            magellan_db: path.to_string_lossy().into_owned(),
            atheneum_db: None,
            language: None,
            enabled: true,
            last_indexed: None,
            file_count: 0,
        })
    }
}
```

- [ ] **Step 4: Wire it into `cross_search` and `cross_navigate`**

```rust
// in cross_search, replace:
//     let projects = if let Some(lang) = language { ... } else { self.meta.list_projects()? };
// with:
let mut projects = if let Some(lang) = language {
    self.meta.list_projects_by_language(lang)?
} else {
    self.meta.list_projects()?
};
if let Some(central) = self.central_project_info() {
    if !projects.iter().any(|p| p.name == central.name) {
        projects.push(central);
    }
}
```

`cross_navigate` calls `cross_search` internally already (`let entries = self.cross_search(query, language, k)?;`), so it needs no separate change — the fix propagates automatically. `ensure_attached_for_name` (used by `cross_navigate` per-entry) resolves via `self.meta.get_project(project_name)`, which won't find `__atheneum_central__` since it's synthetic, not registered — fix `ensure_attached_for_name` to check the synthetic name first:

```rust
// in ensure_attached_for_name, before the meta.get_project lookup:
fn ensure_attached_for_name(&mut self, project_name: &str) -> Result<String> {
    if project_name == Self::CENTRAL_KNOWLEDGE_PROJECT_NAME {
        if let Some(central) = self.central_project_info() {
            return self.ensure_attached(&central);
        }
    }
    let project = self
        .meta
        .get_project(project_name)?
        .ok_or_else(|| anyhow::anyhow!("Project {} not found in meta.db", project_name))?;
    self.ensure_attached(&project)
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p atheneum --lib cross::tests::test_cross_search_always_includes_central_knowledge_db_even_when_unregistered`
Expected: PASS

- [ ] **Step 6: Run full existing cross.rs test suite to confirm no regression**

Run: `cargo test -p atheneum --lib cross::tests`
Expected: all pre-existing tests (`test_sanitize_*`, `test_cross_search_attaches_and_finds_symbols`, `test_cross_navigate_walks_subgraph`) still PASS unchanged.

- [ ] **Step 7: Commit**

```bash
git add crates/atheneum/src/cross.rs
git commit -m "feat(atheneum): always attach central knowledge db in CrossRouter

cross_search/cross_navigate previously only reached databases registered
as projects in magellan's meta.db. The central .atheneum/atheneum.db
knowledge store is not (and should not be) registered as an ordinary
project, so it was silently unreachable from any cross-tool graph
navigation. Adds an explicit always-attached synthetic project entry."
```

---

### Task 2: Envelope, pagination, provenance, and clamping module

**Files:**
- Create: `crates/atheneum-mcp/src/envelope.rs`
- Modify: `crates/atheneum-mcp/src/lib.rs:6-7` (add `pub mod envelope;`)
- Modify: `crates/atheneum-mcp/Cargo.toml` (add `base64` dependency)
- Test: `crates/atheneum-mcp/src/envelope.rs` (inline `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: nothing from other tasks (pure data/logic module).
- Produces (used by Tasks 3-7):
  - `Provenance::{Extracted, Inferred, Ambiguous}` (serializes as `"EXTRACTED"|"INFERRED"|"AMBIGUOUS"`)
  - `EnvelopeError { backend: String, code: String, message: String }`
  - `Envelope { items: Vec<Value>, limit: usize, cursor: Option<String>, has_more: bool, code_stale: Option<bool>, knowledge_stale: Option<bool>, depth_clamped: bool, errors: Vec<EnvelopeError> }` with `Envelope::new(limit: usize) -> Self` and `Envelope::to_value(&self) -> Value`
  - `clamp_limit(requested: Option<usize>) -> usize`
  - `clamp_depth(requested: Option<u32>) -> (u32, bool)` — returns `(clamped_value, was_clamped)`
  - `Cursor { backend: String, offset: usize }`, `encode_cursor(&Cursor) -> String`, `decode_cursor(&str) -> Option<Cursor>`
  - Error code constants: `ERR_PROJECT_NOT_FOUND`, `ERR_BACKEND_UNAVAILABLE`, `ERR_PARSE_ERROR`, `ERR_TIMEOUT`

- [ ] **Step 1: Add `base64` dependency**

```toml
# crates/atheneum-mcp/Cargo.toml, in [dependencies]
base64 = "0.22"
```

- [ ] **Step 2: Write the failing tests**

```rust
// crates/atheneum-mcp/src/envelope.rs

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_limit_defaults_to_20() {
        assert_eq!(clamp_limit(None), 20);
    }

    #[test]
    fn clamp_limit_caps_at_100() {
        assert_eq!(clamp_limit(Some(500)), 100);
    }

    #[test]
    fn clamp_limit_passes_through_valid_value() {
        assert_eq!(clamp_limit(Some(5)), 5);
    }

    #[test]
    fn clamp_limit_floors_zero_to_one() {
        assert_eq!(clamp_limit(Some(0)), 1);
    }

    #[test]
    fn clamp_depth_defaults_to_2_unclamped() {
        assert_eq!(clamp_depth(None), (2, false));
    }

    #[test]
    fn clamp_depth_caps_at_3_and_flags_clamped() {
        assert_eq!(clamp_depth(Some(10)), (3, true));
    }

    #[test]
    fn clamp_depth_passes_through_valid_value_unclamped() {
        assert_eq!(clamp_depth(Some(1)), (1, false));
    }

    #[test]
    fn cursor_round_trips_through_encode_decode() {
        let c = Cursor { backend: "knowledge".to_string(), offset: 42 };
        let encoded = encode_cursor(&c);
        let decoded = decode_cursor(&encoded).expect("cursor should decode");
        assert_eq!(decoded.backend, "knowledge");
        assert_eq!(decoded.offset, 42);
    }

    #[test]
    fn decode_cursor_rejects_garbage() {
        assert!(decode_cursor("not-a-valid-cursor!!!").is_none());
    }

    #[test]
    fn envelope_serializes_with_expected_shape() {
        let mut env = Envelope::new(20);
        env.items.push(serde_json::json!({"name": "foo"}));
        env.has_more = true;
        env.cursor = Some("abc".to_string());
        env.errors.push(EnvelopeError {
            backend: "code".to_string(),
            code: ERR_BACKEND_UNAVAILABLE.to_string(),
            message: "magellan binary not found".to_string(),
        });
        let v = env.to_value();
        assert_eq!(v["limit"], 20);
        assert_eq!(v["has_more"], true);
        assert_eq!(v["cursor"], "abc");
        assert_eq!(v["items"][0]["name"], "foo");
        assert_eq!(v["errors"][0]["code"], ERR_BACKEND_UNAVAILABLE);
    }

    #[test]
    fn provenance_serializes_as_uppercase_tag() {
        assert_eq!(serde_json::to_value(Provenance::Extracted).unwrap(), "EXTRACTED");
        assert_eq!(serde_json::to_value(Provenance::Inferred).unwrap(), "INFERRED");
        assert_eq!(serde_json::to_value(Provenance::Ambiguous).unwrap(), "AMBIGUOUS");
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p atheneum-mcp --lib envelope::tests`
Expected: FAIL — compile error, module contents don't exist yet.

- [ ] **Step 4: Implement the module**

```rust
// crates/atheneum-mcp/src/envelope.rs
//! Shared response envelope for the unified tool API: pagination, provenance
//! tagging, staleness signals, and error aggregation, used identically by
//! every dispatch path (atheneum in-process, code-tool subprocess, envoy HTTP).

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const DEFAULT_LIMIT: usize = 20;
pub const MAX_LIMIT: usize = 100;
pub const DEFAULT_DEPTH: u32 = 2;
pub const MAX_DEPTH: u32 = 3;

pub const ERR_PROJECT_NOT_FOUND: &str = "PROJECT_NOT_FOUND";
pub const ERR_BACKEND_UNAVAILABLE: &str = "BACKEND_UNAVAILABLE";
pub const ERR_PARSE_ERROR: &str = "PARSE_ERROR";
pub const ERR_TIMEOUT: &str = "TIMEOUT";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Provenance {
    Extracted,
    Inferred,
    Ambiguous,
}

#[derive(Debug, Clone, Serialize)]
pub struct EnvelopeError {
    pub backend: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Envelope {
    pub items: Vec<Value>,
    pub limit: usize,
    pub cursor: Option<String>,
    pub has_more: bool,
    pub code_stale: Option<bool>,
    pub knowledge_stale: Option<bool>,
    pub depth_clamped: bool,
    pub errors: Vec<EnvelopeError>,
}

impl Envelope {
    pub fn new(limit: usize) -> Self {
        Self {
            items: Vec::new(),
            limit,
            cursor: None,
            has_more: false,
            code_stale: None,
            knowledge_stale: None,
            depth_clamped: false,
            errors: Vec::new(),
        }
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

pub fn clamp_limit(requested: Option<usize>) -> usize {
    requested.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

pub fn clamp_depth(requested: Option<u32>) -> (u32, bool) {
    let req = requested.unwrap_or(DEFAULT_DEPTH);
    let clamped = req.min(MAX_DEPTH);
    (clamped, clamped != req)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cursor {
    pub backend: String,
    pub offset: usize,
}

pub fn encode_cursor(cursor: &Cursor) -> String {
    let json = serde_json::to_string(cursor).unwrap_or_default();
    base64::engine::general_purpose::STANDARD.encode(json)
}

pub fn decode_cursor(s: &str) -> Option<Cursor> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(s).ok()?;
    let json = String::from_utf8(bytes).ok()?;
    serde_json::from_str(&json).ok()
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p atheneum-mcp --lib envelope::tests`
Expected: PASS (10 tests)

- [ ] **Step 6: Register the module**

```rust
// crates/atheneum-mcp/src/lib.rs, line 6-7, change:
pub mod backend;
pub mod tools;
// to:
pub mod backend;
pub mod envelope;
pub mod tools;
```

- [ ] **Step 7: Commit**

```bash
git add crates/atheneum-mcp/src/envelope.rs crates/atheneum-mcp/src/lib.rs crates/atheneum-mcp/Cargo.toml
git commit -m "feat(atheneum-mcp): add shared response envelope module

Pagination (limit/cursor/has_more), provenance tagging
(EXTRACTED/INFERRED/AMBIGUOUS), two-tier staleness fields, and a uniform
errors[] array — the shape every dispatch adapter in the unified tool API
will return, instead of each backend inventing its own response shape."
```

---

### Task 3: Extend `search` tool with `kind` — code-side fan-out via `CrossRouter`

**Files:**
- Modify: `crates/atheneum-mcp/src/backend.rs` (`Backend` trait, `DirectBackend` impl, `HttpBackend` impl, `MockBackend` test impl in `lib.rs`)
- Modify: `crates/atheneum-mcp/src/tools.rs:160-192` (`search` tool schema + handler)
- Modify: `crates/atheneum-mcp/src/lib.rs` (`MockBackend` in `#[cfg(test)]`)
- Test: `crates/atheneum-mcp/src/backend.rs` (inline)

**Interfaces:**
- Consumes: `atheneum::cross::CrossRouter` (Task 1), `envelope::{Envelope, Provenance, clamp_limit, Cursor, encode_cursor, decode_cursor}` (Task 2).
- Produces: `Backend::search` signature changes from `search(&self, query: &str, k: usize, project: Option<&str>) -> Result<Value>` to `search(&self, params: SearchParams) -> Result<Value>` where:
```rust
pub struct SearchParams {
    pub query: String,
    pub k: usize,
    pub project: Option<String>,
    pub kind: SearchKind,     // default: SearchKind::Knowledge — preserves old behavior exactly
    pub limit: Option<usize>,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchKind { Knowledge, Code, All }
```
Later tasks (5, 6) reuse `SearchKind`-style enums for their own `kind` dispatch.

- [ ] **Step 1: Write the failing test — old callers unaffected**

```rust
// crates/atheneum-mcp/src/backend.rs, inside mod tests (direct backend section)

#[tokio::test]
async fn search_without_kind_defaults_to_knowledge_only_unchanged_shape() {
    let backend = test_direct_backend_with_seeded_memory(); // helper added in this step, see below
    let params = SearchParams {
        query: "seeded".to_string(),
        k: 10,
        project: None,
        kind: SearchKind::Knowledge,
        limit: None,
        cursor: None,
    };
    let result = backend.search(params).await.unwrap();
    // Envelope shape: items present, no code-backend errors since kind=Knowledge
    // never touches CrossRouter.
    assert!(result["items"].is_array());
    assert!(result["errors"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn search_kind_all_merges_knowledge_and_code_with_provenance_tags() {
    let (backend, magellan_db_path, project_name) = test_direct_backend_with_registered_code_project();
    let params = SearchParams {
        query: "shared_probe_symbol".to_string(),
        k: 10,
        project: Some(project_name),
        kind: SearchKind::All,
        limit: None,
        cursor: None,
    };
    let result = backend.search(params).await.unwrap();
    let items = result["items"].as_array().unwrap();
    assert!(
        items.iter().any(|i| i["provenance"] == "EXTRACTED"),
        "expected at least one EXTRACTED (code) hit, got {items:?}"
    );
}
```

(`test_direct_backend_with_seeded_memory` and `test_direct_backend_with_registered_code_project` are small test-only helper functions this step also adds: the first opens a temp `AtheneumGraph`, calls `graph.store_memory(...)` once, wraps it in `DirectBackend::new`; the second additionally builds a temp magellan-shaped SQLite db with one `graph_entities` row named `shared_probe_symbol` and registers it via a temp `MetaRouter`/`meta.register_project`, then constructs `DirectBackend` with a `CrossRouter` pointed at that temp `meta.db`. Follow the exact fixture pattern already used in `crates/atheneum/src/cross.rs`'s `make_magellan_like_db` — do not invent a new fixture style.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p atheneum-mcp --lib backend::tests::search_kind_all_merges_knowledge_and_code_with_provenance_tags`
Expected: FAIL — compile error (`SearchParams`/`SearchKind` don't exist, `search` signature mismatch).

- [ ] **Step 3: Change the `Backend` trait signature**

```rust
// crates/atheneum-mcp/src/backend.rs, near existing trait Backend (line ~126)
// Replace:
//     async fn search(&self, query: &str, k: usize, project: Option<&str>) -> Result<Value>;
// with:
async fn search(&self, params: SearchParams) -> Result<Value>;
```

Add the new types above the trait:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchKind {
    #[default]
    Knowledge,
    Code,
    All,
}

impl SearchKind {
    pub fn from_str_default(s: Option<&str>) -> Self {
        match s {
            Some("code") => SearchKind::Code,
            Some("all") => SearchKind::All,
            _ => SearchKind::Knowledge,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SearchParams {
    pub query: String,
    pub k: usize,
    pub project: Option<String>,
    pub kind: SearchKind,
    pub limit: Option<usize>,
    pub cursor: Option<String>,
}
```

- [ ] **Step 4: Update `DirectBackend` — add a `CrossRouter` handle and implement dispatch**

`DirectBackend` currently only holds `graph: Arc<tokio::sync::Mutex<AtheneumGraph>>`. Add an optional cross-router handle (optional so existing construction sites / tests that don't care about code search keep compiling untouched):

```rust
pub struct DirectBackend {
    graph: Arc<tokio::sync::Mutex<AtheneumGraph>>,
    cross: Option<Arc<tokio::sync::Mutex<atheneum::cross::CrossRouter>>>, // NEW
}

impl DirectBackend {
    pub fn new(graph: Arc<tokio::sync::Mutex<AtheneumGraph>>) -> Self {
        Self { graph, cross: None }
    }

    pub fn with_cross_router(
        graph: Arc<tokio::sync::Mutex<AtheneumGraph>>,
        cross: atheneum::cross::CrossRouter,
    ) -> Self {
        Self { graph, cross: Some(Arc::new(tokio::sync::Mutex::new(cross))) }
    }
}

pub fn direct_from_graph(graph: AtheneumGraph) -> DirectBackend {
    DirectBackend::new(Arc::new(tokio::sync::Mutex::new(graph)))
}
```

Implement the trait method:

```rust
async fn search(&self, params: SearchParams) -> Result<Value> {
    let limit = crate::envelope::clamp_limit(params.limit);
    let mut envelope = crate::envelope::Envelope::new(limit);

    if matches!(params.kind, SearchKind::Knowledge | SearchKind::All) {
        let graph = self.graph.lock().await;
        let knowledge_results = tokio::task::block_in_place(|| {
            graph.lexical_search(&params.query, params.k, params.project.as_deref(), None, None)
        });
        match knowledge_results {
            Ok(results) => {
                for r in results {
                    let mut v = serde_json::to_value(&r)?;
                    v["provenance"] = serde_json::json!(crate::envelope::Provenance::Inferred);
                    v["source"] = serde_json::json!("knowledge");
                    envelope.items.push(v);
                }
            }
            Err(e) => envelope.errors.push(crate::envelope::EnvelopeError {
                backend: "knowledge".to_string(),
                code: crate::envelope::ERR_BACKEND_UNAVAILABLE.to_string(),
                message: e.to_string(),
            }),
        }
    }

    if matches!(params.kind, SearchKind::Code | SearchKind::All) {
        match &self.cross {
            Some(cross) => {
                let mut cross = cross.lock().await;
                let code_results = tokio::task::block_in_place(|| cross.cross_search(&params.query, None, params.k));
                match code_results {
                    Ok(results) => {
                        for r in results {
                            envelope.items.push(serde_json::json!({
                                "project": r.project,
                                "id": r.id,
                                "kind": r.kind,
                                "name": r.name,
                                "file_path": r.file_path,
                                "data": r.data,
                                "provenance": crate::envelope::Provenance::Extracted,
                                "source": "code",
                            }));
                        }
                    }
                    Err(e) => envelope.errors.push(crate::envelope::EnvelopeError {
                        backend: "code".to_string(),
                        code: crate::envelope::ERR_BACKEND_UNAVAILABLE.to_string(),
                        message: e.to_string(),
                    }),
                }
            }
            None => {
                if matches!(params.kind, SearchKind::Code) {
                    envelope.errors.push(crate::envelope::EnvelopeError {
                        backend: "code".to_string(),
                        code: crate::envelope::ERR_BACKEND_UNAVAILABLE.to_string(),
                        message: "code search unavailable: no CrossRouter configured".to_string(),
                    });
                }
            }
        }
    }

    let offset = params
        .cursor
        .as_deref()
        .and_then(crate::envelope::decode_cursor)
        .map(|c| c.offset)
        .unwrap_or(0);
    let total = envelope.items.len();
    let page: Vec<Value> = envelope.items.into_iter().skip(offset).take(limit).collect();
    envelope.has_more = offset + page.len() < total;
    envelope.items = page;
    if envelope.has_more {
        envelope.cursor = Some(crate::envelope::encode_cursor(&crate::envelope::Cursor {
            backend: "search".to_string(),
            offset: offset + envelope.items.len(),
        }));
    }

    Ok(envelope.to_value())
}
```

- [ ] **Step 5: Update `HttpBackend::search` and `lib.rs`'s `MockBackend::search` to match the new signature**

`HttpBackend::search` (line ~352): change its parameter to `params: SearchParams`, forward `params.query`/`params.k`/`params.project` to whatever HTTP call it already makes (unchanged endpoint/body — `kind`/`limit`/`cursor` are new-capability fields this task does not require the HTTP backend to support yet; document that with one comment):

```rust
async fn search(&self, params: SearchParams) -> Result<Value> {
    // NOTE: HTTP backend does not yet support kind/limit/cursor — forwards
    // query/k/project only, same as before this task. Tracked as a gap, not
    // silently dropped: see docs/superpowers/specs/2026-07-30-unified-tool-api-design.md.
    self.get_json(&format!(
        "/atheneum/search?query={}&k={}{}",
        urlencoding::encode(&params.query),
        params.k,
        params.project.as_deref().map(|p| format!("&project={}", urlencoding::encode(p))).unwrap_or_default(),
    )).await
}
```

(Match whatever `HttpBackend::search`'s existing URL-building already does — do not change the endpoint, only the parameter destructuring. Read lines 352-388 before editing to preserve the exact existing query-string format.)

In `crates/atheneum-mcp/src/lib.rs`'s `MockBackend`:

```rust
// Replace:
//     async fn search(&self, _q: &str, _k: usize, _p: Option<&str>) -> anyhow::Result<Value> {
// with:
async fn search(&self, _params: backend::SearchParams) -> anyhow::Result<Value> {
    Ok(Value::Null)
}
```

- [ ] **Step 6: Update the `search` tool schema and handler in `tools.rs`**

```rust
// crates/atheneum-mcp/src/tools.rs, replace the search() function (lines 160-192):
fn search() -> rmcp::handler::server::router::tool::ToolRoute<AtheneumMcpServer> {
    let schema = json!({
        "type": "object",
        "properties": {
            "query": { "type": "string", "description": "Search query" },
            "k": { "type": "integer", "minimum": 1, "maximum": 100, "default": 10, "description": "Number of results to consider before pagination" },
            "project": { "type": "string", "description": "Optional project scope" },
            "kind": {
                "type": "string",
                "enum": ["knowledge", "code", "all"],
                "default": "knowledge",
                "description": "knowledge = atheneum only (default, unchanged behavior). code = magellan/llmgrep cross-project symbol search only. all = both, merged and provenance-tagged."
            },
            "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 20, "description": "Page size" },
            "cursor": { "type": "string", "description": "Opaque pagination cursor from a previous call's has_more response" }
        },
        "required": ["query"]
    });
    let schema: Map<String, Value> = schema.as_object().unwrap().clone();

    rmcp::handler::server::router::tool::ToolRoute::new_dyn(
        Tool::new("search", "Search the knowledge graph and/or cross-project code index. Impact/affected-style code results are a first-pass heuristic, not certainty.", schema),
        |ctx: rmcp::handler::server::tool::ToolCallContext<'_, AtheneumMcpServer>| {
            Box::pin(async move {
                let args = extract_args(ctx.arguments);
                let params = crate::backend::SearchParams {
                    query: args["query"].as_str().unwrap_or("").to_string(),
                    k: args["k"].as_u64().unwrap_or(10) as usize,
                    project: args["project"].as_str().map(String::from),
                    kind: crate::backend::SearchKind::from_str_default(args["kind"].as_str()),
                    limit: args["limit"].as_u64().map(|v| v as usize),
                    cursor: args["cursor"].as_str().map(String::from),
                };
                let result = ctx.service.backend.search(params).await;
                match result {
                    Ok(v) => json_result(v),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
                }
            })
        },
    )
}
```

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo test -p atheneum-mcp --lib`
Expected: PASS — both new tests, and the full existing suite (`all_twenty_tools_registered` etc. in `lib.rs` — tool count unchanged, still 29, since `search` was modified in place, not renamed).

- [ ] **Step 8: Commit**

```bash
git add crates/atheneum-mcp/src/backend.rs crates/atheneum-mcp/src/tools.rs crates/atheneum-mcp/src/lib.rs
git commit -m "feat(atheneum-mcp): extend search tool with kind=knowledge|code|all

Default kind=knowledge preserves today's exact behavior for existing
callers. kind=code/all additionally fans out through CrossRouter (with
the Task 1 central-store fix), merging results with EXTRACTED/INFERRED
provenance tags and the shared pagination envelope."
```

---

### Task 4: Extend `navigate` tool with `kind` — depth-capped cross-project graph walk

**Files:**
- Modify: `crates/atheneum-mcp/src/backend.rs` (`Backend::navigate`, `DirectBackend::navigate`, `HttpBackend::navigate`)
- Modify: `crates/atheneum-mcp/src/tools.rs` (`navigate` tool — find via `grep -n "fn navigate()" crates/atheneum-mcp/src/tools.rs`)
- Modify: `crates/atheneum-mcp/src/lib.rs` (`MockBackend::navigate`)
- Test: `crates/atheneum-mcp/src/backend.rs` (inline)

**Interfaces:**
- Consumes: `envelope::{clamp_depth, Provenance}` (Task 2), `CrossRouter::cross_navigate` (Task 1), `DirectBackend.cross` field (Task 3).
- Produces: `Backend::navigate` signature changes from `navigate(&self, query: &str, k: usize, depth: u32, offset: usize, limit: usize, trace: Option<bool>) -> Result<Value>` to `navigate(&self, params: NavigateParams) -> Result<Value>` where:
```rust
pub struct NavigateParams {
    pub query: String,
    pub k: usize,
    pub depth: Option<u32>,        // None -> envelope::DEFAULT_DEPTH via clamp_depth
    pub offset: usize,
    pub limit: usize,
    pub trace: Option<bool>,
    pub kind: SearchKind,          // default Knowledge — preserves old behavior exactly
}
```

- [ ] **Step 1: Write the failing test**

```rust
// crates/atheneum-mcp/src/backend.rs, inside mod tests

#[tokio::test]
async fn navigate_depth_beyond_max_is_clamped_and_flagged() {
    let backend = test_direct_backend_with_seeded_memory();
    let params = NavigateParams {
        query: "seeded".to_string(),
        k: 5,
        depth: Some(10),
        offset: 0,
        limit: 20,
        trace: None,
        kind: SearchKind::Knowledge,
    };
    let result = backend.navigate(params).await.unwrap();
    assert_eq!(result["depth_clamped"], true);
}

#[tokio::test]
async fn navigate_kind_all_tags_code_hits_ambiguous_beyond_first_hop() {
    let (backend, _db_path, project_name) = test_direct_backend_with_registered_code_project();
    let params = NavigateParams {
        query: "shared_probe_symbol".to_string(),
        k: 5,
        depth: Some(2),
        offset: 0,
        limit: 20,
        trace: None,
        kind: SearchKind::All,
    };
    let _ = project_name; // cross_navigate in this router searches all registered projects
    let result = backend.navigate(params).await.unwrap();
    let items = result["items"].as_array().unwrap();
    assert!(
        items.iter().any(|i| i["source"] == "code"),
        "expected at least one code-sourced navigate item, got {items:?}"
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p atheneum-mcp --lib backend::tests::navigate_depth_beyond_max_is_clamped_and_flagged`
Expected: FAIL — compile error (`NavigateParams` doesn't exist, `navigate` signature mismatch).

- [ ] **Step 3: Add `NavigateParams` and change the trait signature**

```rust
// crates/atheneum-mcp/src/backend.rs, near SearchParams
#[derive(Debug, Clone)]
pub struct NavigateParams {
    pub query: String,
    pub k: usize,
    pub depth: Option<u32>,
    pub offset: usize,
    pub limit: usize,
    pub trace: Option<bool>,
    pub kind: SearchKind,
}

// In trait Backend, replace:
//     async fn navigate(&self, query: &str, k: usize, depth: u32, offset: usize, limit: usize, trace: Option<bool>) -> Result<Value>;
// with:
async fn navigate(&self, params: NavigateParams) -> Result<Value>;
```

- [ ] **Step 4: Implement in `DirectBackend`**

Keep the existing `graph.navigate_with_trace(...)` + `serialize_paginated_view(...)` path for the knowledge side completely untouched in its internals — only change what wraps it:

```rust
async fn navigate(&self, params: NavigateParams) -> Result<Value> {
    let (depth, depth_clamped) = crate::envelope::clamp_depth(params.depth);
    let mut envelope = crate::envelope::Envelope::new(params.limit.max(1));
    envelope.depth_clamped = depth_clamped;

    if matches!(params.kind, SearchKind::Knowledge | SearchKind::All) {
        let graph = self.graph.lock().await;
        let knowledge_result = tokio::task::block_in_place(|| {
            graph.navigate_with_trace(
                &params.query, params.k, depth, None, None, None, params.trace.unwrap_or(false),
            )
        });
        match knowledge_result {
            Ok((results, trace_id)) => {
                for v in results {
                    let mut view = serialize_paginated_view(
                        &v.entry, v.depth, v.entities, v.edges, params.offset, params.limit,
                    );
                    view["provenance"] = serde_json::json!(
                        if v.depth <= 1 { crate::envelope::Provenance::Inferred } else { crate::envelope::Provenance::Ambiguous }
                    );
                    view["source"] = serde_json::json!("knowledge");
                    envelope.items.push(view);
                    if let Some(tid) = &trace_id {
                        envelope.items.last_mut().unwrap()["trace_id"] = serde_json::json!(tid);
                    }
                }
            }
            Err(e) => envelope.errors.push(crate::envelope::EnvelopeError {
                backend: "knowledge".to_string(),
                code: crate::envelope::ERR_BACKEND_UNAVAILABLE.to_string(),
                message: e.to_string(),
            }),
        }
    }

    if matches!(params.kind, SearchKind::Code | SearchKind::All) {
        match &self.cross {
            Some(cross) => {
                let mut cross = cross.lock().await;
                let code_result = tokio::task::block_in_place(|| cross.cross_navigate(&params.query, None, params.k, depth));
                match code_result {
                    Ok(subgraphs) => {
                        for sg in subgraphs {
                            envelope.items.push(serde_json::json!({
                                "project": sg.project,
                                "entry_id": sg.entry_id,
                                "entity_count": sg.entities.len(),
                                "edge_count": sg.edges.len(),
                                "entities": sg.entities.iter().map(|e| serde_json::json!({
                                    "id": e.id, "kind": e.kind, "name": e.name, "file_path": e.file_path,
                                })).collect::<Vec<_>>(),
                                "provenance": if depth <= 1 { crate::envelope::Provenance::Extracted } else { crate::envelope::Provenance::Ambiguous },
                                "source": "code",
                            }));
                        }
                    }
                    Err(e) => envelope.errors.push(crate::envelope::EnvelopeError {
                        backend: "code".to_string(),
                        code: crate::envelope::ERR_BACKEND_UNAVAILABLE.to_string(),
                        message: e.to_string(),
                    }),
                }
            }
            None if matches!(params.kind, SearchKind::Code) => {
                envelope.errors.push(crate::envelope::EnvelopeError {
                    backend: "code".to_string(),
                    code: crate::envelope::ERR_BACKEND_UNAVAILABLE.to_string(),
                    message: "code navigate unavailable: no CrossRouter configured".to_string(),
                });
            }
            None => {}
        }
    }

    Ok(envelope.to_value())
}
```

- [ ] **Step 5: Update `HttpBackend::navigate` and `MockBackend::navigate` signatures**

Same mechanical pattern as Task 3 Step 5 — destructure `NavigateParams` instead of six positional args, preserve existing HTTP call/URL exactly, add the same "does not yet support kind" comment.

- [ ] **Step 6: Update the `navigate` tool in `tools.rs`**

Find the existing schema (`grep -n "fn navigate()" -A 40 crates/atheneum-mcp/src/tools.rs`), add `"kind"` to its `properties` (same enum block as Task 3 Step 6), and change the handler body to construct `NavigateParams { query, k, depth: args["depth"].as_u64().map(|v| v as u32), offset, limit, trace, kind: SearchKind::from_str_default(args["kind"].as_str()) }` instead of six positional args — mirror Task 3 Step 6's handler-construction style exactly.

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo test -p atheneum-mcp --lib`
Expected: PASS, full suite including `all_twenty_tools_registered` (still 29 tools).

- [ ] **Step 8: Commit**

```bash
git add crates/atheneum-mcp/src/backend.rs crates/atheneum-mcp/src/tools.rs crates/atheneum-mcp/src/lib.rs
git commit -m "feat(atheneum-mcp): extend navigate tool with kind + depth clamp

Default kind=knowledge preserves existing navigate_with_trace pagination
behavior byte-for-byte. kind=code/all fans out through CrossRouter's
cross_navigate, depth hard-capped at 3 (envelope::clamp_depth), hops
beyond the first tagged AMBIGUOUS rather than presented as certain."
```

---

### Task 5: `code_query` tool — subprocess adapter for deep structural code queries

**Files:**
- Create: `crates/atheneum-mcp/src/subprocess.rs`
- Modify: `crates/atheneum-mcp/src/lib.rs` (add `pub mod subprocess;`, extend `Backend` trait + `MockBackend`)
- Modify: `crates/atheneum-mcp/src/backend.rs` (`DirectBackend`/`HttpBackend` impls)
- Modify: `crates/atheneum-mcp/src/tools.rs` (register new `code_query` tool)
- Test: `crates/atheneum-mcp/src/subprocess.rs` (inline)

Rationale for not depending on `grounded-mcp`'s crate directly: `grounded-mcp` lives in a separate top-level repo/workspace (`/home/feanor/Projects/grounded-mcp`), not inside this Cargo workspace. A cross-repo path dependency would couple two independently-versioned repos. Instead this replicates grounded-mcp's small (~40-line), already-proven `resolve()`/`run_json()` dispatch pattern verbatim — this is copying a validated pattern across a repo boundary, not reinventing it.

**Interfaces:**
- Consumes: `MetaRouter::get_project` (existing, `crates/atheneum/src/meta.rs:322`) for project-name → `magellan_db` path resolution. `envelope::{Envelope, Provenance, ERR_PROJECT_NOT_FOUND, ERR_BACKEND_UNAVAILABLE, ERR_TIMEOUT, ERR_PARSE_ERROR}` (Task 2).
- Produces: `subprocess::CodeQueryRunner` with `pub async fn run(&self, magellan_db: &str, tool: CodeTool, args: Vec<String>) -> anyhow::Result<Value>`, and `Backend::code_query(&self, params: CodeQueryParams) -> Result<Value>` where:
```rust
pub struct CodeQueryParams {
    pub project: String,
    pub tool: String,   // "magellan" | "llmgrep" | "mirage"
    pub subcommand: String,
    pub args: Vec<String>,
}
```

- [ ] **Step 1: Write the failing test**

```rust
// crates/atheneum-mcp/src/subprocess.rs

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn run_returns_backend_unavailable_error_shape_when_binary_missing() {
        let runner = CodeQueryRunner::with_bin_dir(std::path::PathBuf::from("/nonexistent-bin-dir"));
        let result = runner.run("/tmp/does-not-matter.db", CodeTool::Magellan, vec!["status".to_string(), "--db".to_string(), "/tmp/does-not-matter.db".to_string()]).await;
        assert!(result.is_err(), "expected spawn failure for missing binary, got {result:?}");
    }

    #[tokio::test]
    async fn run_parses_json_stdout() {
        // Uses `echo` as a stand-in "tool" to verify the JSON-parse path
        // without depending on a real magellan binary being installed.
        let runner = CodeQueryRunner::with_bin_dir(std::path::PathBuf::from("/usr/bin"));
        let mut cmd = tokio::process::Command::new("/bin/echo");
        cmd.arg(r#"{"ok":true}"#);
        let value = runner.run_command(cmd, "test_echo").await.unwrap();
        assert_eq!(value["ok"], true);
    }

    #[tokio::test]
    async fn run_falls_back_to_wrapped_text_on_non_json_stdout() {
        let runner = CodeQueryRunner::with_bin_dir(std::path::PathBuf::from("/usr/bin"));
        let mut cmd = tokio::process::Command::new("/bin/echo");
        cmd.arg("plain text, not json");
        let value = runner.run_command(cmd, "test_echo").await.unwrap();
        assert_eq!(value["tool"], "test_echo");
        assert!(value["output"].as_str().unwrap().contains("plain text"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p atheneum-mcp --lib subprocess::tests`
Expected: FAIL — module doesn't exist.

- [ ] **Step 3: Implement `subprocess.rs`** (pattern copied from `grounded-mcp/src/backend.rs`'s `resolve`/`run_json`)

```rust
// crates/atheneum-mcp/src/subprocess.rs
//! Subprocess dispatch for magellan/llmgrep/mirage CLI binaries — the
//! code_query tool's adapter. Pattern copied from grounded-mcp's
//! SubprocessBackend (separate repo, not a workspace dependency — see
//! Task 5 rationale in the implementation plan).

use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;
use tokio::process::Command;

const CODE_TOOL_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy)]
pub enum CodeTool {
    Magellan,
    Llmgrep,
    Mirage,
}

impl CodeTool {
    fn bin_name(self) -> &'static str {
        match self {
            CodeTool::Magellan => "magellan",
            CodeTool::Llmgrep => "llmgrep",
            CodeTool::Mirage => "mirage",
        }
    }
}

pub struct CodeQueryRunner {
    bin_dir: PathBuf,
}

impl CodeQueryRunner {
    pub fn new() -> Self {
        let bin_dir = std::env::var("GROUNDED_BIN_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(shellexpand::tilde("~/.local/bin").to_string()));
        Self { bin_dir }
    }

    pub fn with_bin_dir(bin_dir: PathBuf) -> Self {
        Self { bin_dir }
    }

    fn resolve(&self, name: &str) -> String {
        let candidate = self.bin_dir.join(name);
        if candidate.is_file() {
            candidate.to_string_lossy().into_owned()
        } else {
            name.to_string()
        }
    }

    pub async fn run(&self, magellan_db: &str, tool: CodeTool, args: Vec<String>) -> Result<Value> {
        let mut cmd = Command::new(self.resolve(tool.bin_name()));
        cmd.env_clear();
        cmd.args(&args);
        let _ = magellan_db; // callers pass --db within `args`; kept as a param for call-site clarity
        self.run_command(cmd, tool.bin_name()).await
    }

    pub async fn run_command(&self, mut cmd: Command, label: &str) -> Result<Value> {
        let run = async {
            cmd.stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true)
                .output()
                .await
                .with_context(|| format!("failed to spawn `{label}`"))
        };
        let output = tokio::time::timeout(CODE_TOOL_TIMEOUT, run)
            .await
            .map_err(|_| anyhow!("`{label}` timed out after {CODE_TOOL_TIMEOUT:?}"))??;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            return Err(anyhow!(
                "`{label}` exited with status {}: stderr: {stderr}; stdout: {stdout}",
                output.status
            ));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let trimmed = stdout.trim();
        match serde_json::from_str::<Value>(trimmed) {
            Ok(v) => Ok(v),
            Err(_) => Ok(serde_json::json!({ "tool": label, "output": trimmed })),
        }
    }
}

impl Default for CodeQueryRunner {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p atheneum-mcp --lib subprocess::tests`
Expected: PASS (3 tests)

- [ ] **Step 5: Wire `code_query` into `Backend` trait + `DirectBackend` + tool registration**

```rust
// crates/atheneum-mcp/src/backend.rs — add to trait Backend:
async fn code_query(&self, params: CodeQueryParams) -> Result<Value>;

#[derive(Debug, Clone)]
pub struct CodeQueryParams {
    pub project: String,
    pub tool: String,
    pub subcommand: String,
    pub args: Vec<String>,
}
```

`DirectBackend` needs a `MetaRouter` handle to resolve `project` → `magellan_db` path, and a `CodeQueryRunner`. Add both as fields (constructed alongside `cross` in Task 3's `with_cross_router`, or via a new constructor — reuse `with_cross_router`'s `CrossRouter`, which already owns a `MetaRouter` via `cross.meta()`):

```rust
async fn code_query(&self, params: CodeQueryParams) -> Result<Value> {
    let mut envelope = crate::envelope::Envelope::new(1);
    let Some(cross) = &self.cross else {
        envelope.errors.push(crate::envelope::EnvelopeError {
            backend: "code".to_string(),
            code: crate::envelope::ERR_BACKEND_UNAVAILABLE.to_string(),
            message: "code_query unavailable: no CrossRouter configured".to_string(),
        });
        return Ok(envelope.to_value());
    };
    let magellan_db = {
        let cross = cross.lock().await;
        match cross.meta().get_project(&params.project) {
            Ok(Some(p)) => p.magellan_db,
            Ok(None) => {
                envelope.errors.push(crate::envelope::EnvelopeError {
                    backend: "code".to_string(),
                    code: crate::envelope::ERR_PROJECT_NOT_FOUND.to_string(),
                    message: format!("project '{}' not found in meta.db", params.project),
                });
                return Ok(envelope.to_value());
            }
            Err(e) => {
                envelope.errors.push(crate::envelope::EnvelopeError {
                    backend: "code".to_string(),
                    code: crate::envelope::ERR_BACKEND_UNAVAILABLE.to_string(),
                    message: e.to_string(),
                });
                return Ok(envelope.to_value());
            }
        }
    };

    let tool = match params.tool.as_str() {
        "magellan" => crate::subprocess::CodeTool::Magellan,
        "llmgrep" => crate::subprocess::CodeTool::Llmgrep,
        "mirage" => crate::subprocess::CodeTool::Mirage,
        other => {
            envelope.errors.push(crate::envelope::EnvelopeError {
                backend: "code".to_string(),
                code: crate::envelope::ERR_PARSE_ERROR.to_string(),
                message: format!("unknown tool '{other}', expected magellan|llmgrep|mirage"),
            });
            return Ok(envelope.to_value());
        }
    };

    let mut args = vec![params.subcommand.clone(), "--db".to_string(), magellan_db.clone()];
    args.extend(params.args.clone());
    let runner = crate::subprocess::CodeQueryRunner::new();
    match runner.run(&magellan_db, tool, args).await {
        Ok(v) => {
            let mut tagged = v;
            tagged["provenance"] = serde_json::json!(crate::envelope::Provenance::Extracted);
            tagged["source"] = serde_json::json!("code");
            envelope.items.push(tagged);
        }
        Err(e) => envelope.errors.push(crate::envelope::EnvelopeError {
            backend: "code".to_string(),
            code: if e.to_string().contains("timed out") { crate::envelope::ERR_TIMEOUT.to_string() } else { crate::envelope::ERR_BACKEND_UNAVAILABLE.to_string() },
            message: e.to_string(),
        }),
    }
    Ok(envelope.to_value())
}
```

Add to `HttpBackend`: return `Err(anyhow::anyhow!("code_query not supported over HTTP backend"))` (explicit, not a silent no-op — matches the plan's "never a silent failure" rule). Add to `lib.rs`'s `MockBackend`: `async fn code_query(&self, _p: backend::CodeQueryParams) -> anyhow::Result<Value> { Ok(Value::Null) }`.

Register the tool in `tools.rs` (new function `code_query()`, same `ToolRoute::new_dyn` pattern as Task 3 Step 6):

```rust
fn code_query() -> rmcp::handler::server::router::tool::ToolRoute<AtheneumMcpServer> {
    let schema = json!({
        "type": "object",
        "properties": {
            "project": { "type": "string", "description": "Project name, resolved via magellan's project registry — never a raw db path" },
            "tool": { "type": "string", "enum": ["magellan", "llmgrep", "mirage"] },
            "subcommand": { "type": "string", "description": "e.g. context_impact, refs, cfg" },
            "args": { "type": "array", "items": { "type": "string" }, "description": "Extra CLI flags/args beyond --db" }
        },
        "required": ["project", "tool", "subcommand"]
    });
    let schema: Map<String, Value> = schema.as_object().unwrap().clone();
    rmcp::handler::server::router::tool::ToolRoute::new_dyn(
        Tool::new("code_query", "Deep structural code query (impact/refs/cfg/etc) via magellan/llmgrep/mirage, resolved by project name. Results are EXTRACTED (deterministic) but impact-style answers are a first-pass heuristic.", schema),
        |ctx: rmcp::handler::server::tool::ToolCallContext<'_, AtheneumMcpServer>| {
            Box::pin(async move {
                let args = extract_args(ctx.arguments);
                let params = crate::backend::CodeQueryParams {
                    project: args["project"].as_str().unwrap_or("").to_string(),
                    tool: args["tool"].as_str().unwrap_or("").to_string(),
                    subcommand: args["subcommand"].as_str().unwrap_or("").to_string(),
                    args: args["args"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default(),
                };
                let result = ctx.service.backend.code_query(params).await;
                match result {
                    Ok(v) => json_result(v),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
                }
            })
        },
    )
}
```

Add `router.add_route(code_query());` to `register_all` in `tools.rs`, and bump the expected tool count in `lib.rs`'s `all_twenty_tools_registered` test from `29` to `30` plus an added `assert!(names.contains(&"code_query"));`.

- [ ] **Step 6: Run full test suite**

Run: `cargo test -p atheneum-mcp --lib`
Expected: PASS, including updated tool-count assertion.

- [ ] **Step 7: Commit**

```bash
git add crates/atheneum-mcp/src/subprocess.rs crates/atheneum-mcp/src/backend.rs crates/atheneum-mcp/src/tools.rs crates/atheneum-mcp/src/lib.rs
git commit -m "feat(atheneum-mcp): add code_query tool (subprocess adapter)

Resolves project name -> magellan db path via the existing meta.db
registry (no raw db paths in the tool schema), shells out to
magellan/llmgrep/mirage with a 10s timeout, wraps results in the shared
envelope with EXTRACTED provenance. Errors surface in envelope.errors[],
never as raw subprocess stderr."
```

---

### Task 6: `event` tool — envoy HTTP passthrough adapter

**Files:**
- Create: `crates/atheneum-mcp/src/events.rs`
- Modify: `crates/atheneum-mcp/src/lib.rs` (add `pub mod events;`, extend `Backend` trait + `MockBackend`)
- Modify: `crates/atheneum-mcp/src/backend.rs` (`DirectBackend`/`HttpBackend` impls)
- Modify: `crates/atheneum-mcp/src/tools.rs` (register `event` tool)
- Modify: `crates/atheneum-mcp/Cargo.toml` (add non-optional `reqwest` — currently only under the `http` feature; the event adapter needs it regardless of which atheneum backend feature is active, see Step 1)
- Test: `crates/atheneum-mcp/src/events.rs` (inline)

**Interfaces:**
- Consumes: `envelope::{Envelope, ERR_BACKEND_UNAVAILABLE, ERR_TIMEOUT}` (Task 2).
- Produces: `events::EnvoyClient::new(base_url: String) -> Self`, `pub async fn call(&self, verb: EnvoyVerb, payload: Value) -> anyhow::Result<Value>`, `Backend::event(&self, params: EventParams) -> Result<Value>` where:
```rust
pub struct EventParams {
    pub verb: String, // "send" | "claim" | "heartbeat" | "create_dependency"
    pub payload: Value,
}
```

- [ ] **Step 1: Make `reqwest` a direct dependency**

```toml
# crates/atheneum-mcp/Cargo.toml
# Change:
#     reqwest = { version = "0.12", features = ["json"], optional = true }
#     ...
#     [features]
#     http = ["dep:reqwest"]
# to: remove `optional = true` from reqwest, drop `dep:reqwest` from the
# `http` feature (leave `http = []` if nothing else depends on the flag,
# or check whether other `#[cfg(feature = "http")]` blocks in backend.rs
# still need the flag for HttpBackend — if so, keep `http = []` and gate
# only HttpBackend's module, not the dependency).
reqwest = { version = "0.12", features = ["json"] }
```

Run `grep -n "cfg(feature = \"http\")" crates/atheneum-mcp/src/backend.rs` first to confirm exactly what's gated, and only change what's needed to make `reqwest` unconditionally available — don't remove the `http` feature flag's meaning for `HttpBackend` itself.

- [ ] **Step 2: Write the failing test**

```rust
// crates/atheneum-mcp/src/events.rs

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn call_returns_err_when_envoy_unreachable() {
        // Port 9 is the "discard" service — connection refused/unreachable
        // in any real environment, standing in for envoy being down.
        let client = EnvoyClient::new("http://127.0.0.1:9".to_string());
        let result = client.call(EnvoyVerb::Heartbeat, serde_json::json!({"agent_id": "test"})).await;
        assert!(result.is_err(), "expected connection failure, got {result:?}");
    }

    #[test]
    fn verb_maps_to_expected_path() {
        assert_eq!(EnvoyVerb::Send.path(), "/messages/send");
        assert_eq!(EnvoyVerb::Claim.path(), "/handoffs/claim");
        assert_eq!(EnvoyVerb::Heartbeat.path(), "/agents/heartbeat");
        assert_eq!(EnvoyVerb::CreateDependency.path(), "/dependencies");
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p atheneum-mcp --lib events::tests`
Expected: FAIL — module doesn't exist.

- [ ] **Step 4: Implement `events.rs`** (pattern copied from `envoy-mcp/src/backend.rs`'s `get_json`/`post_json`)

```rust
// crates/atheneum-mcp/src/events.rs
//! Envoy HTTP passthrough — the event tool's adapter. Pattern copied from
//! envoy-mcp's HttpBackend (separate repo, not a workspace dependency —
//! same rationale as subprocess.rs for code_query).

use anyhow::{anyhow, Result};
use serde_json::Value;
use std::time::Duration;

const ENVOY_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy)]
pub enum EnvoyVerb {
    Send,
    Claim,
    Heartbeat,
    CreateDependency,
}

impl EnvoyVerb {
    pub fn path(self) -> &'static str {
        match self {
            EnvoyVerb::Send => "/messages/send",
            EnvoyVerb::Claim => "/handoffs/claim",
            EnvoyVerb::Heartbeat => "/agents/heartbeat",
            EnvoyVerb::CreateDependency => "/dependencies",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "send" => Some(EnvoyVerb::Send),
            "claim" => Some(EnvoyVerb::Claim),
            "heartbeat" => Some(EnvoyVerb::Heartbeat),
            "create_dependency" => Some(EnvoyVerb::CreateDependency),
            _ => None,
        }
    }
}

pub struct EnvoyClient {
    client: reqwest::Client,
    base_url: String,
}

impl EnvoyClient {
    pub fn new(base_url: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    pub async fn call(&self, verb: EnvoyVerb, payload: Value) -> Result<Value> {
        let url = format!("{}{}", self.base_url, verb.path());
        let send = self.client.post(&url).json(&payload).send();
        let resp = tokio::time::timeout(ENVOY_TIMEOUT, send)
            .await
            .map_err(|_| anyhow!("envoy call to {url} timed out after {ENVOY_TIMEOUT:?}"))??;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("HTTP {status} from {url}: {text}"));
        }
        Ok(resp.json().await?)
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p atheneum-mcp --lib events::tests`
Expected: PASS (2 tests)

- [ ] **Step 6: Wire `event` into `Backend` trait + `DirectBackend`/`HttpBackend` + tool registration**

```rust
// backend.rs, trait Backend:
async fn event(&self, params: EventParams) -> Result<Value>;

#[derive(Debug, Clone)]
pub struct EventParams {
    pub verb: String,
    pub payload: Value,
}
```

`DirectBackend` (and `HttpBackend` — this adapter is identical regardless of which atheneum backend mode is active, since it's a separate envoy connection):

```rust
async fn event(&self, params: EventParams) -> Result<Value> {
    let mut envelope = crate::envelope::Envelope::new(1);
    let Some(verb) = crate::events::EnvoyVerb::from_str(&params.verb) else {
        envelope.errors.push(crate::envelope::EnvelopeError {
            backend: "event".to_string(),
            code: crate::envelope::ERR_PARSE_ERROR.to_string(),
            message: format!("unknown event verb '{}', expected send|claim|heartbeat|create_dependency", params.verb),
        });
        return Ok(envelope.to_value());
    };
    let base_url = std::env::var("ENVOY_URL").unwrap_or_else(|_| "http://localhost:9876".to_string());
    let client = crate::events::EnvoyClient::new(base_url);
    match client.call(verb, params.payload).await {
        Ok(v) => envelope.items.push(v),
        Err(e) => envelope.errors.push(crate::envelope::EnvelopeError {
            backend: "event".to_string(),
            code: if e.to_string().contains("timed out") { crate::envelope::ERR_TIMEOUT.to_string() } else { crate::envelope::ERR_BACKEND_UNAVAILABLE.to_string() },
            message: e.to_string(),
        }),
    }
    Ok(envelope.to_value())
}
```

Add the identical method body to `HttpBackend` (it's independent of `self`'s atheneum connection mode — copy the function, don't try to share `self.graph`). Add `async fn event(&self, _p: backend::EventParams) -> anyhow::Result<Value> { Ok(Value::Null) }` to `lib.rs`'s `MockBackend`.

Register in `tools.rs` (new `event()` function, same pattern), schema:

```rust
let schema = json!({
    "type": "object",
    "properties": {
        "verb": { "type": "string", "enum": ["send", "claim", "heartbeat", "create_dependency"] },
        "payload": { "type": "object", "description": "Verb-specific payload, forwarded as-is to envoy" }
    },
    "required": ["verb", "payload"]
});
```

with `Tool::new("event", "Envoy multi-agent coordination passthrough (send/claim/heartbeat/create_dependency). Payload forwarded as-is; response wrapped in the shared envelope.", schema)`. Add `router.add_route(event());`, bump tool-count assertion to `31` with `assert!(names.contains(&"event"));`.

- [ ] **Step 7: Run full test suite**

Run: `cargo test -p atheneum-mcp --lib`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/atheneum-mcp/src/events.rs crates/atheneum-mcp/src/backend.rs crates/atheneum-mcp/src/tools.rs crates/atheneum-mcp/src/lib.rs crates/atheneum-mcp/Cargo.toml
git commit -m "feat(atheneum-mcp): add event tool (envoy HTTP passthrough)

send/claim/heartbeat/create_dependency forward to envoy's :9876 bridge
as-is, 5s timeout, response wrapped in the shared envelope. Envoy verbs
are not forced into CRUD shape, per the design spec's explicit
carve-out."
```

---

### Task 7: Two-tier staleness signal + `refresh` tool

**Files:**
- Modify: `crates/atheneum-mcp/src/backend.rs` (`Backend::search`/`navigate` — populate `code_stale`/`knowledge_stale`; new `Backend::refresh`)
- Modify: `crates/atheneum-mcp/src/subprocess.rs` (add a `magellan status`-based staleness check helper)
- Modify: `crates/atheneum-mcp/src/tools.rs` (register `refresh` tool)
- Modify: `crates/atheneum-mcp/src/lib.rs` (`MockBackend::refresh`, bump tool count)
- Test: `crates/atheneum-mcp/src/backend.rs`, `crates/atheneum-mcp/src/subprocess.rs` (inline)

**Interfaces:**
- Consumes: `CodeQueryRunner::run` (Task 5), `Envelope` (Task 2).
- Produces: `subprocess::CodeQueryRunner::is_code_index_stale(&self, magellan_db: &str) -> anyhow::Result<bool>` (parses `magellan status --db <path> --output json`'s existing dirty/pending-file signal — read `crates/atheneum-mcp/src/subprocess.rs`'s new code against `magellan status --help` output first to confirm the exact field name magellan reports; do not guess the JSON key blind). `Backend::refresh(&self, params: RefreshParams) -> Result<Value>` where `RefreshParams { project: String, refresh_code: bool }`.

- [ ] **Step 1: Confirm magellan's actual status JSON field name before writing the parser**

Run: `magellan status --db ~/.magellan/atheneum/atheneum.db --output json 2>/dev/null | rtk json -`

Read the output. Identify whichever field indicates pending/dirty/untracked files (do not assume a name — this step exists specifically because guessing a JSON field name here would violate the "verify instrumentation before trusting it" rule). Use the exact field name found in Step 3 below.

- [ ] **Step 2: Write the failing test**

```rust
// crates/atheneum-mcp/src/subprocess.rs, add to mod tests

#[tokio::test]
async fn is_code_index_stale_reports_false_for_freshly_indexed_fixture() {
    // Uses a fixture magellan db with zero dirty files, standing in for a
    // just-refreshed project — exact fixture construction depends on the
    // field confirmed in Step 1; wire the test's expected JSON shape to
    // match what Step 1 found, not an assumed shape.
    let runner = CodeQueryRunner::new();
    // ... constructs fixture consistent with Step 1's confirmed field ...
}
```

(This step's exact assertions are filled in only after Step 1's real output is read — per the plan's own "no placeholders" rule this cannot be written blind before that field is confirmed; the task's implementer must run Step 1 first.)

- [ ] **Step 3: Implement `is_code_index_stale` using the confirmed field**

```rust
// crates/atheneum-mcp/src/subprocess.rs
impl CodeQueryRunner {
    pub async fn is_code_index_stale(&self, magellan_db: &str) -> Result<bool> {
        let mut cmd = Command::new(self.resolve("magellan"));
        cmd.env_clear();
        cmd.args(["status", "--db", magellan_db, "--output", "json"]);
        let status = self.run_command(cmd, "magellan_status").await?;
        // Replace `FIELD_NAME_FROM_STEP_1` with whatever Step 1 confirmed
        // (e.g. "dirty_files", "pending_refresh", "untracked_count" > 0).
        Ok(status["FIELD_NAME_FROM_STEP_1"].as_bool().unwrap_or(false)
            || status["FIELD_NAME_FROM_STEP_1"].as_u64().unwrap_or(0) > 0)
    }
}
```

- [ ] **Step 4: Wire `code_stale`/`knowledge_stale` into `search`/`navigate`**

In `DirectBackend::search` and `DirectBackend::navigate` (Tasks 3/4), after the code-kind branch, if `self.cross` is `Some` and a project was resolved, call `runner.is_code_index_stale(&magellan_db).await` and set `envelope.code_stale = Some(result)` (default `None` when kind never touches code, matching the spec's "two separate fields" — `None` means "not applicable to this call," `Some(false)`/`Some(true)` means "checked, and here's the answer"). For `knowledge_stale`: compare `graph`'s latest write timestamp against its latest embedding-pass timestamp — use whatever existing accessor `AtheneumGraph` exposes for this (`grep -n "fn.*last_write\|fn.*last_embed\|fn.*embedding" crates/atheneum/src/*.rs` first; if no such accessor exists yet, set `envelope.knowledge_stale = None` for this task rather than inventing a new AtheneumGraph method — that would be new scope beyond this plan's boundary, flag it as a follow-up in the commit message instead of silently expanding scope).

- [ ] **Step 5: Add `refresh` tool**

```rust
// backend.rs trait:
async fn refresh(&self, params: RefreshParams) -> Result<Value>;

#[derive(Debug, Clone)]
pub struct RefreshParams {
    pub project: String,
    pub refresh_code: bool,
}
```

`DirectBackend::refresh`: resolve `project` via `cross.meta().get_project(...)` (same pattern as Task 5), if `refresh_code` run `magellan refresh --db <path>` via `CodeQueryRunner::run(magellan_db, CodeTool::Magellan, vec!["refresh".into(), "--db".into(), magellan_db.clone()])`, wrap result in `Envelope`. This is the CRUD-split's stated mechanism: "`update` = atheneum (mutate) + magellan `refresh` (propagates to llmgrep/mirage automatically since they read magellan's DB)" — no separate llmgrep/mirage refresh call needed, confirmed by the design spec.

Register the `refresh` tool in `tools.rs` (same pattern), schema `{project: string, refresh_code: boolean, default true}`. Add to `MockBackend`, bump tool count to `32`.

- [ ] **Step 6: Run full test suite**

Run: `cargo test -p atheneum-mcp --lib && cargo test -p atheneum --lib`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/atheneum-mcp/src/backend.rs crates/atheneum-mcp/src/subprocess.rs crates/atheneum-mcp/src/tools.rs crates/atheneum-mcp/src/lib.rs
git commit -m "feat(atheneum-mcp): two-tier staleness signal + refresh tool

code_stale (magellan status, cheap refresh) and knowledge_stale (atheneum
embedding lag, costlier) are separate envelope fields, never conflated.
refresh triggers magellan refresh for a resolved project; llmgrep/mirage
need no separate refresh since they read magellan's own db."
```

---

### Task 8: Error-handling hardening pass — targeted failure-mode tests

**Files:**
- Modify: `crates/atheneum-mcp/src/backend.rs` (inline tests only — no production code changes expected; this task's job is to prove Tasks 3-7's error paths actually behave as designed, and fix them if they don't)

**Interfaces:**
- Consumes: everything from Tasks 1-7. Produces: nothing new — verification task.

- [ ] **Step 1: Write the failing (or newly-passing-by-luck, to be confirmed) tests**

```rust
// crates/atheneum-mcp/src/backend.rs, inside mod tests

#[tokio::test]
async fn search_kind_all_partial_failure_returns_working_backend_results() {
    // Cross router pointed at a meta.db with zero registered projects and
    // no central knowledge db configured -> code branch fails cleanly,
    // knowledge branch (seeded) still returns its items.
    let backend = test_direct_backend_with_seeded_memory_and_empty_cross_router();
    let params = SearchParams {
        query: "seeded".to_string(), k: 10, project: None,
        kind: SearchKind::All, limit: None, cursor: None,
    };
    let result = backend.search(params).await.unwrap();
    assert!(!result["items"].as_array().unwrap().is_empty(), "knowledge results must survive a code-side miss");
}

#[tokio::test]
async fn code_query_unknown_project_returns_project_not_found_not_panic() {
    let backend = test_direct_backend_with_registered_code_project().0;
    let params = CodeQueryParams {
        project: "definitely-not-a-registered-project".to_string(),
        tool: "magellan".to_string(), subcommand: "status".to_string(), args: vec![],
    };
    let result = backend.code_query(params).await.unwrap();
    assert_eq!(result["errors"][0]["code"], crate::envelope::ERR_PROJECT_NOT_FOUND);
}

#[tokio::test]
async fn event_connection_failure_surfaces_in_errors_not_as_panic_or_exception() {
    let backend = test_direct_backend_with_seeded_memory();
    // ENVOY_URL left unset/default in test env — if envoy happens to be
    // running locally during `cargo test`, this test instead asserts the
    // call succeeds; either branch proves no panic/unhandled exception.
    let params = EventParams { verb: "heartbeat".to_string(), payload: serde_json::json!({"agent_id": "test"}) };
    let result = backend.event(params).await;
    assert!(result.is_ok(), "event() must never return a raw exception, always Ok(envelope)");
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p atheneum-mcp --lib backend::tests::search_kind_all_partial_failure_returns_working_backend_results backend::tests::code_query_unknown_project_returns_project_not_found_not_panic backend::tests::event_connection_failure_surfaces_in_errors_not_as_panic_or_exception`

Expected: PASS if Tasks 3-7 were implemented per spec. If any fails, the fix belongs in the task that owns that code path (Task 3 for search, Task 5 for code_query, Task 6 for event) — patch there, re-run, then return here.

- [ ] **Step 3: Commit**

```bash
git add crates/atheneum-mcp/src/backend.rs
git commit -m "test(atheneum-mcp): verify partial-failure and not-found error paths

Confirms kind=all search survives a code-backend miss, code_query
surfaces PROJECT_NOT_FOUND without panicking, and event() never
propagates a raw connection exception past the envelope boundary."
```

---

### Task 9: End-to-end integration test

**Files:**
- Create: `crates/atheneum-mcp/tests/unified_api_e2e.rs`

**Interfaces:**
- Consumes: everything from Tasks 1-7, real (not mocked) `magellan` binary (must be on `PATH` or resolvable via `GROUNDED_BIN_DIR`) and a real `AtheneumGraph`/`CrossRouter` pair.

- [ ] **Step 1: Write the test**

```rust
// crates/atheneum-mcp/tests/unified_api_e2e.rs
//! One real end-to-end pass through resolve -> dispatch -> envelope,
//! using one real magellan subprocess call and one real atheneum
//! in-process call. Not a substitute for Tasks 1-8's unit tests — proves
//! the wiring, not the logic.

use atheneum_mcp::backend::{Backend, SearchKind, SearchParams};

#[tokio::test]
async fn search_kind_all_returns_both_code_and_knowledge_hits_through_real_stack() {
    let tmp = tempfile::tempdir().unwrap();

    // Real magellan db: index a tiny fixture project.
    let fixture_project = tmp.path().join("fixture_project");
    std::fs::create_dir_all(&fixture_project).unwrap();
    std::fs::write(
        fixture_project.join("lib.rs"),
        "pub fn e2e_probe_symbol() -> u32 { 42 }",
    )
    .unwrap();
    let magellan_db = tmp.path().join("fixture.magellan.db");
    let status = std::process::Command::new("magellan")
        .args(["index", "--root", fixture_project.to_str().unwrap(), "--db", magellan_db.to_str().unwrap()])
        .status();
    if status.is_err() || !status.unwrap().success() {
        eprintln!("skipping e2e test: `magellan` binary not available on PATH");
        return;
    }

    // Real meta.db registering the fixture project.
    let meta_path = tmp.path().join("meta.db");
    let mut meta = atheneum::meta::MetaRouter::open_at(&meta_path).unwrap();
    meta.register_project("e2e_fixture", fixture_project.to_str().unwrap(), magellan_db.to_str().unwrap(), None, Some("rust")).unwrap();
    let cross = atheneum::cross::CrossRouter::from_meta(meta, 4);

    // Real atheneum graph, seeded with one memory whose content overlaps
    // the query term so both branches have something to find.
    let atheneum_db = tmp.path().join("fixture.atheneum.db");
    let graph = atheneum::AtheneumGraph::open(&atheneum_db).unwrap();
    graph.store_memory("e2e-note", "note about e2e_probe_symbol behavior", "agent", 0.8, None, None).unwrap();

    let backend = atheneum_mcp::backend::direct::DirectBackend::with_cross_router(
        std::sync::Arc::new(tokio::sync::Mutex::new(graph)),
        cross,
    );

    let result = backend.search(SearchParams {
        query: "e2e_probe_symbol".to_string(),
        k: 10, project: Some("e2e_fixture".to_string()),
        kind: SearchKind::All, limit: None, cursor: None,
    }).await.unwrap();

    let items = result["items"].as_array().unwrap();
    assert!(items.iter().any(|i| i["provenance"] == "EXTRACTED"), "expected a code hit, got {items:?}");
    assert!(items.iter().any(|i| i["provenance"] == "INFERRED"), "expected a knowledge hit, got {items:?}");
    assert!(result["errors"].as_array().unwrap().is_empty(), "expected no errors on a clean real run, got {:?}", result["errors"]);
}
```

- [ ] **Step 2: Run the test**

Run: `cargo test -p atheneum-mcp --test unified_api_e2e -- --nocapture`
Expected: PASS (or a printed skip message if `magellan` isn't on `PATH` in the test environment — acceptable per the test's own guard, not a plan failure).

- [ ] **Step 3: Commit**

```bash
git add crates/atheneum-mcp/tests/unified_api_e2e.rs
git commit -m "test(atheneum-mcp): add real end-to-end test for kind=all search

Exercises the full resolve -> dispatch -> envelope path with one real
magellan subprocess index+query and one real atheneum in-process write,
proving the wiring beyond what Tasks 1-8's mocked unit tests cover."
```

---

### Task 10: Wire `CrossRouter` into `main.rs` production startup

**Added 2026-07-31, post-merge.** Tasks 1-9 were merged (f100c11..8983e81) with
every gate green — and the deployed binary's entire code side was dead. Root
cause: this plan specified the `with_cross_router` seam (Task 3) and proved it
in Task 9's e2e test, but **no task wired it into production startup**.
`main.rs`'s direct-mode branch built the backend via `direct_from_graph`
(`cross: None`), so `code_query`, `refresh`, and `search`/`navigate` with
`kind=code|all` all returned `BACKEND_UNAVAILABLE` / "no CrossRouter
configured" on every real install. Discovered by a live `code_query` call
against the installed v0.6.0 binary — the one check Tasks 1-9 never ran.
**Lesson, now standing policy for this repo: an integration test that
constructs its own wiring proves nothing about the binary's startup path —
verify the deployed artifact with a live call before declaring done.**

**Files:**
- Modify: `crates/atheneum-mcp/src/main.rs` (direct-mode branch only; http mode untouched)
- Modify: `crates/atheneum-mcp/README.md` (the "no CrossRouter configured" paragraph becomes fallback-only)
- Modify: `crates/atheneum-mcp/CHANGELOG.md` (Fixed entry under the same release)

**Interfaces:**
- Consumes: `atheneum::cross::CrossRouter::open()` (cross.rs), builder
  `.with_central_knowledge_db(path)`; `DirectBackend::with_cross_router` (Task 3).

- [ ] **Step 1: Wire the router in the direct-mode branch**

After opening the `AtheneumGraph`, attempt `CrossRouter::open()`; on success
attach `.with_central_knowledge_db(<resolved atheneum db path>)` and construct
the backend via `DirectBackend::with_cross_router`. On failure, log
`tracing::warn!` and fall back to `direct_from_graph(graph)` — a missing
`meta.db` must never crash the server, it degrades exactly as documented.

- [ ] **Step 2: Update README + CHANGELOG** so the degradation paragraph
describes only the fallback case, citing the new `main.rs` lines.

- [ ] **Step 3: Verify against the built binary, not the test harness**

`cargo build --release -p atheneum-mcp`, then drive a real stdio MCP session
(initialize + `tools/call`) against the binary: `code_query` with
`{"project":"magellan","tool":"magellan","subcommand":"status"}` MUST return
magellan's real status JSON in `items[]` tagged `provenance: "EXTRACTED"`, and
`search kind=all` MUST return code-side items. Paste the verbatim transcript
as the evidence — this is the acceptance gate, not the test suite.

Implemented on branch `wire-cross-router-main` (kanban t_6986c004).

---

## Self-Review Notes

**Spec coverage:** Architecture (Tasks 3-6 extend atheneum-mcp in place, no new server) — covered. Components (dispatch/resolve = Task 5's project lookup + Task 1's central-store fix; code-tool adapter = Task 5; knowledge adapter = existing DirectBackend, extended in Tasks 3/4; event adapter = Task 6; envelope assembly = Task 2) — covered. Data flow (orient-first call is NOT separately implemented — see gap below). Error handling (Task 8). Testing (every task; Task 9 e2e). Two-tier staleness + refresh (Task 7). Depth cap (Task 2 + Task 4). Pagination/cursor (Task 2 + Tasks 3/4). Provenance tri-tag (Tasks 3/4/5).

**Gap found during self-review:** the spec's Data Flow section 5 describes an "orient-first call" (cheap entry point suggesting which verb to use next). No task implements this. Given YAGNI/ponytail discipline already established throughout tonight's design work, and that every other tool's `description` field already documents its own purpose (visible to the calling agent via `list_tools`), this is deferred rather than forced into a task: file it as a follow-up, do not add a Task 10 for it now — it's additive sugar, not load-bearing, and nothing in Tasks 1-9 blocks adding it later.

**Type consistency check:** `SearchKind` defined once (Task 3), reused as-is by `NavigateParams` (Task 4) — not redefined. `Envelope`/`EnvelopeError`/`Provenance`/error-code constants defined once (Task 2), imported (`crate::envelope::...`) everywhere else, never redeclared. `CodeQueryRunner` defined once (Task 5), reused by Task 7's staleness check — not duplicated.

**Real bug found and fixed during investigation (not part of this plan, done ahead of it):** `/home/feanor/Projects/.mcp.json`'s `atheneum-mcp` entry had `ATHENEUM_DB` still pointed at the pre-migration `/home/feanor/.magellan/atheneum/atheneum.db` path instead of the canonical `/home/feanor/.atheneum/atheneum.db` — missed in tonight's earlier 11-file migration sweep. Fixed directly (backed up as `.mcp.json.pre-atheneum-migration.bak`). Separately, `crates/atheneum-mcp/src/main.rs`'s hardcoded fallback default (`~/.hermes/atheneum/atheneum.db`, used only when `ATHENEUM_DB` is unset) is also stale — not fixed, since `.mcp.json` now sets the env var explicitly so it's dead code in practice, but worth a follow-up one-line fix outside this plan's scope.
