# Plan: Atheneum Memory Store + Native Pi Router

## Governing specs

- Product: `/home/feanor/Projects/atheneum/specs/memory-router/PRODUCT.md`
- Technical: `/home/feanor/Projects/atheneum/specs/memory-router/TECH.md`

## Dispatch policy

All `agy` work goes through ORCA-IDE orchestration. Do not dispatch through direct SSH, `bg_run`, or ad hoc `memrun` commands.

Use:

- Orca CLI shim: `/home/feanor/.config/orca/linux-orca-cli-shim/orca`
- Strix Orca environment for Strix-side work
- One small phase per worker dispatch
- One deliverable per dispatch
- Independent orchestrator verification before accepting each phase

Do not mutate any candidate canonical `/home/feanor/.atheneum/atheneum.db` until the production migration phase is explicitly approved.

## Revision note

The first adversarial spec review (`ctx_3bf3d2c3d506`) returned `REVISE`. The spec was updated to cover host-specific Desktop/Strix baselines, silent memory-overwrite hazards, dual-layer graph requirements, CLI collisions, public ID semantics, project-state history, ingest-time redaction, fixed depth semantics, and smaller implementation phases.

## Phase 0 — Reconcile hosts, binaries, schemas, and snapshots

### Objective

Build the authoritative environment baseline. Do not implement schema changes in this phase.

### Worker inputs

- Desktop source: `/home/feanor/Projects/atheneum`
- Desktop installed CLI: `/home/feanor/.local/bin/atheneum`
- Desktop DB: `/home/feanor/.atheneum/atheneum.db`
- Strix source: `/home/feanor/Projects/atheneum`
- Strix installed CLI: `/home/feanor/.local/bin/atheneum`
- Strix DB: `/home/feanor/.atheneum/atheneum.db`

### Required outputs

- `tasks/atheneum-memory-router/phase0/report.md`
- per-host source HEAD SHA
- per-host installed CLI SHA-256 and version
- per-host built CLI SHA-256 and version after a controlled build
- migration list from source
- per-host DB file size and SHA-256
- per-host graph stats
- live schema extraction for every current table
- live CLI help extraction with command-collision inventory
- durable DB snapshot paths and checksums, not `/tmp`
- explicit recommendation for canonical target DB and merge source DBs

### Gates

- `cargo test` or the smallest relevant Atheneum test suite passes.
- No production or host-canonical DB mutation.
- Report states whether each installed binary matches the tested source build.
- Orchestrator independently recomputes every reported hash/stat before acceptance.

## Phase 1 — Schema migration on disposable DB copy

### Objective

Implement additive migration v15 from `TECH.md` and test it only on disposable DB copies.

### Worker inputs

- Accepted Phase 0 report
- `specs/memory-router/TECH.md`

### Required outputs

- migration code
- fail-first and idempotency tests
- disposable DB copy path
- migration report with before/after checks
- graph backfill report for new/extended records

### Gates

- Migration applies once.
- Migration applies twice with no duplicate schema changes.
- Existing tables/rows remain present.
- Guarded `ALTER TABLE` uses `PRAGMA table_info`.
- No raw production DB edit.
- New user-visible record types have graph entity variants and dual-write plan.
- Existing tests are not weakened.

## Phase 2A — Memory-record API and routing read path

### Objective

Implement canonical memory envelope APIs and safe read routing:

- memory item append/revision/get
- route memory
- expand memory
- conflict-safe update semantics

### Required outputs

- API implementation
- CLI commands `memory-item-upsert`, `memory-item-get`, `memory-route`, `memory-expand`
- JSON examples
- tests for conflict, revision, budget, and expansion behavior

### Gates

- Existing `memory-store`, `memory-get <key>`, and `memory-list` behavior remains unchanged.
- Content-hash collisions never silently overwrite existing content.
- Summary/bundle/expand depth semantics match `TECH.md`.
- Commands emit machine-readable JSON.

## Phase 2B — State/artifact/receipt/source APIs

### Objective

Implement append-only state history plus artifact, receipt, source, and external-ref APIs:

- project state append/head
- agent state append/head
- artifact record
- receipt record
- source record
- external ref link

