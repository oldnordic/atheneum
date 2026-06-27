# Atheneum Architecture

## Overview

Atheneum is a SQLite-backed graph database for agent coordination. It stores two layers of data:

1. **Graph layer** (sqlitegraph) — entities and edges for relationship navigation
2. **SQL payload layer** (native tables) — typed, queryable data for structured access

This dual-layer design lets LLMs traverse relationships through the graph while applications query typed data through SQL.

## Dual-Layer Design

```
┌─────────────────────────────────────────┐
│           Application / CLI             │
├─────────────────────────────────────────┤
│         AtheneumGraph (API)             │
├─────────────────────────────────────────┤
│  Graph Layer      │    SQL Layer        │
│  (sqlitegraph)    │    (native tables)  │
│                   │                     │
│  graph_entities   │    wiki_pages       │
│  graph_edges      │    journal_sections │
│                   │    agents           │
│                   │    tasks            │
│                   │    ...              │
└─────────────────────────────────────────┘
│              SQLite (disk or :memory:)  │
└─────────────────────────────────────────┘
```

Every write goes to **both** layers:
- `ingest_wiki_page` inserts into `wiki_pages` table AND creates a `WikiPage` graph entity
- `ingest_journal` inserts into `journal_sections` table AND creates `JournalSection` graph entities
- Wikilink edges are created in `graph_edges` as `RelatedTo` edges

## Schema

### Graph Layer (sqlitegraph-managed)

**graph_entities**
| Column    | Type    | Description                          |
|-----------|---------|--------------------------------------|
| id        | INTEGER | Primary key, auto-increment          |
| kind      | TEXT    | Entity type: WikiPage, JournalSection, Agent, Task, Event, ... |
| name      | TEXT    | Human-readable identifier            |
| file_path | TEXT    | Optional source file path            |
| data      | JSON    | Arbitrary JSON payload               |

**graph_edges**
| Column    | Type    | Description                          |
|-----------|---------|--------------------------------------|
| id        | INTEGER | Primary key, auto-increment          |
| from_id   | INTEGER | Source entity ID                     |
| to_id     | INTEGER | Target entity ID                     |
| edge_type | TEXT    | EdgeType as string                   |
| data      | JSON    | Edge metadata                        |

### SQL Layer (atheneum-managed)

In current releases, the SQL layer carries more than wiki/journal payloads. Hot
coordination reads and migrations also depend on typed tables such as
`sessions`, `event_log`, `discoveries`, `memory_entries`, `tasks`, and
`handoffs`, plus the chat-navigation generated columns and FTS support added by
migrations v11-v13.

**wiki_pages** — Markdown wiki pages with YAML frontmatter
| Column       | Type    | Description                          |
|--------------|---------|--------------------------------------|
| id           | INTEGER | Primary key                          |
| path         | TEXT    | UNIQUE file path                     |
| title        | TEXT    | From YAML frontmatter                |
| content_hash | TEXT    | SHA-256 of body for dedup            |
| body         | TEXT    | Markdown body (no frontmatter)       |
| wikilinks    | JSON    | Array of [[link]] targets            |
| project_id   | TEXT    | Optional project filter              |
| metadata     | JSON    | Full parsed frontmatter + extras     |
| created_at   | TEXT    | ISO 8601 timestamp                   |
| updated_at   | TEXT    | ISO 8601 timestamp (on upsert)       |

**journal_sections** — Logseq-style timed journal segments
| Column         | Type    | Description                          |
|----------------|---------|--------------------------------------|
| id             | INTEGER | Primary key                          |
| path           | TEXT    | Source file path                     |
| section_index  | INTEGER | Position in file, UNIQUE with path   |
| time           | TEXT    | Optional HH:MM timestamp             |
| title          | TEXT    | Section header                       |
| body           | TEXT    | Section body                         |
| kanban_updates | JSON    | Array of {task_title, new_status}    |
| wikilinks      | JSON    | Array of [[link]] targets            |
| project_id     | TEXT    | Optional project filter              |
| metadata       | JSON    | Full section data                    |
| created_at     | TEXT    | ISO 8601 timestamp                   |

## Crate API for Plugin Authors

### Opening a Graph

```rust
use atheneum::AtheneumGraph;

// On-disk database
let graph = AtheneumGraph::open(std::path::Path::new("./atheneum.db"))?;

// In-memory (for tests)
let graph = AtheneumGraph::open_in_memory()?;
```

### Wiki Ingestion

```rust
// Single page
let page_id = graph.ingest_wiki_page("wiki/getting-started.md", content, Some("my-project"))?;

// Directory batch
let ids = graph.sync_wiki_directory(std::path::Path::new("./wiki"), Some("my-project"))?;
```

### Wiki Queries

```rust
// Get single page
if let Some(page) = graph.get_wiki_page("wiki/getting-started.md")? {
    println!("Title: {:?}", page.title);
    println!("Wikilinks: {:?}", page.wikilinks);
}

// List pages (optionally filtered by project)
let pages = graph.list_wiki_pages(Some("my-project"))?;

// Find pages that link to a target
let pages = graph.find_pages_by_wikilink("Architecture", Some("my-project"))?;
```

### Graph Navigation

