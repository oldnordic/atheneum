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
- Independent verification before accepting each phase

Do not mutate `/home/feanor/.atheneum/atheneum.db` until the production migration phase is explicitly approved.

## Phase 0 — Reconcile source, binary, and snapshots

### Objective

Prove which source commit corresponds to the installed Atheneum CLI and create safe snapshots for migration testing.

### Worker inputs

- Atheneum source: `/home/feanor/Projects/atheneum`
- Installed CLI: `/home/feanor/.local/bin/atheneum`
- Target release candidate: `/home/feanor/Projects/atheneum/target/release/atheneum`
- Canonical DB: `/home/feanor/.atheneum/atheneum.db`

### Required outputs

- `tasks/atheneum-memory-router/phase0/report.md`
- source HEAD SHA
- installed CLI SHA-256
- built CLI SHA-256
- version output
- migration list from source
- DB file size and SHA-256
- snapshot path created under durable scratch storage, not `/tmp`

### Gates

- `cargo test` or the smallest relevant Atheneum test suite passes.
- No production DB mutation.
- Report states whether installed binary matches current source.

## Phase 1 — Schema migration on disposable DB copy

### Objective

Implement additive migration v15 from `TECH.md` and test it only on disposable DB copies.

### Worker inputs

- Phase 0 report
- `specs/memory-router/TECH.md`

### Required outputs

- migration code
- tests
- disposable DB copy path
- migration report with before/after checks

### Gates

- Migration applies once.
- Migration applies twice with no duplicate schema changes.
- Existing tables/rows remain present.
- No raw production DB edit.
- `git diff --exit-code tests/` only if tests were intentionally added by the worker; existing tests must not be weakened.

## Phase 2 — Canonical record API and CLI

### Objective

Add public store APIs and JSON CLI commands for the new schema:

- memory item upsert/get
- project state get/upsert
- artifact record
- receipt record
- external ref link

### Required outputs

- API implementation
- CLI implementation
- JSON examples
- tests

### Gates

- Commands fail closed on invalid records.
- Commands emit machine-readable JSON.
- Existing `memory-store`, `task-*`, and ledger behavior remains unchanged.

## Phase 3 — Ledger extension and import adapters

### Objective

Extend ledger/import architecture for the new memory envelope and create adapters for Hermes Markdown and Bookkeeper export.

### Required outputs

- canonical NDJSON schema
- Hermes adapter
- Bookkeeper adapter
- dry-run import maps
- duplicate replay report

### Gates

- No raw SQLite mutation.
- Re-running import produces zero duplicates.
- Every imported record preserves source URI and content hash.
- Embeddings and full transcripts are not copied.

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

## Phase 6 — Production migration proposal

### Objective

Prepare, but do not execute, production migration.

### Required outputs

- production backup path and checksum
- disposable migration evidence
- dry-run import evidence
- exact command list
- rollback plan
- operator approval request

### Gates

- Human approval required before touching canonical DB.
- Any mismatch stops the phase.

## Phase 7 — Production migration and router cutover

Only after explicit approval.

### Objective

Apply migration to canonical DB, run imports, install router, and verify Pi behavior.

### Gates

- Canonical DB backup exists.
- Migration and import maps are preserved.
- Post-migration graph stats and samples match expectations.
- Native Pi probe loads router.
- Router answers a real budgeted query.

## Worker prompt template

Use one phase per Orca dispatch. Keep the prompt narrow:

```text
Work only on Phase N from /home/feanor/Projects/atheneum/conductor/tracks/atheneum-memory-router/plan.md.

Read:
- specs/memory-router/PRODUCT.md
- specs/memory-router/TECH.md
- the listed phase inputs

Do not touch production /home/feanor/.atheneum/atheneum.db.
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

## Independent verification checklist

After each worker report:

1. Inspect the actual files.
2. Re-run the decisive tests.
3. Check no production DB mutation occurred.
4. Check no `/tmp` artifacts were used.
5. Check report claims against command output.
6. Accept only if the evidence is reproducible.
