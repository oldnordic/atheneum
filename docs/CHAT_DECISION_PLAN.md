# Chat Navigation + Decision Capture Plan

**Status:** Implemented 2026-06-23 — Phases 1, 2, 3, 4, 5, 6 complete. Ships
in atheneum 0.9.0. Phase 3 ships both the `~/.local/bin/extract-decisions`
script (default) and a native `atheneum extract-decisions` subcommand behind
the `extract` feature. Phase 5 ships as the `plugin/atheneum-decisions/`
companion plugin. Builds on the session-digest plan (Phases 1–3 complete in
atheneum 0.8.0 / envoy 0.3.1).

**Goal:** make the Claude Code chat transcript queryable as a graph —
token-budgeted, directional, SQL-filtered, paginated — and capture real
*decisions* as first-class nodes so the chat + decision chains are navigable
without re-reading full transcripts into context.

**Core principle:** the graph is the index. Search = SQL `WHERE` (cheap). Read =
token-capped extractive (cheap). Navigation = edges (cheap). Full transcript
never loaded into context.

---

## Grounded schema (verified 2026-06-22 against `~/.magellan/atheneum/atheneum.db` + `graph/evidence/recording.rs`)

- `graph_entities(id, kind, name, file_path, data TEXT JSON)` — the chat lives
  here, but content/session/sequence are **inside `data`**, not top-level
  columns. Kinds present: `ReasoningLog` (649), `ToolCall` (1196), `Call`
  (1333), `Session` (342), `Discovery` (1), `Symbol`, `Reference`, `File`,
  `Import`, `Memory`, `Agent`, `Project`, `Event`.
  - **Chat kinds = `ReasoningLog` + `ToolCall` ONLY.** `record_evidence_prompt`
    (`evidence/recording.rs:85`) stores **user messages, assistant text, AND
    thinking blocks all as `kind="ReasoningLog"`**, distinguished by
    `data.role` (`"user"` / `"assistant"` / thinking). Tool uses = `ToolCall`
    (`data.tool_name`, `data.tool_category`). `Call` is a **call-graph** entity
    (caller/callee/line) — NOT chat; exclude it.
  - `ReasoningLog.data` keys: `session_id`, `sequence`, `role`,
    `content_summary`, `content` (audit-schema only; transcript rows have
    `content_summary`), `source`, `input_hash`, `output_hash`, `model`.
  - `ToolCall.data` keys: `session_id`, `sequence`, `tool_name`,
    `tool_category`, `source`, `exit_status`, `input_hash`, `output_hash`.
  - `name` = `<session_id>:<sequence>` for both chat kinds.
  - Indexes: `idx_entities_kind_id`, `idx_entities_kind`, `idx_entities_kind_name`.
    **No index on session_id / sequence / content** → today a session chat query
    = full scan + `json_extract` per row. **This is what Phase 1 fixes.**
- `graph_edges(id, from_id, to_id, edge_type, data)` — `caused_by`, `led_to`,
  `observed_in`, etc. Indexes on from/to/type.
- `discoveries(id, agent_name, discovery_type, target, project_id, metadata,
  created_at, session_id)` — `session_id` is the v11 column (indexed). Decision
  rows = `discovery_type='Decision'` + structured `metadata` JSON. No change
  needed here.
- `event_log(event_id, event_type, entity_id, session_id, payload, timestamp)`
  — tool-call/file-write events. Indexed on session/type/timestamp.
- FTS today: `symbol_fts`, `wiki_pages_fts` only. **No FTS on `graph_entities`.**

**Source transcript:** `~/.claude/projects/<encoded-dir>/<session-id>.jsonl`,
ingested by `atheneum sync-claude-transcript` (Stop hook
`session-stop-sync.fish`). Structured decision signals verified present in the
JSONL: `ExitPlanMode`, `AskUserQuestion` (tool_use with full options input +
matching tool_result with the choice), `TodoWrite`.

