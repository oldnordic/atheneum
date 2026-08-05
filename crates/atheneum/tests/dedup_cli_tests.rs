//! CLI regression test for `store-discovery --dedup` (ledger reconciliation
//! Phase 0). Probe-verified broken on 2026-08-05: an identical payload stored
//! 3x produced 3 discoveries. The fixed behavior: the second `--dedup` store
//! of an identical payload reports the existing id and the Discovery count
//! stays unchanged.

use std::path::Path;
use std::process::Command;

fn store_discovery(db: &Path, meta: &Path, extra: &[&str]) -> serde_json::Value {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_atheneum"));
    cmd.arg("store-discovery")
        .arg(db)
        .arg("test-agent")
        .arg("bug_found")
        .arg("http_handler")
        .arg(meta)
        .arg("--dedup")
        .args(extra);
    let output = cmd.output().expect("run store-discovery");
    assert!(
        output.status.success(),
        "store-discovery failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("store-discovery output is JSON")
}

fn discovery_count(db: &Path) -> u64 {
    let output = Command::new(env!("CARGO_BIN_EXE_atheneum"))
        .arg("graph-stats")
        .arg(db)
        .output()
        .expect("run graph-stats");
    assert!(
        output.status.success(),
        "graph-stats failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stats: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("graph-stats output is JSON");
    // entity_counts is a JSON array of [kind, count] pairs.
    stats["entity_counts"]
        .as_array()
        .map(|pairs| {
            pairs
                .iter()
                .filter(|p| p[0].as_str() == Some("Discovery"))
                .filter_map(|p| p[1].as_u64())
                .sum()
        })
        .unwrap_or(0)
}

#[test]
fn store_discovery_dedup_skips_identical_payload() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("test.db");
    let meta = dir.path().join("meta.json");
    std::fs::write(&meta, r#"{"detail": "connection pool leak"}"#).unwrap();

    let first = store_discovery(&db, &meta, &[]);
    assert_eq!(first["deduped"], serde_json::Value::Null);
    let first_id = first["discovery_id"]
        .as_i64()
        .expect("first store reports an id");
    assert_eq!(discovery_count(&db), 1);

    // Identical payload again with --dedup: reports the existing id, no new row.
    let second = store_discovery(&db, &meta, &[]);
    assert_eq!(second["deduped"], true);
    assert_eq!(second["discovery_id"].as_i64(), Some(first_id));
    assert_eq!(
        discovery_count(&db),
        1,
        "identical --dedup store must not create a second discovery"
    );

    // A third time, to reproduce the original 3x probe exactly.
    let third = store_discovery(&db, &meta, &[]);
    assert_eq!(third["deduped"], true);
    assert_eq!(third["discovery_id"].as_i64(), Some(first_id));
    assert_eq!(discovery_count(&db), 1);
}

#[test]
fn store_discovery_dedup_stores_different_payload() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("test.db");
    let meta_a = dir.path().join("a.json");
    let meta_b = dir.path().join("b.json");
    std::fs::write(&meta_a, r#"{"detail": "leak"}"#).unwrap();
    std::fs::write(&meta_b, r#"{"detail": "race"}"#).unwrap();

    store_discovery(&db, &meta_a, &[]);
    let other = store_discovery(&db, &meta_b, &[]);
    assert_eq!(other["deduped"], serde_json::Value::Null);
    assert_eq!(
        discovery_count(&db),
        2,
        "different payload must still store under --dedup"
    );
}
