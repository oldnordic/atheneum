//! Subprocess dispatch for magellan/llmgrep/mirage CLI binaries.
//!
//! This is the `code_query` tool's adapter. It mirrors grounded-mcp's
//! subprocess resolution and JSON-output wrapping pattern without adding a
//! cross-repo dependency.

use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;
use tokio::process::Command;

const CODE_TOOL_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy)]
pub enum CodeTool {
    Magellan,
    Llmgrep,
    Mirage,
}

impl CodeTool {
    fn bin_name(self) -> &'static str {
        match self {
            CodeTool::Magellan => "magellan",
            CodeTool::Llmgrep => "llmgrep",
            CodeTool::Mirage => "mirage",
        }
    }

    /// Read-only subcommands reachable via `code_query`.
    ///
    /// magellan/llmgrep/mirage are meant to be read-only, derived-from-source
    /// tools behind this endpoint — `refresh` is the one sanctioned mutation
    /// path and has its own dedicated MCP tool. Lists verified against
    /// `magellan --help-full`, `llmgrep --help`, `mirage --help` (2026-07-31)
    /// and manually split into read-only vs. mutating:
    ///
    /// magellan excludes: watch, index, delete, backfill, refresh, migrate,
    /// migrate-backend, repair-edges, temporal-sweep, candidate-fact (submit/
    /// validate mutate; list/review-queue don't, but the subcommand string
    /// doesn't disambiguate the two here so the whole verb is excluded),
    /// registry/catalog (db-discovery side effects, unclear/unneeded).
    /// llmgrep excludes: vector-create (writes a new embedding index),
    /// evolve (mutates labels/scores unless `--dry-run`, which this
    /// allowlist can't verify since it only sees the subcommand name).
    /// mirage excludes: migrate.
    ///
    /// ponytail: this only gates the subcommand name, not `args` — e.g.
    /// `magellan doctor --fix` still mutates if a caller passes `--fix` in
    /// `args`. `context`'s `build` sub-verb has similar ambiguity. Filtering
    /// `args` too is out of scope for this pass; upgrade if a real caller
    /// abuses it.
    pub fn allowed_subcommands(self) -> &'static [&'static str] {
        match self {
            CodeTool::Magellan => &[
                "status",
                "features",
                "query",
                "search",
                "find",
                "refs",
                "get",
                "get-file",
                "chunks",
                "chunk-by-span",
                "chunk-by-symbol",
                "files",
                "label",
                "collisions",
                "ast",
                "find-ast",
                "reachable",
                "dead-code",
                "cycles",
                "condense",
                "paths",
                "slice",
                "source-inventory",
                "context",
                "hopgraph",
                "navigate",
                "orient",
                "temporal-status",
                "temporal-barcode",
                "as-of",
                "doctor",
                "verify",
            ],
            CodeTool::Llmgrep => &[
                "search",
                "ast",
                "find-ast",
                "complete",
                "lookup",
                "explore",
                "navigate",
                "stats",
                "vector-search",
                "export-symbols",
            ],
            CodeTool::Mirage => &[
                "status",
                "paths",
                "cfg",
                "dominators",
                "loops",
                "unreachable",
                "patterns",
                "frontiers",
                "verify",
                "blast-zone",
                "cycles",
                "slice",
                "hotspots",
                "hotpaths",
                "diff",
                "icfg",
                "coverage",
                "docs",
                "risk",
                "suggest",
                "stats",
            ],
        }
    }
}

pub struct CodeQueryRunner {
    bin_dir: PathBuf,
}

impl CodeQueryRunner {
    pub fn new() -> Self {
        let bin_dir = std::env::var("GROUNDED_BIN_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                PathBuf::from(
                    shellexpand::tilde("~/.local/bin/")
                        .to_string()
                        .trim_end_matches('/'),
                )
            });
        Self { bin_dir }
    }

    pub fn with_bin_dir(bin_dir: PathBuf) -> Self {
        Self { bin_dir }
    }

    fn resolve(&self, name: &str) -> String {
        let candidate = self.bin_dir.join(name);
        if candidate.is_file() {
            candidate.to_string_lossy().into_owned()
        } else {
            name.to_string()
        }
    }

    pub async fn run(&self, magellan_db: &str, tool: CodeTool, args: Vec<String>) -> Result<Value> {
        let mut cmd = Command::new(self.resolve(tool.bin_name()));
        cmd.env_clear();
        cmd.args(&args);
        let _ = magellan_db;
        self.run_command(cmd, tool.bin_name()).await
    }

    /// Checks whether a project's magellan index has pending changes.
    ///
    /// `magellan status --output json` (verified 2026-07-30 against a real
    /// db) returns only aggregate counts (`files`, `symbols`, `references`,
    /// `calls`, `code_chunks`, `coverage`) — there is no dirty/pending-file
    /// field to read. `magellan refresh --dry-run --output json` is the
    /// actual mechanism that answers "is this index stale": it diffs the
    /// indexed state against the git working tree and reports `updated`/
    /// `deleted`/`added` file arrays without mutating anything. Staleness is
    /// "any of those three arrays is non-empty".
    pub async fn is_code_index_stale(&self, magellan_db: &str) -> Result<bool> {
        let mut cmd = Command::new(self.resolve("magellan"));
        cmd.env_clear();
        cmd.args([
            "refresh",
            "--db",
            magellan_db,
            "--dry-run",
            "--output",
            "json",
        ]);
        let status = self.run_command(cmd, "magellan_refresh_dry_run").await?;
        let pending = |field: &str| status[field].as_array().is_some_and(|a| !a.is_empty());
        Ok(pending("updated") || pending("deleted") || pending("added"))
    }

    pub async fn run_command(&self, mut cmd: Command, label: &str) -> Result<Value> {
        let run = async {
            cmd.stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true)
                .output()
                .await
                .with_context(|| format!("failed to spawn `{label}`"))
        };
        let output = tokio::time::timeout(CODE_TOOL_TIMEOUT, run)
            .await
            .map_err(|_| anyhow!("`{label}` timed out after {CODE_TOOL_TIMEOUT:?}"))??;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            // Raw stderr/stdout is logged server-side only — never forwarded
            // into the caller-visible error message (envelope.errors[]).
            tracing::warn!(
                label,
                status = %output.status,
                %stderr,
                %stdout,
                "code_query subprocess exited non-zero"
            );
            return Err(anyhow!("`{label}` exited with status {}", output.status));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let trimmed = stdout.trim();
        match serde_json::from_str::<Value>(trimmed) {
            Ok(v) => Ok(v),
            Err(_) => Ok(serde_json::json!({ "tool": label, "output": trimmed })),
        }
    }
}