---

## Phase 1 — schema migration v12: generated columns + FTS — ✅ DONE (b102407)

Migration v12 `chat-columns-fts`: 4 VIRTUAL generated cols (session_id/sequence/role/content_text) + idx_entities_session_seq + idx_entities_session_role_seq + entity_fts FTS5 ext-content w/ backfill + 4 sync triggers. Registered in db/mod.rs MIGRATIONS. `migration_v12_tests.rs` green. Zero insert-path change.

**Scope:** correct the schema so chat queries are index-backed + FTS-backed,
not full-scan + `json_extract`. **Key insight:** `session_id`, `sequence`,
`role`, and content all already live in `data` JSON → add them as VIRTUAL
GENERATED columns. Generated columns derive from `data` automatically, so
**zero Rust insert-path changes** (`record_evidence_prompt`, `insert_tool_call`,
etc. keep working unchanged). Add a composite index + an FTS5 table + sync
triggers. Pure schema addition.

**Files:**
- `crates/atheneum/src/db/` (migration module) — migration v12, name
  `chat-columns-fts`. Schema delta:
  ```sql
  -- Generated columns derive from data JSON. VIRTUAL = computed on read, no
  -- storage cost; indexable in SQLite. ZERO insert-path changes.
  ALTER TABLE graph_entities ADD COLUMN session_id TEXT
    GENERATED ALWAYS AS (json_extract(data, '$.session_id')) VIRTUAL;
  ALTER TABLE graph_entities ADD COLUMN sequence INTEGER
    GENERATED ALWAYS AS (json_extract(data, '$.sequence')) VIRTUAL;
  ALTER TABLE graph_entities ADD COLUMN role TEXT
    GENERATED ALWAYS AS (json_extract(data, '$.role')) VIRTUAL;
  ALTER TABLE graph_entities ADD COLUMN content_text TEXT
    GENERATED ALWAYS AS (
      coalesce(json_extract(data, '$.content_summary'), '') || ' ' ||
      coalesce(json_extract(data, '$.content'), '') || ' ' ||
      coalesce(json_extract(data, '$.tool_name'), '')
    ) VIRTUAL;

  CREATE INDEX idx_entities_session_seq
    ON graph_entities(session_id, sequence);
  CREATE INDEX idx_entities_session_role_seq
    ON graph_entities(session_id, role, sequence);

  -- FTS5 over chat-turn content. External-content table = stays in sync via
  -- triggers below. Only index chat kinds (keeps FTS small).
  CREATE VIRTUAL TABLE entity_fts USING fts5(
    content_text,
    content='graph_entities', content_rowid='id',
    tokenize='porter unicode61'
  );

  -- Backfill existing rows.
  INSERT INTO entity_fts(rowid, content_text)
    SELECT id, content_text FROM graph_entities
    WHERE kind IN ('ReasoningLog', 'ToolCall');

  -- Sync triggers (FTS5 external content does not auto-sync).
  CREATE TRIGGER entity_fts_ai AFTER INSERT ON graph_entities
    WHEN new.kind IN ('ReasoningLog','ToolCall')
    BEGIN
      INSERT INTO entity_fts(rowid, content_text) VALUES (new.id, new.content_text);
    END;
  CREATE TRIGGER entity_fts_ad AFTER DELETE ON graph_entities
    WHEN old.kind IN ('ReasoningLog','ToolCall')
    BEGIN
      INSERT INTO entity_fts(entity_fts, rowid, content_text) VALUES('delete', old.id, old.content_text);
    END;
  CREATE TRIGGER entity_fts_au AFTER UPDATE ON graph_entities
    WHEN old.kind IN ('ReasoningLog','ToolCall') OR new.kind IN ('ReasoningLog','ToolCall')
    BEGIN
      INSERT INTO entity_fts(entity_fts, rowid, content_text) VALUES('delete', old.id, old.content_text);
      INSERT INTO entity_fts(rowid, content_text) VALUES (new.id, new.content_text);
    END;
  ```
