# Atheneum Manual

## Installation

### From crates.io

```bash
cargo add atheneum
```

### From source

```bash
git clone https://github.com/oldnordic/atheneum
cd atheneum
cargo build --release
```

---

## Overview

Atheneum is an embedded graph database for AI agent coordination. It stores discoveries, decisions, session histories, task handoffs, and knowledge across agent sessions — replacing ad-hoc file dumps with a queryable, persistent graph.

It is used as a library (embedded in your agent runtime) or accessed via envoy's HTTP bridge (`GET/POST /atheneum/*`).

---

## Opening a Graph

```rust
use atheneum::AtheneumGraph;
use std::path::Path;

// Persistent — creates file if absent
let graph = AtheneumGraph::open(Path::new("atheneum.db"))?;

// In-memory — for tests and ephemeral sessions
let graph = AtheneumGraph::open_in_memory()?;
```

The database schema is auto-migrated on `open()`. No separate migration step required.

### Upgrading An Existing Atheneum DB

For the main Atheneum knowledge database, upgrades are intended to be
in-place. You should not need to recreate the database or re-ingest your wiki,
memory, discovery, session, or task data just because the binary changed.

What happens on open:

- Forward-only SQL migrations are applied automatically.
- Existing graph entities and typed SQL rows are preserved.
- Additive schema changes such as new columns, indexes, generated columns, or
  FTS tables are stamped onto the existing DB in place.
- Wiki/memory/discovery content remains where it is; migration is not a
  re-import pass.

Recommended upgrade procedure:

```bash
# 1. Back up the database file first
cp ~/.local/share/atheneum/atheneum.db ~/.local/share/atheneum/atheneum.db.bak

# 2. Open it with the new binary (any normal read command is enough)
atheneum graph-stats ~/.local/share/atheneum/atheneum.db

# 3. Sanity-check the main data surfaces
atheneum sessions-recent ~/.local/share/atheneum/atheneum.db --limit 5
atheneum discoveries-recent ~/.local/share/atheneum/atheneum.db --limit 5
atheneum memory-list ~/.local/share/atheneum/atheneum.db --limit 5
```

What you do **not** need to do in the normal case:

- Recreate the Atheneum DB from scratch
- Re-sync all wiki pages just because the schema version changed
- Re-store memories or discoveries
- Rebuild any HNSW index unless you explicitly use `semantic-search`

When an extra maintenance step is useful:

- If you explicitly enabled `semantic-search`, run `atheneum reindex <db>` to
  rebuild the optional HNSW human-search index after an upgrade or large import.
- If wiki full-text search was previously left inconsistent by an external
  writer, Atheneum's open/health path can repair the FTS structures without a
  full DB rebuild.

---

## Configuration

Atheneum reads `~/.config/atheneum/config.toml` (or `$XDG_CONFIG_HOME/atheneum/config.toml`). A missing file is not an error — sensible defaults are used.

### Default config file

```toml
[atheneum]
db = "~/.local/share/atheneum/atheneum.db"
meta_db = "~/.local/share/atheneum/meta.db"

[llm]
provider = "ollama"
base_url = "http://localhost:11434"
model = "codellama"
api_key = ""

[embeddings]
provider = "hash"
dimension = 128
base_url = "http://localhost:11434"
model = "nomic-embed-text"
api_key = ""

[integrations]
# Cross-tool integration is opt-in. Each tool stays standalone by default.
[integrations.magellan]
enabled = false
config = "~/.config/magellan/config.toml"

[integrations.envoy]
enabled = false
url = "http://localhost:9876"
```

### CLI

```bash
# Create the default config file (idempotent; use --force to overwrite)
atheneum config init

# Print the currently effective configuration as JSON
atheneum config show
```

### Library

```rust
use atheneum::{Config, load_config, save_config};

let cfg = load_config()?;                         // from default location
let path = cfg.db_path();                         // tilde-expanded PathBuf
let meta = cfg.meta_db_path();

save_config(&Config::default())?;                 // write defaults to disk
```

Environment overrides follow the convention `ATHENEUM_<SECTION>_<KEY>` where supported by callers. Paths may contain leading `~`, which is expanded via `$HOME`.

The meta.db routing layer (`MetaRouter::open()`) honors `atheneum.meta_db` from this config and falls back to the XDG default if the config is missing or invalid.

---

## Maintainer Checklist

When changing Atheneum itself, keep the docs and gates in sync:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
cargo deny check
gitleaks detect --verbose --config .gitleaks.toml
semgrep ci --oss-only --config .semgrep/rules/
```

Rules:

- Update `CHANGELOG.md` for every user-visible fix, behavior change, or workflow change.
- Update `MANUAL.md` when you add or change a public function, CLI command, flag, or operator workflow.
- Refresh `README.md`, `ARCHITECTURE.md`, and `API.md` before a release when public workflows or signatures changed.
- Prefer adding the manual/changelog update in the same patch as the code change so the docs cannot drift.
- A repo-local `.claude/` wrapper (if present on your machine) is not published — run it from the repo root for your own gating, but rely on the `cargo` commands above for the published check.

---

## Agent Sessions

Sessions track every coding session — who, when, what branch, how many tool calls, cost.

```rust
use atheneum::graph::{AtheneumGraph, SessionParams};

graph.record_session(SessionParams {
    session_id: "abc-123".into(),
    agent_name: "claude-main".into(),
    project: "my-project".into(),
    tool: "claude-code".into(),
    trigger: "cli".into(),           // "cli" | "subagent" | "hook"
    model: Some("claude-sonnet-4".into()),
    git_branch: Some("feat/auth".into()),
    git_head: Some("a1b2c3d".into()),
    parent_session_id: None,          // set for subagents
})?;
```

### Ending a Session

```rust
use atheneum::graph::EndSessionParams;

graph.end_session(EndSessionParams {
    session_id: "abc-123".into(),
    exit_status: "end_turn".into(),
    prompt_count: 12,
    tool_call_count: 47,
    file_write_count: 3,
    commit_count: 1,
    test_run_count: 2,
    total_input_tokens: 50_000,
    total_output_tokens: 8_000,
    total_cost_usd: 0.15,
})?;
```

### Querying Recent Sessions

```rust
// Last 3 sessions for a project (newest first)
let sessions = graph.query_sessions("my-project", 3, None)?;

// Children of a specific session
let children = graph.query_sessions("my-project", 10, Some("parent-session-id"))?;

