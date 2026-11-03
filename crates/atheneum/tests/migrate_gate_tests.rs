//! Migration Gate Tests
//!
//! TDD tests proving:
//! 1. (A) Reading an existing DB with pending migrations fails loudly with a clear error
//!    naming current schema version, pending versions, and exact command to migrate.
//!    Crucially, reading MUST NOT silently mutate or auto-migrate the database.
//! 2. (¬A) Explicitly migrating (via `atheneum migrate <db>` or `--migrate`) succeeds,
//!    updates the version to current (15), and subsequent reads succeed.
//! 3. (fresh) `atheneum init <new-path>` creates a new DB with the full current schema (15).
//! 4. (newer-schema) A DB with `user_version` (or `atheneum_schema_version`) bumped beyond
//!    what this binary supports fails loudly and refuses partial reads.

use std::path::Path;
use std::process::Command;

/// Construct a fixture DB initialized only up to schema version 14.
fn create_v14_fixture_db(path: &Path) {
    let cfg = sqlitegraph::SqliteConfig::new().with_pragma("busy_timeout", "5000");
    let inner = sqlitegraph::SqliteGraph::open_with_config(path, &cfg).expect("open sqlitegraph");
    drop(inner);
    let conn = rusqlite::Connection::open(path).expect("open rusqlite");
    atheneum::db::run_migrations_up_to(&conn, 14).expect("run migrations up to 14");
}

fn get_db_version(path: &Path) -> (u32, u32) {
    let conn = rusqlite::Connection::open(path).expect("open rusqlite");
    let pragma_version: u32 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap_or(0);
    let table_version: u32 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM atheneum_schema_version",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    (table_version, pragma_version)
}

#[test]
fn test_pending_migrations_gate_fails_loudly_on_read_and_succeeds_after_migrate() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("fixture_v14.db");
    create_v14_fixture_db(&db_path);

    let (v_table, v_pragma) = get_db_version(&db_path);
    assert_eq!(v_table, 14, "Initial DB should be at schema version 14");
    assert_eq!(v_pragma, 14, "Initial DB user_version should be 14");

    // (A) Read command MUST fail loudly naming current version, pending version(s), and migrate command
    let output = Command::new(env!("CARGO_BIN_EXE_atheneum"))
        .arg("graph-stats")
        .arg(&db_path)
        .output()
        .expect("run graph-stats");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("{}\n{}", stdout, stderr);

    assert!(
        !output.status.success(),
        "Read command on a DB with pending migrations MUST fail! Output was: {}",
        combined
    );
    assert!(
        combined.contains("14"),
        "Error message must name current schema version (14). Got: {}",
        combined
    );
    assert!(
        combined.contains("15"),
        "Error message must name pending migration (15). Got: {}",
        combined
    );
    assert!(
        combined.contains("atheneum migrate"),
        "Error message must name exact command `atheneum migrate`. Got: {}",
        combined
    );

    // Verify no silent mutation happened on read attempt!
    let (v_table_after_fail, _) = get_db_version(&db_path);
    assert_eq!(
        v_table_after_fail, 14,
        "Database schema MUST NOT have been silently mutated on failed read!"
    );

    // (¬A) Dedicated `atheneum migrate <db>` subcommand applies migrations
    let migrate_output = Command::new(env!("CARGO_BIN_EXE_atheneum"))
        .arg("migrate")
        .arg(&db_path)
        .output()
        .expect("run atheneum migrate");

    let migrate_combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&migrate_output.stdout),
        String::from_utf8_lossy(&migrate_output.stderr)
    );
    assert!(
        migrate_output.status.success(),
        "atheneum migrate should succeed! Output: {}",
        migrate_combined
    );

    let (v_table_after_migrate, v_pragma_after_migrate) = get_db_version(&db_path);
    assert_eq!(
        v_table_after_migrate, 15,
        "Schema table version should now be 15"
    );
    assert_eq!(
        v_pragma_after_migrate, 15,
        "PRAGMA user_version should now be 15"
    );

    // Now the same read command succeeds
    let read_again = Command::new(env!("CARGO_BIN_EXE_atheneum"))
        .arg("graph-stats")
        .arg(&db_path)
        .output()
        .expect("run graph-stats again");

    assert!(
        read_again.status.success(),
        "Read command should succeed after migration! Error: {}",
        String::from_utf8_lossy(&read_again.stderr)
    );
}

#[test]
fn test_global_migrate_flag_allows_migration_during_command() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("fixture_v14_flag.db");
    create_v14_fixture_db(&db_path);

    // Run read command with --migrate
    let output = Command::new(env!("CARGO_BIN_EXE_atheneum"))
        .arg("--migrate")
        .arg("graph-stats")
        .arg(&db_path)
        .output()
        .expect("run graph-stats with --migrate");

    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success(),
        "Command with --migrate should succeed! Output: {}",
        combined
    );

    let (v_table, v_pragma) = get_db_version(&db_path);
    assert_eq!(v_table, 15, "DB should be migrated to 15");
    assert_eq!(v_pragma, 15, "PRAGMA user_version should be 15");
}

#[test]
fn test_fresh_init_creates_full_current_schema() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("new_db.db");

    let output = Command::new(env!("CARGO_BIN_EXE_atheneum"))
        .arg("init")
        .arg(&db_path)
        .output()
        .expect("run atheneum init");

    assert!(
        output.status.success(),
        "init should succeed on fresh path: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let (v_table, v_pragma) = get_db_version(&db_path);
    assert_eq!(v_table, 15, "Fresh DB should have all migrations up to 15");
    assert_eq!(v_pragma, 15, "Fresh DB user_version should be 15");

    let read_output = Command::new(env!("CARGO_BIN_EXE_atheneum"))
        .arg("graph-stats")
        .arg(&db_path)
        .output()
        .expect("read fresh db");

    assert!(
        read_output.status.success(),
        "read on freshly initialized DB should succeed"
    );
}

#[test]
fn test_newer_schema_fails_loudly() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("future_v99.db");
    create_v14_fixture_db(&db_path);

    // Bump user_version to 99 beyond known schema
    {
        let conn = rusqlite::Connection::open(&db_path).expect("open rusqlite");
        conn.execute_batch("PRAGMA user_version = 99;")
            .expect("bump pragma user_version");
    }

    let output = Command::new(env!("CARGO_BIN_EXE_atheneum"))
        .arg("graph-stats")
        .arg(&db_path)
        .output()
        .expect("run graph-stats on future db");

    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !output.status.success(),
        "Read on DB with future schema must fail loudly! Output: {}",
        combined
    );
    assert!(
        combined.contains("newer than"),
        "Error should state that schema is newer than supported. Got: {}",
        combined
    );
    assert!(
        combined.contains("99"),
        "Error should reference version 99. Got: {}",
        combined
    );
}
