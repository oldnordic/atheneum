# Atheneum Migration Gate: Opt-In Migrations & Auto-Migrate Prevention Report

## 1. Executive Summary & Root Cause

**Incident**: On 2026-11-03, installing a fresh v15 binary to `~/.local/bin` auto-applied migration v15 to the live production database on first open, bypassing the mandatory approval gate.
**Root Cause**: Previously in `crates/atheneum/src/graph/mod.rs`, `AtheneumGraph::open()` unconditionally invoked `run_startup_migrations()` on every open, which automatically iterated through all pending migrations in `MIGRATIONS` and committed them to SQLite.
**Resolution**:
1. Opening an existing database with pending migrations now checks schema version prior to any mutations or repairs. If migrations are pending and not explicitly requested, `AtheneumGraph::open` immediately aborts with a loud error naming the current version, pending version(s), and the exact command to migrate.
2. Opening a database whose schema version is newer than supported by the binary aborts immediately with a clear error.
3. Migrations only apply when explicitly requested via either the dedicated `atheneum migrate <db-path>` subcommand or the global `--migrate` CLI flag (or `ATHENEUM_MIGRATE=1` environment variable).
4. Fresh database creation (`atheneum init <new-path>` or `open_in_memory()`) creates the full current schema directly without treating it as a migration.
5. All read-only commands on an up-to-date database proceed unchanged with zero schema modifications.

---

## 2. Compatibility Audit

Every site where a database is opened across the codebase was audited:

| Component / File | Open Method | Migration Gate Behavior | Status |
| :--- | :--- | :--- | :--- |
| `crates/atheneum/src/graph/mod.rs` | `AtheneumGraph::open` | Read-only schema probe before graph init. Fails loudly if pending migrations or newer schema. Zero silent mutation. | Verified |
| `crates/atheneum/src/graph/mod.rs` | `AtheneumGraph::open_with_migrate` | Explicitly permits applying pending migrations to up-to-date version. | Verified |
| `crates/atheneum/src/graph/mod.rs` | `AtheneumGraph::migrate` | Dedicated migration executor for `atheneum migrate <path>`. Fails loudly on newer schema. Returns applied version list. | Verified |
| `crates/atheneum/src/graph/mod.rs` | `AtheneumGraph::open_in_memory` | In-memory fresh database creation; initializes full schema up to current head. | Verified |
| `crates/atheneum/src/cli/dispatch.rs` | `atheneum <subcommand>` (77 entry points) | Global `--migrate` flag is extracted at CLI entry point, activating `set_global_allow_migrate(true)`. Without `--migrate`, all 77 commands fail loudly on pending migrations. | Verified |
| `crates/atheneum/src/cli/dispatch.rs` | `atheneum migrate <db-path>` | Dedicated migration subcommand. Safely applies migrations and reports status. | Verified |
| `crates/atheneum/src/cli/dispatch.rs` | `atheneum init <db-path>` | Fresh DB initialization creates full current schema (v15) with `Health: OK`. | Verified |
| `crates/atheneum-mcp/src/main.rs` | `AtheneumGraph::open(&path)` | Uses standard `AtheneumGraph::open`. Fails loudly if live DB has pending migrations; does NOT auto-migrate. Supports `ATHENEUM_MIGRATE=1` if explicit migration is configured. | Verified |
| `crates/atheneum/src/db/wiki_fts.rs` | `ensure_wiki_fts_healthy(path)` | Deferred until AFTER migration gate validation so corrupted shadow table repair never mutates an unmigrated DB. | Verified |
| `crates/atheneum/src/graph/extract_decisions.rs` | `AtheneumGraph::open(&config.db)` | Standard open path; guarded by migration gate. | Verified |
| `crates/atheneum/src/meta.rs` | `rusqlite::Connection::open(&path)` | Meta-router SQLite registry (`~/.atheneum/meta.db`). Separate independent schema, not part of `atheneum.db`. | Verified |
| `crates/atheneum/src/graph/magellan_bridge.rs` | `Connection::open_with_flags(..., SQLITE_OPEN_READ_ONLY)` | Read-only connection to external Magellan index database. | Verified |
| `crates/atheneum/src/db/{claims, execution, knowledge, evidence, memory, memory_router, planning, transcripts, chat}.rs` | Transaction / connection arguments | All payload layer modules accept transactions/connections from `AtheneumGraph`; none open SQLite connections directly. | Verified |