for s in sessions {
    println!("{} {} {}tc {}fw last:{:?}",
        s.started_at, s.git_branch.unwrap_or_default(),
        s.tool_call_count, s.file_write_count, s.last_tool);
}
```

### Runtime Cache Stats

Atheneum now keeps a process-local concurrent query cache for the hottest repeated read paths. You can inspect that runtime state directly:

```rust
let stats = graph.runtime_stats();
println!(
    "hits={} misses={} memory_q={} session_q={} wiki_q={}",
    stats.cache_hits,
    stats.cache_misses,
    stats.memory_queries,
    stats.session_queries,
    stats.wiki_queries,
);
```

Current cached reads:

- `query_memory()` / `list_memory()`
- `query_sessions()`
- `query_events()`
- `query_knowledge()` / `query_knowledge_in_project()`
- `list_wiki_pages()`

Writes invalidate the relevant cache domain automatically after successful mutation.

### CLI Observability Shortcuts

For operator workflows, you do not need to open SQLite directly:

```bash
atheneum session-trace <db-path> --session <id> [--limit N]
atheneum tool-usage <db-path> --session <id> [--limit N]
atheneum discoveries-recent <db-path> [--project P] [--agent A] [--limit N]
atheneum handoffs-recent <db-path> [--project P] [--agent A] [--limit N]
atheneum events-recent <db-path> [--session ID] [--type T] [--limit N]
atheneum sessions-recent <db-path> [--project P] [--agent A] [--limit N] [--exclude-project P ...]
```

These commands all return JSON so they can be consumed by local agents and shell tooling without a separate adapter layer.

### Session Digest (Bootstrap Grounding)

`session-digest` composes a bounded, ranked plain-text packet so a new
session can ground on what prior sessions in the same project actually did —
decisions made, files touched, open tasks — instead of re-discovering from
scratch. It is extractive (no model call): it composes real rows from
`sessions`, `event_log`, `graph_entities` (ReasoningLog / Memory) and
`discoveries` into a compact packet, ranked by recency and truncated to a
token budget.

```bash
# Plain-text digest (default), bounded to ~500 tokens
atheneum session-digest <db-path> --project my-project --last 3 --tokens 500

# Structured JSON for programmatic consumption
atheneum session-digest <db-path> --project my-project --last 3 --json
```

Activity (tool calls, file writes, top files) is **computed from `event_log`**
rather than trusted from the `sessions` ledger columns, which the session
recorder leaves at zero. If the `--project` filter matches nothing (project
tagging is sparse — many sessions are tagged `tmp`), the digest falls back to
the most recent sessions across all projects and prints a notice line. The
packet ends with thread-anchor ReasoningLog entity ids that you can follow
with `atheneum navigate <db> <query> --kind ReasoningLog --depth N` to walk a
decision thread. Discoveries stored with `--session` are also linked into a
`caused_by`/`led_to` chain per session (most-recent earlier same-session
decision), so `atheneum thread <db> <query> [--depth N] [--tokens T]` walks
the chain directly — lexical match on `ReasoningLog` + `Discovery` entry
points, then BFS along those edges only, bounded to a token budget. The
human renderer prints each entry's decision metadata (`source` / `sequence`
/ `chosen` / `rationale` / `alternatives`) when the entry is a `Decision`,
then the chain edges literally (`from ──caused_by/led_to──> to` with named
endpoints), then the BFS-expanded related entities. `--json` returns the raw
subgraphs unchanged.

Attribute a discovery to a session so it appears in that session's digest
block:

```bash
atheneum store-discovery <db-path> claude Decision gemv_q4_0 meta.json \
  --session c663d1ff --project rocmforge
```

Library usage:

```rust
let text = graph.compose_digest(Some("my-project"), 3, 500)?;
let value = graph.compose_digest_json(Some("my-project"), 3)?;
```

### Tool Call Evidence

```rust
use atheneum::graph::ToolCallParams;

graph.record_evidence_tool_call(ToolCallParams {
    session_id: "abc-123".into(),
    tool_name: "Edit".into(),
    tool_version: None,
    input_hash: Some("deadbeef".into()),
    input_summary: Some("write src/lib.rs".into()),
    output_hash: None,
    output_summary: Some("ok".into()),
    exit_status: "success".into(),
    latency_ms: 234,
    input_tokens_est: None,
    tool_category: "file_write".into(),
})?;
```

### Subagent Handover

```rust
// Subagent writes this on stop — the parent reads it
graph.record_subagent_handover(
    "sub-session-id",
    "Fixed SQL param ordering in query_sessions. evidence.rs line 547.",
    &["src/graph/evidence.rs".to_string()],
    "end_turn",
)?;
```

---

## Discoveries

Discoveries are non-obvious facts, invariants, and decisions stored so future agents don't re-discover them.

```rust
use serde_json::json;

let id = graph.store_discovery(
    "claude",           // agent name
    "Bug",              // discovery type
    "query_sessions",   // target symbol
    json!({
        "file": "src/graph/evidence.rs",
        "line": 547,
        "why": "anonymous ? params required when project is None and parent_id is Some",
        "project_id": "atheneum"
    }),
)?;
```

### Querying Discoveries

```rust
// By target symbol
let discoveries = graph.query_discoveries("query_sessions")?;

// By project (no target required — for session bootstrap context injection)
let recent = graph.recent_project_context("atheneum", 8)?;
```

### Preview Candidate Matches

For fuzzy identifiers, Atheneum can return ranked existing candidates without mutating the graph:

```rust
let candidates = graph.preview_entity_candidates(
    "HTTP Router",
    5,
    Some("atheneum"),
    Some("WikiPage"),
    0.2,
)?;

for candidate in candidates {
    println!("{} {} {:.3}", candidate.kind, candidate.name, candidate.score);
}
```

This is intended for preview/disambiguation flows where you want to inspect likely matches before storing new memory, discovery, or wiki links.

### Query Validation And Repair

Atheneum can preview a navigation query plan before execution:

```rust
let plan = graph.preview_navigate_query(
    "timezone",
    5,
    2,
    None,
    Some("memories"),
)?;

assert!(plan.executable);
assert_eq!(plan.resolved_kind.as_deref(), Some("Memory"));
assert!(plan.kind_repaired);
```

This plan stage:

- trims accidental whitespace from the query
- resolves common entity-kind aliases such as `memory`, `memories`, `wiki`, and `discoveries`
- rejects unknown kinds before traversal instead of silently returning empty results
- records warnings/errors so repaired execution is explicit to callers

### Preview Before Commit

Atheneum can also preview normalized discovery, memory, and handoff payloads before writing:

```rust
let discovery = graph.preview_discovery(
    "codex",
    "pattern",
    "query_cache",
    serde_json::json!({"summary": "cache repeated reads", "project_id": "atheneum"}),
    5,
    0.2,
)?;

let memory = graph.preview_memory(
    "timezone",
    "UTC+1",
    "user",
    0.9,
    None,
    None,
    5,
    0.2,
)?;

