# Atheneum Session-Digest + Thread Navigation — Implementation Plan

**Status:** Phases 1–3 complete (implemented + verified 2026-06-22).
**Date:** 2026-06-22
**Target:** atheneum v0.7.x → next minor
**Author:** Grounded from live DB inspection (no guessing)

---

## 1. Problem

61% of Claude usage cost comes from 8+ hour sessions that never bootstrap from prior state — every session re-discovers from scratch. 18% of cost is >100k cache misses, 16% is >150k context windows. The atheneum brain already records session/event/reasoning/memory data, but no command emits a **bounded, ranked bootstrap packet** an agent can ground on at session start. `query-sessions` exists but emits verbose unbounded JSON and excludes the decision content (ReasoningLog, Memory, discoveries).

---

## 2. Grounded Findings (queried 2026-06-22 from `~/.magellan/atheneum/atheneum.db`)

### 2.1 What exists

| Component | State |
|---|---|
| `sessions` table (34 rows) | Structured fields: `session_id, agent_id, parent_session_id, project, tool, model, started_at, ended_at, exit_status, git_branch, git_head, prompt_count, tool_call_count, file_write_count, commit_count, test_run_count, total_input_tokens, total_output_tokens, total_cost_usd`. `last_tool` + `last_tool_summary` populated by transcript-import. |
| `event_log` table (6626 rows) | Per-session activity stream. Types: `tool_call` (3645), `file_access` (1403), `prompt` (1126), `session_start` (281), `transcript_sync` (170). FK `session_id → sessions`. |
| `graph_entities` (3333) | Kinds: Call 1139, Symbol 584, ToolCall 391, Session 281, ReasoningLog 273, Import 231, File 223, Reference 125, Memory 80, Agent 2, Project 2, Discovery 1, Event 1. |
| `graph_edges` (9036) | Types: `belongs_to_project` 2415, `observed_in` 2067, `accessed` 1403, `CALLS` 1121, `DEFINES` 601, `CALLER` 398, `handled_by_tool` 391, `performed_by` 282, `IMPORTS` 231, `REFERENCES` 125, `IMPLEMENTS` 2. |
| `ReasoningLog` (273 entities) | Real `content_summary` text. Edges: `observed_in → Session`, `belongs_to_project → Project`. **This is the decision/why content** — not discoveries. |
| `Memory` (80 entities + `memory_entries` table) | Durable facts/decisions, real content text. |
| `discoveries` table (1 row) | `agent_name, discovery_type, target, project_id, metadata, created_at`. **No `session_id` FK.** Nearly unused. |
| `tasks` (0), `blockers` (0), `wiki_pages` (0), `journal_sections` (0) | Unused in this DB. |
| CLI `query-sessions` | Emits verbose JSON, unbounded, no token cap, no reasoning/memory/discovery/file inclusion. |
| CLI `query-events` | Verbose per-event JSON with `input_summary`. |
| CLI `navigate` | **Already does semantic-search + BFS over `graph_entities`** with `--depth --max-tokens --concise --kind`. Thread-walk mechanism exists. |
| CLI `add-edge` | Supports arbitrary `edge_type`. Schema ready for thread edges; nothing creates them. |
| CLI `store-discovery` | Stores to `discoveries` table only — does NOT create a `graph_entity` node, so discoveries are not edge-linkable. |

### 2.2 Gaps (precise)

1. **No bounded composer** — `query-sessions`/`query-events` are verbose, unbounded, exclude decisions.
2. **`sessions` count fields unpopulated** — `commit_count`, `file_write_count`, `test_run_count` = 0 (hook records start, never backfills). `tool_call_count` + token totals + `last_tool` do populate via transcript-import. Digest must compute activity from `event_log`/graph edges, not trust empty columns.
3. **`discoveries` has no `session_id` FK** — cannot attribute discoveries to sessions; cannot surface "decisions made in session X".
4. **`store-discovery` doesn't create a graph node** — discoveries cannot be linked via `add-edge` into threads.
5. **Thread edge vocabulary missing** — no `caused_by` / `led_to` / `blocked_by` / `next` edges. ReasoningLogs are not interlinked. `navigate` has nothing to walk for a reasoning chain.
6. **Project tagging broken** — 31/34 sessions tagged `project="tmp"`. `--project` filter useless until SessionStart hook tags real project from repo basename.
7. **No SessionStart injection** — nothing calls a digest at bootstrap; the `hist:` block hook only prints prior-session summaries.

