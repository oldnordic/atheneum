//! Atheneum: Agent Coordination Graph Database
//!
//! A graph-based memory system for tracking multi-agent workflows,
//! decisions, and their provenance.

pub mod config;
mod cross;
pub(crate) mod db;
pub mod graph;
pub mod meta;

#[cfg(feature = "web-ui")]
pub mod web_ui;

pub use meta::{MetaRouter, ProjectInfo};

pub use cross::{CrossEdge, CrossRouter, CrossSearchResult, CrossSubgraph};

pub use config::{
    default_config_path, expand_tilde, load as load_config, load_from as load_config_from,
    save as save_config, save_to as save_config_to, Config, EmbedProvider, EmbeddingsConfig,
    IntegrationConfig, IntegrationsConfig, LlmConfig, LlmProvider, SwapGuardMode,
};

pub use graph::{
    AtheneumGraph, BrokenLinkMode, ClaudeTranscriptImportParams, ClaudeTranscriptImportSummary,
    ConsolidationConfig, ConsolidationReport, DisambiguationResult, DiscoveryPreview, DreamConfig,
    DreamFinding, DreamMode, DreamPhase, DreamReport, EdgeType, EntityType, HandoffPreview,
    JournalSection, KanbanStatus, KanbanUpdate, LintConfig, LintReport, MaintainConfig,
    MaintainReport, MemoryPatch, MemoryPreview, ModelInfo, NavigateQueryPlan, ProvenanceData,
    QueryIntent, ResolvedEntity, RuntimeStats, SearchResult, SeedMemory, UpsertAction,
    UpsertResult, WikiPage, WikiSearchResult,
};

// Re-export the sqlitegraph types so consumers can name them as
// `atheneum::GraphEntity` and pick up exactly the sqlitegraph version
// atheneum itself depends on. This avoids the dual-major-version trap
// when downstream crates also pull in an older sqlitegraph directly.
pub use sqlitegraph::{GraphEdge, GraphEntity};

// Re-export wiki/journal ingestion and query functions
pub use graph::wiki::{
    content_hash, extract_kanban_updates, extract_wikilinks, parse_journal_sections,
};
