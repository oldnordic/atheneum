# atheneum-decisions

Companion plugin for [atheneum](https://github.com/oldnordic/atheneum) — session bootstrap, per-turn memory prefetch, decision capture, general memory, and CLI reference. Works against any atheneum SQLite database directly; no envoy server required.

Ships one shared content set (skills, commands) with **per-agent manifests and hook script sets**:

| Agent | Manifest | Hooks |
|---|---|---|
| Claude Code | `.claude-plugin/plugin.json` + `hooks/hooks.json` | `hooks/*.py` (Claude `hookSpecificOutput` JSON schema) |
| Kimi Code CLI | `kimi.plugin.json` | `hooks/kimi/*.py` (plain-text stdout, Kimi payload fields) |

## Requirements

- The `atheneum` CLI on `$PATH` (`cargo install atheneum`)
- `python3` (hooks are stdlib-only, cross-platform)
- For prefetch hints specifically: the `memory-prefetch-hints` binary, installed alongside `atheneum` from the same crate

## What it does

**Hooks:**
- `SessionStart` → `session-bootstrap.py`: injects a bounded session digest (recent activity, decisions, open tasks for the current project) as context before the first turn.
- `UserPromptSubmit` → `prefetch-hints.py`: runs `memory-prefetch-hints` against each prompt and the live session ID, injecting ranked `Memory` candidates (BM25 + TF-IDF + recency + session continuity + optional trajectory bonus) as context for that turn.
- `Stop` → `decision-gate.py`: non-blocking reminder if a session did real work but recorded zero `Decision` rows.

**Skills** (model-invoked automatically based on task context):
- `record-decision` — records a genuine architectural/implementation choice as a `Decision` row, source-tagged `skill`, dedup'd on `(session_id, target, source, chosen)`.
- `remember` — records a durable non-decision fact via `memory-store` (upserts by key, so re-recording an updated fact doesn't duplicate it).
- `atheneum-cli` — reference for the rest of the `atheneum` CLI surface (tasks, wiki/journal sync, handoffs, graph introspection) for anything the above don't cover.

**Commands** (explicit, user-invoked):
- `/decision <target> <chosen> [rationale]` — manual fallback for `record-decision`. (Kimi: `/atheneum-decisions:decision`.)
- `/recall <query>` or `/recall --key <key>` — manual search/lookup fallback for when the automatic prefetch hint wasn't enough. (Kimi: `/atheneum-decisions:recall`.)

## Environment variables

- `ATHENEUM_DB` — path to the atheneum SQLite database. Resolution order: `ATHENEUM_DB` → `~/.magellan/atheneum/atheneum.db` → `~/.hermes/atheneum/atheneum.db` (Kimi set; the Claude set defaults to the `.hermes` path directly).
- `ATHENEUM_TRAJECTORY_PATH` — optional, enables trajectory-graph lookup in prefetch hints if set to a valid PSF1/PSF2 blob path.
- `ATHENEUM_BIN` / `ATHENEUM_PREFETCH_BIN` — optional explicit path to the `atheneum` / `memory-prefetch-hints` binaries. Hook scripts don't always inherit a full login-shell `PATH`; each hook falls back to `~/.local/bin/<name>` if a plain `PATH` lookup fails, but set these directly if the binaries live somewhere else.

## Install

### Claude Code

```bash
# Test locally without installing
claude --plugin-dir ./plugin/atheneum-decisions

# Or add this repo as a marketplace and install from it
/plugin marketplace add oldnordic/atheneum
/plugin install atheneum-decisions@atheneum
```

### Kimi Code CLI

```
/plugins install /home/feanor/Projects/atheneum/plugin/atheneum-decisions
/reload
```

Notes for Kimi:
- The trust prompt on third-party install defaults to cancel — accept it.
- Installation copies the plugin to `~/.kimi-code/plugins/managed/atheneum-decisions/` and the CLI runs the copy; after editing this source, reinstall (`/plugins install <dir>` again) and `/reload`.
- The MCP server (`atheneum-mcp`) is NOT declared in the Kimi manifest — it is already wired globally in `~/.kimi-code/mcp.json`. If you'd rather manage it from the plugin panel (`M` key), remove the `mcp.json` entry and add an `mcpServers` block to `kimi.plugin.json` (`command: "atheneum-mcp"` works since `~/.local/bin` is on `PATH`).
- Kimi hook scripts print plain text to stdout (Kimi may append hook stdout to context); the `Stop` gate's reminder therefore goes to stdout, not stderr like the Claude variant.

## Design notes

Every hook is defensive by construction: missing DB, missing `atheneum`/`memory-prefetch-hints` binary, empty query, or any subprocess failure exits `0` with no output. A session without atheneum configured behaves exactly as if this plugin weren't installed — nothing blocks, nothing errors visibly.
