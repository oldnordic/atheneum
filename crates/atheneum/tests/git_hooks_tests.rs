use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate is nested under repo/crates/atheneum")
        .to_path_buf()
}

#[test]
fn git_hooks_enforce_completion_and_banned_pattern_gates() {
    let root = repo_root();
    let pre_commit = std::fs::read_to_string(root.join(".githooks/pre-commit"))
        .expect(".githooks/pre-commit should exist");
    let pre_push = std::fs::read_to_string(root.join(".githooks/pre-push"))
        .expect(".githooks/pre-push should exist");

    assert!(pre_commit.contains("conductor/(tracks\\.md|tracks/.*/(plan|spec)\\.md)$"));
    assert!(pre_commit.contains("COMPLETED"));
    assert!(pre_commit.contains("PENDING_VERIFICATION"));
    assert!(pre_commit.contains("cargo check --workspace"));
    assert!(pre_commit.contains("cargo test --workspace"));

    assert!(pre_push.contains("scripts/banned_patterns_lint.py"));
    assert!(pre_push.contains("Banned-pattern check passed"));
}
