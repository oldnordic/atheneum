# Ledger Reconciliation Spec — merge legacy atheneum store into canon (pre-registered)

Date: 2026-08-05. Status: registered BEFORE implementation. Bars below are fixed;
report whatever the run produces. INCOMPLETE with state beats fabrication.

## Context (verified facts, not recollection)

- Split-brain incident (canon discovery 770610): two live atheneum memory stores
  diverged for weeks.
  - Canon: `~/.atheneum/atheneum.db` (8.8GB) — daemon-configured
    (`~/.config/atheneum/config.toml`), held open by envoy / atheneum /
    atheneum-mcp. THIS IS THE LIVE TARGET; daemons keep running throughout.
  - Legacy: `~/.magellan/atheneum/atheneum.db` (8.4GB) — holds the agent-written
    ledger (discoveries back to at least 2026-07-21; discovery id floor observed
    707332; true volume unknown — measure it in Phase 2).
- Already merged (2026-08-05, CLI loop, verified): 50 discoveries with
  timestamp >= 2026-08-02, canon ids ~7705xx-7706xx.
- User ruling: `~/.atheneum/` = memory stores; `~/.magellan/<project>/` = code
  indexes only. After the merge, `~/.magellan/atheneum/` gets the real atheneum
  code index.
- Known CLI bug to fix first (probe-verified 2026-08-05):
  `atheneum store-discovery --dedup` does NOT dedup — identical content stored
  3x produced 3 discoveries. The import path needs working dedup.

## Hard constraints

- NEVER stop/restart/pkill envoy or the atheneum daemons. Canon stays live.
  All tooling must tolerate a live WAL target (busy timeout + retry).
- Import must go through the SAME entity-creation code paths as
  `store-discovery` / `memory-store` / `task-create` (edges, events, FTS
  invariants). No raw SQL inserts into canon.
- Raw sqlite3 is allowed ONLY for the `.backup` snapshot (DB administration),
  never for reading/writing entities.

## Phase 0 — fix `--dedup`

Make `store-discovery --dedup` actually skip when a discovery with the same
(agent, discovery_type, target, content_hash) already exists. Regression test:
store identical payload twice with --dedup → second call reports existing id,
store count unchanged. Bars: new unit test passes; full `cargo test` green.

## Phase 1 — tooling (new CLI subcommands)

1. `atheneum export-ledger <db> [--until <rfc3339>] [--kinds discoveries,memories,tasks]`
   → NDJSON, one record per line, full fidelity: kind, agent, discovery_type,
   target, project_id, metadata body, content_hash, created timestamp.
2. `atheneum import-ledger <db> <file.ndjson> [--dry-run]`
   → inserts each record through the normal store paths, skipping any record
   whose (kind, agent, target, content_hash) already exists in the target.
   Prints exact counts: merged / skipped / failed, and writes a per-record
   map file (old content_hash → new id) for audit.

Bars: `--dry-run` on a real export reports counts without mutating the target
(verify: target file mtime unchanged); round-trip test on a scratch DB pair
(export 3 records, import into empty DB, re-export, content_hash sets equal).

## Phase 2 — the merge run (exact commands, report raw output)

1. `sqlite3 ~/.atheneum/atheneum.db ".backup ~/.atheneum/atheneum.db.bak.pre-reconciliation-20260805"`
   — verify the backup exists and is non-trivial (`ls -l`).
2. Measure legacy volume: `atheneum export-ledger ~/.magellan/atheneum/atheneum.db
   --until 2026-08-02T00:00:00Z` → record line count per kind.
   (The `--until` boundary is load-bearing: Aug 2+ is already merged, and the
   merged copies have amended source strings, so hash-dedup would NOT catch
   them. Do not export the already-merged window.)
3. Dry-run import into canon → report counts.
4. Real import into canon → report counts. Bar: failed = 0.
5. Post-merge verification (all must pass):
   - 10 randomly chosen legacy discoveries from July are retrievable in canon
     via `atheneum search` as Discovery entities (quote the queries).
   - 10 of the skipped records confirmed already-present in canon by name.
   - `curl -sf http://127.0.0.1:9876/health` OK after the run.
   - canon discovery total delta == merged count (state both counts).

## Phase 3 — retire the legacy path, build the real code index

1. `mkdir ~/.magellan/atheneum/legacy-memory-store-2026-08-05` and MOVE the
   legacy atheneum.db (plus its -shm/-wal) there. Do not delete anything.
2. Build the code index: `magellan watch --root ./crates --db
   ~/.magellan/atheneum/atheneum.db --scan-initial` from this repo.
   Bar: `magellan find --db ~/.magellan/atheneum/atheneum.db --name
   store_discovery` resolves to the CLI handler; `magellan status` file count
   is in the hundreds, not zero.

## Gates

- `cargo test` green; full-path clippy
  (`~/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/cargo-clippy
  clippy --all-targets -- -D warnings`) clean.
- No banned patterns (todo!/unimplemented!/#[allow]/TODO comments).
- Flow: branch `lab/ledger-reconciliation` → verify → merge master → push →
  reinstall `~/.local/bin/atheneum`. Commit prefix `feat(ledger)`.

## Contract

End the reply with REAL output of: the dry-run counts, the real-import counts,
the 10 July retrieval probes, the canon delta numbers, `git log --oneline -3`,
and `ls -l` of the backup. Any bar that fails is reported as FAIL with the raw
output — no narrative fixes.