- `crates/atheneum/tests/migration_v12_tests.rs` (new) — fresh DB migrates to
  v12; existing DB upgrades, generated columns populate, FTS backfilled, indexes
  present; insert via `record_evidence_prompt` → generated columns + FTS row
  appear with no code change; query plan uses the index + FTS.

**Behavior:**
- After v12: `SELECT ... WHERE session_id=? ORDER BY sequence` uses
  `idx_entities_session_seq` (no scan, no per-row json_extract).
- `--search` uses `entity_fts MATCH ?` (sub-ms).
- Generated columns are read-only + always consistent with `data` (derived, not
  stored) — no desync risk.
- Pre-v12 DBs: migration runs on next open (existing atheneum migration path).
  Fresh DBs: v12 applied at init.

**Verification:**
- `EXPLAIN QUERY PLAN SELECT id FROM graph_entities WHERE session_id=? ORDER BY
  sequence` → uses `idx_entities_session_seq` (not a scan).
- `EXPLAIN QUERY PLAN SELECT id FROM entity_fts WHERE entity_fts MATCH 'HNSW'`
  → FTS5 scan.
- Insert a ReasoningLog via `record_evidence_prompt` → `SELECT session_id,
  sequence, role, content_text FROM graph_entities WHERE id=last` returns
  derived values; `SELECT rowid FROM entity_fts WHERE content_text MATCH
  '<word>'` finds it. Confirms zero insert-path change.
- Migration on the live DB: row count of `entity_fts` ≈ ReasoningLog + ToolCall
  count (649 + 1196). Generated-column spot-checks match `json_extract`.
- `cargo fmt --check` + `cargo clippy --all-targets -- -D warnings` + `cargo test` clean.

**Risks:**
- **SQLite version**: generated columns need SQLite ≥ 3.31; FTS5 + `porter`
  need the FTS5 extension. Verify the bundled SQLite (atheneum uses `rusqlite`
  bundled or system?). If system SQLite is old, switch that build to bundled.
  — Check at Phase 1 start.
- VIRTUAL generated columns are recomputed on read; for a chat query reading
  ~hundreds of rows this is fine (the index on them is what matters; the index
  stores derived values). If read cost shows up, switch `session_id`/`sequence`
  to STORED generated columns (small storage cost, no recompute).
- FTS trigger on every `graph_entities` insert (incl. non-chat kinds) — the
  `WHEN` guard limits FTS writes to chat kinds, so overhead is bounded.
- `content` field absent on transcript ReasoningLogs (only `content_summary`) —
  `content_text` coalesces both + `tool_name`, so search covers all chat text.

**Done when:** migration v12 lands, chat queries are index + FTS backed, zero
insert-path Rust changes, tests green, live DB upgrades cleanly.

---

## Phase 2 — `atheneum chat` command (navigation surface) — ✅ DONE (f434a7e)

`atheneum chat <db> <sid> --only-decisions --walk` token-budgeted navigation over the v12 columns + FTS.

**Scope:** token-budgeted, directional, filterable, paginated chat reader over
the Phase-1 schema. Works on data already ingested — no new capture needed.
This is the thing you navigate with.

**CLI:**
```
atheneum chat <db> --session <id> \
  [--tokens N=500]              # extractive budget; walk in direction, stop at N
  [--direction recent|chrono]   # recent=DESC (bottom→top, default), chrono=ASC
  [--kinds ReasoningLog,ToolCall]   # which turn kinds (default: both)
  [--role user|assistant|thinking] # filter ReasoningLog by role
  [--search "query"]            # entity_fts MATCH (Phase 1); sub-ms
  [--only-decisions]            # discoveries where discovery_type='Decision'
  [--offset N --limit L]        # pagination
  [--walk]                      # from each hit, follow caused_by/led_to chain
  [--json]
```