```rust
// Outgoing links from a page
let targets = graph.outgoing_wikilinks(page_id)?;
for target in targets {
    println!("Links to: {} (kind: {})", target.name, target.kind);
}

// Incoming links (backlinks) to a page
let sources = graph.incoming_wikilinks(page_id)?;
for source in sources {
    println!("Linked from: {} (kind: {})", source.name, source.kind);
}
```

### Journal Ingestion

```rust
// Single file
let section_ids = graph.ingest_journal("journal/2024-01-15.md", content, Some("my-project"))?;

// Directory batch
let ids = graph.sync_journal_directory(std::path::Path::new("./journal"), Some("my-project"))?;
```

### Journal Queries

```rust
let sections = graph.query_journal_sections("journal/2024-01-15.md")?;
for section in sections {
    println!("[{}] {}", section.time.unwrap_or_default(), section.title);
    for update in section.kanban_updates {
        println!("  '{}' -> {:?}", update.task_title, update.new_status);
    }
}
```

### Raw SQL Access

For queries not covered by the API, use `with_raw_connection`:

```rust
let count: i64 = graph.with_raw_connection(|conn| {
    conn.query_row(
        "SELECT COUNT(*) FROM wiki_pages WHERE project_id = ?1",
        rusqlite::params!["my-project"],
        |r| r.get(0),
    )
})?;
```

## Wikilink Resolution

When `ingest_wiki_page` encounters `[[Target]]`:

1. Extract all `[[...]]` patterns from body
2. For each target, look for existing `WikiPage` entity:
   - Exact name match
   - Case-insensitive path suffix match (`Dest` matches `wiki/dest.md`)
3. If found: create `RelatedTo` edge from source to target
4. If not found: create stub `WikiPage` entity, then create edge

Stub entities have `{"stub": true, "name": "Target"}` in their data. They become real pages when the actual file is ingested (the entity is updated, not recreated).

## Building Plugins

Atheneum is designed as a library first. The CLI is a thin wrapper around `AtheneumGraph`.

### Rust Plugin

Add to `Cargo.toml`:
```toml
[dependencies]
atheneum = { path = "../atheneum" }
```

Use the API directly:
```rust
use atheneum::{AtheneumGraph, WikiPage};

fn my_plugin(graph: &AtheneumGraph) -> anyhow::Result<Vec<WikiPage>> {
    graph.list_wiki_pages(None)
}
```

### Building Bindings

Atheneum is a Rust library. Python/JS bindings are not yet implemented. The `AtheneumGraph` API is the stable interface; future bindings will wrap it.

## Migrations

Migrations are forward-only and applied automatically on `open` / `open_in_memory`. The migration registry lives in `src/db/mod.rs`:

```rust
const MIGRATIONS: &[(u32, &str, Migration)] = &[
    (1, "execution-domain", execution::migrate_v1_execution),
    (2, "planning-domain", planning::migrate_v2_planning),
    (3, "knowledge-domain", knowledge::migrate_v3_knowledge),
    (4, "evidence-domain", evidence::migrate_v4_evidence),
    (5, "hook-compat", hook_compat::migrate_v5_hook_compat),
    (6, "transcript-imports", transcripts::migrate_v6_transcripts),
    (7, "memory-domain", memory::migrate_v7_memory),
    (8, "planning-archive-fix", planning::migrate_v8_planning_archive_fix),
    (9, "wiki-fts", wiki_fts::migrate_v9_wiki_fts),
    (10, "wiki-fts-path", wiki_fts::migrate_v10_wiki_fts_path),
    (11, "discoveries-session-id", knowledge::migrate_v11_discoveries_session),
    (12, "chat-columns-fts", chat::migrate_v12_chat_columns_fts),
    (13, "discovery-context-snapshot", knowledge::migrate_v13_discovery_context_snapshot),
];
```

To add a new table:
1. Write a migration function in the appropriate domain module
2. Register it in `MIGRATIONS` with the next version number
3. Add query methods to `AtheneumGraph`
4. Add tests

## Operational Hot Spots

Recent Mirage hotspot analysis identifies a small set of complexity-heavy
surfaces that dominate operator and release risk:

| Function | Why It Matters |
|----------|----------------|
| `CrossRouter::open` | Cross-project routing bootstrap and federation |
| `ensure_wiki_fts_healthy` | Self-repair path for wiki FTS corruption |
| `open_wal` | SQLite/WAL open path for the core store |
| `run` / `main` | CLI dispatch and flag parsing surface |

## Edge Types

| EdgeType       | Usage                                      |
|----------------|--------------------------------------------|
| RelatedTo      | Wikilinks, semantic relationships          |
| Created        | Event -> Entity (provenance)               |
| PerformedBy    | Event -> Agent (who did it)                |
| AssignedTo     | Task -> Agent (responsibility)             |
| Called         | Tool call relationships                    |
| CausedBy       | Event -> Event (causal chain)              |
| Modified       | ToolCall -> Entity (modification)          |
| VerifiedBy     | Entity -> Entity (audit/gate verification) |

## Testing

Use `AtheneumGraph::open_in_memory()` for fast, isolated tests:

```rust
#[test]
fn test_wiki_ingest() {
    let graph = AtheneumGraph::open_in_memory().unwrap();
    let id = graph.ingest_wiki_page("test.md", "# Hello", None).unwrap();
    assert!(id > 0);
}
```
