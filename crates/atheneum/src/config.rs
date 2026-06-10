//! Configuration management for Atheneum.
//!
//! Loads settings from `~/.config/atheneum/config.toml` (or
//! `$XDG_CONFIG_HOME/atheneum/config.toml`). Missing file is not an error —
//! defaults are returned. Invalid file fails fast with a parse error.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// LLM provider type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum LlmProvider {
    #[default]
    Ollama,
    OpenAi,
    Anthropic,
    Custom,
}

/// LLM configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    pub provider: LlmProvider,
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    pub api_key: String,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: LlmProvider::Ollama,
            base_url: "http://localhost:11434".to_string(),
            model: "codellama".to_string(),
            api_key: String::new(),
        }
    }
}

/// Embedding provider type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum EmbedProvider {
    #[default]
    Hash,
    Ollama,
    OpenAi,
}

/// Embedding configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingsConfig {
    pub provider: EmbedProvider,
    #[serde(default = "default_embed_dimension")]
    pub dimension: usize,
    #[serde(default = "default_embeddings_base_url")]
    pub base_url: String,
    #[serde(default = "default_embeddings_model")]
    pub model: String,
    #[serde(default)]
    pub api_key: String,
}

fn default_embed_dimension() -> usize {
    128
}

fn default_embeddings_base_url() -> String {
    "http://localhost:11434".to_string()
}

fn default_embeddings_model() -> String {
    "nomic-embed-text".to_string()
}

impl Default for EmbeddingsConfig {
    fn default() -> Self {
        Self {
            provider: EmbedProvider::Hash,
            dimension: default_embed_dimension(),
            base_url: default_embeddings_base_url(),
            model: default_embeddings_model(),
            api_key: String::new(),
        }
    }
}

/// Paths owned by this atheneum instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtheneumPaths {
    #[serde(default = "default_db_path")]
    pub db: String,
    #[serde(default = "default_meta_db_path")]
    pub meta_db: String,
}

fn default_db_path() -> String {
    data_dir()
        .join("atheneum.db")
        .to_string_lossy()
        .into_owned()
}

fn default_meta_db_path() -> String {
    data_dir().join("meta.db").to_string_lossy().into_owned()
}

impl Default for AtheneumPaths {
    fn default() -> Self {
        Self {
            db: default_db_path(),
            meta_db: default_meta_db_path(),
        }
    }
}

/// Integration opt-in for another tool.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IntegrationConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Tool-specific path or URL. For magellan this is its config path;
    /// for envoy this is its HTTP URL.
    #[serde(default)]
    pub config: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

/// Cross-tool integrations section.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IntegrationsConfig {
    #[serde(default)]
    pub magellan: IntegrationConfig,
    #[serde(default)]
    pub envoy: IntegrationConfig,
}

/// Root Atheneum configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub atheneum: AtheneumPaths,
    #[serde(default)]
    pub llm: LlmConfig,
    #[serde(default)]
    pub embeddings: EmbeddingsConfig,
    #[serde(default)]
    pub integrations: IntegrationsConfig,
}

impl Config {
    /// Default atheneum.db path from config, with `~` expanded.
    pub fn db_path(&self) -> PathBuf {
        expand_tilde(&self.atheneum.db)
    }

    /// Default meta.db path from config, with `~` expanded.
    pub fn meta_db_path(&self) -> PathBuf {
        expand_tilde(&self.atheneum.meta_db)
    }
}

/// Default config directory: `~/.config/atheneum` (XDG compliant).
pub fn default_config_dir() -> PathBuf {
    config_dir().join("atheneum")
}

/// Default config file path: `~/.config/atheneum/config.toml`.
pub fn default_config_path() -> PathBuf {
    default_config_dir().join("config.toml")
}

/// Load configuration from the default location.
pub fn load() -> Result<Config> {
    load_from(&default_config_path())
}

/// Load configuration from an explicit path.
pub fn load_from(path: &Path) -> Result<Config> {
    if !path.exists() {
        return Ok(Config::default());
    }
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config from {}", path.display()))?;
    toml::from_str(&content)
        .with_context(|| format!("Failed to parse config from {}", path.display()))
}

/// Save configuration to the default location.
pub fn save(config: &Config) -> Result<()> {
    save_to(config, &default_config_path())
}

/// Save configuration to an explicit path.
pub fn save_to(config: &Config, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create config directory {}", parent.display()))?;
    }
    let content =
        toml::to_string_pretty(config).with_context(|| "Failed to serialize config to TOML")?;
    std::fs::write(path, content)
        .with_context(|| format!("Failed to write config to {}", path.display()))
}

/// Expand leading `~` to `$HOME`. Falls back to the original string if
/// `$HOME` is not set.
pub fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

fn config_dir() -> PathBuf {
    std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|_| PathBuf::from(".config"))
}

fn data_dir() -> PathBuf {
    std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|_| PathBuf::from(".atheneum"))
        .join("atheneum")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let cfg = Config::default();
        assert!(cfg.atheneum.db.ends_with("atheneum.db"));
        assert!(cfg.atheneum.meta_db.ends_with("meta.db"));
        assert_eq!(cfg.embeddings.provider, EmbedProvider::Hash);
        assert_eq!(cfg.embeddings.dimension, 128);
        assert!(!cfg.integrations.magellan.enabled);
    }

    #[test]
    fn test_parse_full_config() {
        let toml_str = r#"
[atheneum]
db = "~/custom/atheneum.db"
meta_db = "~/custom/meta.db"

[llm]
provider = "openai"
base_url = "https://api.openai.com/v1"
model = "gpt-4"
api_key = "sk-test"

[embeddings]
provider = "ollama"
dimension = 768

[integrations]
magellan = { enabled = true, config = "~/.config/magellan/config.toml" }
envoy = { enabled = true, url = "http://localhost:9876" }
"#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.atheneum.db, "~/custom/atheneum.db");
        assert_eq!(cfg.llm.provider, LlmProvider::OpenAi);
        assert_eq!(cfg.embeddings.provider, EmbedProvider::Ollama);
        assert_eq!(cfg.embeddings.dimension, 768);
        assert!(cfg.integrations.magellan.enabled);
        assert_eq!(
            cfg.integrations.magellan.config.as_deref(),
            Some("~/.config/magellan/config.toml")
        );
        assert_eq!(
            cfg.integrations.envoy.url.as_deref(),
            Some("http://localhost:9876")
        );
    }

    #[test]
    fn test_expand_tilde() {
        std::env::set_var("HOME", "/home/testuser");
        assert_eq!(expand_tilde("~/foo"), PathBuf::from("/home/testuser/foo"));
        assert_eq!(
            expand_tilde("/absolute/path"),
            PathBuf::from("/absolute/path")
        );
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let cfg = Config::default();
        save_to(&cfg, tmp.path()).unwrap();
        let loaded = load_from(tmp.path()).unwrap();
        assert_eq!(loaded.atheneum.db, cfg.atheneum.db);
        assert_eq!(loaded.atheneum.meta_db, cfg.atheneum.meta_db);
    }

    #[test]
    fn test_load_missing_returns_defaults() {
        let loaded = load_from(Path::new("/nonexistent/path/config.toml")).unwrap();
        assert_eq!(loaded.embeddings.provider, EmbedProvider::Hash);
    }
}