**Files:**
- `crates/atheneum/src/graph/chat.rs` (new) — `ChatQuery` params +
  `query_chat(...)` returning a bounded `ChatReport` (rows + token total +
  `has_more`). Uses the Phase-1 generated columns + `entity_fts` directly.
- `crates/atheneum/src/main.rs` — `chat` subcommand dispatch + renderer
  (chronological or recent, role-tagged, content snippet per turn, token total
  + `has_more` footer).
- `crates/atheneum/tests/chat_tests.rs` (new) — synthetic session with known
  user/assistant/tool turns; assert direction, token cap, `--role` filter,
  `--search` (FTS), pagination, `--only-decisions`, `--walk`.

**SQL (grounded, post-Phase-1):**
```sql
-- chat turns for a session, index-backed
SELECT id, kind, role, sequence, content_text, data
FROM graph_entities
WHERE session_id = :sid
  AND kind IN (:kinds)
  AND (:role IS NULL OR role = :role)
  AND (:search IS NULL OR id IN (SELECT rowid FROM entity_fts WHERE entity_fts MATCH :search))
ORDER BY sequence <DIR>
LIMIT :limit OFFSET :offset;

-- decisions for a session
SELECT id, target, metadata, created_at
FROM discoveries
WHERE discovery_type = 'Decision' AND session_id = :sid
ORDER BY created_at <DIR>
LIMIT :limit OFFSET :offset;
```
Token budget = extractive: fetch rows in order, accumulate tokens from
`content_text`, stop when `>= --tokens`. Emit `has_more` when the window had
more rows. `--walk` = for each emitted row, BFS along `graph_edges`
(`caused_by`/`led_to` only) reusing `thread`'s edge walk; include a one-line
chain snippet per hit.

**Behavior:**
- `--direction recent --tokens 500` → last ~500 tokens (find the latest
  decision / what just happened).
- `--direction chrono --tokens 500 --offset 500` → page forward from the start.
- `--search "HNSW"` → FTS matches only, recent-first, token-capped.
- `--role assistant` → assistant turns only.
- `--only-decisions` → the session's decision log (empty until Phase 3
  populates decisions — assert on synthetic data).
- `--walk` → each hit gets a chain trace (pivot to `thread`).

**Verification:**
- `atheneum chat <db> --session <real-id> --tokens 500` → bounded output,
  `has_more` correct, multi-line, role labels present.
- `--direction recent` vs `chrono` → reversed order, same row set.
- `--search` → only FTS matches; non-matching term returns empty (not error);
  sub-ms via `EXPLAIN QUERY PLAN`.
- `--role assistant` → only `role='assistant'` ReasoningLogs.
- `--offset/--limit` → correct slice; `--tokens` cap respected within slice.
- `--only-decisions` → only `discovery_type='Decision'` rows.
- `--walk` → chain snippet follows `caused_by`/`led_to`.
- `cargo fmt --check` + `cargo clippy --all-targets -- -D warnings` + `cargo test` clean.

**Risks:**
- Token counting: approximate (whitespace-split or char/4). Match the existing
  `session-digest`/`thread` tokenizer for consistency.
- `--walk` over a session with no chain edges (decisions absent) → emit the row
  with an empty chain note, not an error.
- Renderer must handle `content_text` possibly empty (e.g. a ToolCall with no
  searchable text) — show `tool_name` + `data` summary instead.

**Done when:** `atheneum chat` navigates a real session token-budgeted in both
directions with `--role`/`--search`/pagination, no full-transcript load, tests
green.

---

## Phase 3 — decision backfill (extract-time, B) — ✅ DONE (34708d0 + script)

`~/.local/bin/extract-decisions` operator script (Ollama qwen3.5, source `llm-extract`, hallucination guard, resumable `--all`, `--dry-run`). `decision_exists` dedup + `recent_discoveries --session` gap-fix shipped.

