//! Atheneum: Agent Coordination Graph Database
//!
//! A graph-based memory system for tracking multi-agent workflows,
//! decisions, and their provenance.

pub(crate) mod db;
pub mod graph;

pub use graph::{
    AtheneumGraph, ClaudeTranscriptImportParams, ClaudeTranscriptImportSummary,
    DisambiguationResult, DiscoveryPreview, DreamConfig, DreamFinding, DreamMode, DreamPhase,
    DreamReport, EdgeType, EntityType, HandoffPreview, JournalSection, KanbanStatus, KanbanUpdate,
    MemoryPreview, NavigateQueryPlan, ProvenanceData, RuntimeStats, SearchResult, WikiPage,
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