let handoff = graph.preview_handoff(
    "claude1",
    "claude2",
    Some("atheneum"),
    serde_json::json!({"task": "finish review", "files_analyzed": ["src/lib.rs"]}),
    5,
    0.2,
)?;
```

These preview APIs:

- do not insert entities or edges
- return deterministic `content_hash` values
- include exact existing matches plus fuzzy candidate matches, even when the fuzzy score alone would have filtered them out

### CLI Navigate Kind Filters

The CLI `navigate` command now accepts `--kind` and reports the repaired/validated plan in its JSON output:

```bash
atheneum navigate ./atheneum.db timezone --kind memories
```
- are intended for operator review or agent-side "propose first, commit later" flows

---

## Memory Prefetch Hints

`memory-prefetch-hints` is a separate binary (not an `atheneum` subcommand)
installed alongside the CLI. It ranks `Memory` entities against a query and
returns a token-budgeted JSON candidate list, meant to run once at session
start so an agent already has relevant memories before the first turn.

```bash
memory-prefetch-hints ./atheneum.db --query "flash-attn split-KV coherence fix" --k 5
```

```json
{
  "query": "flash-attn split-KV coherence fix",
  "candidates": [
    {
      "handle": 695590,
      "kind": "Memory",
      "name": "feedback-verify-instrumentation",
      "score": 0.745,
      "score_breakdown": {"bm25": 1.0, "tf_idf": 0.48, "recency": 0.1, "session_continuity": 0.0, "trajectory_bonus": 0.0, "kind_weight": 0.015},
      "estimated_tokens": 412
    }
  ]
}
```

### Scoring a live session higher

Pass `--session-id` with the current session's ID to give entities from that
same session a `session_continuity` bonus, instead of scoring only whatever
coincidental overlap exists between candidates already in the result batch:

```bash
memory-prefetch-hints ./atheneum.db --query "..." --session-id "$SESSION_ID"
```

### Trajectory-graph lookup (optional)

If you have a taught PSF1/PSF2 trajectory blob (see `docs/` for the format
notes), pass it alongside a matching query vector to get a `trajectory_bonus`
and `"prefetch": true`/`"handle_kind": "trajectory"` on matching candidates:

```bash
memory-prefetch-hints ./atheneum.db --query "1 some query" \
    --trajectory ./trajectories.psf --trajectory-query "1.0"
