use atheneum::{AtheneumGraph, GroundedClaim};

#[test]
fn test_grounded_claims_migration_and_lifecycle() {
    let graph = AtheneumGraph::open_in_memory().expect("open in memory graph");

    let claim = GroundedClaim {
        id: "claim_001".to_string(),
        entity_id: 42,
        project: "test_project".to_string(),
        file_path: "src/bus.rs".to_string(),
        symbol_name: Some("EventBus::dispatch".to_string()),
        ast_hash: "hash_abc_123".to_string(),
        receipt_hash: Some("receipt_xyz_456".to_string()),
        status: "verified".to_string(),
        created_at: "2026-08-29T22:00:00Z".to_string(),
        last_verified_at: "2026-08-29T22:00:00Z".to_string(),
    };

    graph.pin_grounded_claim(&claim).expect("insert claim");

    let entity_claims = graph.get_claims_for_entity(42).expect("get entity claims");
    assert_eq!(entity_claims.len(), 1);
    assert_eq!(entity_claims[0], claim);

    let project_claims = graph
        .list_claims(Some("test_project"))
        .expect("list claims");
    assert_eq!(project_claims.len(), 1);
    assert_eq!(project_claims[0].status, "verified");

    // Audit report when verified
    let report = graph.audit_claims("test_project").expect("audit claims");
    assert_eq!(report.total_claims, 1);
    assert_eq!(report.verified_claims, 1);
    assert_eq!(report.stale_claims, 0);
    assert!(report.stale_entity_ids.is_empty());

    // Flip to stale when code diverges
    graph
        .update_claim_status("claim_001", "stale")
        .expect("update status to stale");
    let stale_ids = graph
        .list_stale_entity_ids("test_project")
        .expect("list stale");
    assert_eq!(stale_ids, vec![42]);

    let stale_report = graph
        .audit_claims("test_project")
        .expect("audit stale claims");
    assert_eq!(stale_report.stale_claims, 1);
    assert_eq!(stale_report.stale_entity_ids, vec![42]);
}

#[test]
fn test_claim_verification_detects_modified_code() {
    use std::fs;
    use tempfile::tempdir;

    let dir = tempdir().expect("tempdir");
    let src_dir = dir.path().join("src");
    fs::create_dir_all(&src_dir).expect("create src dir");
    let file_path = src_dir.join("bus.rs");
    fs::write(&file_path, "fn dispatch() { /* v1 */ }\n").expect("write file");

    let graph = AtheneumGraph::open_in_memory().expect("open graph");

    // Pin claim
    let initial_hash = atheneum::compute_file_sha256(&file_path).expect("compute hash");
    let claim = GroundedClaim {
        id: "claim_bus_01".to_string(),
        entity_id: 100,
        project: "my_project".to_string(),
        file_path: "src/bus.rs".to_string(),
        symbol_name: Some("dispatch".to_string()),
        ast_hash: initial_hash,
        receipt_hash: None,
        status: "verified".to_string(),
        created_at: "2026-08-29T22:00:00Z".to_string(),
        last_verified_at: "2026-08-29T22:00:00Z".to_string(),
    };
    graph.pin_grounded_claim(&claim).expect("pin claim");

    // Verify when clean
    let rep1 = graph
        .verify_project_claims(dir.path(), "my_project", true)
        .expect("verify clean");
    assert_eq!(rep1.verified_claims, 1);
    assert_eq!(rep1.stale_claims, 0);

    // Modify file on disk
    fs::write(&file_path, "fn dispatch() { /* v2 modified code */ }\n").expect("modify file");

    // Verify after modification
    let rep2 = graph
        .verify_project_claims(dir.path(), "my_project", true)
        .expect("verify modified");
    assert_eq!(rep2.verified_claims, 0);
    assert_eq!(rep2.stale_claims, 1);
    assert_eq!(rep2.stale_entity_ids, vec![100]);

    // Graph query reflects staleness
    let stale_ids = graph
        .list_stale_entity_ids("my_project")
        .expect("list stale");
    assert_eq!(stale_ids, vec![100]);
}

#[test]
fn test_stale_aware_session_digest_flags_outdated_memory() {
    let graph = AtheneumGraph::open_in_memory().expect("open graph");

    // Store a memory entity for project "my_project"
    let memory_id = graph
        .store_memory(
            "arch_event_bus",
            "EventBus persists before dispatch",
            "project",
            0.95,
            Some("my_project"),
            None,
        )
        .expect("store memory");

    // Pin a claim for this memory entity
    let claim = GroundedClaim {
        id: "claim_mem_01".to_string(),
        entity_id: memory_id,
        project: "my_project".to_string(),
        file_path: "src/bus.rs".to_string(),
        symbol_name: Some("dispatch".to_string()),
        ast_hash: "hash_v1".to_string(),
        receipt_hash: None,
        status: "stale".to_string(), // Flagged as stale
        created_at: "2026-08-29T22:00:00Z".to_string(),
        last_verified_at: "2026-08-29T22:00:00Z".to_string(),
    };
    graph.pin_grounded_claim(&claim).expect("pin claim");

    // Compose digest
    let digest = graph
        .compose_digest(Some("my_project"), 3, 500)
        .expect("compose digest");

    // The memory content must be prefixed with [STALE: CODE DIVERGED]
    assert!(
        digest.contains("[STALE: CODE DIVERGED] EventBus persists before dispatch"),
        "digest did not flag stale memory: {digest}"
    );
}

#[test]
fn test_verify_project_claims_prevents_absolute_path_escape() {
    use std::fs;
    use tempfile::tempdir;

    let dir = tempdir().expect("tempdir");
    let src_dir = dir.path().join("src");
    fs::create_dir_all(&src_dir).expect("create src dir");
    let file_path = src_dir.join("bus.rs");
    fs::write(&file_path, "fn dispatch() { /* v1 */ }\n").expect("write file");

    let graph = AtheneumGraph::open_in_memory().expect("open graph");

    let initial_hash = atheneum::compute_file_sha256(&file_path).expect("compute hash");
    // Even if pinned with an absolute file path like "/src/bus.rs", it must safely join to repo_root
    let claim = GroundedClaim {
        id: "claim_abs_01".to_string(),
        entity_id: 200,
        project: "abs_project".to_string(),
        file_path: "/src/bus.rs".to_string(),
        symbol_name: Some("dispatch".to_string()),
        ast_hash: initial_hash,
        receipt_hash: None,
        status: "verified".to_string(),
        created_at: "2026-08-29T22:00:00Z".to_string(),
        last_verified_at: "2026-08-29T22:00:00Z".to_string(),
    };
    graph.pin_grounded_claim(&claim).expect("pin claim");

    let report = graph
        .verify_project_claims(dir.path(), "abs_project", true)
        .expect("verify claims");
    assert_eq!(report.verified_claims, 1);
    assert_eq!(report.invalid_claims, 0);
}