---

## 3. Fail-First Log (`tasks/atheneum-migrate-gate/fail-first.log`)

Prior to implementing the gate, `crates/atheneum/tests/migrate_gate_tests.rs` was executed. The test fixture applied migrations 1..=14:
- `(A)`: Reading the v14 fixture database failed because the unpatched binary silently auto-migrated to v15 and returned success (status 0).
- `(newer-schema)`: Reading a database with `PRAGMA user_version = 99` failed because the unpatched binary ignored schema version and opened the database.
- `(--migrate / subcommand)`: Failed because `--migrate` and `migrate` were unknown CLI commands.

```
   Compiling atheneum v0.15.0 (/home/feanor/orca/workspaces/atheneum/Atheneum-Migrate-Gate/crates/atheneum)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.42s
     Running tests/migrate_gate_tests.rs (target/debug/deps/migrate_gate_tests-2d95e29a68810b18)

running 4 tests
test test_fresh_init_creates_full_current_schema ... ok
test test_global_migrate_flag_allows_migration_during_command ... FAILED
test test_newer_schema_fails_loudly ... FAILED
test test_pending_migrations_gate_fails_loudly_on_read_and_succeeds_after_migrate ... FAILED

failures:

---- test_global_migrate_flag_allows_migration_during_command stdout ----
Unknown command: --migrate

---- test_newer_schema_fails_loudly stdout ----
thread 'test_newer_schema_fails_loudly' panicked at crates/atheneum/tests/migrate_gate_tests.rs:211:5:
Read on DB with future schema must fail loudly! Output: { "edge_counts": [], ... }

---- test_pending_migrations_gate_fails_loudly_on_read_and_succeeds_after_migrate stdout ----
thread 'test_pending_migrations_gate_fails_loudly_on_read_and_succeeds_after_migrate' panicked at crates/atheneum/tests/migrate_gate_tests.rs:61:5:
Read command on a DB with pending migrations MUST fail! Output was: { "edge_counts": [], ... }

failures:
    test_global_migrate_flag_allows_migration_during_command
    test_newer_schema_fails_loudly
    test_pending_migrations_gate_fails_loudly_on_read_and_succeeds_after_migrate

test result: FAILED. 1 passed; 3 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.44s
```

---

## 4. Passing Log (`tasks/atheneum-migrate-gate/tests-pass.log`)

After implementing the migration gate, CLI `migrate` subcommand, and `--migrate` option handling:

```
   Compiling atheneum v0.15.0 (/home/feanor/orca/workspaces/atheneum/Atheneum-Migrate-Gate/crates/atheneum)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 1.47s
     Running tests/migrate_gate_tests.rs (target/debug/deps/migrate_gate_tests-2d95e29a68810b18)

running 4 tests
test test_fresh_init_creates_full_current_schema ... ok
test test_global_migrate_flag_allows_migration_during_command ... ok
test test_newer_schema_fails_loudly ... ok
test test_pending_migrations_gate_fails_loudly_on_read_and_succeeds_after_migrate ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.45s
```

Library unit tests:
```
test result: ok. 152 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.11s
```

---

## 5. Manual Bipolar Proof Against Real Snapshot

Using a copy of the real Strix phase0 snapshot (`/home/feanor/scratch/dispatch/atheneum-memory-router/phase0/strix/atheneum.db.snapshot`, schema version 13) copied to `tasks/atheneum-migrate-gate/scratch/strix-probe.db` (original snapshot was strictly never mutated):

### 5.1 Probe 1: Cold Read Fails Loudly Without Migrating
```bash
$ ./target/debug/atheneum graph-stats tasks/atheneum-migrate-gate/scratch/strix-probe.db
Error: database schema is out of date: current schema version is 13, pending migration(s): [14, 15]. Run `atheneum migrate /home/feanor/orca/workspaces/atheneum/Atheneum-Migrate-Gate/tasks/atheneum-migrate-gate/scratch/strix-probe.db` (or pass --migrate) to apply migrations.
$ echo $?
1
```
Verifying zero silent mutation:
```bash
$ sqlite3 tasks/atheneum-migrate-gate/scratch/strix-probe.db "SELECT COALESCE(MAX(version), 0) FROM atheneum_schema_version; PRAGMA user_version;"
13
0
```

