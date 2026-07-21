---
name: atheneum-cli
description: Use when a task needs atheneum functionality beyond decisions/memory/recall -- task tracking (kanban), wiki/journal sync, handoffs, decision-chain walks, session/event observability, or graph introspection. Covers the full `atheneum` CLI surface. Skip if the atheneum-mcp MCP server is already connected in this session -- prefer its tools over shelling out to the CLI when both are available.
---

# Atheneum CLI

`atheneum` is a single self-contained binary over one SQLite file -- no
server, no envoy dependency required. Every subcommand takes the DB path as
its first argument:

```bash
DB="${ATHENEUM_DB:-$HOME/.magellan/atheneum/atheneum.db}"
atheneum <subcommand> "$DB" [args...]
```

If the `atheneum-mcp` MCP server is connected in this session, prefer its
tools (`search`, `navigate`, `query_memory`, `store_memory`, etc.) over
shelling out here -- same underlying graph, no process-spawn overhead. This
skill is for when MCP isn't available, or for subcommands MCP doesn't
expose (kanban tasks, wiki/journal sync, raw graph introspection).

For decisions, general memory, and search, the dedicated `record-decision`
skill, `remember` skill, and `/recall` command already cover the common
path -- reach for this skill for everything else.

## Command categories

| Category | Commands | Use for |
|----------|----------|---------|
| Ingest | `sync-wiki`, `sync-journal`, `sync-logseq`, `sync-claude-transcript`, `store-discovery`, `add-edge` | Bulk-loading docs/transcripts, manual discovery/edge creation |
| Tasks | `task-create`, `task-list`, `task-update`, `task-done`, `task-archive` | Kanban-style task tracking scoped to a project |
| Librarian | `lint`, `maintain`, `models-list`, `dashboard` | Graph-health checks, orphan/broken-link repair |
| Dream | `dream`, `dream-semantic` | Reflective consolidation -- merge/dedupe related memories |
| Query & Navigation | `search`, `navigate`, `thread`, `chat`, `wiki-search`, `decision-search`, `session-digest`, `query-knowledge`, `query-sessions`, `query-events`, `graph-stats`, `entity`, `edge`, `neighbors` | Everything read-only: lexical search, graph walks, decision-chain traversal, session/event history |
| Observability | `session-trace`, `tool-usage`, `discoveries-recent`, `handoffs-recent`, `events-recent`, `sessions-recent` | Recent cross-agent/cross-session activity |
| Maintenance | `reindex`, `consolidate` | Rebuild search index, merge discoveries into Knowledge |

Full flag reference for every subcommand: `references/full-cli.md` in this
skill, or `crates/atheneum/README.md` / `MANUAL.md` in the atheneum repo
(same content, kept in sync).

## Most commonly needed commands

```bash
# Bounded bootstrap digest for a project (what session-bootstrap hook uses)
atheneum session-digest "$DB" --project <name> --last 3 --tokens 500

# Walk a decision chain (caused_by/led_to) for a topic
atheneum thread "$DB" "<query>" --depth 3 --tokens 1500

# Recent cross-agent activity
atheneum discoveries-recent "$DB" --project <name> --limit 10
atheneum sessions-recent "$DB" --project <name> --limit 10

# Kanban task tracking
atheneum task-create "$DB" "<title>" "<description>" --project <name>
atheneum task-list "$DB" --project <name> --status IN_PROGRESS
```

All commands return JSON except `session-digest` (plain text unless
`--json`) and a handful of observability reads that print human-readable
tables -- check `--help` on the specific subcommand if the shape is unclear.