```

The query's *first token* is matched against each trajectory node's
`source_token` as an exact string — this is a raw token-ID match, not a
semantic one, so it's most useful when the query is itself constructed from
the same token space the trajectory was taught from (e.g. driven by another
tool), not free-form natural language.

### Wiring into an agent runtime

The Hermes `atheneum` plugin calls this binary from `_run_prefetch_hints`,
passing the plugin's own `--db-path`, `--session-id`, and (if configured via
`ATHENEUM_TRAJECTORY_PATH`) `--trajectory`/`--trajectory-query`. Any other
agent runtime can shell out to the same binary the same way — it only needs
a database path and a query string; everything else is optional.

---

## Decision Capture from Chat Transcripts

Claude Code chat transcripts (`~/.claude/projects/*/*.jsonl`) carry the
structured-choice signals that are genuinely *decisions* — `AskUserQuestion`
(a human-answered choice), `ExitPlanMode` (a plan approved for execution),
`TaskCreate`, and `TodoWrite`. Atheneum captures those as first-class
`Decision` discoveries so the graph holds the decision chain, not just the
chat text. Three commands, one capture model.

### Capture model

Each captured decision is stored with `discovery_type = "Decision"` and a
metadata block carrying the structured fields:

| Field | Meaning |
|-------|---------|
| `source` | Which signal produced it: `askuser`, `exitplan`, `taskcreate`, `todowrite` (live watcher / backfiller), or `llm-extract` (post-hoc LLM extractor) |
| `chosen` | The selected option / approved plan / created task subject |
| `alternatives` | The options that were not chosen (AskUserQuestion labels, ExitPlanMode `allowedPrompts`, etc.) |
| `rationale` | Why — the chosen option's description, the task description, or the LLM extractor's reasoning |
| `sequence` | The tool-call sequence number within the session, for ordering |
| `session_id` | The transcript session (file stem) the decision belongs to |

**Dedup key:** `session_id` + `sequence` + `target` + `source`. Both the
live watcher and the backfiller call `decision_exists` before insert, so a
decision is captured once even if the same transcript is scanned repeatedly.
A decision captured live (`source = "askuser"`) and re-extracted post-hoc
(`source = "llm-extract"`) is intentionally *not* collapsed — that
cross-layer double is the documented tradeoff, because the two layers have
different fidelity and you may want both records.

### `chat` — token-budgeted chat navigation

`atheneum chat <db> --session <id>` walks a session's records in `sequence`
order, emitting `role` + a content snippet per record and bounding output to
a token budget. `--only-decisions` narrows the walk to the session's
`Decision` discoveries (from any source), deduped by the capture key, and
renders each with its `source` + `sequence` inline plus the `chosen` /
`rationale` / `alternatives` / `why` metadata as indented sub-lines, so the
mode reads as a rationale-bearing view rather than a bare index. Add `--walk`
to append a `caused_by` / `led_to` chain snippet per decision when those edges
exist.

```bash
# Full session walk, bounded to ~2k tokens
atheneum chat ./atheneum.db --session abc-123 --tokens 2000

# Just the structured decisions captured for that session
atheneum chat ./atheneum.db --session abc-123 --only-decisions --json

# Decisions with their linked chain snippet
atheneum chat ./atheneum.db --session abc-123 --only-decisions --walk
```

### `extract-decisions` — one-shot LLM backfill (operator script)

`extract-decisions` is a standalone operator script (`~/.local/bin/`,
reusing the `dream` + `remember-to-atheneum` pattern), **not** an `atheneum`
subcommand. It runs a local LLM (Ollama `qwen3.5` by default) over a
session's transcript, extracts decision-shaped turns from assistant `text`
+ `thinking` blocks, and stores each via `atheneum store-discovery` — so
each extracted decision is linked into the session thread (`caused_by` /
`led_to`) for free. It covers the decisions that *lack* a Tier-1 structured
signal (the watcher catches those deterministically). No cloud / Claude API
calls.

```bash
# One session, store
extract-decisions <session-id>

# Every transcript, resumable (skips sessions that already have an
# llm-extract Decision); --force re-extracts a session
extract-decisions --all
extract-decisions --all --force --project atheneum

# Preview only, store nothing
extract-decisions <session-id> --dry-run
extract-decisions --all --dry-run
```

Options: `--db PATH` (default `$ATHENEUM_DB` or
`~/.magellan/atheneum/atheneum.db`), `--project NAME`, `--agent NAME`
(default `claude`), `--model NAME` (default `qwen3.5`), `--transcripts-dir`,
`--max-chars N` (per-chunk cap, default 20000), `--force`, `--verbose`.

**Native subcommand:** the same backfill is also available as an `atheneum`
subcommand — `atheneum extract-decisions <db> [--all | <session-id>] [...]`
(see `atheneum extract-decisions` with no args for the full usage). It is a
Rust port of the script, built behind the `extract` Cargo feature (default
off; enable with `--features extract` or `--all-features`). It calls Ollama
in-process (`ureq`), applies the same prompt/schema, hallucination guard,
sequence recovery, and `--all` resumability, and stores `Decision` rows via
`graph.store_discovery` directly — no temp file, no shell-out to
`store-discovery`. The operator script remains the default (no special build
needed); use the subcommand when you want one binary and no Python dep.

**Backend choice — LLM or heuristic:** the subcommand has two extraction
backends and the user picks one per run, so the tradeoff is explicit:

```bash
# Default: local Ollama LLM (qwen3.5). Higher precision on prose decisions.
atheneum extract-decisions <db> <sid> --transcripts-dir <dir>
atheneum extract-decisions <db> <sid> --mode llm        # explicit

# Heuristic: rule-based, no LLM, no network. Zero deps.
atheneum extract-decisions <db> <sid> --transcripts-dir <dir> --heuristic
atheneum extract-decisions <db> <sid> --mode heuristic

# Or set it once for the shell:
set -x ATHENEUM_EXTRACT_MODE heuristic
```

The heuristic backend catches decision-shaped sentences that carry an explicit
rationale clause (`because` / `since` / `so that`), reuses the same
hallucination guard + store/dedup plumbing, and writes `source = "heuristic"`
(distinct from `llm-extract`, so the two backends are separately resumable and
distinguishable in the graph). It is deterministic, so re-runs dedup exactly on
`(target, chosen)`. **Tradeoff:** lower recall + some false positives vs the
LLM — a trigger phrase without a rationale clause is dropped (precision
filter), and a real decision phrased without a trigger word is missed. Use it
when Ollama is unavailable or you want a deterministic, offline pass; run
`--dry-run` first to review what it would store.

**Hallucination guard:** a decision is accepted only if `target`, `chosen`,
and `rationale` each contain a real alphabetic token (≥3 chars), rejecting
placeholder fill. **Idempotency:** LLM extraction is non-deterministic, so a
store-mode run skips any session that already has a Decision from the *same*
backend's `source` tag (pre-scan via `atheneum discoveries-recent --session
<sid>`); re-running the same backend is a no-op and `--all` is resumable.
Run `--dry-run` before `--all`.

### `watch-decisions` — live capture

Tails the same transcript files in a loop and stores `Decision` rows in real
time. In-memory per-file cursor (offset / inode / mtime); a half-written
final line is re-read on the next scan, never fabricated into a decision.

```bash
# Always-on, 2s poll (the shipped systemd unit)
atheneum watch-decisions ./atheneum.db --interval 2 --project atheneum

# Single cold-cursor scan — safe for cron; decision_exists dedup is the
# cross-invocation safety net because each --once call re-reads the file
atheneum watch-decisions ./atheneum.db --once --project atheneum
```

The watcher is **detect-only** at the Tier-1 layer. The SessionStop
`sync-claude-transcript` hook still owns full transcript ingest (prompt
summaries, tool-call evidence, accessed-file edges) at session end — the
watcher adds the structured-decision layer on top, it does not replace
ingest. A standalone `atheneum-decision-watcher.service` systemd unit ships
the always-on path; it opens the same WAL-mode DB as envoy with a
`busy_timeout`, so concurrent reads and the watcher's append-only writes do
not contend.

### Observing captured decisions

```bash
# Decisions for one session (any source)
atheneum discoveries-recent ./atheneum.db --session abc-123 --type Decision --limit 50

# All decisions across the project
atheneum discoveries-recent ./atheneum.db --project atheneum --type Decision
```

`session-digest` surfaces decisions from all sources — the digest's decision
section filters on `discovery_type = 'Decision'` and labels each with its
`source`, so live-watcher, backfiller, and manual `store_discovery` rows
appear together.

### Cooperative skill capture (Phase 5)

The highest-fidelity layer is a Claude Code companion plugin,
`plugin/atheneum-decisions/` (shipped in this repo), that records a decision
*as the model makes it* — `source = "skill"`. Three components:

- **`record-decision` skill** — auto-triggers on choosing between approaches /
  an architectural tradeoff, writes a `metadata.json` (`chosen` /
  `alternatives` / `rationale` / `target`), and calls
  `atheneum store-discovery <db> claude Decision <target> /tmp/dec.json
  --session $CLAUDE_CODE_SESSION_ID --dedup`.
- **`/decision <target> <chosen> [rationale]` command** — manual fallback
  using the same store path.
- **`decision-gate` Stop hook** — non-blocking; warns when a session made
  tool calls but recorded zero Decision rows.

The skill/command layer has no stable transcript `sequence`, so it dedups on
`(session_id, target, source, chosen)` via `AtheneumGraph::decision_exists_chosen`,
surfaced in the CLI as `store-discovery --dedup` (skip a duplicate Decision
insert; print `deduped: true`) and `--force` (bypass). The watcher's
sequence-keyed dedup is unchanged; cross-layer doubles (different `source`)
are an accepted tradeoff, not a bug.

```bash
# Same store path the skill uses — opt-in dedup
atheneum store-discovery ./atheneum.db claude Decision storage-engine dec.json \
  --session $CLAUDE_CODE_SESSION_ID --dedup
# → {"discovery_id": 1, ...} on first call
# → {"deduped": true, "discovery_id": null, ...} on a repeat of the same choice
```

**Install the plugin (local marketplace):**

```bash
# From the atheneum repo root — register a local marketplace and install
claude plugin marketplace add ./plugin
claude plugin install atheneum-decisions@atheneum-decisions
```

Then `/decision` and the `record-decision` skill are active; the Stop-gate
warns on sessions with work but no recorded decisions.

---

## Knowledge Graph

```rust
// Store a linked discovery
let id = graph.store_discovery_in_project(
    "claude", "Decision", "auth-middleware",
    Some("my-project"),
    json!({ "why": "legal compliance", "risk": "high" }),
)?;

// Query knowledge for a symbol+project
let knowledge = graph.query_knowledge_in_project("auth-middleware", Some("my-project"))?;
```

---

## Task Planning

```rust
use atheneum::graph::AtheneumGraph;
use serde_json::json;

// Create a task
let task_id = graph.create_task("Implement session handover", Some("my-project"))?;

// Add requirements
graph.add_requirement(task_id, "Writes git diff on stop", None)?;

// Update status
graph.update_task_status(task_id, atheneum::graph::KanbanStatus::InProgress)?;
```

---

## Wiki Ingestion

Atheneum parses Markdown files with frontmatter and `[[wikilinks]]` into the knowledge graph.

```rust
let content = r#"---
title: "Session Accountability"
type: concept
---
# Session Accountability
See also [[envoy]] and [[grounded-coding]].
"#;

let entity_id = graph.ingest_wiki_page("session-accountability.md", content, None)?;
```

### Journal Sections

```rust
// Journals use ## HH:MM | Title headers and Kanban lines
let journal = r#"
## 14:23 | Fixed param bug
Corrected SQL ordering in evidence.rs.

## 15:00 | Deployed
"envoy" -> DONE
"#;
let sections = graph.parse_journal_sections(journal)?;
graph.ingest_journal_sections(&sections, Some("my-project"))?;
```

### Searching Wiki Pages

Atheneum uses an FTS5 index over `wiki_pages` for full-text search. Results are ranked by BM25 and include an excerpt only; the full body is never returned by the search API, so you can safely feed results into a context window without accidentally dumping entire articles.

```rust
let hits = graph.search_wiki_pages("session accountability", Some("my-project"), 0, 10)?;
for hit in &hits {
    println!("{} (score={}): {}", hit.path, hit.score, hit.excerpt);
}
```

### Backfilling Wiki Pages

If wiki pages were inserted directly into the `wiki_pages` SQL table (for example, by an older helper script), they may exist as rows but not as proper `WikiPage` graph entities with wikilink edges. `backfill_wiki_pages_to_graph` re-ingests each row through `ingest_wiki_page`, repairing stubs and restoring navigation.

```rust
let fixed = graph.backfill_wiki_pages_to_graph(Some("my-project"))?;
println!("repaired {} pages", fixed.len());
```

### FTS Index Resilience

The `wiki_pages_fts` FTS5 virtual table can be left internally inconsistent when an external SQLite writer (system `sqlite3`, Python, another tool) touches the database between atheneum runs. `AtheneumGraph::open()` detects this on every open and self-heals before creating the connection pool:

1. Probes `wiki_pages_fts` on a fresh connection.
2. If the probe fails, purges the virtual-table entry and shadow tables directly from `sqlite_master` with `PRAGMA writable_schema=ON`.
3. Recreates the table and triggers on another fresh connection.
4. Runs `delete-all` → repopulate from `wiki_pages` → `rebuild` on a fourth fresh connection to finalize shadow-table invariants.
5. Checkpoints WAL so the pool connections open onto a consistent DB.

After healing, `sync-wiki`, `search-wiki`, and `backfill-wiki` work normally. The process is idempotent: a healthy index passes the probe and skips all destructive steps. You do not need to run any manual Python repair script.

---

## HopGraph

HopGraph is an optional retrieval mode: **embeddings find the door, graph walk retrieves the room.** Unlike flat RAG, HopGraph uses vector similarity only to locate entry points, then expands connected knowledge via graph traversal.

This is not the mandatory agent path. Grounded LLM workflows can navigate and query Atheneum directly through graph edges and typed SQL payload tables without any HNSW index. The vector path is kept opt-in because it primarily helps human fuzzy search and costs real memory/CPU to maintain.

### Token-Budgeted Retrieval

```rust
use atheneum::graph::{AtheneumGraph, EdgeType};

let graph = AtheneumGraph::open(Path::new("atheneum.db"))?;

let views = graph.hopgraph_query(
    "session accountability",      // query text
    3,                             // k: max entry-point entities
    2,                             // depth: BFS expansion depth
    Some(&[EdgeType::Explains, EdgeType::Wikilink]),  // allowed edge types
    2000,                          // max_tokens budget per view
    None,                          // project_id filter
)?;

for view in &views {
    println!("entry={} entities={} edges={}",
        view.entry_id, view.entities.len(), view.edges.len());
}
```

`hopgraph_query` performs: lexical search → filtered BFS subgraph → token-budgeted truncation. Orphan edges (pointing to removed entities) are dropped. The entry entity is always kept regardless of budget.

### Filtered Subgraph Walk

```rust
use atheneum::graph::EdgeType;

// Walk only Explains and Wikilink edges from an entity
let view = graph.get_subgraph_filtered(
    entity_id,
    3,      // depth
    Some(&[EdgeType::Explains, EdgeType::Wikilink]),
)?;
```

### Embedding Backends

```rust
// Default: HashEmbedder (128-dim, zero deps, always available)
let dim = graph.embedder_dimension(); // 128

// Switch to neural embeddings (requires --features neural-embed)
#[cfg(feature = "neural-embed")]
{
    use atheneum::graph::OllamaEmbedder;
    graph.set_embedder(Box::new(OllamaEmbedder::nomic_embed_text()));
    graph.build_search_index()?; // rebuild index with new dimension (768)
    assert_eq!(graph.embedder_dimension(), 768);
}
```

| Backend | Dimension | Dependencies | Quality |
|---------|-----------|-------------|---------|
| `HashEmbedder` | 128 | None | Token overlap only ("car" ≠ "automobile") |
| `OllamaEmbedder` | 768 | ollama + nomic-embed-text | Semantic similarity |

### Discovery Consolidation

Merge duplicate Discovery entities into deduplicated Knowledge entities:

```rust
// Consolidate a single target
let knowledge_id = graph.consolidate_discoveries("query_sessions", Some("my-project"))?;

// Consolidate all targets in a project
let results = graph.consolidation_pass(Some("my-project"))?;
for (target, kid) in &results {
    println!("{} → knowledge {}", target, kid);
}
```

Consolidation creates `DerivedFrom` edges from Knowledge → source Discoveries. Idempotent — re-running returns the existing Knowledge entity.

### Bridge Wiki to Code Symbols

```rust
graph.link_wiki_to_symbols(
    "/path/to/.magellan/magellan/magellan.db",
    "claude",
    Some("my-project"),
)?;
```

For each wiki page's `[[wikilinks]]`, queries the magellan DB for matching code symbols, imports them as Discovery entities, and creates `Explains` edges from wiki page → symbol. Idempotent.

---

## Search

```rust
// Full-text search
let results = graph.full_text_search("query_sessions")?;

// Lexical search. Default build: bag-of-tokens scan over graph_entities.
// With --features semantic-search: HNSW hash-projected index + lexical fallback.
// Matches on shared tokens — not neural/semantic. "car" won't match "automobile".
let results = graph.lexical_search("SQL parameter ordering bug", 5, Some("atheneum"), None, None)?;

// Token-budgeted search — truncate results to fit a context window.
let results = graph.lexical_search("SQL parameter ordering bug", 5, Some("atheneum"), None, Some(500))?;
```

---

## Memory

Memory entries are stable facts stored distinct from Knowledge (merged discoveries) and WikiPage (documents). Each memory has a key, scope, confidence score, and optional project.

Scopes: `user` (preferences), `project` (project facts), `agent` (agent behavior), `memory` (general notes).

Memories are upserted -- storing with the same key, scope, and project_id updates the existing entry instead of creating a duplicate.

```rust
use atheneum::AtheneumGraph;
use std::path::Path;

let graph = AtheneumGraph::open(Path::new("atheneum.db"))?;

// Store a memory
let id = graph.store_memory(
    "timezone",           // key
    "UTC+1",              // content
    "user",               // scope
    0.9,                  // confidence (0.0-1.0)
    None,                 // project_id
    None,                 // tags
)?;

// Retrieve by key
let items = graph.query_memory("timezone", Some("user"), None)?;

// List all memories in a scope
let all = graph.list_memory(Some("user"), None)?;
```

---

## Dream

Dream is atheneum's reflective consolidation pass. It scans memories for problems -- duplicates, stale entries, contradictions, and verbosity -- and either reports them (dry run) or merges them (auto-merge).

What dream does:
1. **SCAN** -- reads all memories in scope
2. **DEDUPLICATE** -- finds near-duplicates using trigram Jaccard similarity (entries that say the same thing differently)
3. **STALE** -- flags entries not updated in N days with low confidence
4. **CONTRADICTION** -- detects same key across different scopes with low content similarity
5. **VERBOSE** -- scores content length vs unique-word ratio
6. **CONSOLIDATED** -- merges findings, creates `SupersededBy` edges pointing old entries to replacements

There are two dream commands:
- `dream` -- runs consolidation over memory entries
- `wiki-dream` -- runs the same pipeline over wiki page entities

```rust
use atheneum::{AtheneumGraph, DreamConfig, DreamMode};
use std::path::Path;

let graph = AtheneumGraph::open(Path::new("atheneum.db"))?;

// Dry run -- report only, no mutations
let report = graph.dream_pass(
    DreamMode::DryRun,
    None,                   // scope filter (None = all)
    Some("my-project"),     // project filter
    &DreamConfig::default(),
)?;
for finding in &report.findings {
    println!("{:?}: {}", finding.phase, finding.description);
}

// Auto-merge -- actually create SupersededBy edges
let report = graph.dream_pass(DreamMode::AutoMerge, None, None, &DreamConfig::default())?;

// Wiki dream -- same pipeline for wiki pages
let wiki_report = graph.wiki_dream_pass(DreamMode::AutoMerge, Some("my-project"), &DreamConfig::default())?;
```

---

## CLI Commands

### Ingest

```bash
# Initialize a new graph database
atheneum init <db-path>

# Sync a wiki directory into the graph
atheneum sync-wiki <db-path> <wiki-dir> [project-id]

# Sync journal files
atheneum sync-journal <db-path> <journal-dir> [project-id]

# Recursively sync a Logseq graph root
atheneum sync-logseq <db-path> <wiki-root> [project-id]

# Import a Claude Code transcript JSONL
atheneum sync-claude-transcript <db-path> <transcript.jsonl> [project-id] [agent-name]

# Store a discovery
atheneum store-discovery <db-path> <agent> <type> <target> [metadata.json]

# Create a relation between two entities
atheneum add-edge <db-path> <from-id> <to-id> <edge-type> [data.json|--data 'json']
```

`sync-logseq` expects a Logseq-style root with `pages/` and/or `journals/`. It recursively ingests markdown files under those directories. Wiki page `[[links]]` are stored as first-class `wikilink` edges, enabling graph traversal through article and note relationships.

`sync-claude-transcript` expects a Claude Code transcript JSONL, typically under `~/.claude/projects/<encoded-project>/<session-id>.jsonl`. It imports prompt summaries, assistant replies, observed tool calls, `accessed` file relations for `Read`/`Edit`/`Write`, and session token/cache totals. Re-running on the same append-only transcript imports only new lines because Atheneum stores a transcript cursor in SQL.

`store-discovery` takes an optional JSON file for metadata. The metadata JSON can contain fields like `project_id`, `why`, `file`, `line`.

`add-edge` creates a typed edge between two entities. Valid edge types include: `performed_by`, `assigned_to`, `called`, `accessed`, `modified`, `verified_by`, `caused_by`, `created`, `related_to`, `mentions`, `wikilink`, `implements`, `depends_on`, `tested_by`, `fixed_by`, `regressed_by`, `observed_in`, `belongs_to_project`, `similar_failure`, `requires_skill`, `handled_by_tool`, `explains`, `derived_from`, `superseded_by`, `consolidated_from`.

### Tasks

```bash
# Create a new task
atheneum task-create <db-path> <title> [description] [--project P]

# List tasks (default: non-archived)
atheneum task-list <db-path> [--project P] [--status S]

# List archived tasks explicitly
atheneum task-list <db-path> --status ARCHIVED [--project P]

# Update task status
atheneum task-update <db-path> <task-id> <status>

# Mark task as DONE
atheneum task-done <db-path> <task-id>

# Archive a task
atheneum task-archive <db-path> <task-id>
```

Valid statuses: `TODO`, `IN_PROGRESS`, `DONE`, `BLOCKED`, `ARCHIVED`.

### Memory

```bash
# Store a memory
atheneum memory-store <db-path> <key> <content> [--scope S] [--confidence N] [--project P]

# Retrieve memory by key
atheneum memory-get <db-path> <key> [--scope S] [--project P]

# List memories (paginated; default limit 1000)
atheneum memory-list <db-path> [--scope S] [--project P] [--offset N] [--limit N]
```

Memories are upserted -- storing with the same key + scope + project updates the existing entry. Default scope is `user`, default confidence is `1.0`.

### Dream

```bash
# Run reflective memory consolidation pass
atheneum dream <db-path> [--scope S] [--project P] [--dry-run|--auto-merge]

# Run consolidation over wiki pages
atheneum wiki-dream <db-path> [--project P] [--dry-run|--auto-merge]
```

`--dry-run` (default) reports findings without modifying the graph. `--auto-merge` creates `SupersededBy` edges pointing old entries to their replacements.

Output is a JSON `DreamReport` with findings organized by phase (DEDUPLICATE, STALE, CONTRADICTION, VERBOSE, CONSOLIDATED).

### Query and Navigation

```bash
# Lexical search over all entities (optional HNSW candidate index with --features semantic-search)
atheneum search <db-path> <query> [--k N] [--project P] [--max-tokens N]

# Search then BFS-walk graph subgraphs
atheneum navigate <db-path> <query> [--k N] [--depth N] [--project P] [--kind K] [--max-tokens N] [--concise]

# Walk a decision chain — discoveries linked by caused_by/led_to per session
atheneum thread <db-path> <query> [--k N=3] [--depth D=3] [--tokens T=1500] [--project P] [--json]

# Query a wiki page by path
atheneum query-wiki <db-path> <path>

# Full-text search over wiki pages (excerpts only; no full body)
atheneum search-wiki <db-path> <query> [--limit N] [--offset N] [--project P]

# Backfill wiki pages written directly to the SQL table into the graph
atheneum backfill-wiki <db-path> [--project P]

# Query journal sections by path
atheneum query-journal <db-path> <path>

# Aggregated knowledge for a target
atheneum query-knowledge <db-path> <target> [--project P] [--max-tokens N]

# Session history
atheneum query-sessions <db-path> [--project P] [--offset N] [--limit N]

# Event log
atheneum query-events <db-path> [--session <id>] [--type <type>] [--offset N] [--limit N]

# Session summary plus recent events
atheneum session-trace <db-path> --session <id> [--limit N]

# Tool-call breakdown for one session
atheneum tool-usage <db-path> --session <id> [--limit N]

# Recent discoveries
atheneum discoveries-recent <db-path> [--project P] [--agent A] [--limit N]

# Recent handoffs
atheneum handoffs-recent <db-path> [--project P] [--agent A] [--limit N]

# Recent events
atheneum events-recent <db-path> [--session ID] [--type T] [--limit N]

# Recent sessions
atheneum sessions-recent <db-path> [--project P] [--agent A] [--limit N] [--exclude-project P ...]

# List wiki pages (default limit 1000)
atheneum list-pages <db-path> [--project P] [--offset N] [--limit N]

# Print a graph entity as JSON
atheneum entity <db-path> <entity-id>

# Print a graph edge as JSON
atheneum edge <db-path> <edge-id>

# One-hop edges or BFS subgraph
atheneum neighbors <db-path> <entity-id> [--depth N]

# Graph topology counts
atheneum graph-stats <db-path>
```

`search` matches on shared tokens -- not semantic similarity. "car" will not match "automobile". Good for symbol and identifier search. Use `--max-tokens` to truncate the result list before it reaches your LLM context window. The default build scans `graph_entities` with a bag-of-tokens scorer; the optional `semantic-search` feature adds an HNSW candidate index for human fuzzy lookup, with lexical ranking and lexical fallback still defining the final result order.

`search-wiki` uses the FTS5 index over `wiki_pages` (`title`, `body`, and `path`). It returns ranked excerpts only; the full article body is never included in the output. Prefix queries work automatically: searching `rout` matches `Router`, `Routes`, and path fragments like `wiki/router.md`. If FTS5 returns no hits, `search-wiki` falls back to a graph-entity name/path/title substring search so partial concept queries still find stored pages. Use `--limit` and `--offset` for pagination, and `--project` to scope the search.

`list-pages` returns metadata for every wiki page (path, title, project, timestamps) without requiring a query. Use it to browse what is stored before searching, or to enumerate pages for export.

`backfill-wiki` re-ingests every `wiki_pages` SQL row through `ingest_wiki_page`. Use it to repair pages that were written directly to the SQL table without creating a proper `WikiPage` graph entity or wikilink edges. It skips pages whose graph entity already has a body and is not marked as a stub.

`navigate` performs a search, then expands each hit into a subgraph using BFS. The `--kind` flag filters by entity type (accepts aliases like `memory`, `memories`, `wiki`, `discoveries`). The output includes the validated query plan plus subgraph views. Use `--max-tokens` to truncate each subgraph view to a token budget (the entry entity is always kept; neighbors are dropped until the budget fits). Use `--concise` to emit compact Markdown instead of JSON — designed for pasting into a language-model context window.

`query-knowledge` aggregates discoveries and handoffs for a target. Use `--max-tokens` to limit the total response size; discoveries are dropped first, then handoffs, and `"truncated": true` is set when truncation occurs.

### Observability Commands

The following commands are available for querying and inspecting session, tool, discovery, and handoff activity without running raw SQL queries:

- `session-trace` returns a specific session's summary plus its associated events and tool calls, showing a timeline of agent activity.
- `tool-usage` aggregates tool call counts for a specific session, providing a breakdown of tool invocations.
- `discoveries-recent` returns a list of recent discoveries, with optional filtering by project and agent.
- `handoffs-recent` returns a list of recent handoffs, with optional filtering by project and agent.
- `events-recent` retrieves recent events, allowing filtering by session ID and event type.
- `sessions-recent` retrieves recent sessions, with optional project and agent filtering. `--exclude-project P` (repeatable) hides named project buckets — e.g. `tmp` and `Projects`, the honest fallback names for sessions run from `/tmp` or a non-repo parent dir — without re-attributing the rows. The `LIMIT` applies after exclusion.

### Cross-Project Registry (Meta)

How does atheneum know which projects exist? It **reads magellan's canonical
project registry directly** (`~/.magellan/meta.db`, maintained automatically by
`magellan.service`). This means every project magellan indexes is visible to
atheneum with zero manual setup — you do not have to register each project by
hand. Atheneum attaches magellan's registry as a read-only source and layers
its own small *overlay* on top for enrichment data that magellan does not store
(such as programming language or an atheneum-specific database path).

```bash
# List all known projects (auto-discovered from magellan's registry
# plus any enrichment in atheneum's overlay)
atheneum meta-list

# List projects filtered by language
atheneum meta-list --language rust
```

`meta-list` shows every enabled project — typically the full set of magellan
indexes (e.g. all 25 indexed databases on this machine). Use `--language` to
filter to one programming language.

**Optional enrichment with `meta-register`.** In most cases you do not need
this, because magellan's registry already supplies the project name, root, and
database path. `meta-register` is for adding the extra fields magellan does not
own — the language tag and an atheneum-specific database path. It writes into
atheneum's overlay (`~/.local/share/atheneum/meta.db`, i.e.
`$XDG_DATA_HOME/atheneum/meta.db`); re-registering the same name updates those
fields. If magellan is not installed at all, the overlay becomes the full
registry, so atheneum keeps working standalone.

```bash
# Optional: add enrichment (language, atheneum-db) to a project
atheneum meta-register envoy /path/to/envoy \
  /path/to/envoy/.magellan/magellan.db \
  --atheneum-db /path/to/envoy/atheneum.db \
  --language rust
```

### Cross-Project Queries

Atheneum can query across magellan-indexed codebases without importing their data. It uses the project list from magellan's canonical registry (plus its own overlay) as a routing table, and lazily `ATTACH DATABASE` each project's magellan DB on demand.

```bash
# Search for a symbol across all Rust projects
atheneum cross-search "build_router" --language rust --k 10

# Search across all registered projects (no language filter)
atheneum cross-search "checkpoint" --k 20

# Navigate: search + BFS subgraph walk per project
atheneum cross-navigate "error handling" --language rust --k 5 --depth 2
```

Output is JSON. `cross-search` returns ranked symbol hits with project, name, kind, and file path. `cross-navigate` returns one subgraph view per entry point, including entities and edges from each attached magellan database.

The router keeps an LRU cache of attached databases (default capacity 8). Projects whose database is missing, unreadable, or has an incompatible schema (e.g. not yet fully indexed, so its `graph_entities` table is absent) are skipped with a warning rather than aborting the whole query. This lets cross-search run cleanly across a registry that mixes mature and freshly-registered projects.

#### End-to-End Example: Finding All HTTP Router Implementations

Imagine you maintain three Rust projects — `envoy`, `magellan`, and `atheneum` — and you want to see how each one implements its HTTP router. Here is the complete workflow:

**Step 1 — Index each project with magellan (one-time per project):**

```bash
cd ~/Projects/envoy
magellan watch --root ./src --db ~/.magellan/envoy/envoy.db --scan-initial

cd ~/Projects/magellan
magellan watch --root ./src --db ~/.magellan/magellan/magellan.db --scan-initial

cd ~/Projects/atheneum
magellan watch --root ./src --db ~/.magellan/atheneum/atheneum.db --scan-initial
```

**Step 2 — Atheneum sees them automatically (no registration needed):**

Because atheneum reads magellan's canonical registry, every project you indexed
in Step 1 is already visible — run `atheneum meta-list` to confirm. You only
need `meta-register` if you want to tag a project's language or point it at an
atheneum-specific database (optional enrichment):

```bash
# Optional: tag languages so --language filters work
atheneum meta-register envoy ~/Projects/envoy \
  ~/.magellan/envoy/envoy.db --language rust

atheneum meta-register magellan ~/Projects/magellan \
  ~/.magellan/magellan/magellan.db --language rust

atheneum meta-register atheneum ~/Projects/atheneum \
  ~/.magellan/atheneum/atheneum.db --language rust
```

**Step 3 — Search across all three projects for "build_router":**

```bash
atheneum cross-search "build_router" --language rust --k 10
```

Sample output:

```json
{
  "results": [
    {
      "project": "envoy",
      "name": "build_router",
      "kind": "Function",
      "file": "src/server.rs",
      "line": 42,
      "score": 1.0
    },
    {
      "project": "magellan",
      "name": "build_router",
      "kind": "Function",
      "file": "src/http/mod.rs",
      "line": 88,
      "score": 1.0
    }
  ]
}
```

**Step 4 — Navigate deeper: see what each router calls and what calls it:**

```bash
atheneum cross-navigate "build_router" --language rust --k 3 --depth 2
```

This returns one subgraph per entry-point match. Each subgraph shows the function's callers, callees, and related symbols — directly from each project's magellan DB, without copying data into atheneum.

**Step 5 — Use in a script or agent:**

```rust
use atheneum::CrossRouter;

fn main() -> anyhow::Result<()> {
    let mut router = CrossRouter::open()?;

    // Find all Rust projects that have a "build_router" function
    let hits = router.cross_search("build_router", Some("rust"), 10)?;
    for hit in &hits {
        println!("{}: {} in {}:{}",
            hit.project, hit.name, hit.file, hit.line.unwrap_or(0));
    }

    // For each hit, expand 2 hops of graph context
    let views = router.cross_navigate("build_router", Some("rust"), 3, 2)?;
    for view in &views {
        println!("project={} entities={} edges={}",
            view.project, view.subgraph.entities.len(), view.subgraph.edges.len());
    }

    Ok(())
}
```

**What happens under the hood:**

1. `CrossRouter::open()` reads `~/.config/atheneum/config.toml` to find `meta_db` (or uses the XDG default).
2. `cross_search` queries the `project_registry` table for enabled Rust projects, then `ATTACH DATABASE`es each magellan DB one at a time (or reuses cached attachments).
3. Each attached DB is queried via a cross-schema `UNION ALL` over `graph_entities` and `graph_edges`.
4. Results are ranked by exact-match score and returned as a single list.
5. `cross_navigate` does the same search, then runs BFS per entry point per project, returning subgraph views with full entity/edge detail.

**Cleaning up:**

```bash
# Remove a project from the registry (soft-disable)
atheneum meta-register envoy ... --disable

# Re-enable
atheneum meta-register envoy ...  # omit --disable
```

### Config

```bash
# Create the default config file at ~/.config/atheneum/config.toml
atheneum config init

# Overwrite an existing config file
atheneum config init --force

# Print the effective configuration as JSON
atheneum config show
```

`config init` writes the default TOML (XDG paths, local Ollama defaults, disabled cross-tool integrations). `config show` reads the file (or defaults if missing) and prints JSON, which is useful for debugging path expansion and integration flags.

### Maintenance

```bash
# Rebuild optional HNSW human-search index (requires --features semantic-search; no-op otherwise)
atheneum reindex <db-path>

# Merge discoveries into Knowledge entities
atheneum consolidate <db-path> [target] [--project P]

# Print version
atheneum --version

# Print help
atheneum help
```

`reindex` rebuilds the optional HNSW index over all entities and then runs a WAL checkpoint to reclaim disk space. Useful only when you explicitly enabled `semantic-search` for human fuzzy search. No-op when the feature is disabled, which is the normal agent-oriented build. (Prior to v0.6.2, the checkpoint call could panic with "Execute returned results"; it now uses `query_row` because `PRAGMA wal_checkpoint` returns a row.)

`consolidate` merges all Discovery entities for a target (or all targets) into deduplicated Knowledge entities with `DerivedFrom` edges. Idempotent -- re-running returns the existing Knowledge entity.

---

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `default` | yes | Core graph, wiki, sessions, planning, search, thread — lexical (bag-of-tokens) search + BFS graph navigation |
| `semantic-search` | no | Optional HNSW candidate index for human fuzzy lookup in `search` (opt-in; heavy — index + embedder). Off by default; agent retrieval uses lexical search, graph traversal, and SQL payload queries |
| `neural-embed` | no | Ollama neural embeddings (requires `ureq`, ollama + nomic-embed-text) |
| `extract` | no | Native `atheneum extract-decisions` subcommand — Rust port of the `~/.local/bin/extract-decisions` script (LLM backend requires `ureq` + ollama, default `qwen3.5`; `--heuristic` backend needs no LLM/network) |
| `web` | no | Web dashboard (axum + askama templates) |
| `cli` | no | `atheneum` CLI binary |
| `async` | no | Async runtime support |

---

## Error Handling

All functions return `anyhow::Result<T>`. Errors include context about which operation failed.

```rust
match graph.record_session(params) {
    Ok(()) => {},
    Err(e) => eprintln!("Session record failed: {:#}", e),
}
```

---

## Thread Safety

`AtheneumGraph` uses internal `Mutex` locking. The `pub` methods take `&self` (shared reference) and handle synchronization internally. For concurrent access from multiple threads, wrap in `Arc<AtheneumGraph>` or use connection pooling per thread.

---

## Requirements

- Rust 1.75+
- SQLite 3.35+ with JSON1 extension (bundled via rusqlite by default)

## License

GPL-3.0-only -- see [LICENSE](LICENSE).