**Scope:** fix the existing noisy graph. A script runs a local LLM over a
session's full transcript JSONL, identifies decision-shaped turns, and stores
each as a `Discovery` with `discovery_type='Decision'` + `session_id`. Works on
old transcripts — no model cooperation needed. Makes `--only-decisions` +
`--walk` carry real signal on historical data.

**Files:**
- `~/.local/bin/extract-decisions` (new shell script, reuses the `dream` +
  `remember-to-atheneum` pattern) — args: `<session-id>` or `--all`. For each
  transcript:
  1. `jq` the JSONL → assistant text + thinking + tool_use/tool_result.
  2. Batch into prompts for the local LLM (`ollama run lfm2.5:8b` — no Claude
     API burn) with a strict output schema:
     `[{target, chosen, alternatives[], rationale, sequence}]`.
  3. For each decision: write `metadata.json` to a temp file, call
     `atheneum store-discovery <db> claude Decision <target> /tmp/dec.json
     --session <sid>`. `store_discovery` auto-links it into the thread
     (`caused_by` prior, `led_to` inverse) per Phase 2 of the session-digest plan.
- `atheneum extract-decisions <db> [--all | <session-id>] [...]` subcommand
  — the Rust port, implemented behind the `extract` feature (default off). Same
  prompt/schema as the script, calls Ollama in-process (`ureq`), stores via
  `graph.store_discovery` directly (no temp file / shell-out), and dedups with
  the same `decision_exists`-equivalent check. The `~/.local/bin/extract-decisions`
  script remains the default fallback (no special build needed); the native
  subcommand is enabled with `--features extract` (or `--all-features`).

**Decision schema (metadata JSON):**
```json
{
  "chosen": "0.8.0 minor bump",
  "alternatives": ["0.7.1 consolidate", "0.8.0"],
  "rationale": "HNSW default-off is breaking → semver minor",
  "target": "atheneum version bump",
  "session_id": "<sid>",
  "sequence": 42,
  "source": "llm-extract",
  "file": null, "line": null
}
```

**Behavior:**
- `extract-decisions <sid>` → N Decision rows in `discoveries`, linked into the
  session thread.
- `--all` → iterates every transcript; idempotent (skip if a Decision with same
  `session_id`+`sequence`+`target`+`source` already exists — pre-scan via
  `atheneum discoveries-recent --session <sid>`).
- `--dry-run` → print extracted decisions, store nothing.

**Verification:**
- Run on this session's transcript (`c88ac6c2...`) → expect ≥2 Decision rows
  (version-bump choice, branch-strategy choice). Verify via
  `atheneum discoveries-recent <db> --session <sid>` + `atheneum chat
  --only-decisions`.
- `atheneum thread <db> "version bump"` → Decision nodes in the chain.
- Idempotency: re-run → no duplicate rows.
- `--dry-run` → JSON list printed, `discoveries` count unchanged.

**Risks:**
- LLM hallucinated decisions → require `chosen` + `rationale` non-empty;
  `--dry-run` review before `--all`.
- LLM latency: `lfm2.5:8b` over a long transcript = seconds-to-minutes per
  session. Batch turns (not per-turn) to amortize. `--all` over hundreds of
  sessions = long run; make it resumable (skip sessions already with Decisions).
- Prompt size: summarize long sessions first (reuse `session-digest` output as
  context) to cap the LLM prompt.

**Done when:** `--all` backfills Decision rows for existing transcripts,
idempotent + `--dry-run` reviewable, `thread`/`chat --only-decisions` show real
decisions on historical sessions.

---

## Phase 4 — live decision watcher (C) — ✅ DONE (dfcdfaa)

