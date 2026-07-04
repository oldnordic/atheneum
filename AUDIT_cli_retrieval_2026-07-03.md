# Atheneum CLI Retrieval Bug Investigation — 2026-07-03

## Scope

The Hermes plugin (atheneum_memory_search / atheneum_memory_list / atheneum_memory_store)
works correctly — data is stored and retrieved. The bugs are in the **atheneum CLI**
itself: several commands fail to find data that exists in the DB.

## Data Presence (confirmed via sqlite3)

| Table | Rows | Notes |
|-------|------|-------|
| `memory_entries` | 188 | scope: memory(54), reference(25), session_summary(23), user(23), project(18), session:*(various) |
| `wiki_pages` | 661 | With FTS table (wiki_pages_fts) |
| `discoveries` (table) | 806 | Decision(381), Finding(11), Bug(3), Pattern(3), Insight(4), others |
| `graph_entities` kind=Discovery | 806 | Mirror of discoveries table |
| `graph_entities` kind=ReasoningLog | 7529 | Chat/reasoning turns |
| `graph_entities` kind=WikiPage | 661 | Mirror of wiki_pages |
| `graph_entities` kind=Memory | 188 | Mirror of memory_entries |
| `journal_sections` | 0 | EMPTY — sync-journal never run or not wired |

## ROOT CAUSE (confirmed by subagent + live verification)

The Hermes config sets `project_id: forge`, which gets appended as `--project forge`
to every atheneum CLI call the Hermes plugin makes. But the data in the DB has
DIFFERENT project_id values:

| Data type | project_id values in DB | Matches `forge`? |
|-----------|------------------------|------------------|
| wiki_pages | NULL (all 661) | NO — `WHERE project_id = 'forge'` = 0 rows |
| discoveries (Decision) | Projects(232), core(54), models(40), splice(14), rocmforge(12), memoria(9), atheneum(5), NULL(15) | NO — ZERO have `forge` |
| memory_entries | forge(134), rocmforge(30), claude-code-session(11), ... | PARTIAL — 134/188 match |

**Live proof:**
```
$ atheneum list-pages <db> --project forge --limit 3    → count: 0
$ atheneum list-pages <db> --limit 3                     → count: 3
$ atheneum discoveries-recent <db> --type Decision --project forge → count: 0
$ atheneum discoveries-recent <db> --type Decision                  → count: 3
```

Every `--project forge` filter silently drops the data. This is the PRIMARY bug —
not a query/search implementation issue, but a data-tag mismatch between the
Hermes config's project scope and the actual project_id labels on stored data.

### Three compounding defects

1. **Data-tag mismatch**: wiki_pages and decisions were ingested under different
   project labels (or NULL) than the Hermes config's `project_id: forge`.
2. **No dedicated wiki-search / decision tools**: the MCP server exposes only
   generic `search` + `query_memory` (which does exact name match, not content
   search). No wiki-search, no decision-search.
3. **`query_memory` is useless for recall**: `memory.rs:286` uses `WHERE name = ?`
   (exact entity name match), not content search.

### Fix direction

Three options (not mutually exclusive):
- (a) **Retag data**: `UPDATE wiki_pages SET project_id='forge'` (or the correct
  project). Quick but doesn't fix the structural issue.
