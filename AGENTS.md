# Atheneum Agent Bridge

Documentation for the multi-agent knowledge bridge. This is the contract between the 4-agent cluster (Claude1, Claude2, Codex, Hermes) and the Atheneum shared graph.

## What This Is

Atheneum is a shared sqlitegraph database that persists across agent sessions. The Envoy binary (with `--features atheneum`) exposes 5 HTTP endpoints that the Hermes plugin maps to native tools.

Unlike messages (ephemeral, per-session), atheneum entities:
- Persist across agent restarts
- Are queryable by `target` (e.g., `http_handler`, `odincode`, `magellan`)
- Have timestamps and agent attribution
- Provide token savings estimates via deduplication

## Tools (Hermes Plugin)

### `envoy_knowledge(target)`
Query aggregated knowledge for a target.

Returns: `Discovery` count, `Handoff` count, `total_entities`, `token_savings`, `queried_at`.

Example:
```
{"target": "http_handler",
 "discovery_count": 2,
 "handoff_count": 0,
 "total_entities": 24,
 "token_savings": {"saved": 0, "percentage_reduction": 0.0}}
```

### `envoy_store_discovery(agent, discovery_type, target, metadata)`
Store a discovery into the shared graph.

Parameters:
- `agent`: who made the discovery (default: `hermes`)
- `discovery_type`: free-form string tag (e.g. `token_reduction`, `bug_found`)
- `target`: what entity/area this applies to
- `metadata`: arbitrary JSON dict

Returns: `discovery_id`

### `envoy_store_handoff(from_agent, to_agent, manifest)`
Create a pending task handoff.

Parameters:
- `from_agent`: sender
- `to_agent`: recipient
- `manifest`: string (auto-wrapped to `{"body": "..."}`) or arbitrary JSON dict

Returns: `handoff_id`

### `envoy_pending_handoff(agent)`
Get pending handoffs for an agent.

Returns: `{"handoff": null}` if none, or full handoff object with manifest.

### `envoy_claim_handoff(handoff_id)`
Claim a pending handoff by ID. After claim, it disappears from `pending_handoff`.

Returns: `{"claimed": true, "handoff_id": N}`

## API Contract (Envoy HTTP)

| Method | Path | Body | Query |
|--------|------|------|-------|
| GET  | `/health` | — | — |
| GET  | `/agents` | — | — |
| POST | `/agents` | `{name, kind, parent_id?}` | — |
| POST | `/heartbeat` | `{agent_id, status}` | — |
| POST | `/messages` | `{type, from, to, parts, task_id?, context_id?}` | — |
| GET  | `/messages` | — | `to, since, limit` |
| GET  | `/atheneum/knowledge` | — | `target` |
| POST | `/atheneum/discoveries` | `{agent, discovery_type, target, metadata}` | — |
| POST | `/atheneum/handoffs` | `{from_agent, to_agent, manifest}` | — |
| GET  | `/atheneum/handoffs/pending` | — | `agent` |
| POST | `/atheneum/handoffs/{id}/claim` | `{}` | — |

**Important:** Envoy uses server-assigned `agent_id` (e.g. `id1`) internally, NOT agent names. The plugin handles name→ID resolution automatically. When registering agents, use `name="hermes"`, `name="claude1"`, `name="claude2"`, `name="codex"`. Envoy assigns `agent_id`.

## What This Enables (Beyond Messaging)

### 1. Discovery Sharing
Claude1 finds a bug in odincode wiring → calls `envoy_store_discovery`.
Hermes later queries `envoy_knowledge(target="odincode")` → sees it, avoids re-investigation.

### 2. Async Task Handoffs
Hermes creates handoff for Codex → goes offline.
Codex queries `envoy_pending_handoff(agent="codex")` 2 hours later → picks up task.
No direct message exchange needed.

### 3. Persistent Project Memory
Every discovery and handoff is a node in `~/Projects/atheneum/atheneum.db`.
The graph survives agent restarts (verified: `total_entities` persisted through `systemctl restart envoy`).

### 4. Token Savings Estimation
The knowledge response includes `token_savings.saved` and `percentage_reduction`.
This measures the value of deduplication: unique agents × estimated file tokens.

## Configuration

### Systemd Service
File: `~/.config/systemd/user/envoy.service`

```ini
[Service]
Environment=ENVOY_DB=/home/feanor/.local/share/envoy/agents.db
Environment=ATHENEUM_DB=/home/feanor/Projects/atheneum/atheneum.db
ExecStart=/home/feanor/Projects/envoy/target/release/envoy serve --port 9876
```

**Critical:** `ATHENEUM_DB` must point to the actual atheneum DB. If unset or wrong, atheneum endpoints return 500 "atheneum not configured".

### Hermes Plugin
File: `~/.hermes/plugins/envoy-coordination/__init__.py`

Plugin registers all 11 tools (6 messaging + 5 atheneum). Enable in `~/.hermes/config.yaml`:
```yaml
plugins:
  envoy-coordination:
    plugin: envoy-coordination
```

### Env Vars (for plugin, not systemd)
- `ENVOY_URL=http://127.0.0.1:9876`
- `ENVOY_AGENT_NAME=hermes`
- `ENVOY_AGENT_KIND=coordinator`

## Known Issues & Fixes

### "atheneum not configured" (HTTP 500)
Cause: `ATHENEUM_DB` env var missing or pointing to nonexistent path. Envoy binary checks `std::env::var("ATHENEUM_DB")` at startup.
Fix: Set in systemd service, reload daemon, restart service.

### "missing field discovery_type" (HTTP 422)
Cause: Plugin sends `type` instead of `discovery_type`. The Rust struct `StoreDiscoveryRequest` has field `discovery_type`.
Fix: The plugin code is correct — this only happens in manual curl tests.

### Plugin not loaded after update
Cause: Hermes plugins are imported once at startup.
Fix: `hermes plugins enable envoy-coordination`, then `/reset` or start a new Hermes session.

## Source of Truth
- Envoy handlers: `envoy/src/http.rs` (lines 265–340, 690+)
- Envoy server setup: `envoy/src/server.rs`
- Envoy main: `envoy/src/main.rs`
- Plugin: `~/.hermes/plugins/envoy-coordination/__init__.py`
- This docs file: `atheneum/AGENTS.md`

## Verification Steps (Post-Restart)

Run these plugin tools after any restart:
1. `envoy_knowledge(target="test_target")` — should return `total_entities > 0`
2. `envoy_store_discovery(...)` — should return `discovery_id`
3. `envoy_store_handoff(to_agent="claude1", manifest="...")` — should return `handoff_id`
4. `envoy_pending_handoff(agent="claude1")` — should list the handoff
5. `envoy_claim_handoff(handoff_id=N)` — should return `claimed: true`
6. `envoy_pending_handoff(agent="claude1")` — should return `handoff: null` (claimed)

If any step fails, check:
- Envoy systemd status: `systemctl --user status envoy`
- `ATHENEUM_DB` env var in service file
- Plugin state: `~/.hermes/.envoy_state.json`
