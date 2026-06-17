# Atheneum

Embedded graph database for agent coordination — episodic memory, knowledge persistence, and session accountability across coding sessions.

Part of the **grounded-coding ecosystem**.

## Crates

| Crate | Description | Version |
|-------|-------------|---------|
| [`atheneum`](./crates/atheneum) | Core library + CLI | [![Crates.io](https://img.shields.io/crates/v/atheneum)](https://crates.io/crates/atheneum) |
| [`atheneum-mcp`](./crates/atheneum-mcp) | MCP server (optional) | — |

## Quick Install

```bash
cargo add atheneum
```

For the CLI binary:

```bash
cargo install atheneum
```

## Documentation

- [Crate README](./crates/atheneum/README.md)
- [Manual](./crates/atheneum/MANUAL.md)
- [API Reference](./crates/atheneum/API.md)
- [CHANGELOG](./crates/atheneum/CHANGELOG.md)

## Resilience

`AtheneumGraph::open()` automatically repairs a corrupt `wiki_pages_fts` FTS5 index if an external SQLite writer left the shadow tables inconsistent, so `sync-wiki`, `search-wiki`, and `backfill-wiki` keep working without manual intervention.

## License

GPL-3.0-only — see [LICENSE](./LICENSE).