- (b) **Make project_id=forge a no-op filter**: treat the configured project_id as
  "no filter" when searching wiki/decisions (they're cross-project knowledge).
- (c) **Add wiki-search and decision-search CLI commands** + expose them as agent
  tools, and stop forcing `--project forge` onto wiki/decision reads.

Recommended: (c) — the wiki/decision knowledge is inherently cross-project and
should not be scoped to a single project_id.

---

## Bug A-1 [BUG]: `query-wiki` requires exact full filesystem path

**Severity**: BUG (usability — wiki pages unreachable via CLI for normal users)

**Evidence**:
```
$ atheneum query-wiki ~/.magellan/atheneum/atheneum.db magellan.md
No wiki page found at path: magellan.md

$ atheneum query-wiki ~/.magellan/atheneum/atheneum.db /home/feanor/wiki/magellan.md
No wiki page found at path: /home/feanor/wiki/magellan.md
```

**Root cause**: `get_wiki_page` (`crates/atheneum/src/graph/wiki.rs:433`) uses
`WHERE path = ?1` — exact match only. The stored paths are full filesystem paths
(`/home/feanor/wiki/pages/magellan.md`). There is:
1. No partial/fuzzy path matching (user types `magellan.md`, DB has the full path)
2. No title-based lookup (user types "Magellan", DB has title field)
3. No wiki-search command at all (the FTS table exists but is never queried by any CLI command)

**Fix needed**: Add a `wiki-search <db> <query> [--k N]` CLI command that uses the
existing `wiki_pages_fts` table (already built, just not queried). Also make
`query-wiki` do a `LIKE '%<path>'` fallback if exact match fails.

## Bug A-2 [BUG]: `thread` (decision chain) fails on common queries due to crude token matching

**Severity**: BUG (decisions exist but are hard to find)

**Evidence**:
```
$ atheneum thread ~/.magellan/atheneum/atheneum.db "HNSW" --project forge
# thread: HNSW
_No decision-chain matches found._

$ atheneum thread ~/.magellan/atheneum/atheneum.db "sparse inference"
_No decision-chain matches found._

$ atheneum thread ~/.magellan/atheneum/atheneum.db "gate weight upload"
_3 entry point(s) · depth up to 3 · token budget ~1500_  [WORKS]
```

**Root cause**: `thread_query` (`crates/atheneum/src/graph/navigation.rs:567`)
calls `lexical_search(query, k, project_id, Some("Discovery"))` which uses
`fallback_lexical_search` — a bag-of-tokens scorer
(`crates/atheneum/src/graph/search.rs:415`). Short queries like "HNSW" produce
few tokens; if none overlap with the discovery's target/content text, score=0
and the discovery is invisible. The search has no fuzzy matching, no substring
matching, no FTS — just exact token set intersection.

**Fix needed**: The `lexical_search` function should ALSO try FTS5 (the
`wiki_pages_fts` pattern) or at minimum substring matching on the
`discoveries.target` field. A `decision-search <db> <query>` CLI command that
queries `discoveries WHERE target LIKE '%query%' OR chosen LIKE '%query%'`
would be the minimal fix.

## Bug A-3 [BUG]: No `wiki-search` CLI command exists

**Severity**: BUG (feature gap — 661 wiki pages are completely unsearchable via CLI)

**Evidence**: The CLI help lists `query-wiki <db> <path>` (exact path lookup)
and `list-pages <db>` (paginated list), but NO content search. The `search`
command finds WikiPage entities via lexical token scoring, but that's generic
entity search — it doesn't use the `wiki_pages_fts` table that was
specifically built for full-text wiki search.

**Fix needed**: Add `wiki-search <db> <query> [--k N] [--project P]` that
queries `wiki_pages_fts` via `MATCH`:
```sql
SELECT w.* FROM wiki_pages_fts f
JOIN wiki_pages w ON w.rowid = f.rowid
WHERE wiki_pages_fts MATCH ?1
ORDER BY rank
LIMIT ?2
```

## Bug A-4 [BUG]: No `decision-search` / `discovery-search` CLI command

**Severity**: BUG (381 decisions are only reachable via `discoveries-recent --type Decision`
which lists chronologically, not by content)

**Evidence**:
```
$ atheneum discoveries-recent ~/.magellan/atheneum/atheneum.db --type Decision --limit 3
[Works — returns recent decisions chronologically]
```
But there's no way to search decisions BY CONTENT. The `thread` command tries
but fails on short queries (Bug A-2). The `search` command finds Discovery
entities but they're drowned out by WikiPage entities in unfiltered mode.

**Fix needed**: Add `decision-search <db> <query> [--project P] [--k N]` that
queries:
```sql
SELECT * FROM discoveries
WHERE discovery_type = 'Decision'
AND (target LIKE '%query%' OR json_extract(metadata, '$.chosen') LIKE '%query%'
     OR json_extract(metadata, '$.why') LIKE '%query%')
ORDER BY created_at DESC LIMIT k
```

## Bug A-5 [BUG]: `journal_sections` table is EMPTY (0 rows)

**Severity**: BUG (journal sync never ran or is broken)

**Evidence**:
```
sqlite3 ~/.magellan/atheneum/atheneum.db "SELECT COUNT(*) FROM journal_sections;"
0
```

The `sync-journal <db> <dir>` CLI command exists. It was either never run, or
the journal source directory has no `.md` files. The user's wiki is at
`/home/feanor/wiki` with a `journals/` subdirectory. Need to verify:
1. Does `/home/feanor/wiki/journals/` exist and have content?
2. Has `sync-journal` ever been run?
3. Is the `query-journal` command tested?

## Bug A-6 [IMPROVE]: `memory-bootstrap` includes session noise

**Severity**: IMPROVE

**Evidence**:
```
$ atheneum memory-bootstrap ~/.magellan/atheneum/atheneum.db --project forge --tokens 500
{
  "memories": [
    {
      "confidence": 0.5,
      "content": "User: hi\nAssistant: hi Luiz — what's on the table?",
      "key": "turn_20260703_124853_5347c6_1",
      "scope": "session:20260703_124853_5347c6",
```

The memory-bootstrap returns session-scoped turn-by-turn chat logs (confidence 0.5)
mixed with real durable memories (scope=memory, confidence 1.0). The
low-confidence session noise crowds out high-value memories in the
token-budgeted output. Should either filter scope=session:* or prioritize
scope=memory first.

## Positive findings (what works)

- `memory-store` / `memory-get` / `memory-list`: all work correctly
- `search` (lexical/HNSW): works for memory and wiki entities
- `navigate`: works (graph walk from search seeds)
- `discoveries-recent --type Decision`: works (chronological list)
- `sync-wiki`: works (661 pages ingested with FTS)
- `store-discovery`: works (806 discoveries stored)
- `dream` (memory consolidation): exists and runs
- Hermes plugin (`atheneum_memory_search`): works correctly (tested live)

## Recommended fix priority

1. **A-1 + A-3**: Add `wiki-search` CLI command + fix `query-wiki` path matching.
   The FTS table exists — this is mostly wiring.
2. **A-2 + A-4**: Add `decision-search` CLI command + improve `thread` token matching.
   Decisions are the highest-value knowledge artifact and currently nearly unreachable.
3. **A-5**: Investigate why journal_sections is empty; run sync-journal if appropriate.
4. **A-6**: Filter memory-bootstrap to exclude session noise or prioritize scope=memory.

---

*Generated by Hermes Agent, 2026-07-03. All evidence from live CLI tests + source
inspection (file:line cited). Subagent deep-dive results pending.*
