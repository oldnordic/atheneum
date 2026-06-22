# Atheneum

Embedded graph database for agent coordination — episodic memory, knowledge persistence, and session accountability across coding sessions.

Part of the **grounded-coding ecosystem**.

## Technical Architecture

**Role** — persistence layer for coding agents: what was discovered, what sessions did, what tasks exist, what knowledge is worth keeping.

**Data model**
- Episodic memory — sessions, events (tool calls), discoveries, tasks, handoffs
- Evidence + decision chains via `caused_by` / `led_to` links
- FTS5 wiki page index (search, sync, backfill, auto-repair)

**Coordination surface**
- Session accountability — `session-trace`, `tool-usage`, `sessions-recent`
- Cross-agent activity — `discoveries-recent`, `handoffs-recent`, `events-recent`
- Token-bounded session digests (bootstrap packets for new sessions)
- Decision-chain walks — `thread` with depth + token budget

**Interfaces**
- Rust library (`AtheneumGraph`) + CLI
- MCP server (`atheneum-mcp`)
- Exposed via [agent-envoy](https://crates.io/crates/agent-envoy) HTTP endpoints

**Keyword index:** episodic memory · knowledge graph · agent coordination · session accountability · decision chains · handoff protocol · token-bounded digests · MCP · FTS5 · SQLiteGraph · Rust

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

## Operator Queries

The CLI now covers the common observability reads that previously pushed operators toward raw SQLite:

```bash
# Inspect one session with recent events and tool calls
atheneum session-trace ~/.magellan/atheneum/atheneum.db --session <session-id> --limit 10

# Aggregate tool usage for one session
atheneum tool-usage ~/.magellan/atheneum/atheneum.db --session <session-id> --limit 20

# Review recent cross-agent activity
atheneum discoveries-recent ~/.magellan/atheneum/atheneum.db --project envoy --limit 10
atheneum handoffs-recent ~/.magellan/atheneum/atheneum.db --agent claude4 --limit 10
atheneum events-recent ~/.magellan/atheneum/atheneum.db --type tool_call --limit 10
atheneum sessions-recent ~/.magellan/atheneum/atheneum.db --project envoy --limit 10

# Bounded bootstrap packet so a new session grounds on prior work
atheneum session-digest ~/.magellan/atheneum/atheneum.db --project envoy --last 3 --tokens 500

# Walk a decision chain — discoveries linked by caused_by/led_to per session
atheneum thread ~/.magellan/atheneum/atheneum.db "adopt graph chain" --depth 3 --tokens 1500
```

## License

GPL-3.0-only — see [LICENSE](./LICENSE).