`atheneum watch-decisions` (graph::watch) — in-memory cursor, partial-line tolerance, Tier-1 detector (AskUserQuestion/ExitPlanMode/TaskCreate/TodoWrite), `decision_exists` dedup, `--once` cron-safe. Verified live on transcript 07ff7531: 22 decisions, 100% precision (exitplan empty-input skipped), idempotent rerun. systemd unit shipped.

**Scope:** real-time structured decision capture. A long-running process tails
active transcripts incrementally (abtop's `parse_transcript(path, from_offset)`
pattern — atheneum's `sync-claude-transcript` parser already understands the
JSONL; add incremental offset + a detector). Tier-1 detector only: structured
tool signals, no LLM, 100% precision.

**Files:**
- `crates/atheneum/src/graph/watch.rs` (new) — `watch_decisions(db, config)`:
  - `HashMap<session_id, byte_offset>` cache (abtop pattern).
  - Tick loop (1–2s): scan `~/.claude/projects/*/*.jsonl` for active sessions
    (mtime recent), `parse_transcript(path, cached_offset)` → new lines.
  - Tier-1 detector on new `tool_use` lines:
    - `ExitPlanMode` → one Decision per planned choice; `target` = plan subject,
      `chosen` = planned approach, `alternatives` = [] (plan text), rationale
      from plan text, `source = "exitplan"`.
    - `AskUserQuestion` tool_use + matching `tool_result` → `chosen` = selected
      option, `alternatives` = all option labels, `rationale` = chosen option's
      description, `target` = question header, `source = "askuser"`.
    - `TodoWrite` (new tasks only) → `target` = task title, `chosen` = task
      subject, `source = "todowrite"`.
  - On hit → `store_discovery(discovery_type="Decision", session_id, metadata)`
    in-process (call the graph fn directly, not the CLI — same code path).
- `crates/atheneum/src/main.rs` — `watch-decisions <db>` subcommand (foreground
  + `--once` for a single scan).
- `~/.config/systemd/user/atheneum-decision-watcher.service` (new) —
  `ExecStart=/home/feanor/.local/bin/atheneum watch-decisions <db>`,
  `Restart=on-failure`, `WantedBy=default.target`.
- `crates/atheneum/tests/watch_tests.rs` (new) — synthetic transcript with
  known `ExitPlanMode`/`AskUserQuestion`/`TodoWrite` lines; assert the detector
  emits the right Decision rows with correct `source` + `chosen`.

**Behavior:**
- As a session progresses, ExitPlanMode/AskUserQuestion/TodoWrite events become
  Decision rows within seconds — no session-end wait.
- `session-digest` at the next session start surfaces them (already does, via
  the discoveries join).
- `--once` → single scan + exit (for testing / cron).

**Verification:**
- Synthetic transcript with `AskUserQuestion` + `tool_result` → exactly one
  Decision row, `source="askuser"`, `chosen` = selected option label.
- Synthetic `ExitPlanMode` → Decision row(s), `source="exitplan"`.
- Live: start the service, run a session that uses AskUserQuestion (this session
  did), confirm a Decision row appears in `discoveries` with `session_id` set.
- Idempotent: re-scan same bytes → no duplicate (offset cache + dedup on
  `session_id`+`sequence`+`source`).
- `systemctl --user is-active atheneum-decision-watcher` → active.

**Risks:**
- Transcript rotation / multiple config dirs (`CLAUDE_CONFIG_DIR`) → reuse
  abtop's `refresh_config_dirs` discovery logic (default `~/.claude` + env +
  `/proc/<pid>/environ`).
- Dedup vs Phase 3 backfill: a decision captured live + extracted post-hoc could
  double. Dedup key = `session_id`+`sequence`+`target`+`source`; both layers
  check before insert.
- Offset invalidation if the JSONL is truncated/rewritten → detect file size
  shrink, reset offset to 0, re-scan (abtop does this: `identity_changed`).

**Done when:** service runs, live sessions emit structured Decision rows in real
time, dedup holds, tests green.

---