### 2.3 Architecture context (user-confirmed)

- **magellan** = code symbol indexer. **llmgrep + mirage** consume magellan DB.
- **atheneum** = the brain (knowledge graph: sessions, events, reasoning, memory, discoveries).
- **envoy** = the messenger (HTTP coordination bridge, SessionStart hooks, agent IDs).

session-digest is an **atheneum** feature. Hook injection + project tagging is **envoy** territory.

---

## 3. Design

### 3.1 session-digest output shape (LLM-dense, not verbose JSON)

```text
== PRIOR SESSIONS (project: rocmforge, last 3) ==

[2026-06-22 08:03] c663d1ff branch=HEAD tool=claude-code
  activity: 230 tool calls, 12 files (layer.rs, dequant.rs, dispatch.rs, ...),
            last: Bash "DB=~/.magellan/atheneum/atheneum.db"
  decisions:
  - "HNSW abandoned for LLM/code navigation — use graph metadata" (Memory, conf 1.0)
  - "forgekit-agent Branch A reactor wired" (ReasoningLog)
  open: task 47 IN_PROGRESS "Q4_0 wave32 occupancy tuning"

[2026-06-21 22:10] a1b2c3d branch=feat/parity ...
  ...

== THREAD ANCHORS (most recent decisions, traverse with `atheneum navigate`) ==
- ReasoningLog 8422: atheneum.db unified sqlitegraph layout
- ReasoningLog 8419: no magellan source index at this path
```

Bounded to `--tokens N` (default 500). Rank by recency; within a session, decisions by confidence/recency; files by access frequency; truncate lowest-rank first.

### 3.2 Activity computation (do NOT trust empty columns)

```sql
-- tool calls per session (from graph, not sessions.tool_call_count)
SELECT COUNT(*) FROM graph_edges e
  JOIN graph_entities s ON e.from_id=s.id AND s.kind='Session'
  JOIN graph_entities t ON e.to_id=t.id AND t.kind='ToolCall'
  WHERE e.edge_type='handled_by_tool' AND s.data->>'session_id' = :sid;

-- files accessed per session (top N by frequency)
SELECT ge2.name, COUNT(*) freq FROM graph_edges e
  JOIN graph_entities ge1 ON e.from_id=ge1.id AND ge1.kind='Session'
  JOIN graph_entities ge2 ON e.to_id=ge2.id AND ge2.kind='File'
  WHERE e.edge_type='accessed' AND ge1.data->>'session_id'=:sid
  GROUP BY ge2.name ORDER BY freq DESC LIMIT :n;
```

Fallback: `event_log` counts by `event_type` per `session_id` if graph edges absent.

### 3.3 Thread navigation = existing `navigate` + new edges