### Required outputs

- API implementation
- CLI commands
- JSON examples
- dual-write graph entity tests

### Gates

- Public IDs are graph entity IDs.
- Project/agent state updates append versions and preserve history.
- Artifact and receipt foreign keys use graph entity IDs where specified.
- Commands fail closed on invalid records.

## Phase 3 — Ledger extension and import adapters

### Objective

Extend ledger/import architecture for the new memory envelope and create adapters for Hermes Markdown and Bookkeeper export.

### Required outputs

- canonical NDJSON schema
- Hermes adapter
- Bookkeeper adapter
- dry-run import maps
- duplicate replay report
- conflict/revision report

### Gates

- No raw SQLite mutation.
- Re-running import produces zero duplicates.
- Every imported record preserves source URI and content hash.
- Memory hash collisions produce conflict or revision entries, never silent overwrites.
- Embeddings and full transcripts are not copied.
- Secret/sensitive records are rejected, redacted, or excluded from export as specified.

## Phase 4 — Native Pi router package

### Objective

Create `/home/feanor/Projects/pi-memory-router` as a native Pi extension using Atheneum CLI calls.

### Required outputs

- package
- tools:
  - `memory_route`
  - `memory_expand`
  - `project_state`
  - `memory_record`
  - `receipt_record`
- commands:
  - `/memory-route`
  - `/memory-expand`
  - `/project-state`
- tests
- real Pi load probe

### Gates

- No MCP dependency.
- No raw SQLite access.
- Budget limits are enforced.
- Full memory bodies require explicit expansion.
- Missing evidence is labeled.
- Secret/sensitive records are excluded from prompt output.

## Phase 5 — Subagent bundle producer

### Objective

Produce compact Orca/agy task bundles from project memory.

### Required outputs

- bundle schema
- `memory_bundle` tool or CLI command
- example bundle for one real project
- tests

### Gates

- Bundle stays within configured token budget.
- Includes objective, constraints, decisions, blockers, receipts, and memory IDs.
- Reports omitted count.
- Does not include full history.
- Secret/sensitive content is excluded or redacted.

## Phase 6 — Production migration proposal

### Objective

Prepare, but do not execute, production migration.

### Required outputs

- selected canonical DB and merge-source list
- production backup path and checksum
- disposable migration evidence
- dry-run import evidence
- exact command list
- rollback plan
- operator approval request

### Gates

- Human approval required before touching the selected canonical DB.
- Any source/binary/hash/schema mismatch stops the phase.

## Phase 7 — Production migration and router cutover

Only after explicit approval.

### Objective

Apply migration to the selected canonical DB, run imports, install router, and verify Pi behavior.

### Gates

- Canonical DB backup exists.
- Migration and import maps are preserved.
- Post-migration graph stats and samples match expectations.
- Native Pi probe loads router.
- Router answers a real budgeted query.
- Secret/sensitive records remain excluded from output and export.

## Worker prompt template

Use one phase per Orca dispatch. Keep the prompt narrow:

```text
Work only on Phase N from /home/feanor/Projects/atheneum/conductor/tracks/atheneum-memory-router/plan.md.

Read:
- specs/memory-router/PRODUCT.md
- specs/memory-router/TECH.md
- the listed phase inputs

Do not touch any /home/feanor/.atheneum/atheneum.db.
Do not use raw SQLite writes.
Do not weaken existing tests.
Do not create /tmp artifacts.

Deliver:
- the exact files changed
- commands run and outputs
- fail-first evidence where applicable
- a concise report at the required path

Stop and report BLOCKED if any required input is missing or contradictory.
```

## Orchestrator verification checklist

After each worker report:

1. Independently inspect the actual files.
2. Recompute hashes, versions, graph stats, and schema tables claimed by the worker.
3. Re-run the exact verification commands where possible.
4. Check `git diff --exit-code tests/` for unauthorized test changes.
5. Check dual-layer graph visibility for every new record type.
6. Check idempotent replay and conflict behavior.
7. Check that no host-specific path was treated as universal.
8. Check that no `/tmp` artifacts or raw SQLite writes were used.
9. Record acceptance or rejection with a durable receipt.
