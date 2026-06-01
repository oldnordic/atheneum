# Atheneum

Agent coordination graph database — episodic and semantic memory for multi-agent workflows.

Atheneum persists knowledge, discoveries, and task handoffs across agent sessions using an embedded SQLite graph database (via [sqlitegraph](https://crates.io/crates/sqlitegraph)).

## Quickstart

```rust
use atheneum::AtheneumGraph;
use std::path::Path;

// In-memory (ephemeral)
let graph = AtheneumGraph::open_in_memory()?;

// Persistent
let graph = AtheneumGraph::open(Path::new("atheneum.db"))?;

// Ingest a wiki article with frontmatter
let content = r#"---
title: "My Note"
type: concept
---

# My Note

See also [[Related Concept]].
"#;
let id = graph.ingest_wiki_page("my-note.md", &content, None)?;
```

## Features

- **Graph storage** — Nodes (`Knowledge`, `Agent`, `Task`, `Event`) and typed edges (`Created`, `RelatedTo`, `AssignedTo`, `BlockedBy`, ...).
- **Wiki ingestion** — Parse Markdown frontmatter, extract `[[wikilinks]]`, create stub entities for missing targets.
- **Journal sections** — Parse `## HH:MM | Title` headers and Kanban updates (`"Task" -> TODO`).
- **Kanban tracking** — `TODO → IN_PROGRESS → DONE/BLOCKED` with `update_task_status`.
- **Discovery & handoffs** — Store agent discoveries and async task handoffs with token-savings estimates.
- **Ontology** — Register `Class`/`Property` schemas for typed agent reasoning.
- **Search** — Full-text search over wiki pages, semantic vector search via HNSW (sqlitegraph).
- **Project scoping** — All entities and queries support an optional `project_id` namespace.

## CLI

```bash
cargo run --bin atheneum -- sync-wiki   <db> <dir> [project]
cargo run --bin atheneum -- sync-journal <db> <dir> [project]
cargo run --bin atheneum -- query-wiki   <db> <path>
cargo run --bin atheneum -- query-journal <db> <path>
```

## Requirements

- Rust 1.80+
- SQLite 3.35+ (with JSON1 extension)

## License

GPL-3.0-only