## Phase 5 — cooperative skill capture (A) — ✅ DONE (this branch)

**Implementation notes (deviation from plan, correctness fix):** the plan
specified the skill-layer dedup key as `session_id`+`sequence`+`target`+`source`
(same as Phase 3/4). That key is correct for the transcript watcher, where turns
have a stable order, but a re-fired skill decision has *no* stable sequence —
so a sequence key would let the duplicate through. The skill/manual layer
therefore dedups on `(session_id, target, source, chosen)` via the new
`AtheneumGraph::decision_exists_chosen`, which is what "the same choice was
already recorded" means for that layer. The CLI `store-discovery` gained
`--dedup` (opt-in) + `--force` (bypass) so the skill and `/decision` can guard
their own inserts. Cross-layer doubles (different `source`) remain an accepted
tradeoff, unchanged. The watcher's sequence-keyed `decision_exists` is
untouched.

**Files (atheneum repo `plugin/atheneum-decisions/`):**
- `.claude-plugin/plugin.json` — name, skills + commands + hooks.
- `skills/record-decision/SKILL.md` — triggers on choosing between approaches
  / architectural tradeoff; writes `metadata.json`, calls
  `atheneum store-discovery ... --session $CLAUDE_CODE_SESSION_ID --dedup`,
  `source = "skill"`.
- `commands/decision.md` — `/decision <target> <chosen> [rationale]` manual
  fallback (same store path + `--dedup`).
- `hooks/hooks.json` + `hooks/decision-gate.fish` — Stop hook, non-blocking:
  warns if the session made tool calls but recorded zero Decision rows.

**Verified:**
- `decision_exists_chosen_keys_on_session_target_source_chosen` (lib unit test):
  exact repeat = duplicate; different source / chosen / target / session =
  distinct. Pass.
- CLI `--dedup` E2E on a temp DB: first store inserts, repeat → `deduped: true`
  + `discovery_id: null`, `--force` bypasses, different `chosen` inserts fresh.
  3 Decision rows after 4 calls. Pass.
- Gate hook (temp DB): tool-calls + 0 decisions → warns; +1 decision → silent;
  idle session (no tool calls) → silent; missing DB → silent exit 0. Pass.
- `/decision` path = the same `store-discovery --dedup` call, verified above.
- `cargo fmt --all -- --check`, `cargo clippy --all-features -- -D warnings`,
  `cargo test --all-features` green.
- Skill auto-trigger requires a live Claude Code session with the plugin
  loaded; not simulated non-interactively. Install with the local marketplace
  command in MANUAL.md to exercise it.

**Scope:** highest-fidelity capture — the model records a decision as it makes
one, via a Claude Code plugin skill that auto-triggers on choice-shaped moments.
Optional layer on top of Phase 3/4.

**Files (Claude Code plugin `atheneum-decisions`):**
- `plugin.json` — name, skills + commands + hooks.
- `skills/record-decision/SKILL.md` — description tuned to trigger on
  "choosing between approaches / architectural tradeoff / picking an option."
  Body: write `metadata.json` to temp file, call
  `atheneum store-discovery <db> claude Decision <target> /tmp/dec.json
  --session $CLAUDE_CODE_SESSION_ID`. `source = "skill"`.
- `commands/decision.md` — `/decision <target> <chosen>` manual fallback.
- `hooks/decision-gate.fish` (Stop, soft-warn) — if session had
  `tool_call_count > 0` but `0` Decision rows with this `session_id`, print a
  reminder. Non-blocking.

**Verification:**
- Skill triggers on a simulated choice → Decision row with `source="skill"`,
  full `alternatives`+`rationale`.
- `/decision foo bar` → row stored.
- Stop-gate: tool calls + 0 decisions → reminder; with a decision → silent.

**Risks:**
- Skill over/under-triggering → tune description; non-blocking, so false
  negatives just mean Phase 3/4 cover it.
