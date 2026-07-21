---
allowed-tools: Bash(atheneum search:*), Bash(atheneum memory-get:*), Bash(atheneum query-knowledge:*)
description: Manually search atheneum's memory graph for prior facts, decisions, or discoveries -- explicit fallback for when the automatic prefetch-hints context wasn't enough or a specific key is known.
argument-hint: <query> or --key <key>
disable-model-invocation: false
---

# /recall

`$ARGUMENTS` is either a free-text query, or `--key <key>` for an exact
lookup by key.

**Exact lookup** (when `$ARGUMENTS` starts with `--key`):

```bash
DB="${ATHENEUM_DB:-$HOME/.magellan/atheneum/atheneum.db}"
atheneum memory-get "$DB" "<key>"
```

**Free-text search** (otherwise):

```bash
DB="${ATHENEUM_DB:-$HOME/.magellan/atheneum/atheneum.db}"
atheneum search "$DB" "<query>" --k 8
```

If the free-text search returns nothing useful, also try a broader
aggregated view of what's known about the topic:

```bash
atheneum query-knowledge "$DB" "<query>"
```

Report the results to the user: for each hit, the name/kind and a short
excerpt of the content, not the raw JSON. If nothing matches, say so plainly
rather than padding the answer -- a real "nothing found" is more useful than
a stretch match.
