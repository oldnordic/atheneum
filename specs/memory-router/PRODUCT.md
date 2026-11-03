# PRODUCT: Atheneum Memory Store and Native Router

## Problem

The current Pi/agent memory stack has several stores but no single, safe, budgeted way for the main agent and subagents to know project state:

- Atheneum stores graph entities, tasks, discoveries, wiki pages, sessions, evidence runs, and simple memories.
- Pi Hermes Memory stores user/project memories, failure lessons, skills, and session summaries in Markdown plus SQLite.
- Memory Bookkeeper stores evidence-bound records and transcript citations with embeddings.
- Atheneum MCP exposes some graph tools, but MCP is not the desired memory interface for Pi agents.
- Current bootstrap/search behavior can inject broad context, but it does not give agents a canonical state envelope or a disciplined way to ask for exactly the memory kinds they need.

Result: agents can miss the current project state, re-read full history, duplicate memories, or treat stale/unverified memory as fact.

## Goal

Make Atheneum the canonical coordination memory store and add a native Pi memory router that gives agents compact, budgeted, evidence-linked memory answers without MCP and without full-context injection.

The system must preserve existing data. Migration and merge work must be additive, replayable, and independently verifiable.

## Users

- Main Pi agent working in a repository.
- Pi subagents that need a compact project/task brief.
- Orca-dispatched workers such as `agy`, which must receive a small state bundle rather than full history.
- Human operator auditing what the agents were told.

## Current-state findings

Verified read-only inspection on 2026-11-03:

- Installed CLI: `/home/feanor/.local/bin/atheneum`, version `v0.14.0`.
- Canonical DB: `/home/feanor/.atheneum/atheneum.db`, about 3.0 GB.
- Current graph size: 1,062,379 entities and 3,493,232 edges from `atheneum graph-stats`.
- Current source supports forward-only migrations through version 14 (`crates/atheneum/src/db/mod.rs`).
- Current SQL domains include:
  - execution: `agents`, `reasoning_logs`, `tool_calls`
  - planning: `tasks`, `requirements`, `blockers`
  - knowledge: `discoveries`, `wiki_pages`, `journal_sections`
  - evidence: `sessions`, `commits`, `test_runs`, `bench_runs`, `releases`, `fix_chains`, `event_log`
  - memory: `memory_entries`
  - grounded claims: `grounded_claims`
- `memory_entries` currently has only `key`, `scope`, `content`, `confidence`, `project_id`, `created_at`, and `updated_at` (`src/db/memory.rs`).
- Atheneum already has ledger export/import for `Discovery`, `Memory`, and `Task`, using NDJSON and normal store paths rather than raw SQL (`src/graph/ledger.rs`).
- `pi-atheneum` is a native Pi extension that correctly uses the Atheneum CLI rather than MCP or raw SQLite, but it only wraps search, memory get/store, discovery storage, bootstrap, and dream.
- Memory Bookkeeper health passed with 429 active evidence-bound records and EmbeddingGemma-2 256-dimensional CPU embeddings.
- Pi Hermes Memory is installed and searchable; its storage includes `~/.pi/agent/pi-hermes-memory/sessions.db` plus Markdown memory trees.

## Required behavior

### 1. Canonical memory envelope

Atheneum must support a typed memory envelope with at least:

- stable ID
- kind
- project
- title
- summary
- full content
- status
- confidence
- created_at / updated_at
- optional valid_from / valid_to
- optional TTL
- evidence references
- source URI
- content hash
- token estimate
- sensitivity label
- supersedes / superseded_by linkage

Required kinds:

- `project_state`
- `decision`
- `task_state`
- `article`
- `source`
- `artifact`
- `receipt`
- `blocker`
- `agent_state`
- `lesson`
- `preference`

### 2. Project state head

Each project must have an explicit, bounded state head containing:

- current objective
- active tracks/tasks
- current branch/head where known
- constraints
- open blockers
- recent verified decisions
- recent receipts/artifacts
- what changed since the last state update
- source references for every consequential claim

The router must be able to return this state head under a strict token budget.

### 3. Budgeted query routing

Agents must be able to ask for memory by:

- query text
- project
- memory kind
- status
- time range
- confidence floor
- evidence requirement
- token budget
- result depth: `summary`, `bundle`, or `expand`

A query returns compact candidates first. Full bodies are fetched only through explicit expansion of selected IDs.

### 4. Evidence policy

Memory is advisory, not proof.

- `decision`, `receipt`, `artifact`, and `task_state` records should carry evidence references whenever consequential.
- The router must label records with missing evidence.
- Fresh repository state, commands, tests, and receipts override memory.
- Secret-bearing or sensitive records must be excluded or redacted before prompt injection.

### 5. No-loss merge behavior

Merging Atheneum, Hermes, and Bookkeeper data must:

- begin from durable backups/snapshots and checksums
- use export files that can be replayed
- use dry-run import before any real import
- use idempotent deduplication keys
- preserve source identity and source URI
- record an import map for every imported/skipped/failed record
- never merge by directly editing SQLite rows

Initial merge scope:

1. Atheneum ledger kinds: discoveries, memories, tasks.
2. Hermes Markdown memory and selected session summaries through an adapter.
3. Bookkeeper records through an adapter that preserves citations and source URIs.

Raw transcript bodies and embeddings are not copied into Atheneum in the first migration. Atheneum stores references to them.

### 6. Native Pi router

The router must be a native Pi package, not an MCP server.

Initial tools:

- `memory_route`: budgeted candidate search
- `memory_expand`: expand selected IDs
- `project_state`: return the current state head
- `memory_record`: store or update canonical memory
- `receipt_record`: store verification/artifact receipt

The router may invoke local CLIs such as `atheneum` and `memory-bookkeeper`, but must not expose raw SQL access to the model.

### 7. Subagent bundle

For Orca/agy dispatch, the router must produce a compact task bundle:

- task objective
- constraints
- relevant decisions
- current blockers
- required receipts
- selected memory IDs
- token budget and actual token estimate
- omitted-count note

Workers receive this bundle; they do not receive full history.

## Non-goals

- Do not delete or rewrite existing memories as part of migration.
- Do not replace Bookkeeper embeddings in phase 1.
- Do not copy full transcripts into Atheneum.
- Do not make memory authoritative over fresh verification.
- Do not force all provider traffic through JSON mode.
- Do not build an MCP replacement server; use a native Pi extension.

## Failure behavior

- Missing Atheneum CLI or DB: router reports unavailable; it does not fabricate memory.
- Malformed import record: record is skipped and logged in the import map.
- Migration failure: transaction rolls back and open fails loudly.
- Budget exceeded: router truncates candidates and reports omitted count.
- Missing evidence: record is labeled `evidence_missing`, not silently trusted.

## Acceptance criteria

1. Source inspection documents every existing table/domain and the new additive fields.
2. Migration applies cleanly to a disposable copy of the production DB.
3. Migration is idempotent and records a schema version.
4. Dry-run import produces deterministic counts and an import map.
5. Re-running dry-run or real import produces no duplicates.
6. Router returns budgeted summaries with expandable IDs.
7. Router never injects full memory bodies without explicit expansion.
8. Native Pi load probe shows the router tools and commands without MCP.
9. All new behavior has fail-first and bipolar tests where applicable.