- Dedup key = `session_id`+`sequence`+`target`+`source` (same as Phase 3/4).

**Done when:** skill auto-fires on choices, `/decision` works, gate warns
correctly, no double-capture.

---

## Phase 6 — integration + docs — ✅ DONE (this branch)

CHANGELOG 0.8.0→0.9.0 (root + crate). MANUAL.md + README.md: `chat` + `watch-decisions` + `extract-decisions` (script) + decision-capture model. `discoveries-recent --session --type` + `session-digest` decisions block (Decision-filtered + source label). `cargo fmt/clippy --all-features -D warnings/test` green (328 tests). End-to-end CLI verified on temp DB: watch-decisions→discoveries-recent→chat--only-decisions. Live v11→v12 DB upgrade shipped separately (envoy stopped).

**Scope:** tie the layers together, verify against real data, update docs.

**Tasks:**
- Verify `atheneum chat --only-decisions` + `--walk` against Decision rows from
  Phase 3/4/5 (all sources, deduped).
- Verify `session-digest` surfaces decisions from all sources (confirm
  `discovery_type='Decision'` filter + `source` label in the digest).
- Observability: `atheneum discoveries-recent --type Decision` (add `--type`
  filter if absent).
- `CHANGELOG.md` (root + crate) `[Unreleased]` → version bump (0.8.0 → 0.9.0:
  schema migration v12 + `chat` + `watch-decisions` + `extract-decisions`).
- `crates/atheneum/MANUAL.md` + `README.md` — `chat` + `watch-decisions` +
  `extract-decisions` usage; decision-capture model documented.
- `docs/CHAT_DECISION_PLAN.md` — phase ✅ DONE logs + verification (mirror
  session-digest plan format).
- envoy: if the watcher folds into envoy instead of standalone (decision point
  below), update envoy CHANGELOG + service unit.

**Decision point — where the watcher runs:** standalone
`atheneum-decision-watcher.service` (Phase 4 default) vs a thread inside envoy
(envoy is always-on + has the atheneum bridge). Standalone keeps envoy
coordination-only; envoy-folded saves a service. **Recommend standalone**; revisit at Phase 4 start.

**Done when:** all layers verified against real data, docs current, CHANGELOG
bumped, CI green.

---

## Build order + dependencies

```
Phase 1 (schema v12: columns + FTS)  ── foundation; zero insert-path changes
   │
Phase 2 (chat CLI)                   ── uses Phase-1 columns + FTS; fast from start
   │
Phase 3 (backfill B)                 ── independent of 1/2; feeds --only-decisions
   │
Phase 4 (watcher C)                  ── independent; real-time structured decisions
   │
Phase 5 (skill A)                    ── ✅ DONE; cooperative high-fidelity
   │
Phase 6 (integration + docs)         ── after 1+2+3+4 (5 optional); ship
```

**Recommended sequence:** 1 → 2 → 3 → 4 → 6 (skip 5 unless wanted). Phase 1
corrects the schema (the thing you flagged) — index + FTS so every later query
is cheap. Phase 2 is the navigation surface on top. Phase 3 is the data-fixer
(highest leverage — converts the existing 649 noisy ReasoningLogs into real
decision chains). Phase 4 adds real-time. Phase 6 ties together + ships.

**Why schema-first:** without Phase 1, `--search` = full scan + `json_extract`
per row, and session filtering = same. The whole point — navigate without
burning tokens — is undercut if the *query* itself is expensive. Generated
columns + FTS make the find sub-ms; the `--tokens` budget then caps only the
*return*. Both ends cheap.

**Token-cost discipline (cross-cutting):** every retrieval command takes
`--tokens N` and is extractive (walk in order, stop at N). No command returns
unbounded chat. Search/filter happens in SQL (index/FTS) before token counting.
Graph walks (`--walk`, `thread`) are edge-bounded + token-bounded. Through-line
from the session-digest plan.