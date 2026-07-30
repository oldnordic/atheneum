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
            return Err(anyhow!(
                "`{label}` exited with status {}: stderr: {stderr}; stdout: {stdout}",
                output.status
            ));
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
}
