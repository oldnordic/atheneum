# Full atheneum CLI Reference

Every subcommand's first argument is the DB path. Kept in sync with
`crates/atheneum/README.md` in the atheneum repo -- if this drifts, that file
is the source of truth.

```
INGEST:
  init <db>                               Initialize a new graph database
  sync-wiki <db> <dir> [project]          Ingest .md files as wiki pages
  sync-journal <db> <dir> [project]       Ingest .md files as journal sections
  sync-logseq <db> <root> [project]       Recursively ingest Logseq pages/ and journals/
  sync-claude-transcript <db> <jsonl> [project] [agent]  Import Claude transcript
  store-discovery <db> <agent> <type> <target> [meta.json]  Store a discovery
  add-edge <db> <from> <to> <edge-type> [data.json]        Create a relation

GROUNDED CLAIMS:
  claim-pin <db> <entity-id> <project> <file-path> [--symbol <name>] [--id <receipt>]  Pin a falsifiable claim to live source code
  claim-verify <db> <repo-root> [--project P] [--apply]  Audit and verify claims against live filesystem
  audit <db> [--project P]                Compute staleness and claim verification report

TASKS:
  task-create <db> <title> [desc] [--project P]    Create a new task
  task-list <db> [--project P] [--status S]        List tasks (default: non-archived)
  task-update <db> <task-id> <status>              Update task status
  task-done <db> <task-id>                         Mark task as DONE
  task-archive <db> <task-id>                      Archive a task

MEMORY:
  memory-store <db> <key> <content> [--scope S] [--confidence N] [--project P]  Store a memory (upserts by key+scope+project)
  memory-get <db> <key> [--scope S] [--project P]      Retrieve memory by key
  memory-list <db> [--scope S] [--project P] [--offset N] [--limit N]  List memories (paginated, default limit 1000)
  memory-update <db> --id N [--content C] [--importance N] [--tags a,b --replace-tags]  Patch an existing memory in place
  pin <db> --id N                                   Pin an entity (always in seed_memory, cache-eviction immune)
  unpin <db> --id N                                 Unpin an entity

LIBRARIAN:
  lint <db-path> [--stale-days N]                   Graph-health check: orphans, broken wikilinks, stale superseded_by edges
  maintain <db-path> [--apply] [--stale-days N] [--rewire-threshold F] [--broken-link-mode <stub|sever>]  Rewire orphans, stub/sever broken links, resolve contradictions
  models-list <db-path>                             List models loaded on a local model server
  dashboard <db-path> [--port N]                     Web dashboard server (feature: web-ui)

DREAM:
  dream <db> [--scope S] [--project P] [--dry-run|--auto-merge]  Reflective memory consolidation
  dream-semantic <db> [--apply]                     Merge closely-related/redundant concepts (local-model prompt, lexical fallback)

QUERY & NAVIGATION:
  search <db> <query> [--k N] [--project P] [--max-tokens N]         Lexical search (optional HNSW candidate index with --features semantic-search)
  navigate <db> <query> [--k N] [--depth N] [--project P] [--kind K] [--max-tokens N]  Search then walk subgraphs
  thread <db> <query> [--k N] [--depth D=3] [--tokens T=1500] [--project P] [--json]  Walk a decision chain (caused_by/led_to edges)
  chat <db> --session <id> [--tokens T] [--direction recent|chrono] [--kinds K] [--role R] [--search Q] [--only-decisions] [--walk] [--offset N --limit L] [--json]  Token-budgeted walk of a session's chat records (or just its decisions)
  watch-decisions <db> [--once] [--interval S=2] [--config-dir D]... [--project P] [--agent A] [--dry-run]  Live-tail transcripts, capture structured decisions
  extract-decisions <db> [--all|<session-id>] [--dry-run] [--force] [--project P] [--agent A] [--model M] [--transcripts-dir D] [--max-chars N] [--ollama-url U] [--heuristic|--mode llm|heuristic] [--verbose]  Backfill decisions (LLM or --heuristic; feature: extract)
  wiki-search <db> <query> [--project P] [--limit N]  Full-text search over wiki pages (FTS5, falls back to name match)
  decision-search <db> <query> [--project P] [--limit N]  Content search over Decision discoveries (target/chosen/why)
  seed-memory <db> [--project P] [--tokens N]       Token-bounded, concept-grouped knowledge-base summary
  trace-get <db> --id N                             Replay a past navigate --trace query
  query-wiki <db> <path>                            Query a wiki page by path
  query-journal <db> <path>                         Query journal sections by path
  query-knowledge <db> <target> [--project P] [--max-tokens N]       Aggregated knowledge
  query-sessions <db> [--project P] [--offset N] [--limit N]  Session history
  query-events <db> [--session <id>] [--type <t>] [--offset N] [--limit N]  Event log
  session-digest <db> [--project P] [--last N] [--tokens T] [--json]  Bounded bootstrap digest
  session-trace <db> --session <id> [--limit N]     Session summary plus recent events
  tool-usage <db> --session <id> [--limit N]        Tool breakdown for one session
  discoveries-recent <db> [--project P] [--agent A] [--session S] [--type T] [--limit N]  Recent discoveries (filter by session and/or type)
  handoffs-recent <db> [--project P] [--agent A] [--limit N]     Recent handoffs
  events-recent <db> [--session ID] [--type T] [--limit N]       Recent events
  sessions-recent <db> [--project P] [--agent A] [--limit N] [--exclude-project P ...]     Recent sessions (hide non-repo project buckets)
  list-pages <db> [--project P] [--offset N] [--limit N]  List wiki pages (default limit 1000)
  entity <db> <id>                                  Print entity as JSON
  edge <db> <id>                                    Print edge as JSON
  neighbors <db> <id> [--depth N]                   One-hop edges or BFS subgraph
  graph-stats <db>                                  Graph topology counts

MAINTENANCE:
  reindex <db>                                      Rebuild optional HNSW human-search index
  consolidate <db> [target] [--project P]           Merge discoveries into Knowledge
```

## Companion binary: memory-prefetch-hints

Separate `[[bin]]` target, not an `atheneum` subcommand:

```
memory-prefetch-hints <db-path> --query <query> [--k 5] [--max-tokens 500]
    [--session-id <id>] [--trajectory <path>] [--trajectory-query <f32,f32,...>]
```

This is what the plugin's `prefetch-hints` UserPromptSubmit hook calls
automatically. Manual use is rarely needed -- see `API.md` in the atheneum
repo for the full scoring breakdown if debugging a ranking result.