impl Default for CodeQueryRunner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes a fake `magellan` executable into a fresh temp dir that
    /// ignores its args and prints `json_stdout`, so `is_code_index_stale`
    /// can be tested without a real magellan binary or db.
    fn fake_magellan_bin_dir(json_stdout: &str) -> tempfile::TempDir {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let script_path = dir.path().join("magellan");
        let mut file = std::fs::File::create(&script_path).unwrap();
        writeln!(file, "#!/bin/sh").unwrap();
        writeln!(file, "cat <<'EOF'\n{json_stdout}\nEOF").unwrap();
        drop(file);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        dir
    }

    #[tokio::test]
    async fn is_code_index_stale_reports_false_for_freshly_indexed_fixture() {
        // Fixture mirrors real `magellan refresh --dry-run --output json`
        // output for a project with no pending changes (confirmed shape:
        // `{"updated":[],"deleted":[],"added":[],"unchanged":N,"dry_run":true}`,
        // captured against ~/.magellan/atheneum/atheneum-mcp.db on 2026-07-30).
        let dir = fake_magellan_bin_dir(
            r#"{"updated":[],"deleted":[],"added":[],"unchanged":4,"dry_run":true}"#,
        );
        let runner = CodeQueryRunner::with_bin_dir(dir.path().to_path_buf());
        let stale = runner
            .is_code_index_stale("/tmp/does-not-matter.db")
            .await
            .unwrap();
        assert!(!stale, "freshly indexed fixture must report not-stale");
    }

    #[tokio::test]
    async fn is_code_index_stale_reports_true_when_files_are_pending() {
        let dir = fake_magellan_bin_dir(
            r#"{"updated":["src/lib.rs"],"deleted":[],"added":[],"unchanged":3,"dry_run":true}"#,
        );
        let runner = CodeQueryRunner::with_bin_dir(dir.path().to_path_buf());
        let stale = runner
            .is_code_index_stale("/tmp/does-not-matter.db")
            .await
            .unwrap();
        assert!(stale, "pending `updated` entry must report stale");
    }

    #[tokio::test]
    async fn run_returns_backend_unavailable_error_shape_when_binary_missing() {
        let runner =
            CodeQueryRunner::with_bin_dir(std::path::PathBuf::from("/nonexistent-bin-dir"));
        let result = runner
            .run(
                "/tmp/does-not-matter.db",
                CodeTool::Magellan,
                vec![
                    "status".to_string(),
                    "--db".to_string(),
                    "/tmp/does-not-matter.db".to_string(),
                ],
            )
            .await;
        assert!(
            result.is_err(),
            "expected spawn failure for missing binary, got {result:?}"
        );
    }

    #[tokio::test]
    async fn run_parses_json_stdout() {
        let runner = CodeQueryRunner::with_bin_dir(std::path::PathBuf::from("/usr/bin"));
        let mut cmd = tokio::process::Command::new("/bin/echo");
        cmd.arg(r#"{"ok":true}"#);
        let value = runner.run_command(cmd, "test_echo").await.unwrap();
        assert_eq!(value["ok"], true);
    }

    #[tokio::test]
    async fn run_falls_back_to_wrapped_text_on_non_json_stdout() {
        let runner = CodeQueryRunner::with_bin_dir(std::path::PathBuf::from("/usr/bin"));
        let mut cmd = tokio::process::Command::new("/bin/echo");
        cmd.arg("plain text, not json");
        let value = runner.run_command(cmd, "test_echo").await.unwrap();
        assert_eq!(value["tool"], "test_echo");
        assert!(value["output"].as_str().unwrap().contains("plain text"));
    }

    #[tokio::test]
    async fn run_command_failure_message_excludes_raw_stderr_and_stdout() {
        // Global constraint: caller-visible errors must never contain raw
        // subprocess stderr/stdout text.
        let runner = CodeQueryRunner::with_bin_dir(std::path::PathBuf::from("/bin"));
        let mut cmd = tokio::process::Command::new("/bin/sh");
        cmd.arg("-c")
            .arg("echo SECRET_STDOUT_MARKER_ABC123; echo SECRET_STDERR_MARKER_XYZ789 >&2; exit 7");
        let err = runner
            .run_command(cmd, "test_sh")
            .await
            .expect_err("expected non-zero exit to produce an Err");
        let message = err.to_string();
        assert!(
            !message.contains("SECRET_STDOUT_MARKER_ABC123"),
            "raw stdout leaked into caller-visible error message: {message:?}"
        );
        assert!(
            !message.contains("SECRET_STDERR_MARKER_XYZ789"),
            "raw stderr leaked into caller-visible error message: {message:?}"
        );
        assert!(
            message.contains("test_sh") && message.contains("exit status: 7"),
            "expected message to still identify the failing command and status, got {message:?}"
        );
    }
}