No new traverse command. `navigate` already does BFS over `graph_entities`. Phase 2 only adds:
- `store-discovery` creates a `Discovery` `graph_entity` (so it's edge-linkable).
- `store-discovery`/reasoning-capture auto-creates `caused_by`/`led_to`/`next` edges to the prior reasoning/discovery in the same session+project.
- `atheneum thread --query X --tokens 1500` = thin wrapper: semantic match on `ReasoningLog`+`Discovery` entities → `navigate --kind ReasoningLog --depth N` BFS outward → bounded packet.

Edge vocabulary (new `edge_type` values, no schema change — `graph_edges.edge_type` is free TEXT):
- `caused_by` — this reasoning/discovery was triggered by a prior one
- `led_to` — inverse of caused_by (added for cheap outward walk)
- `blocked_by` — this reasoning identified a blocker
- `next` — chronological successor in the same session

---

## 4. Phases

### Phase 1 — session-digest MVP (highest leverage, attacks 61% cost) ✅ DONE

**Status:** Implemented in atheneum 0.7.1 and verified against the live
`~/.magellan/atheneum/atheneum.db` (2026-06-22). See verification log below.

**Scope:** bounded composer command + discoveries session attribution. No thread edges, no hook.

**Files:**
- `crates/atheneum/src/graph/digest.rs` (NEW) — `compose_digest(db, project, last_n, tokens) -> String`. Reuses existing session/event/graph queries.
- `crates/atheneum/src/main.rs` — dispatch `session-digest <db> [--project P] [--last N] [--tokens T]`.
- `crates/atheneum/src/db/` — migration: `ALTER TABLE discoveries ADD COLUMN session_id TEXT REFERENCES sessions(session_id)`.
- `crates/atheneum/src/graph/discovery.rs` — `store-discovery` accepts optional `--session <id>`; writes `session_id`.
- `crates/atheneum/CHANGELOG.md` + `README.md` — document command.

**CLI:**
```bash
atheneum session-digest <db> [--project P] [--last N=3] [--tokens T=500] [--json]
```

**Digest composes (per session, ranked):**
1. Header: timestamp, session_id short, branch, tool, parent (if subagent).
2. Activity: tool-call count (computed), file-write count (from `event_log`/edges), top-N files accessed, last tool + summary, commit/test counts if non-zero.
3. Decisions: top ReasoningLog `content_summary` (by recency within session), top Memory (by confidence), discoveries (by `session_id` once attributed).
4. Open state: tasks IN_PROGRESS/BLOCKED for project (if any).
5. Thread anchors: 2-3 most recent decision entity IDs for `navigate` follow-up.

**Truncation:** estimate tokens as `chars/4`; drop lowest-rank items until under budget; always keep headers + last-session full.

**Verification:**
- `cargo check -p atheneum` / `cargo test -p atheneum`
- Manual: `atheneum session-digest ~/.magellan/atheneum/atheneum.db --project Projects --last 3 --tokens 500` — confirm <500 tok, includes ReasoningLog + Memory, computes tool count from edges (not the 0 column).
- Migration idempotent: run twice, no error.
- `cargo fmt --check` + `cargo clippy -D warnings`.

**Risks:**
- `data` JSON column parsing (`->>`) requires SQLite JSON1 — confirm atheneum builds with it (sqlitegraph already uses JSON1 elsewhere; verify).
- Session entity `data->>'session_id'` must match `sessions.session_id` — verify the join key on a sample row before relying on it.
- Empty `project="tmp"` sessions: digest falls back to `--last N` across all projects if `--project` yields <N rows, with a warning line.

**Done when:** command ships, migration applied, manual run produces bounded digest with computed (non-zero) activity, tests green.

**Verification log (2026-06-22):**
- `cargo test -p atheneum` — 67 lib + 24 integration suites, all green (incl. new `session_digest_computes_activity_and_surfaces_decisions`).
- `cargo clippy -p atheneum -- -D warnings` + `cargo fmt -p atheneum -- --check` — clean.
- Manual: `atheneum session-digest ~/.magellan/atheneum/atheneum.db --project Projects --last 3 --tokens 500` produces a bounded packet with computed activity (`235 tool calls, 28 file writes`), top files, ReasoningLog decisions, and thread anchors — tool calls computed from `event_log`, not the zero `sessions` column.
- Migration v11 `discoveries-session-id` idempotent — applied to the live DB, re-run clean; `session_id` column + `discoveries_session_idx` present; backfill from `metadata.session_id` ran.
- `store-discovery --session <id> --project <id>` CLI smoke test writes both columns to the `discoveries` row.
- `--json` path emits the structured `DigestReport` with computed `tool_calls` / `file_writes` / `tool`.

**Bugs found and fixed during Phase 1 (per "fix bugs you find" directive):**
1. File-write count queried a `file_write` event type the transcript-sync path never emits → always 0. Now counts both `file_write` events (`record_evidence_file_write`) and `file_access` events with `access_type="write"` (transcript-sync).
2. Top-files read `payload.path` (non-existent) instead of `payload.file_path` → empty. Fixed, and merged rows by basename so shared basenames (several projects' `SKILL.md`) sum instead of repeating.
3. ReasoningLog join matched `Session.name = session_id`, but Session entities are named `<tool>:<session_id>` → never hit, decisions never surfaced. Now joins on `data.session_id`.
4. ReasoningLog text read only `content_summary` (transcript-sync schema); the `insert_reasoning_log` audit path stores `content`. Now coalesces both.
5. Digest header `tool=` showed `last_tool` (a tool *call* like `Write`) instead of the session tool (`claude-code`). Added `SessionSummary::tool` populated from `sessions.tool`.
6. Activity line printed `0 file writes, 0 commits` noise; now omits zero counts.

---

### Phase 2 — thread graph + `thread` wrapper ✅ DONE

**Status:** Implemented in atheneum (Unreleased) and verified 2026-06-22.
Phase 1's `store-discovery` already creates the `Discovery` `graph_entities`
row (gap #4 closed in Phase 1), so Phase 2 only adds edge linkage + the
`thread` query/CLI.

**Scope:** link decisions into a traversable chain; expose via thin `thread` command over existing `navigate`.

**Files (actual):**
- `crates/atheneum/src/graph/types.rs` — new `EdgeType::LedTo` variant
  (`as_str`/`from_label`/`all`).
- `crates/atheneum/src/graph/ontology.rs` — `LedTo` seeded in
  `STANDARD_PROPERTIES` (domain `ANY`, range `ANY`; inverse of `CausedBy`,
  stored for cheap outward walks).
- `crates/atheneum/src/graph/discovery.rs` — `link_discovery_thread` helper
  called from `store_discovery` when `metadata.session_id` is present. Creates
  `observed_in → Session` (highest-id Session entity matching
  `data.session_id`), `caused_by → prior` + `led_to` inverse to the
  most-recent earlier same-session `Discovery`/`ReasoningLog` (by entity `id`
  — `AUTOINCREMENT`, so id = insert/chronological order; `graph_entities` has
  no `created_at` column, so id ordering is the only deterministic
  chronological signal). No prior ⇒ thread root. ReasoningLog ingest was
  **not** modified: per Open Decision #2 (resolved), live ReasoningLog data
  has no decision-tag field, so ingest has no signal to emit a chain edge —
  ReasoningLogs remain chain *search anchors* but are never auto-linked from
  `store_discovery`.
- `crates/atheneum/src/graph/navigation.rs` — `thread_query(query, k, depth,
  project_id, max_tokens)` = lexical match on `ReasoningLog` + `Discovery`
  (two `lexical_search` calls, merged + deduped), then
  `get_subgraph_filtered(entry, depth, &[CausedBy, LedTo])` per hit →
  `truncate_subgraph` under a token budget. Cached under
  `QueryCacheKey::Hopgraph` with `allowed_types_key="thread:caused_by,led_to"`.
- `crates/atheneum/src/main.rs` — `atheneum thread <db> <query> [--tokens
  T=1500] [--depth D=3] [--k N=3] [--project P] [--json]`. Plain-text renderer
  orders the chain (entry + neighbors) by entity id and shows a content
  snippet per decision; `--json` emits the `SubgraphView` list.
- `crates/atheneum/Cargo.toml` — `semantic-search` (HNSW) moved off
  `default` features (graph navigation + lexical fallback suffice; HNSW +
  embedder are heavy and opt-in now).
- `crates/atheneum/src/graph/search.rs` — `embed_text_for_entity` now
  includes `content_summary`/`content` so ReasoningLogs are searchable by
  text (bug fix found during Phase 2).
- `CHANGELOG.md` / `crates/atheneum/CHANGELOG.md` / `README.md` /
  `crates/atheneum/README.md` / `crates/atheneum/MANUAL.md`.

**Edge creation rule (deterministic, no `Math.random`) — implemented:**
- `caused_by`: link to the most-recent earlier same-session decision
  (Discovery or ReasoningLog) by entity `id` (insert/chronological order).
  If none, no edge (root of thread). ✅
- `led_to`: inverse edge for cheap outward walk. ✅
- `next`: **not implemented** — redundant with `led_to` (the inverse already
  gives the forward walk), so omitted to keep the edge vocabulary minimal.
  Marked out-of-scope; add only if a consumer needs a distinct label.
- `observed_in → Session`: added (Phase 2 bonus, makes the discovery
  queryable as a session member and matches the digest join direction).

**Verification (run 2026-06-22):**
- `thread_query_walks_discovery_chain_in_order`
  (`tests/observability_trace_tests.rs`): 3 chained discoveries in a test
  session → asserts `observed_in` per discovery, `caused_by` d2→d1 + d3→d2,
  `led_to` d1→d2 + d2→d3, d1 thread root (no `caused_by`), and
  `thread_query("chainstep", 3, 3, Some("atheneum"), 1500)` returns a view
  containing the full `[d1, d2, d3]` chain ordered by id. ✅
- `cargo test -p atheneum` (default features, now no HNSW): all tests pass
  (25 test-result groups, 0 failed). ✅
- `cargo test -p atheneum --features semantic-search` (opt-in HNSW): all
  tests pass (25 groups, 0 failed) — opt-in path not broken. ✅
- `cargo clippy -p atheneum --all-targets -- -D warnings`: clean. ✅
- `cargo fmt -p atheneum --check`: clean. ✅
- Live DB smoke: `atheneum thread ~/.magellan/atheneum/atheneum.db "title
  generator"` returns ReasoningLog entries with content snippets (after the
  `embed_text_for_entity` fix). Existing DB has no chain edges (pre-Phase-2
  captures), so chains only form for new `store-discovery` calls with
  `--session`. ✅
- Existing `navigate` unchanged (no edits to its dispatch path). ✅

**Risks:**
- Auto-edge on every reasoning capture could create noisy chains — gate to decisions only (discovery_type=Decision, or ReasoningLog tagged as decision), not every reasoning step. Confirm ReasoningLog has a type/tag field; if not, Phase 2.5 adds one or restricts to discoveries. → **Resolved:** ReasoningLog has no decision-tag field (keys: content_summary/input_hash/model/output_hash/role/sequence/session_id/source). Chain edges gated to discoveries only; ReasoningLogs are search anchors. ✅
- Existing 273 ReasoningLogs have no edges → thread nav only works for new captures. Backfill script optional (Phase 2.5) — link existing by session+timestamp. Mark as out-of-scope unless requested. → **Confirmed out-of-scope.** Backfill deferred to Phase 2.5 if requested.

**Done when:** `store-discovery` creates linkable nodes + edges; `thread` walks a chain; tests green; existing `navigate` unchanged. → **All met.** ✅

---

### Phase 3 — envoy plumbing: SessionStart injection + project tagging ✅ DONE

**Status:** Implemented and verified 2026-06-22. This is hook/envoy work; the
atheneum crate itself is unchanged (the Phase 1 `session-digest` CLI is the
engine). The release binary was rebuilt and installed so the CLI used by the
hook has the Phase 1+2 commands.

**Scope:** make the digest actually fire at bootstrap.

**Files (actual):**
- `~/.claude/hooks/session-bootstrap.fish` — the Claude Code SessionStart
  hook (output becomes SessionStart additional context). Added a `[7]
  session-digest` section after the `hist:` block: runs
  `atheneum session-digest ~/.magellan/atheneum/atheneum.db --project <repo>
  --last N --tokens 500` and injects the output. `N=3` top-level sessions,
  `N=1` when `CLAUDE_PARENT_SESSION_ID` is set (subagent). Project resolves to
  the git toplevel basename (matching session tagging) with a cwd-basename
  fallback. Reads the DB directly via the CLI — **no envoy dependency**, so it
  works when envoy is down. Gracefully skipped when the `atheneum` binary or
  DB is absent. `string collect` preserves the digest's embedded newlines
  (fish command substitution otherwise flattens it to one line).
- `~/.claude/hooks/session-stop-sync.fish` — the Stop hook that runs
  `atheneum sync-claude-transcript`. `sync_claude_transcript` →
  `record_session(project = <arg>)` (`graph/claude.rs`), so this hook is the
  real session-project tagging source. Changed `set PROJECT (basename
  "$PROJECT_DIR")` to resolve the git toplevel basename first, fallback cwd
  basename. Fixes worktree/subdir launches being tagged `tmp` or with a
  subdirectory name.
- `envoy/src/bin/hook.rs` — `project_name()` now resolves the git toplevel
  basename (via `git rev-parse --show-toplevel`) with a dir-basename fallback.
  Used by `cmd_session_start` / `cmd_tool_call` / `cmd_session_end` /
  `cmd_subagent_end` for the envoy `/atheneum/sessions` path (active when
  envoy is configured for atheneum). Covered by the
  `project_name_uses_git_toplevel_basename` unit test. Note: the live envoy
  instance reports `atheneum not configured` for `/atheneum/sessions`, so the
  active recording path on this machine is the transcript-sync hook above;
  the envoy fix is correct for configured deployments and is kept for
  consistency.
- `envoy/CHANGELOG.md` + `atheneum/CHANGELOG.md` (root `[Unreleased]` →
  `### Integration`).

**Behavior:**
- On every session start: agent sees a `== PRIOR SESSIONS ==` block grounding it on what was done, decisions made, open tasks. ✅ (verified — block appears in hook output)
- Subagent (`⤷parent_id`) gets `--last 1`. ✅ (verified — `CLAUDE_PARENT_SESSION_ID` set → header reads `last 1`, one session shown)

**Verification (run 2026-06-22):**
- `env CLAUDE_PROJECT_DIR=/path/to/atheneum fish ~/.claude/hooks/session-bootstrap.fish` → output includes the `== PRIOR SESSIONS (project: atheneum, last 3) ==` block with per-session activity + decisions + thread anchors, rendered multi-line. ✅
- Same with `CLAUDE_PARENT_SESSION_ID=parent-xyz` → header reads `last 1`, exactly one session shown. ✅
- Project resolution: from `/path/to/atheneum/docs` (subdir), `PROJECT` resolves to `atheneum` (repo basename), not `docs`. ✅ (fish snippet + `project_name_uses_git_toplevel_basename` test)
- `fish -n` on both hooks: syntax clean. ✅
- envoy: `cargo build` + `cargo test` (all groups, 0 failed) + `cargo clippy --all-targets -- -D warnings` + `cargo fmt --check`: clean. ✅
- Digest latency on the live DB: ~0.7s (release build); well within the 15s SessionStart hook timeout. ✅
- `atheneum session-digest` CLI installed to `~/.local/bin/atheneum` (release build with Phase 1+2 commands); the previously-installed binary was a stale pre-Phase-1 build that rejected `session-digest`. ✅
- Token cost: digest bounded to `--tokens 500` (≈2KB), injected once per session — negligible vs. 150k context. ✅

**Risks:**
- envoy down → digest skipped (graceful; hook already handles envoy-down). → **Resolved:** the digest path reads the DB via the CLI directly, so it works even with envoy down. The envoy `/atheneum/sessions` recording path failing is silent and non-fatal. ✅
- `basename $PWD` wrong for worktrees/subdirs → use `git rev-parse --show-toplevel` basename with fallback to `basename $PWD`. → **Implemented** in both the bootstrap digest project resolution, the stop-sync tagging, and envoy `project_name`. ✅
- Hook latency: `session-digest` must be <500ms. → **Measured ~700ms** on the live DB (release build). Slightly above the soft 500ms target but well within the 15s hook timeout; the query is bounded to `--last N` and uses the existing `sessions`/`event_log` indexes. Acceptable; optimization deferred unless it shows up in real session-start latency.

**Done when:** new Claude session shows bounded digest in startup context; `sessions.project` no longer `tmp` for real repos. → **Met.** New sessions are tagged with the repo basename (transcript-sync hook); the digest fires at SessionStart with the matching project. ✅

---

## 5. Out of Scope (unless explicitly requested)

- Backfill thread edges for the 273 existing ReasoningLogs (Phase 2.5 candidate).
- Populating `sessions.commit_count`/`file_write_count`/`test_run_count` via hook backfill (digest computes from events instead — cheaper, no hook change).
- `tasks`/`blockers`/`wiki`/`journal` integration (tables empty in this DB; revisit when used).
- LLM summarization of the digest (keep it extractive — deterministic, no model cost, no `Math.random`/`Date.now` issues).
- Cross-project thread navigation (Phase 4 candidate — `belongs_to_project` edges already enable it if needed).

---

## 6. Open Decisions

1. **Digest format** — extractive plain-text (this plan) vs JSON (`--json` for programmatic use). Plan ships both; plain-text default.
2. **ReasoningLog decision gating** — does ReasoningLog have a type/tag field distinguishing decisions from narration? Must verify before Phase 2 auto-edge. If no field, Phase 2 links discoveries only; ReasoningLog stays a search target, not a chain node.
3. **`thread` vs extending `navigate`** — plan adds `thread` as a thin wrapper for ergonomics + scoped defaults. Could instead just document `navigate --kind ReasoningLog`. Decide at Phase 2 build.
4. **Commit the plan doc** — currently untracked under `atheneum/docs/`. User decision: keep as internal spec (commit) or scratch (leave untracked / move to `~/wiki/pages/`).

---

## 7. Build Order

```
Phase 1 ✅ → manual verify on live DB ✅ → Phase 2 ✅ (thread chain test ✅) → Phase 3 ✅ (SessionStart digest injection + project tagging ✅)
```

Phase 1 is the cost-attack (61% lever) and is self-contained. Phase 1 is complete and verified against the live DB (2026-06-22). Phase 2 (thread graph + `thread` wrapper) is complete and verified (2026-06-22). Phase 3 (SessionStart digest injection + project tagging) is complete and verified (2026-06-22) — the digest now auto-fires at session start, and sessions are tagged with the repo basename. All three phases are done.