### 5.2 Probe 2: Explicit Migration via `atheneum migrate`
```bash
$ ./target/debug/atheneum migrate tasks/atheneum-migrate-gate/scratch/strix-probe.db
Successfully applied migration(s) [14, 15]. Database is now at schema version 15.
$ echo $?
0
```
Verifying version bump on disk:
```bash
$ sqlite3 tasks/atheneum-migrate-gate/scratch/strix-probe.db "SELECT COALESCE(MAX(version), 0) FROM atheneum_schema_version; PRAGMA user_version;"
15
15
```

### 5.3 Probe 3: Subsequent Read on Migrated DB Succeeds
```bash
$ ./target/debug/atheneum graph-stats tasks/atheneum-migrate-gate/scratch/strix-probe.db
{
  "edge_counts": [["performed_by", 14], ["caused_by", 1], ["led_to", 1]],
  "entity_counts": [["Memory", 54], ["Discovery", 14], ["Event", 14], ["Task", 9], ["Agent", 3]],
  "runtime": { ... },
  "total_edges": 16,
  "total_entities": 94
}
$ echo $?
0
```

### 5.4 Probe 4: Idempotent Rerun of `atheneum migrate`
```bash
$ ./target/debug/atheneum migrate tasks/atheneum-migrate-gate/scratch/strix-probe.db
Database is already up to date at schema version 15.
```

### 5.5 Probe 5: Global `--migrate` Flag On Command
Using a fresh v13 snapshot copy `strix-flag.db`:
```bash
$ ./target/debug/atheneum --migrate graph-stats tasks/atheneum-migrate-gate/scratch/strix-flag.db
{
  "edge_counts": [["performed_by", 14], ["caused_by", 1], ["led_to", 1]],
  "entity_counts": [["Memory", 54], ["Discovery", 14], ["Event", 14], ["Task", 9], ["Agent", 3]],
  "total_edges": 16,
  "total_entities": 94
}
$ sqlite3 tasks/atheneum-migrate-gate/scratch/strix-flag.db "PRAGMA user_version;"
15
```

### 5.6 Probe 6: Future Schema Rejection
Setting `PRAGMA user_version = 99;`:
```bash
$ ./target/debug/atheneum graph-stats tasks/atheneum-migrate-gate/scratch/strix-newer.db
Error: database schema version 99 is newer than supported version 15 (this binary only supports up to version 15)
$ ./target/debug/atheneum migrate tasks/atheneum-migrate-gate/scratch/strix-newer.db
Error: database schema version 99 is newer than supported version 15 (this binary only supports up to version 15)
```

### 5.7 Probe 7: Fresh DB Initialization via `init`
```bash
$ ./target/debug/atheneum init tasks/atheneum-migrate-gate/scratch/fresh.db
Initializing Atheneum graph at: tasks/atheneum-migrate-gate/scratch/fresh.db
Graph initialized successfully
   Health: OK
$ sqlite3 tasks/atheneum-migrate-gate/scratch/fresh.db "SELECT COALESCE(MAX(version), 0) FROM atheneum_schema_version; PRAGMA user_version;"
15
15
```

---

## 6. Verification Gates Passed

- `cargo fmt --all -- --check`: Clean (0 diffs, exit 0)
- `TMPDIR="$PWD/target/tmp" cargo clippy --all-targets -- -D warnings`: Clean (0 warnings, exit 0)
- `TMPDIR="$PWD/target/tmp" cargo test -p atheneum --lib`: 152 passed, 0 failed (exit 0)
- `TMPDIR="$PWD/target/tmp" cargo test -p atheneum --test migrate_gate_tests`: 4 passed, 0 failed (exit 0)
- Live production database `/home/feanor/.atheneum/atheneum.db`: Untouched.
- Temporary directory policy: Zero writes to `/tmp`.
- Installation policy: Zero installation to `~/.local/bin`.
- Git policy: Working tree unstaged, zero commits created.
